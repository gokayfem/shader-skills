// Lays the depth the software rasterizer wrote for a shadow cascade over the cascade's depth
// map: one triangle over the whole map, writing each pixel's depth where this drawing left
// one, under the same depth test as the hardware rasterizer's clusters. The cull's arguments
// drop the triangle when no cluster went to software, so an empty drawing costs nothing.

#include "vg/raster_common.wgsl"

@group(1) @binding(0) var<storage, read> shadow_depth: array<u32>;

#include "vg/depth_stamp.wgsl"

@vertex
fn vs_main(@builtin(vertex_index) vertex: u32) -> @builtin(position) vec4f {
    // Three corners covering the clip square.
    let x = f32(i32(vertex & 1u) * 4 - 1);
    let y = f32(i32(vertex >> 1u) * 4 - 1);
    return vec4f(x, y, 0.0, 1.0);
}

@fragment
fn fs_main(@builtin(position) position: vec4f) -> @builtin(frag_depth) f32 {
    let pixel = vec2u(position.xy);
    let word = shadow_depth[pixel.y * u32(view.viewport.x) + pixel.x];
    if stamp_of(word) != view.stamp {
        discard;
    }
    return stamped_depth(word);
}
