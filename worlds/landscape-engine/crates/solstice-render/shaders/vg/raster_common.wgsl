// Bindings shared by the software and hardware rasterizers.

#include "common/math.wgsl"
#include "common/scene.wgsl"
#include "common/skin.wgsl"
#include "common/skin_vertex.wgsl"
#include "common/sway_vertex.wgsl"
#include "vg/pass_view.wgsl"

@group(0) @binding(0) var<uniform> view: PassView;
@group(0) @binding(1) var<storage, read> instances: array<Instance>;
@group(0) @binding(2) var<storage, read> clusters: array<Cluster>;
// The page pool, as words: a page's vertices, four words each, and then its triangles. One
// binding rather than a view of each keeps the masked rasterizers within 8 storage buffers.
@group(0) @binding(3) var<storage, read> pool: array<u32>;
@group(0) @binding(4) var<storage, read> visible: array<VisibleCluster>;
@group(0) @binding(5) var<storage, read> state: CullStateRead;
@group(0) @binding(6) var bone_texture: texture_2d<f32>;
// The wind, for what sways in it: see common/sway_vertex.wgsl.
@group(0) @binding(7) var wind_field: texture_2d<f32>;

// Where a vertex goes in the world, from its place in the mesh after any pose. Only views
// through a camera see things sway: depth maps from the sun, the seabed's and the GI voxels
// keep them at rest, as they are kept from frame to frame.
fn place_vertex(instance: Instance, posed: vec3f) -> vec3f {
    let world = transform_point(instance, posed);
    if view.is_ortho != 0u {
        return world;
    }
    return swayed_position(instance, posed, world, view.camera_position);
}

// One binding holds vertices and triangles alike. See common/pool.wgsl.
fn vertex_word(word: u32) -> u32 {
    return pool[word];
}

fn triangle_word(word: u32) -> u32 {
    return pool[word];
}

// Visibility ID: visible-cluster index in the high bits, triangle in the low 7.
fn encode_id(visible_index: u32, triangle: u32) -> u32 {
    return (visible_index << 7u) | triangle;
}

fn local_vertex(packed_triangle: u32, corner: u32) -> u32 {
    return (packed_triangle >> (corner * 8u)) & 0xffu;
}
