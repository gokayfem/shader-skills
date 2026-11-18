// Per-frame constants for the main view. Twin of `FrameUniform` in frame.rs.

// A run of instances in a colour of their own (`Respray` in frame.rs): a truck that stands
// about, which nobody drives. None of them when `count` is 0.
struct Respray {
    paint: vec3f,
    first: u32,
    was: vec3f,
    count: u32,
}

struct Frame {
    view_proj: mat4x4f,           // Includes the temporal jitter.
    inv_view_proj: mat4x4f,       // Inverse of the jittered matrix.
    unjittered_view_proj: mat4x4f,
    prev_view_proj: mat4x4f,      // Previous frame, without jitter.
    view: mat4x4f,
    camera_position: vec3f,
    frame_index: u32,
    viewport: vec2f,
    inv_viewport: vec2f,
    jitter_ndc: vec2f,
    near: f32,
    debug_mode: u32,
    sun_direction: vec3f,         // Points towards the sun.
    sun_illuminance: f32,
    sun_color: vec3f,
    sky_intensity: f32,
    light_count: u32,
    time: f32,
    gi_intensity: f32,
    // Zero makes the final image use only the current frame, with no history.
    temporal_accumulation: u32,
    // Scales texture gradients, for upscaling and temporal filtering.
    mip_scale: f32,
    // Radiance of the night sky's own glow and of the brightest stars.
    night_sky: f32,
    stars: f32,
    // Radiance of the moon's sunlit highlands; 0 draws no moon.
    moon_radiance: f32,
    // The celestial sphere: its north pole, and the March equinox where right ascension starts.
    star_pole: vec3f,
    // The sun's illuminance at noon, which night is measured against.
    daylight_illuminance: f32,
    star_equinox: vec3f,
    // Seconds since the previous frame, which temporal filters scale their blend by.
    frame_seconds: f32,
    // Where the sun and moon are drawn: the true sun, whatever lights the scene, and the moon.
    sun_disc: vec3f,
    // The calm sea's level; far below everything when there is no sea.
    sea_level: f32,
    moon_disc: vec3f,
    _pad2: f32,
    // The lamps of whatever is driven, as one beam: where it shines from and how far it
    // reaches (no lamp when that is 0), the way it points and the cosine of half its spread,
    // and how bright it is.
    lamp_position: vec3f,
    lamp_range: f32,
    lamp_direction: vec3f,
    lamp_spread: f32,
    lamp_intensity: vec3f,
    _pad3: f32,
    // The coat of whatever is driven (`Coat` in frame.rs): its paint's colour, and its rust,
    // dust and mud. The rows that take a place in the world to the car's own space, metres
    // from the ground under its middle, x to its right, y up, z to its back; nothing wears
    // a coat when `coat_count` is 0.
    coat_x: vec4f,
    coat_y: vec4f,
    coat_z: vec4f,
    coat_paint: vec3f,
    coat_rust: f32,
    coat_soil: vec3f,
    coat_mud: f32,
    coat_was: vec3f,
    coat_dust: f32,
    coat_first: u32,
    coat_count: u32,
    coat_seed: u32,
    coat_wet: f32,
    // Where its front and rear axles are along z, half its track, and its wheels' radius.
    coat_axles: vec4f,
    // Trucks that stand about, each in its own colour.
    resprays: array<Respray, 8>,
    // The ruts pressed into the ground (ruts.rs, ground/ruts.wgsl): the middle of the square
    // of them that is held, in the world's x and z; half its width in metres, 0 when nothing
    // is held; and one over the width of ground the map of them wraps round in.
    rut_window: vec4f,
    // The colour of churned soil, and the width of a cell of that map, in metres.
    rut_soil: vec3f,
    rut_cell: f32,
}

const DEBUG_NONE: u32 = 0u;
const DEBUG_CLUSTERS: u32 = 1u;
const DEBUG_TRIANGLES: u32 = 2u;
const DEBUG_RASTER_MODE: u32 = 3u;
const DEBUG_INSTANCES: u32 = 4u;
const DEBUG_ALBEDO: u32 = 5u;
const DEBUG_NORMALS: u32 = 6u;
const DEBUG_DIFFUSE_GI: u32 = 7u;
const DEBUG_REFLECTIONS: u32 = 8u;
const DEBUG_VOXELS: u32 = 9u;
// 10 is DEBUG_OCEAN, in ocean/shade.wgsl.
const DEBUG_WETNESS: u32 = 11u;
const DEBUG_MOTION: u32 = 12u;
const DEBUG_PARTICLES: u32 = 13u;
const DEBUG_PARTICLE_LIGHT: u32 = 14u;
const DEBUG_WIND: u32 = 15u;

fn pixel_to_ndc(pixel: vec2f, inv_viewport: vec2f) -> vec2f {
    let uv = pixel * inv_viewport;
    return vec2f(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
}

// World position from a pixel centre and its reversed-Z depth.
fn reconstruct_world(frame: Frame, pixel: vec2f, depth: f32) -> vec3f {
    let ndc = pixel_to_ndc(pixel, frame.inv_viewport);
    let h = frame.inv_view_proj * vec4f(ndc, depth, 1.0);
    return h.xyz / h.w;
}
