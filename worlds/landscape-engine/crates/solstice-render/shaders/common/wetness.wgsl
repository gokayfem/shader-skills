// How water changes a surface's material (docs/wetness.md).
//
// Water wets a surface in two stages (Lagarde 2013, "Water drop 2: wet surfaces").
//   1. It fills the pores of a porous surface: sand, soil, rough stone. Water in the pores lowers
//      the contrast of refractive index between grains and what surrounds them, so light goes
//      deeper and more is absorbed: the surface darkens and its colour deepens (Lekner and Dorf
//      1988). Measured wet sands and soils reflect about half what they do dry (Twomey, Bohren
//      and Mergenthaler 1986). The surface turns a little smoother too.
//   2. Once the pores are full, or at once on a surface with none, such as paint or metal, a
//      film forms on top. It is smooth, so the surface turns glossy; on ground facing up it
//      fills the hollows, and the fine bumps drown under its flat top.
//
// The including shader declares `frame: Frame` and, for where the water has been,
// `waterline: WaterlineParams` and `waterline_map: texture_2d<f32>` (ocean/waterline.rs).

#include "ocean/waterline_params.wgsl"

// A soaked porous surface's albedo is its dry albedo raised to this power: between 1, dry, and
// 2, the limit Lekner and Dorf give for a highly porous one. Sand of albedo 0.35 comes out at
// about half, as measured; darker channels lose a larger share, so colours deepen.
const WET_DARKENING_POWER: f32 = 1.6;
// How much smoother a porous surface turns as its pores fill.
const WET_PORE_SMOOTHING: f32 = 0.7;
// Roughness of a film of water, as GGX roughness.
const WET_FILM_ROUGHNESS: f32 = 0.07;
// Wetness at which a porous surface's pores are full and a film starts to form.
const WET_PORES_FULL: f32 = 0.6;
// Over the last few metres of the water-line map's window, what it holds fades into the calm sea's
// level that lies beyond it, so the wet line has no edge where the map ends.
const WATERLINE_EDGE_FADE: f32 = 8.0;
// Above the highest the water has reached, water still creeps up through the pores: a few
// centimetres on rock, more through sand. How far, in metres, for a surface without pores and
// for the most porous.
const WET_FRINGE: f32 = 0.03;
const WET_FRINGE_POROUS: f32 = 0.2;

// How porous a material is, 0 to 1, as it has no such value of its own: rough dielectrics,
// such as sand and stone, are; smooth ones, such as paint or polished stone, are much less;
// metals are not at all.
fn porosity(roughness: f32, metallic: f32) -> f32 {
    return (1.0 - metallic) * smoothstep(0.3, 0.8, roughness);
}

// Over the spot `xz`: the highest the water has stood lately, and where it stands now, as heights
// in the world, from the water-line map. Beyond the map, the calm sea's level for both: it is too
// far off for a wet line to show.
fn water_heights(xz: vec2f) -> vec2f {
    let at = xz / waterline.grid.x - vec2f(0.5);
    let base = floor(at);
    let f = at - base;
    var corners = array<vec2f, 4>(
        vec2f(frame.sea_level),
        vec2f(frame.sea_level),
        vec2f(frame.sea_level),
        vec2f(frame.sea_level),
    );
    for (var i = 0u; i < 4u; i++) {
        let texel = base + vec2f(f32(i & 1u), f32(i >> 1u));
        if in_waterline_window(texel, waterline.window.xy) {
            corners[i] = textureLoad(waterline_map, waterline_index(texel), 0).rg;
        }
    }
    let held = mix(mix(corners[0], corners[1], f.x), mix(corners[2], corners[3], f.x), f.y);
    let size = waterline.grid.x * waterline.grid.y;
    let from_edge = min(xz - waterline.window.xy * waterline.grid.x, waterline.window.xy * waterline.grid.x + size - xz);
    let inside = clamp(min(from_edge.x, from_edge.y) / WATERLINE_EDGE_FADE, 0.0, 1.0);
    return mix(vec2f(frame.sea_level), held, inside);
}

// How wet the surface at height `y`, as porous as `porous`, is, under `heights` from
// `water_heights`: soaked wherever the water has reached lately, and above that over a fringe
// that porous surfaces wick higher.
fn wetness_at(y: f32, heights: vec2f, porous: f32) -> f32 {
    let fringe = mix(WET_FRINGE, WET_FRINGE_POROUS, porous);
    return 1.0 - smoothstep(0.0, fringe, y - heights.x);
}

// How far below the water the surface at height `y` lies now, 0 to 1, under `heights`. A film's
// gloss comes from its own surface against the air; under the sea there is none, and the sea's
// surface above gives the reflection instead. The pores still fill.
fn submerged_at(y: f32, heights: vec2f) -> f32 {
    return 1.0 - smoothstep(-0.1, 0.0, y - heights.y);
}

struct WetSurface {
    albedo: vec3f,
    roughness: f32,
    normal: vec3f,
    // How wet the material was taken to be, cavities included.
    wetness: f32,
}

// How wet the sea has made the surface at `world`, as rough as `roughness` and as metallic as
// `metallic`, with `cavity`: its wetness, and how far under the water it lies now.
fn sea_wetness(roughness: f32, metallic: f32, cavity: f32, world: vec3f) -> vec2f {
    // Without a sea, its level lies far below everything, and nothing is wet.
    if frame.sea_level <= -1.0e29 {
        return vec2f(0.0);
    }
    let porous = porosity(roughness, metallic);
    let heights = water_heights(world.xz);
    let wetness = wetness_at(world.y, heights, porous);
    if wetness <= 0.0 {
        return vec2f(0.0);
    }
    // Hollows fill first, so a cavity is a little wetter than the surface around it.
    let w = clamp(wetness + (1.0 - cavity) * 0.25 * wetness, 0.0, 1.0);
    return vec2f(w, submerged_at(world.y, heights));
}

// The surface with base colour `albedo`, `roughness` and `metallic`, shading normal `normal` and
// the geometry's own normal `surface_normal`, as wet as `w`, and as far under water as
// `submerged`: from the sea, the rain, or both.
fn wet_material(
    albedo: vec3f,
    roughness: f32,
    metallic: f32,
    normal: vec3f,
    surface_normal: vec3f,
    w: f32,
    submerged: f32,
) -> WetSurface {
    var out = WetSurface(albedo, roughness, normal, w);
    if w <= 0.0 {
        return out;
    }
    let porous = porosity(roughness, metallic);

    // Pores filling.
    let pores = clamp(w / WET_PORES_FULL, 0.0, 1.0) * porous;
    let soaked = pow(max(albedo, vec3f(0.0)), vec3f(WET_DARKENING_POWER));
    out.albedo = mix(albedo, soaked, pores);
    out.roughness = roughness * mix(1.0, WET_PORE_SMOOTHING, pores);

    // A film: on porous surfaces once the pores are full, on others as soon as they are wet.
    let film = mix(w, smoothstep(WET_PORES_FULL, 1.0, w), porous) * (1.0 - submerged);
    out.roughness = mix(out.roughness, WET_FILM_ROUGHNESS, film);
    // On ground facing up the film pools in the hollows and drowns the bumps; on steep faces it
    // runs off, thin, and follows them.
    let pooling = film * smoothstep(0.5, 0.9, surface_normal.y);
    out.normal = normalize(mix(normal, surface_normal, pooling));
    return out;
}

// The surface as wet as the sea has made it at `world`. Cavities hold water first.
fn wet_surface(
    albedo: vec3f,
    roughness: f32,
    metallic: f32,
    normal: vec3f,
    surface_normal: vec3f,
    cavity: f32,
    world: vec3f,
) -> WetSurface {
    let sea = sea_wetness(roughness, metallic, cavity, world);
    return wet_material(albedo, roughness, metallic, normal, surface_normal, sea.x, sea.y);
}
