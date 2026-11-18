// How much sunlight the clouds let through to the ground, over a square around the camera. The
// lighting passes multiply the sun by it, which casts the clouds' shadows as they drift past.

@group(0) @binding(0) var shape_noise: texture_3d<f32>;
@group(0) @binding(1) var detail_noise: texture_3d<f32>;
@group(0) @binding(2) var weather_map: texture_2d<f32>;
@group(0) @binding(3) var noise_sampler: sampler;
@group(0) @binding(4) var output: texture_storage_2d<rgba16float, write>;

@group(0) @binding(5) var<storage, read> coverage_cdf: array<f32, 256>;

#include "clouds/common.wgsl"

const SAMPLES: i32 = 16;

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3u) {
    let size = textureDimensions(output);
    if any(id.xy >= size) {
        return;
    }
    let ground = clouds.shadow_origin + (vec2f(id.xy) + 0.5) / vec2f(size) * clouds.shadow_extent;
    var light = 1.0;
    if clouds.enabled > 0.5 && clouds.coverage > 0.001 {
        // From the ground towards the sun, across the layer. A low sun crosses more of it,
        // up to a limit beyond which the shadow map is too small to matter.
        let to_sun = frame.sun_direction;
        let rise = max(to_sun.y, 0.08);
        let enter = clouds.base / rise;
        let exit = clouds.top / rise;
        let length = min(exit - enter, 12000.0);
        let step = length / f32(SAMPLES);
        var depth = 0.0;
        for (var i = 0; i < SAMPLES; i++) {
            let t = enter + (f32(i) + 0.5) * step;
            let height = to_sun.y * t;
            depth += cloud_density(ground + to_sun.xz * t, max(height, clouds.base + 1.0), true) * step;
        }
        depth *= EXTINCTION * clouds.density;
        // Light scattered on through the cloud still reaches the ground, so even thick cloud
        // leaves some of the sun's light, as the octaves of multiple scattering do.
        light = max(exp(-depth) * 0.7 + exp(-depth * 0.1) * 0.3, 0.04);
        // Rain falling from the cloud dims the light further.
        light *= 1.0 - 0.3 * clouds.rain;
    }
    textureStore(output, id.xy, vec4f(light, 0.0, 0.0, 0.0));
}
