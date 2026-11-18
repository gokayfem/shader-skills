// Still water: ponds, pools and basins, each a level sheet over a rectangle of the scene. Where a
// pond's surface is nearer than the opaque scene, shades it and puts it over the lit scene, and
// gives the water's own depth and motion to everything after. Elsewhere the lit scene passes
// through unchanged.
//
// No waves are simulated and nothing is drawn: each pixel's view ray meets each pond's plane, and
// the basin's own walls, standing above the water, hide the sheet's edge. The surface is as the
// sea's is shaded (ocean/shade.wgsl), ruffled by a few trains of ripples.

#include "common/environment.wgsl"
#include "common/brdf.wgsl"
#include "lighting/sun_shadow.wgsl"
#include "gi/screen_trace.wgsl"
#include "ocean/optics.wgsl"
#include "rain/rings.wgsl"
#include "common/wind.wgsl"

// Twin of `MAX_PONDS` in ocean/pond.rs.
const MAX_PONDS: u32 = 8u;

struct Pond {
    // The rectangle: least x and z, then greatest.
    bounds: vec4f,
    // The water's height in x.
    level: vec4f,
}

// Twin of `PondUniform` in ocean/pond.rs.
struct Ponds {
    count: vec4u,
    absorption: vec4f,
    scattering: vec4f,
    // The wind over the water, packed (common/wind.wgsl, `unpack_wind`).
    wind: array<vec4f, 2>,
    ponds: array<Pond, MAX_PONDS>,
}

// The opaque scene, lit, with its depth and motion.
@group(0) @binding(0) var lit: texture_2d<f32>;
@group(0) @binding(1) var opaque_depth: texture_2d<f32>;
@group(0) @binding(2) var opaque_motion: texture_2d<f32>;
@group(0) @binding(3) var<uniform> ponds: Ponds;
@group(0) @binding(4) var out_color: texture_storage_2d<rgba16float, write>;
@group(0) @binding(5) var out_depth: texture_storage_2d<r32float, write>;
@group(0) @binding(6) var out_motion: texture_storage_2d<rg32float, write>;
// The sun's shadows, as the lighting pass has them.
@group(0) @binding(7) var<uniform> shadows: Shadows;
@group(0) @binding(8) var shadow_maps: texture_depth_2d_array;
@group(0) @binding(9) var shadow_sampler: sampler_comparison;
// For reflections of the scene: the opaque scene's closest-depth pyramid, and its normals.
@group(0) @binding(10) var depth_pyramid: texture_2d<f32>;
@group(0) @binding(11) var normal_tex: texture_2d<f32>;

// Depth of water over which a pond fades into its basin, in metres.
const EDGE_FADE: f32 = 0.03;
// Still water's roughness, as GGX alpha, seen close: a mirror, but for what the ripples do.
const STILL_ROUGHNESS: f32 = 0.006;
// The most rays a pixel reflects along (`ponds.count.y` says how many), and how far out along
// the bell of facet tilts a ray may be drawn from, in standard deviations: the tail beyond
// brings little light and much noise.
const MOST_RAYS: u32 = 8u;
const TILT_LIMIT: f32 = 2.2;
// The steps of the R2 sequence, which spreads successive samples evenly over a square.
const R2: vec2f = vec2f(0.7548776662, 0.5698402910);
// Trains of ripples, as a light air leaves on sheltered water: each a direction, a wavelength in
// metres and a height in metres.
const RIPPLES: u32 = 5u;
const RIPPLE_DIRECTION: array<vec2f, 5> = array<vec2f, 5>(
    vec2f(0.966, 0.259),
    vec2f(0.743, -0.669),
    vec2f(0.208, 0.978),
    vec2f(-0.588, 0.809),
    vec2f(0.951, -0.309),
);
const RIPPLE_LENGTH: array<f32, 5> = array<f32, 5>(1.9, 1.1, 0.63, 0.37, 0.21);
const RIPPLE_HEIGHT: array<f32, 5> = array<f32, 5>(0.0022, 0.0013, 0.0007, 0.0004, 0.00022);

// How hard the wind ruffles still water, against a breeze of this speed, 10 m up, in m/s; the
// least ruffling still air leaves; and the most a gale raises on a pond.
const RUFFLE_BREEZE: f32 = 5.0;
const RUFFLE_STILL: f32 = 0.15;
const RUFFLE_MOST: f32 = 2.5;

// The ripples' slope at `at`, for a pixel `footprint` metres across, and the mean square of the
// slopes too fine to show in it, which roughen the surface instead. The wind raises them: the
// trains running with it most, those across it little, and harder where a gust is crossing, so
// gusts show on the water as dark, ruffled patches running downwind, as cat's paws do.
fn ripples(at: vec2f, footprint: f32) -> vec3f {
    let air = unpack_wind(ponds.wind[0], ponds.wind[1]);
    let wind = wind_velocity(air, at);
    let speed = length(wind);
    let toward = wind / max(speed, 1.0e-3);
    let strength = clamp(speed * speed / (RUFFLE_BREEZE * RUFFLE_BREEZE), RUFFLE_STILL, RUFFLE_MOST);
    var slope = vec2f(0.0);
    var hidden = 0.0;
    for (var i = 0u; i < RIPPLES; i++) {
        let k = TAU / RIPPLE_LENGTH[i];
        // Deep-water waves: the shorter, the slower.
        let phase = dot(RIPPLE_DIRECTION[i], at) * k - sqrt(9.81 * k) * frame.time;
        let along = max(dot(RIPPLE_DIRECTION[i], toward), 0.0);
        let steepness = RIPPLE_HEIGHT[i] * k * strength * (0.25 + 0.75 * along * along);
        // A ripple shows while a pixel covers under half of it.
        let shown = 1.0 - smoothstep(0.25, 0.5, footprint / RIPPLE_LENGTH[i]);
        slope += RIPPLE_DIRECTION[i] * (steepness * cos(phase) * shown);
        hidden += 0.5 * steepness * steepness * (1.0 - shown * shown);
    }
    return vec3f(slope, hidden);
}

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3u) {
    let size = vec2u(frame.viewport);
    if any(id.xy >= size) {
        return;
    }
    let pixel = vec2i(id.xy);
    let center = vec2f(id.xy) + vec2f(0.5);
    let behind = textureLoad(lit, pixel, 0);
    let scene_depth = textureLoad(opaque_depth, pixel, 0).r;

    // The nearest pond the view ray comes down onto. Seen from below, a pond is not drawn.
    let eye = frame.camera_position;
    let ray = normalize(reconstruct_world(frame, center, 0.5) - eye);
    var reach = 1.0e30;
    for (var i = 0u; i < min(ponds.count.x, MAX_PONDS); i++) {
        let pond = ponds.ponds[i];
        let height = eye.y - pond.level.x;
        if height <= 0.0 || ray.y >= -1.0e-5 {
            continue;
        }
        let t = height / -ray.y;
        let at = eye.xz + ray.xz * t;
        if all(at >= pond.bounds.xy) && all(at <= pond.bounds.zw) {
            reach = min(reach, t);
        }
    }
    var water = 0.0;
    var world = eye;
    if reach < 1.0e30 {
        world = eye + ray * reach;
        let clip = frame.view_proj * vec4f(world, 1.0);
        water = clip.z / max(clip.w, 1.0e-6);
    }
    // Reversed depth: nearer is larger. No water here, or something stands in front of it; or a
    // debug view of the opaque scene, which water would hide.
    if water <= scene_depth || frame.debug_mode != DEBUG_NONE {
        textureStore(out_color, pixel, behind);
        textureStore(out_depth, pixel, vec4f(scene_depth, 0.0, 0.0, 0.0));
        textureStore(out_motion, pixel, textureLoad(opaque_motion, pixel, 0));
        return;
    }

    let view = -ray;
    // How much water one pixel covers: wider at grazing angles.
    let next = reconstruct_world(frame, center + vec2f(1.0, 0.0), water);
    let footprint = length(next - world) / sqrt(max(abs(view.y), 0.02));
    let ruffle = ripples(world.xz, footprint);
    // Rain rings the water and roughens it (rain/rings.wgsl).
    let rained = rain_rings(world.xz, frame.time, clouds.rain, footprint);
    let normal = normalize(vec3f(-ruffle.x - rained.x, 1.0, -ruffle.y - rained.y));
    let roughness = sqrt(STILL_ROUGHNESS * STILL_ROUGHNESS + ruffle.z + rained.z);

    let n_dot_v = max(dot(normal, view), 1.0e-3);
    let fresnel = WATER_F0 + (1.0 - WATER_F0) * pow(1.0 - n_dot_v, 5.0);
    // A few reflected rays per pixel, each off a facet tilted by the surface's roughness. The
    // tilts follow a sequence that spreads them evenly, over this pixel's rays and on through
    // the frames temporal accumulation gathers, each pixel starting somewhere of its own: so
    // the blurred reflection of a ruffled pond settles smooth, where rays drawn afresh each
    // frame leave it speckled wherever a dark bank meets a bright sky.
    let start = vec2f(
        interleaved_gradient_noise(vec2f(id.xy), 0u),
        interleaved_gradient_noise(vec2f(id.yx) + vec2f(31.0, 17.0), 0u),
    );
    let rays = clamp(ponds.count.y, 1u, MOST_RAYS);
    let basis = orthonormal_basis(normal);
    var reflected = vec3f(0.0);
    for (var ray_index = 0u; ray_index < rays; ray_index++) {
        let sample = f32((frame.frame_index % 4096u) * rays + ray_index);
        let jitter = fract(start + sample * R2);
        let tilt = min(sqrt(-2.0 * log(max(1.0 - jitter.x, 1.0e-4))), TILT_LIMIT);
        let spread = tilt * roughness * 0.7071;
        let facet = normalize(normal + (basis[0] * cos(TAU * jitter.y) + basis[1] * sin(TAU * jitter.y)) * spread);
        var direction = reflect(-view, facet);
        if dot(direction, normal) <= 0.0 {
            direction = reflect(-view, normal);
        }
        reflected += reflection(world, normal, direction);
    }
    reflected /= f32(rays);

    // Sunlight at the water, and the shadows of what stands in its way. Shadows show on water
    // by dimming its highlight and the light it scatters from within; what it reflects is
    // untouched.
    let noise = interleaved_gradient_noise(vec2f(id.xy) + vec2f(5.3, 11.7), frame.frame_index);
    let shadow = sun_shadow(world, vec3f(0.0, 1.0, 0.0), reach, noise);
    let sun_light = sun_irradiance_at(world) * shadow;

    let sun = frame.sun_direction;
    let n_dot_l = dot(normal, sun);
    var highlight = vec3f(0.0);
    if n_dot_l > 0.0 {
        let h = normalize(view + sun);
        let specular = d_ggx(max(dot(normal, h), 0.0), roughness) * v_smith(n_dot_v, n_dot_l, roughness)
            * f_schlick(vec3f(WATER_F0), max(dot(view, h), 0.0));
        highlight = specular * n_dot_l * sun_light;
    }

    // Light scattered back out of the water's body: its share of what is not absorbed, of the
    // light from the sky and sun that gets in.
    let extinction = ponds.absorption.rgb + ponds.scattering.rgb;
    let albedo = ponds.scattering.rgb / max(extinction, vec3f(1.0e-4));
    let skylight = clear_sky_radiance(vec3f(0.0, 1.0, 0.0)) * PI;
    let sunlight = sun_light * max(sun.y, 0.0);
    let body = albedo * (skylight + sunlight) / PI * 0.5;

    // What is seen through the water. A pond is shallow and its bed slopes up to meet it, so
    // the sea's way of finding the bed along the bent ray (ocean/optics.wgsl) lands now on the
    // bed, now on the bank beyond, and the picture breaks into patches with hard edges. Here
    // the bed is the one straight behind, shifted only as far as the ripples tilt the surface:
    // it shimmers, and is whole. The light still crosses the water twice, down to the bed in
    // the sun's bent direction and back up to the eye along the bent view.
    var underwater = body;
    if scene_depth > 0.0 {
        let bed = reconstruct_world(frame, center, scene_depth);
        let deep = max(world.y - bed.y, 0.0);
        let bent = refract(-view, normal, 1.0 / 1.33);
        let sun_in = refract(-frame.sun_direction, vec3f(0.0, 1.0, 0.0), 1.0 / 1.33);
        let path = deep / max(-bent.y, 0.25);
        let down = deep / max(-sun_in.y, 0.2);
        // The ripples' tilt moves what is seen by a share of the depth, never past the bank.
        let wobble = (normal.xz / max(normal.y, 0.5)) * min(deep, 0.5) * 0.35;
        let moved = frame.view_proj * vec4f(bed + vec3f(wobble.x, 0.0, wobble.y), 1.0);
        var uv = center * frame.inv_viewport;
        if moved.w > 0.0 {
            let ndc = moved.xy / moved.w;
            let there = vec2f(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
            let last = vec2i(frame.viewport) - vec2i(1);
            let depth_there = textureLoad(opaque_depth, clamp(vec2i(there * frame.viewport), vec2i(0), last), 0).r;
            // Only where that spot too lies under the water.
            if depth_there > 0.0 && depth_there < water {
                uv = there;
            }
        }
        let seen = textureSampleLevel(lit, clamp_sampler, uv, 0.0).rgb;
        underwater = seen * exp(-extinction * (path + down)) + body * (vec3f(1.0) - exp(-extinction * path));
    }
    var color = mix(underwater, reflected, fresnel) + highlight;
    color = aerial_perspective(color, world, view, reach);
    // Where the water thins to nothing against the basin it fades out rather than ending in a
    // line.
    if scene_depth > 0.0 {
        let bed = reconstruct_world(frame, center, scene_depth);
        color = mix(behind.rgb, color, smoothstep(0.0, EDGE_FADE, max(world.y - bed.y, 0.0)));
    }

    let prev_clip = frame.prev_view_proj * vec4f(world, 1.0);
    let current_clip = frame.unjittered_view_proj * vec4f(world, 1.0);
    let motion = (prev_clip.xy / prev_clip.w - current_clip.xy / current_clip.w) * vec2f(0.5, -0.5);

    textureStore(out_color, pixel, vec4f(color, 1.0));
    textureStore(out_depth, pixel, vec4f(water, 0.0, 0.0, 0.0));
    textureStore(out_motion, pixel, vec4f(motion, 0.0, 0.0));
}
