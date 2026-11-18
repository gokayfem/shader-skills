// The software rasterizer's coverage hooks for opaque surfaces: every sample counts, and all of
// these compile away. vg/cover_mask.wgsl is the other kind.

fn software_list_first() -> u32 {
    return state.sw_base;
}

fn software_list_count() -> u32 {
    return state.sw_count - state.sw_base;
}

fn cover_vertex(vertex: u32, packed: PackedVertex, instance: u32, w: f32) {}

fn cover_triangle(v0: u32, v1: u32, v2: u32, a: vec3f, b: vec3f, c: vec3f, instance: u32) {}

fn covered(weight_a: f32, weight_b: f32, weight_c: f32, pixel: vec2f) -> bool {
    return true;
}
