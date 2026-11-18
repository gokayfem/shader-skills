// Draws a depth map's specks: the clusters under a texel across, which mark the same texel or
// two whichever of their triangles is drawn. Culling has worked out the texels each one's bounds
// touch and its depth (vg/cull.wgsl), so one thread draws each, where the software rasterizer
// gives a cluster a workgroup of a hundred and more. They go into the same buffer, under the
// same stamp, and vg/shadow_merge.wgsl lays both over the map.

#include "vg/pass_view.wgsl"

// As culling packs it: the first texel, 13 bits each way, then how many more texels it reaches
// each way, 2 bits each; and its reversed depth, in 24 bits.
struct Speck {
    place: u32,
    depth: u32,
}

@group(0) @binding(0) var<uniform> view: PassView;
@group(0) @binding(1) var<storage, read> specks: array<Speck>;
@group(0) @binding(2) var<storage, read> state: CullStateRead;
@group(1) @binding(0) var<storage, read_write> shadow_depth: array<atomic<u32>>;

#include "vg/depth_stamp.wgsl"

// Twin of SPECK_GROUP_SIZE in vg/cull_control.wgsl.
const GROUP_SIZE: u32 = 64u;

@compute @workgroup_size(GROUP_SIZE)
fn main(@builtin(global_invocation_id) id: vec3u, @builtin(num_workgroups) groups: vec3u) {
    let count = min(state.speck_count, arrayLength(&specks));
    let width = u32(view.viewport.x);
    // A dispatch can hold at most 65535 workgroups, so each thread takes every so-many-th speck.
    for (var item = id.x; item < count; item += groups.x * GROUP_SIZE) {
        let speck = specks[item];
        let first = vec2u(speck.place & 0x1fffu, (speck.place >> 13u) & 0x1fffu);
        let more = vec2u((speck.place >> 26u) & 3u, (speck.place >> 28u) & 3u);
        let word = (view.stamp << DEPTH_STAMP_SHIFT) | speck.depth;
        for (var y = 0u; y <= more.y; y++) {
            for (var x = 0u; x <= more.x; x++) {
                atomicMax(&shadow_depth[(first.y + y) * width + first.x + x], word);
            }
        }
    }
}
