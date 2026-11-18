// The software rasterizer's body, shared by the visibility and the depth-only entry points,
// which each define, before including this: `write_visibility` and the bindings it writes to;
// `depth_bias`, the offset added to a triangle's depth from its slope on screen; and which list
// to draw and what counts as covered (vg/cover_none.wgsl and vg/cover_mask.wgsl).
//
// Hardware rasterizers shade 2x2 quads and waste most of their work on triangles near
// pixel size. A compute shader has no such overhead, so small clusters come here. The
// culling pass guarantees they are small on screen, which keeps this loop simple: no
// clipping, and coordinates fit in 32-bit fixed point. A perspective view's clusters are in
// front of the near plane too; an orthographic view's are rejected by depth, per pixel.

// One workgroup per cluster, one thread per triangle, as Nanite does: the cluster's vertices
// are transformed once, in parallel, into workgroup memory, and every triangle is then
// rasterized at the same time. Matches MAX_CLUSTER_TRIANGLES.
const GROUP_SIZE: u32 = 128u;
// Sub-pixel precision, matching the 8 bits typical of hardware so edges shared with
// hardware-rasterized neighbours land on the same samples.
const SUBPIXEL: f32 = 256.0;
const SUBPIXEL_I: i32 = 256;
// A sanity bound on triangle extent. Anything larger would overflow the fixed-point
// edge functions; the cluster-size limit in culling keeps real triangles far below it.
const MAX_EXTENT_PX: f32 = 512.0;
// Matches MAX_CLUSTER_VERTICES.
const MAX_VERTICES: u32 = 256u;

var<workgroup> screen: array<vec3f, MAX_VERTICES>;
var<workgroup> cluster_count: u32;

@compute @workgroup_size(GROUP_SIZE)
fn main(
    @builtin(workgroup_id) group: vec3u,
    @builtin(num_workgroups) groups: vec3u,
    @builtin(local_invocation_index) lane: u32,
) {
    if lane == 0u {
        cluster_count = software_list_count();
    }
    let count = workgroupUniformLoad(&cluster_count);

    // A dispatch can hold at most 65535 workgroups, so each takes every so-many-th cluster.
    for (var item = group.x; item < count; item += groups.x) {
        let visible_index = software_list_first() + item;
        let entry = visible[visible_index];
        let cluster = clusters[entry.cluster];
        let instance = instances[entry.instance];

        let vertex_count = cluster_vertex_count(cluster);
        for (var v = lane; v < vertex_count; v += GROUP_SIZE) {
            let packed = load_vertex(cluster, v);
            let posed = posed_position(instance, cluster, v, decode_position(instance, packed));
            let world = place_vertex(instance, posed);
            let clip = view.raster_view_proj * vec4f(world, 1.0);
            cover_vertex(v, packed, entry.instance, clip.w);
            let ndc = clip.xyz / clip.w;
            screen[v] = vec3f(
                (ndc.x * 0.5 + 0.5) * view.viewport.x,
                (0.5 - ndc.y * 0.5) * view.viewport.y,
                ndc.z,
            );
        }
        workgroupBarrier();

        if lane < cluster_triangle_count(cluster) {
            let packed = load_triangle(cluster, lane);
            let corners = vec3u(local_vertex(packed, 0u), local_vertex(packed, 1u), local_vertex(packed, 2u));
            rasterize_triangle(
                screen[corners.x],
                screen[corners.y],
                screen[corners.z],
                encode_id(visible_index, lane),
                instance_two_sided(instance),
                corners,
                entry.instance,
            );
        }
        // The next cluster reuses the vertex memory.
        workgroupBarrier();
    }
}

fn edge_f(a: vec2f, b: vec2f, p: vec2f) -> f32 {
    return (b.x - a.x) * (p.y - a.y) - (b.y - a.y) * (p.x - a.x);
}

fn edge(a: vec2i, b: vec2i, p: vec2i) -> i32 {
    return (b.x - a.x) * (p.y - a.y) - (b.y - a.y) * (p.x - a.x);
}

// The top-left fill rule, in Y-down screen space for clockwise triangles: samples
// exactly on an edge belong to the triangle only if that edge is a top or left edge.
fn fill_bias(a: vec2i, b: vec2i) -> i32 {
    let d = b - a;
    let is_top = d.y == 0 && d.x > 0;
    let is_left = d.y < 0;
    return select(-1, 0, is_top || is_left);
}

fn rasterize_triangle(
    a_in: vec3f,
    b_given: vec3f,
    c_given: vec3f,
    id: u32,
    two_sided: bool,
    corners_given: vec3u,
    instance: u32,
) {
    var b_in = b_given;
    var c_in = c_given;
    var corners = corners_given;
    // A back face of a two-sided material is drawn as the front face it is from behind.
    if two_sided && edge_f(a_in.xy, b_in.xy, c_in.xy) > 0.0 {
        b_in = c_given;
        c_in = b_given;
        corners = corners_given.xzy;
    }
    let bounds_min = floor(min(min(a_in.xy, b_in.xy), c_in.xy));
    let bounds_max = ceil(max(max(a_in.xy, b_in.xy), c_in.xy));
    let extent = bounds_max - bounds_min;
    if extent.x > MAX_EXTENT_PX || extent.y > MAX_EXTENT_PX {
        return;
    }

    // Fixed point relative to the bounding box keeps products inside 32 bits.
    let origin = bounds_min;
    let a = vec2i(round((a_in.xy - origin) * SUBPIXEL));
    let b = vec2i(round((b_in.xy - origin) * SUBPIXEL));
    let c = vec2i(round((c_in.xy - origin) * SUBPIXEL));

    // Front faces are counter-clockwise with Y up, so clockwise on this Y-down grid.
    let area = edge(a, b, c);
    if area >= 0 {
        return; // Back-facing or degenerate.
    }

    // Swap two corners so that inside is positive for all edge functions.
    let p0 = a;
    let p1 = c;
    let p2 = b;
    let z0 = a_in.z;
    let z1 = c_in.z;
    let z2 = b_in.z;
    let inv_area = 1.0 / f32(-area);

    // Only visit pixel centres inside the rounded triangle's bounds. Using whole pixel
    // edges visits an extra row/column on both sides of tiny grass and leaf triangles,
    // often even when no sample can be covered. Work from the same fixed-point vertices
    // as the edge tests so rounding cannot discard a sample on a top/left edge.
    // The numerators stay nonnegative, including for triangles outside the viewport.
    let low = min(min(a, b), c);
    let high = max(max(a, b), c);
    let first = max(vec2i(origin) + (low + vec2i(127)) / SUBPIXEL_I, vec2i(0));
    let last = min(vec2i(origin) + (high + vec2i(128)) / SUBPIXEL_I - vec2i(1), vec2i(view.viewport) - 1);
    if first.x > last.x || first.y > last.y {
        return;
    }
    let bias = depth_bias(a_in, b_in, c_in);
    cover_triangle(corners.x, corners.y, corners.z, a_in, b_in, c_in, instance);

    let bias0 = fill_bias(p1, p2);
    let bias1 = fill_bias(p2, p0);
    let bias2 = fill_bias(p0, p1);

    // Evaluate at the first pixel centre, then step incrementally.
    let start = (first - vec2i(origin)) * SUBPIXEL_I + vec2i(SUBPIXEL_I / 2);
    var row0 = edge(p1, p2, start) + bias0;
    var row1 = edge(p2, p0, start) + bias1;
    var row2 = edge(p0, p1, start) + bias2;

    let step_x0 = -(p2.y - p1.y) * SUBPIXEL_I;
    let step_x1 = -(p0.y - p2.y) * SUBPIXEL_I;
    let step_x2 = -(p1.y - p0.y) * SUBPIXEL_I;
    let step_y0 = (p2.x - p1.x) * SUBPIXEL_I;
    let step_y1 = (p0.x - p2.x) * SUBPIXEL_I;
    let step_y2 = (p1.x - p0.x) * SUBPIXEL_I;

    let width = u32(view.viewport.x);
    for (var y = first.y; y <= last.y; y++) {
        var e0 = row0;
        var e1 = row1;
        var e2 = row2;
        for (var x = first.x; x <= last.x; x++) {
            if (e0 | e1 | e2) >= 0 {
                let w0 = f32(e0 - bias0) * inv_area;
                let w1 = f32(e1 - bias1) * inv_area;
                let w2 = f32(e2 - bias2) * inv_area;
                let depth = w0 * z0 + w1 * z1 + w2 * z2;
                // Tested before the bias, as the hardware clips before it biases.
                if depth > 0.0 && covered(w0, w2, w1, vec2f(f32(x), f32(y))) {
                    write_visibility(u32(y) * width + u32(x), depth + bias, id);
                }
            }
            e0 += step_x0;
            e1 += step_x1;
            e2 += step_x2;
        }
        row0 += step_y0;
        row1 += step_y1;
        row2 += step_y2;
    }
}
