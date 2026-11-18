// The sun's cascaded shadows, sampled as the lighting and water passes both need them.
//
// Needs common/environment.wgsl, and, declared by whatever includes this:
//   var<uniform> shadows: Shadows;
//   var shadow_maps: texture_depth_2d_array;
//   var shadow_sampler: sampler_comparison;

const CASCADE_COUNT: u32 = 4u;
// The layer after the cascades holds movable instances, a quarter of it for each cascade at
// half its resolution (shadow.rs).
const MOVING_LAYER: i32 = 4;

struct Shadows {
    view_proj: array<mat4x4f, 4>,
    // View distance at the far end of each cascade.
    splits: vec4f,
    // World size of one shadow texel per cascade.
    texel_size: vec4f,
    // 1 for each cascade whose movable instances were drawn this frame, else 0.
    moving: vec4f,
}

const DISK_SAMPLES: u32 = 12u;

// Twin of `moving_quarter` in shadow.rs: where a cascade's quarter starts, in UV.
fn moving_quarter(cascade: u32) -> vec2f {
    return vec2f(f32(cascade % 2u), f32(cascade / 2u)) * 0.5;
}

// How lit `world` is in one map: a whole layer, or a part of one starting at `origin` and
// `scale` of it across, with texels `texel` metres across.
fn sample_map(cascade: u32, layer: i32, origin: vec2f, scale: f32, texel: f32, world: vec3f, normal: vec3f, angle: f32) -> f32 {
    // Offset towards the surface normal, by more as the sun grazes the surface: a texel then
    // spans a long stretch of it, and the surface would otherwise shadow itself. This matters
    // most in the far cascades, whose texels are large.
    let n_dot_l = clamp(dot(normal, frame.sun_direction), 0.05, 1.0);
    let slope = min(sqrt(1.0 - n_dot_l * n_dot_l) / n_dot_l, 8.0);
    let offset_world = world + normal * texel * (1.2 + slope * 0.8);
    let clip = shadows.view_proj[cascade] * vec4f(offset_world, 1.0);
    let uv = vec2f(clip.x * 0.5 + 0.5, 0.5 - clip.y * 0.5);
    if any(uv < vec2f(0.0)) || any(uv > vec2f(1.0)) || clip.z > 1.0 {
        return 1.0;
    }

    // Penumbra sized in world units, so that it stays consistent across cascades. A rotated
    // Vogel disc turns the blocky steps of few samples into noise that TAA can resolve.
    let size = f32(textureDimensions(shadow_maps).x);
    let filter_radius = max(0.018 / texel, 1.0) * 1.5 / size;
    // Samples stay inside the map's own part of the layer, half a texel in from its edge.
    let inset = 0.5 / size;
    var sum = 0.0;
    for (var i = 0u; i < DISK_SAMPLES; i++) {
        let r = sqrt((f32(i) + 0.5) / f32(DISK_SAMPLES));
        let theta = f32(i) * 2.399963 + angle;
        let offset = vec2f(cos(theta), sin(theta)) * r * filter_radius;
        let at = clamp(origin + uv * scale + offset, origin + inset, origin + scale - inset);
        sum += textureSampleCompareLevel(shadow_maps, shadow_sampler, at, layer, clip.z);
    }
    return sum / f32(DISK_SAMPLES);
}

fn sample_cascade(cascade: u32, world: vec3f, normal: vec3f, angle: f32) -> f32 {
    let texel = shadows.texel_size[cascade];
    var lit = sample_map(cascade, i32(cascade), vec2f(0.0), 1.0, texel, world, normal, angle);
    // What moves casts its shadow from its own map, with texels twice as large.
    if shadows.moving[cascade] > 0.5 {
        lit *= sample_map(cascade, MOVING_LAYER, moving_quarter(cascade), 0.5, texel * 2.0, world, normal, angle);
    }
    return lit;
}

// Whether the sun reaches a point in the air, `distance` from the camera: one tap, unfiltered.
// Past the cascades the air is taken as lit.
fn air_tap(point: vec3f, distance: f32) -> f32 {
    for (var cascade = 0u; cascade < CASCADE_COUNT; cascade++) {
        if distance < shadows.splits[cascade] {
            let clip = shadows.view_proj[cascade] * vec4f(point, 1.0);
            let uv = vec2f(clip.x * 0.5 + 0.5, 0.5 - clip.y * 0.5);
            if any(uv < vec2f(0.0)) || any(uv > vec2f(1.0)) || clip.z > 1.0 {
                return 1.0;
            }
            return textureSampleCompareLevel(shadow_maps, shadow_sampler, uv, i32(cascade), clip.z);
        }
    }
    return 1.0;
}

const AIR_SAMPLES: u32 = 4u;

// How much of the air between the camera and a surface `distance` away the sun reaches. `view`
// points from the surface to the camera. The taps are jittered, for TAA to resolve.
fn air_lit(view: vec3f, distance: f32, noise: f32) -> f32 {
    var lit = 0.0;
    for (var i = 0u; i < AIR_SAMPLES; i++) {
        let along = distance * (f32(i) + noise) / f32(AIR_SAMPLES);
        lit += air_tap(frame.camera_position - view * along, along);
    }
    return lit / f32(AIR_SAMPLES);
}

fn sun_shadow(world: vec3f, normal: vec3f, distance: f32, noise: f32) -> f32 {
    var cascade = CASCADE_COUNT;
    for (var i = 0u; i < CASCADE_COUNT; i++) {
        if distance < shadows.splits[i] {
            cascade = i;
            break;
        }
    }
    if cascade >= CASCADE_COUNT {
        return 1.0;
    }
    // Dither across the end of a cascade, so that the change in resolution is invisible.
    let split = shadows.splits[cascade];
    let previous_split = select(0.0, shadows.splits[cascade - 1u], cascade > 0u);
    let blend_start = mix(previous_split, split, 0.85);
    if distance > blend_start && cascade + 1u < CASCADE_COUNT {
        if noise < (distance - blend_start) / (split - blend_start) {
            cascade += 1u;
        }
    }
    var shadow = sample_cascade(cascade, world, normal, noise * TAU);
    // Fade out towards the end of the last cascade, so that shadows have no visible edge.
    if cascade + 1u == CASCADE_COUNT {
        shadow = mix(shadow, 1.0, smoothstep(split * 0.8, split, distance));
    }
    return shadow;
}
