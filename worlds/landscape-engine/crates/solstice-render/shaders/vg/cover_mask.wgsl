// The software rasterizer's coverage hooks for cut-out surfaces. Each cluster's texture
// coordinates are kept with its transformed vertices, and a triangle's few pixels share one mip
// level, from how its texture coordinates change across the screen.

var<workgroup> corner_uv: array<vec2f, 256>;
var<workgroup> corner_inv_w: array<f32, 256>;

// The triangle this thread is drawing.
var<private> triangle_uv: array<vec2f, 3>;
var<private> triangle_inv_w: vec3f;
var<private> triangle_tier: u32;
var<private> triangle_slot: u32;
var<private> triangle_level: f32;
// For glass, the share of its pixels it is drawn in; below zero for anything else.
var<private> triangle_glass: f32;

fn software_list_first() -> u32 {
    return state.opaque_end + state.masked_sw_base;
}

fn software_list_count() -> u32 {
    return state.masked_sw_count - state.masked_sw_base;
}

fn cover_vertex(vertex: u32, packed: PackedVertex, instance: u32, w: f32) {
    let placed = instances[instance];
    let material = materials[instance_material(placed)];
    corner_uv[vertex] = decode_uv(placed, packed) * material.uv_scale;
    corner_inv_w[vertex] = 1.0 / w;
}

fn cover_triangle(v0: u32, v1: u32, v2: u32, a: vec3f, b: vec3f, c: vec3f, instance: u32) {
    let material = materials[instance_material(instances[instance])];
    triangle_uv = array<vec2f, 3>(corner_uv[v0], corner_uv[v1], corner_uv[v2]);
    triangle_inv_w = vec3f(corner_inv_w[v0], corner_inv_w[v1], corner_inv_w[v2]);
    triangle_tier = material.texture_tier;
    triangle_slot = material.texture_slot;
    triangle_glass = glass_share(material);

    let ab = b.xy - a.xy;
    let ac = c.xy - a.xy;
    let inverse = 1.0 / (ab.x * ac.y - ac.x * ab.y);
    let du = triangle_uv[1] - triangle_uv[0];
    let dv = triangle_uv[2] - triangle_uv[0];
    let per_x = (du * ac.y - dv * ab.y) * inverse;
    let per_y = (dv * ab.x - du * ac.x) * inverse;
    triangle_level = mask_level(triangle_tier, max(dot(per_x, per_x), dot(per_y, per_y)));
}

fn covered(weight_a: f32, weight_b: f32, weight_c: f32, pixel: vec2f) -> bool {
    if triangle_glass >= 0.0 {
        return glass_kept(triangle_glass, pixel, glass_turn(view.raster_view_proj));
    }
    let weighted = vec3f(weight_a, weight_b, weight_c) * triangle_inv_w;
    let weights = weighted / (weighted.x + weighted.y + weighted.z);
    let uv = triangle_uv[0] * weights.x + triangle_uv[1] * weights.y + triangle_uv[2] * weights.z;
    return mask_solid(triangle_tier, triangle_slot, uv, triangle_level);
}
