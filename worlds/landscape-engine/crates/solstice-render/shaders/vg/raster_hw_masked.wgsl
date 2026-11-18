// Hardware rasterization of cut-out clusters: the same vertices as vg/raster_hw.wgsl, with their
// texture coordinates, and a fragment stage that keeps only the solid part.

#include "vg/raster_hw.wgsl"
#include "vg/vis_write.wgsl"
#include "vg/mask.wgsl"

struct MaskedOutput {
    @builtin(position) position: vec4f,
    @location(0) @interpolate(flat) id: u32,
    @location(1) uv: vec2f,
    @location(2) @interpolate(flat) material: u32,
}

// Hardware's masked range grows down from the end of the list.
@vertex
fn vs_masked(@builtin(vertex_index) v: u32, @builtin(instance_index) i: u32) -> MaskedOutput {
    let visible_index = arrayLength(&visible) - 1u - (state.masked_hw_base + i);
    let corner = cluster_vertex(v, visible_index);
    var out: MaskedOutput;
    out.position = corner.position;
    out.id = corner.id;
    out.uv = vec2f(0.0);
    out.material = 0u;

    let entry = visible[visible_index];
    let cluster = clusters[entry.cluster];
    let triangle = v / 3u;
    if triangle < cluster_triangle_count(cluster) {
        let instance = instances[entry.instance];
        out.material = instance_material(instance);
        let packed = load_triangle(cluster, triangle);
        let vertex = load_vertex(cluster, local_vertex(packed, v % 3u));
        out.uv = decode_uv(instance, vertex) * materials[out.material].uv_scale;
    }
    return out;
}

fn solid_here(uv: vec2f, material_index: u32, pixel: vec2f) -> bool {
    let per_x = dpdx(uv);
    let per_y = dpdy(uv);
    let material = materials[material_index];
    let share = glass_share(material);
    if share >= 0.0 {
        return glass_kept(share, pixel, glass_turn(view.raster_view_proj));
    }
    let level = mask_level(material.texture_tier, max(dot(per_x, per_x), dot(per_y, per_y)));
    return mask_solid(material.texture_tier, material.texture_slot, uv, level);
}

@fragment
fn fs_masked(
    @builtin(position) position: vec4f,
    @location(0) @interpolate(flat) id: u32,
    @location(1) uv: vec2f,
    @location(2) @interpolate(flat) material: u32,
) {
    if !solid_here(uv, material, position.xy) {
        return;
    }
    let pixel = vec2u(position.xy);
    let index = pixel.y * u32(view.viewport.x) + pixel.x;
    write_visibility(index, position.z, id);
}

// For depth maps.
@fragment
fn fs_masked_depth(
    @builtin(position) position: vec4f,
    @location(1) uv: vec2f,
    @location(2) @interpolate(flat) material: u32,
) {
    if !solid_here(uv, material, position.xy) {
        discard;
    }
}
