// Hardware rasterization of clusters by vertex pulling.
//
// WebGPU has no mesh shaders, so a cluster is one instance of a draw with enough
// vertices for the largest cluster. Vertices past the cluster's real triangle count
// collapse to a degenerate point, which the rasterizer discards cheaply.

#include "vg/raster_common.wgsl"

struct VertexOutput {
    @builtin(position) position: vec4f,
    @location(0) @interpolate(flat) id: u32,
}

fn cluster_vertex(vertex_index: u32, visible_index: u32) -> VertexOutput {
    var out: VertexOutput;
    out.position = vec4f(0.0);
    out.id = 0u;

    let entry = visible[visible_index];
    let cluster = clusters[entry.cluster];
    let triangle = vertex_index / 3u;
    if triangle >= cluster_triangle_count(cluster) {
        return out;
    }

    let instance = instances[entry.instance];
    let packed_triangle = load_triangle(cluster, triangle);
    var clip: array<vec4f, 3>;
    for (var corner = 0u; corner < 3u; corner++) {
        let local = local_vertex(packed_triangle, corner);
        let vertex = load_vertex(cluster, local);
        let posed = posed_position(instance, cluster, local, decode_position(instance, vertex));
        let world = place_vertex(instance, posed);
        clip[corner] = view.raster_view_proj * vec4f(world, 1.0);
    }

    // The pipeline draws both faces, since one draw holds both kinds of material. Front faces
    // are counter-clockwise, which makes this determinant positive. Working in homogeneous
    // coordinates keeps it right for triangles that cross behind the camera (Olano and Greer 1997).
    let facing = determinant(mat3x3f(clip[0].xyw, clip[1].xyw, clip[2].xyw));
    if facing <= 0.0 && !instance_two_sided(instance) {
        return out;
    }

    out.position = clip[vertex_index % 3u];
    out.id = encode_id(visible_index, triangle);
    return out;
}

// The opaque hardware range grows down from where the opaque lists end.
fn opaque_hardware_entry(instance_index: u32) -> u32 {
    return state.opaque_end - 1u - (state.hw_base + instance_index);
}

@vertex
fn vs_visibility(@builtin(vertex_index) v: u32, @builtin(instance_index) i: u32) -> VertexOutput {
    return cluster_vertex(v, opaque_hardware_entry(i));
}

// Depth-only entry point, used by shadow maps.
@vertex
fn vs_depth(@builtin(vertex_index) v: u32, @builtin(instance_index) i: u32) -> @builtin(position) vec4f {
    return cluster_vertex(v, opaque_hardware_entry(i)).position;
}
