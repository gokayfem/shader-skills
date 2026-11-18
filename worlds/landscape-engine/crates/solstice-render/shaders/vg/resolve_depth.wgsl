// The G-buffer's depth, from the visibility buffer: what vg/resolve.wgsl writes along with the
// rest, done apart on a GPU that allows four storage textures per stage, too few for both.

#include "common/frame.wgsl"

@group(0) @binding(0) var<uniform> frame: Frame;
@group(0) @binding(1) var<storage, read> vis_depth: array<u32>;
@group(0) @binding(2) var out_depth: texture_storage_2d<r32float, write>;

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3u) {
    let size = vec2u(frame.viewport);
    if id.x >= size.x || id.y >= size.y {
        return;
    }
    let depth = bitcast<f32>(vis_depth[id.y * size.x + id.x]);
    textureStore(out_depth, id.xy, vec4f(depth, 0.0, 0.0, 0.0));
}
