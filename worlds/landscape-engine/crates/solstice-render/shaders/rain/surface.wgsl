// What rain does to a surface: how wet it is, and whether a puddle lies on it, with the rings
// the drops make in the puddle (docs/weather-effects.md). Resolve applies it with the sea's
// wetness, through the same wet material (common/wetness.wgsl).
//
// - Only where the sky reaches: the rain map says how high the first surface the rain meets
//   stands over each spot, so the stone under an arch stays dry. A wall faces the rain sideways:
//   whether it is sheltered is read a little out from it, so an overhang above keeps it dry.
// - Faces turned up wet fully; walls less, and most on the side the wind drives the rain at;
//   faces turned down not at all. Sheltered surfaces only grow damp, slowly, in a long rain.
// - On walls, water runs down in streaks.
// - Puddles gather in the hollows of level ground as the rain goes on, and shrink after it.
//   Within them the surface is a sheet of water: smooth, dark, its own bumps drowned, and
//   ringed where the drops fall.
//
// The including shader declares `material_sampler` and includes common/wetness.wgsl.

#include "rain/params.wgsl"
#include "common/noise.wgsl"

@group(3) @binding(0) var<uniform> rain: RainParams;
@group(3) @binding(1) var rain_map: texture_2d<f32>;
@group(3) @binding(2) var rain_ripples: texture_2d<f32>;

// How deep, in metres, the water in a hollow must stand to show as a puddle, and to cover it.
const PUDDLE_SHOWS: f32 = 0.002;
const PUDDLE_COVERS: f32 = 0.008;
// How far out from a steep face, in metres, its shelter is read.
const WALL_REACH: f32 = 0.35;
// A puddle's water: smooth, and a little darker than soaked ground beneath it.
const PUDDLE_ROUGHNESS: f32 = 0.012;
const PUDDLE_DARKENING: f32 = 0.85;
// The most rain wets a fully porous surface, as the wet material counts it: its pores full,
// and a sheen beginning (common/wetness.wgsl's WET_PORES_FULL is 0.6).
const RAIN_POROUS_WETNESS: f32 = 0.8;
// How much the rings tilt the water's surface.
const RIPPLE_STRENGTH: f32 = 1.0;

struct RainWet {
    // How wet the rain has made the surface, 0 to 1.
    wet: f32,
    // How much of it a puddle covers, 0 to 1.
    puddle: f32,
    // The slope of the puddle's water, from the rings, in x and z.
    ripple: vec2f,
}

// The rain map's texels about `xz`, between their centres.
fn rain_map_smooth(xz: vec2f) -> vec4f {
    let t = rain_map_texel(rain, xz) - vec2f(0.5);
    let last = vec2i(i32(RAIN_MAP_TEXELS) - 1);
    let base = vec2i(floor(t));
    let f = t - floor(t);
    let a = textureLoad(rain_map, clamp(base, vec2i(0), last), 0);
    let b = textureLoad(rain_map, clamp(base + vec2i(1, 0), vec2i(0), last), 0);
    let c = textureLoad(rain_map, clamp(base + vec2i(0, 1), vec2i(0), last), 0);
    let d = textureLoad(rain_map, clamp(base + vec2i(1, 1), vec2i(0), last), 0);
    return mix(mix(a, b, f.x), mix(c, d, f.x), f.y);
}

// How open to the rain the surface at `world`, facing `n`, is: 1 open, 0 sheltered.
fn rain_open(world: vec3f, n: vec3f) -> f32 {
    if !in_rain_map(rain, world.xz) {
        // Beyond the map, only what faces down is sheltered.
        return 1.0;
    }
    // Steep faces read their shelter a little out from them.
    let steep = 1.0 - smoothstep(0.4, 0.8, n.y);
    let at = world.xz + normalize(n.xz + vec2f(1.0e-5)) * WALL_REACH * steep;
    if !in_rain_map(rain, at) {
        return 1.0;
    }
    // Each texel's verdict, weighed between texels over a 4x4 tent, as soft shadows are:
    // looked up one texel at a time, the edge of a shelter shows as a row of squares.
    let t = rain_map_texel(rain, at) - vec2f(0.5);
    let base = vec2i(floor(t)) - vec2i(1);
    let f = t - floor(t);
    let last = vec2i(i32(RAIN_MAP_TEXELS) - 1);
    var open = 0.0;
    var weight = 0.0;
    for (var y = 0; y < 4; y++) {
        for (var x = 0; x < 4; x++) {
            let texel = textureLoad(rain_map, clamp(base + vec2i(x, y), vec2i(0), last), 0);
            // A tent two texels wide each way.
            let d = abs(vec2f(f32(x - 1), f32(y - 1)) - f);
            let w = max(0.0, 1.0 - d.x * 0.5) * max(0.0, 1.0 - d.y * 0.5);
            // Nothing drawn there is open sky.
            let verdict = select(smoothstep(-SHELTER_MARGIN, 0.0, world.y - texel.x), 1.0, texel.w < 0.5);
            open += verdict * w;
            weight += w;
        }
    }
    return open / max(weight, 1.0e-5);
}

fn rain_wetness(world: vec3f, n: vec3f) -> RainWet {
    var out = RainWet(0.0, 0.0, vec2f(0.0));
    if rain.rain.z <= 0.0 && rain.rain.w <= 0.0 {
        return out;
    }
    let open = rain_open(world, n);
    // Up wets fully; walls less, more on the side the wind drives the rain at; down not at all.
    let wind = rain.wind.xy;
    let windward = clamp(-dot(normalize(n.xz + vec2f(1.0e-5)), wind / max(length(wind), 1.0e-3)), 0.0, 1.0);
    let wall = 0.45 + 0.4 * windward * clamp(length(wind) / 8.0, 0.0, 1.0);
    let facing = mix(wall, 1.0, smoothstep(0.3, 0.8, n.y)) * smoothstep(-0.6, -0.1, n.y);
    var wet = rain.rain.z * facing * open;
    // Sheltered surfaces grow damp only in a long rain, from splashes and the wet air.
    wet = max(wet, 0.3 * rain.rain.z * rain.rain.z * smoothstep(-0.6, -0.1, n.y));
    // Water running down walls, in streaks that shift as they go.
    let steep = 1.0 - smoothstep(0.3, 0.7, n.y);
    if steep > 0.0 && wet > 0.0 {
        let along = dot(world.xz, vec2f(0.7071, 0.7071)) * 7.0;
        let streaks = gradient_noise(vec3f(along, world.y * 0.8 + rain.camera.w * 1.5, 0.0), 0x2545f491u).x;
        wet *= mix(1.0, clamp(0.75 + streaks * 0.9, 0.35, 1.0), steep);
    }
    out.wet = clamp(wet, 0.0, 1.0);

    // Puddles, on level ground open to the sky.
    if rain.rain.w > 0.0 && n.y > 0.85 && open > 0.5 && in_rain_map(rain, world.xz) {
        let map = rain_map_smooth(world.xz);
        // The water's edge wanders, as real puddles' do, rather than tracing the hollow.
        let wander = gradient_noise(vec3f(world.xz * 1.7, 0.0), 0x68e31da4u).x * 0.004
            + gradient_noise(vec3f(world.xz * 6.0, 1.0), 0xb5297a4du).x * 0.0012;
        let depth = map.y * rain.rain.w - PUDDLE_SHOWS * 0.5 + wander;
        out.puddle = smoothstep(PUDDLE_SHOWS, PUDDLE_COVERS, depth) * smoothstep(0.85, 0.95, n.y) * open;
        if out.puddle > 0.0 && rain.rain.y > 0.0 {
            let ripple = textureSampleLevel(rain_ripples, material_sampler, world.xz, 0.0);
            out.ripple = ripple.xy * RIPPLE_STRENGTH;
        }
    }
    return out;
}

// A wet surface with a puddle lying on it, `puddle` of it covered.
fn under_puddle(surface: WetSurface, surface_normal: vec3f, r: RainWet) -> WetSurface {
    var out = surface;
    if r.puddle <= 0.0 {
        return out;
    }
    let water_normal = normalize(vec3f(-r.ripple.x, 1.0, -r.ripple.y));
    out.albedo = mix(surface.albedo, surface.albedo * PUDDLE_DARKENING, r.puddle);
    out.roughness = mix(surface.roughness, PUDDLE_ROUGHNESS, r.puddle);
    out.normal = normalize(mix(surface.normal, water_normal, r.puddle));
    out.wetness = max(surface.wetness, r.puddle);
    return out;
}
