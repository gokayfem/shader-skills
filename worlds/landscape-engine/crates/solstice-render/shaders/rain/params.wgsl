// What every rain pass reads: the rain now, where the camera is, and the rain map, which says
// for each spot around the camera how high the first surface the rain meets stands, how deep a
// puddle can gather there, and how high an eave above it drips from (docs/weather-effects.md).
// Twin of `RainUniform` in rain/mod.rs.

struct RainParams {
    // The rain map: the world's (x, z) at its first texel's corner, metres per texel, and 1 once
    // it has been drawn.
    map: vec4f,
    // The camera, and seconds since the rain began.
    camera: vec4f,
    // The mean wind 10 m up, (east, south), and how far the air has carried everything.
    wind: vec4f,
    // Rain rate in mm/h, how dense the visible rain is (0..1), how wet open surfaces are (0..1),
    // and how full the puddles are (0..1).
    rain: vec4f,
    // The visible drops' typical diameter in metres, their fall speed in m/s, the exposure their
    // streaks are drawn over in seconds, and how many layers of drops each ring draws.
    drops: vec4f,
    // How strongly eaves drip (0..1), the wind's gustiness, seconds since the last frame, and 1
    // if anything sways.
    extra: vec4f,
    // The light volume: its first cell's corner in the world, and metres per cell.
    light: vec4f,
}

// Twin of `MAP_TEXELS` in rain/mod.rs.
const RAIN_MAP_TEXELS: u32 = 512u;
// Surfaces lower than this, in metres, under the first surface the rain meets are sheltered.
const SHELTER_MARGIN: f32 = 0.15;
// Heights in the rain map below this mean nothing was drawn there.
const NOTHING_BELOW: f32 = -1.0e5;

fn rain_map_texel(p: RainParams, xz: vec2f) -> vec2f {
    return (xz - p.map.xy) / p.map.z;
}

fn in_rain_map(p: RainParams, xz: vec2f) -> bool {
    let t = rain_map_texel(p, xz);
    return p.map.w > 0.5 && all(t >= vec2f(0.0)) && all(t < vec2f(f32(RAIN_MAP_TEXELS)));
}
