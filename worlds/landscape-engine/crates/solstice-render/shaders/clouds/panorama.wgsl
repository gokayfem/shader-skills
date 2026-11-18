// The clouds over the whole sky at low resolution, laid out like the sky-view table. Lighting
// passes add it to the clear sky, so skylight, reflections and haze all see the clouds: an
// overcast sky dims and greys the light, and a sunlit cumulus brightens it.

@group(0) @binding(0) var shape_noise: texture_3d<f32>;
@group(0) @binding(1) var detail_noise: texture_3d<f32>;
@group(0) @binding(2) var weather_map: texture_2d<f32>;
@group(0) @binding(3) var noise_sampler: sampler;
@group(0) @binding(4) var output: texture_storage_2d<rgba16float, write>;

@group(0) @binding(5) var<storage, read> coverage_cdf: array<f32, 256>;

#include "clouds/common.wgsl"

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3u) {
    let size = textureDimensions(output);
    if any(id.xy >= size) {
        return;
    }
    let uv = (vec2f(id.xy) + 0.5) / vec2f(size);
    let direction = sky_view_direction(uv);
    var result = vec4f(0.0, 0.0, 0.0, 1.0);
    if direction.y > -0.02 {
        let clouds_here = clouds_along(normalize(vec3f(direction.x, max(direction.y, 0.002), direction.z)), 32, 4, 0.5);
        result = vec4f(clouds_here.radiance, clouds_here.transmittance);
    }
    textureStore(output, id.xy, result);
}
