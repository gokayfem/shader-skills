// Works out the Milky Way once, into a map of the celestial sphere: glow in RGB, as the square
// root of its share of MILKY_WAY_RANGE, and how thick its faint stars are in A.

#include "common/math.wgsl"
#include "sky/milky_way.wgsl"

@group(0) @binding(0) var output: texture_storage_2d<rgba8unorm, write>;

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3u) {
    let size = textureDimensions(output);
    if any(id.xy >= size) {
        return;
    }
    let uv = (vec2f(id.xy) + vec2f(0.5)) / vec2f(size);
    let galaxy = milky_way(celestial_direction(uv));
    let glow = sqrt(clamp(galaxy.rgb / MILKY_WAY_RANGE, vec3f(0.0), vec3f(1.0)));
    textureStore(output, id.xy, vec4f(glow, clamp(galaxy.a, 0.0, 1.0)));
}
