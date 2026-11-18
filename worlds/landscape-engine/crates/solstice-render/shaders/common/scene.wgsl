// Scene data as laid out in GPU buffers. Twin of solstice-asset's `gpu` module and
// solstice-render's `scene` module; change them together.

struct Cluster {
    center: vec3f,
    radius: f32,
    lod_center: vec3f,
    lod_radius: f32,
    lod_error: f32,
    vertex_offset: u32,
    triangle_offset: u32,
    counts: u32, // vertex_count | triangle_count << 8 | CLUSTER_CHILDREN_MISSING
}

struct Node {
    center: vec3f,
    radius: f32,
    lod_center: vec3f,
    lod_radius: f32,
    max_parent_error: f32,
    first: u32,
    count: u32,
    is_leaf: u32, // 0 for an interior node. See the NODE_ constants in vg/cull.wgsl.
}

// x y | z n | u v, sixteen bits each: three words of the page pool. See common/pool.wgsl.
struct PackedVertex {
    a: u32,
    b: u32,
    c: u32,
}

struct Instance {
    // Affine transforms stored as the top three rows of the matrix.
    world_0: vec4f,
    world_1: vec4f,
    world_2: vec4f,
    prev_world_0: vec4f,
    prev_world_1: vec4f,
    prev_world_2: vec4f,
    normal_0: vec4f,
    normal_1: vec4f,
    normal_2: vec4f,
    quant_origin: vec3f,
    quant_step: f32,
    uv_origin: vec2f,
    uv_step: vec2f,
    root_node: u32,
    // Material index, with the INSTANCE_ flags in its top bits so passes need not look them up.
    material: u32,
    max_scale: f32,
    // Texture layer << 16 | (log2(uv scale) + 128) * 256, or NO_TEXTURE.
    texture_info: u32,
    // How brightly the material's own light shines here: 1 as it is, 0 dark.
    glow: f32,
    // Where this instance's bones start in the bone texture, or NO_BONES.
    bones: u32,
    // What to add to a cluster's number to find its bone list.
    bone_lists: i32,
    // The farthest its pose carries anything, in the mesh's units. Added to spheres it can't track
    // bone by bone: its nodes, and its LOD spheres, so that both sides of a LOD choice agree.
    lod_margin: f32,
}

struct Material {
    base_color: vec4f,
    emissive: vec3f,
    flags: u32, // MATERIAL_TWO_SIDED | MATERIAL_FOLIAGE | MATERIAL_MASKED
    roughness: f32,
    metallic: f32,
    uv_scale: f32,
    normal_strength: f32,
    texture_layer: i32,
    macro_variation: f32,
    // Which texture array holds the layer right now, and where in it.
    texture_tier: u32,
    texture_slot: u32,
}

struct Light {
    position: vec3f,
    range: f32,
    intensity: vec3f,
    _pad: u32,
}

// One selected cluster. The index of this entry is what the visibility buffer stores.
struct VisibleCluster {
    instance: u32,
    cluster: u32,
}

// Twins of Material::TWO_SIDED, Material::FOLIAGE and Material::MASKED.
const MATERIAL_TWO_SIDED: u32 = 1u;
const MATERIAL_FOLIAGE: u32 = 2u;
const MATERIAL_MASKED: u32 = 4u;
// Twin of Material::BLENDS: ground of three kinds, this material's and the next two's,
// blended by the shares the mesh's texture coordinates hold (land.rs).
const MATERIAL_BLENDS: u32 = 32u;
// Twins of Material::PAINT and Material::INSIDE: the paint of something driven, and what is
// in its cab or under its bonnet (vg/resolve.wgsl, `coat`).
const MATERIAL_PAINT: u32 = 64u;
const MATERIAL_INSIDE: u32 = 128u;
// Twin of Material::GLASS: seen through, by being drawn in a share of its pixels (vg/mask.wgsl).
const MATERIAL_GLASS: u32 = 256u;
// Twins of the INSTANCE_ flags in scene.rs.
const INSTANCE_TWO_SIDED: u32 = 0x80000000u;
const INSTANCE_MOVABLE: u32 = 0x40000000u;
const INSTANCE_HIDDEN: u32 = 0x20000000u;
const INSTANCE_STAYS_IN_GI: u32 = 0x10000000u;
const INSTANCE_MASKED: u32 = 0x08000000u;
const INSTANCE_SWAYS: u32 = 0x04000000u;
const INSTANCE_TREE: u32 = 0x02000000u;
const INSTANCE_FLUTTERS: u32 = 0x01000000u;
const INSTANCE_BLENDS: u32 = 0x00800000u;
const INSTANCE_MATERIAL_MASK: u32 = 0x007fffffu;

fn instance_material(i: Instance) -> u32 {
    return i.material & INSTANCE_MATERIAL_MASK;
}

fn instance_movable(i: Instance) -> bool {
    return (i.material & INSTANCE_MOVABLE) != 0u;
}

fn instance_hidden(i: Instance) -> bool {
    return (i.material & INSTANCE_HIDDEN) != 0u;
}

fn instance_stays_in_gi(i: Instance) -> bool {
    return (i.material & INSTANCE_STAYS_IN_GI) != 0u;
}

fn instance_masked(i: Instance) -> bool {
    return (i.material & INSTANCE_MASKED) != 0u;
}

fn instance_two_sided(i: Instance) -> bool {
    return (i.material & INSTANCE_TWO_SIDED) != 0u;
}

fn cluster_triangle_count(c: Cluster) -> u32 {
    return (c.counts >> 8u) & 0xffu;
}

fn cluster_vertex_count(c: Cluster) -> u32 {
    return c.counts & 0xffu;
}

fn transform_point(i: Instance, p: vec3f) -> vec3f {
    let h = vec4f(p, 1.0);
    return vec3f(dot(i.world_0, h), dot(i.world_1, h), dot(i.world_2, h));
}

fn transform_point_prev(i: Instance, p: vec3f) -> vec3f {
    let h = vec4f(p, 1.0);
    return vec3f(dot(i.prev_world_0, h), dot(i.prev_world_1, h), dot(i.prev_world_2, h));
}

fn transform_normal(i: Instance, n: vec3f) -> vec3f {
    let h = vec4f(n, 0.0);
    return normalize(vec3f(dot(i.normal_0, h), dot(i.normal_1, h), dot(i.normal_2, h)));
}

fn decode_position(i: Instance, v: PackedVertex) -> vec3f {
    let grid = vec3f(f32(v.a & 0xffffu), f32(v.a >> 16u), f32(v.b & 0xffffu));
    return i.quant_origin + grid * i.quant_step;
}

fn sign_extend_8(x: u32) -> f32 {
    return f32(i32(x << 24u) >> 24u);
}

// The normal is octahedral, a signed byte for each coordinate.
fn decode_normal(v: PackedVertex) -> vec3f {
    let e = vec2f(sign_extend_8(v.b >> 16u), sign_extend_8(v.b >> 24u)) / 127.0;
    return oct_decode(e);
}

fn decode_uv(i: Instance, v: PackedVertex) -> vec2f {
    return i.uv_origin + vec2f(f32(v.c & 0xffffu), f32(v.c >> 16u)) * i.uv_step;
}
