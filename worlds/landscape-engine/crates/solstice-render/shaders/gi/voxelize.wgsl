// Bakes the triangles of selected clusters into a clipmap level of the GI volume.
//
// This is the scene representation that lighting rays trace. Because culling chooses
// clusters by LOD error against the voxel size, coarse levels read only coarse triangles
// and voxelization cost stays nearly independent of source triangle counts.
//
// Each triangle is scanned over the two axes its normal is least aligned with, marking
// the depth range it covers per column. This leaves no holes. Each row visits only the columns
// between the triangle's edges, so work follows the area it really covers. Testing its whole
// bounding box instead is far slower for the long, thin triangles that flat surfaces simplify
// into, whose boxes can span the grid while they touch only a sliver of it.

#include "common/math.wgsl"
#include "common/scene.wgsl"
#include "common/pool.wgsl"
#include "vg/pass_view.wgsl"

struct Params {
    origin: vec3f,
    voxel_size: f32,
    resolution: u32,
    level_offset: u32, // First voxel of this level in `voxels`.
    _pad0: u32,
    _pad1: u32,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> instances: array<Instance>;
@group(0) @binding(2) var<storage, read> clusters: array<Cluster>;
@group(0) @binding(3) var<storage, read> vertices: array<u32>;
@group(0) @binding(4) var<storage, read> triangles: array<u32>;

// The page pool, bound twice. See common/pool.wgsl.
fn vertex_word(word: u32) -> u32 {
    return vertices[word];
}

fn triangle_word(word: u32) -> u32 {
    return triangles[word];
}

@group(0) @binding(5) var<storage, read> visible: array<VisibleCluster>;
@group(0) @binding(6) var<storage, read> state: CullStateRead;
@group(0) @binding(7) var<storage, read> materials: array<Material>;
// Three words per voxel: albedo with occupancy flag, octahedral normal, emissive RGBM.
@group(0) @binding(8) var<storage, read_write> voxels: array<atomic<u32>>;
@group(0) @binding(9) var color_textures: texture_2d_array<f32>;
@group(0) @binding(10) var material_sampler: sampler;

// Texture mip used for voxel colour: an area average, not any one texel.
const COLOR_MIP: f32 = 5.0;
const EMISSIVE_RANGE: f32 = 128.0;

struct VoxelData {
    albedo: u32,
    normal: u32,
    emissive: u32,
}

fn encode_emissive(e: vec3f) -> u32 {
    let peak = clamp(max(e.x, max(e.y, e.z)) / EMISSIVE_RANGE, 1.0e-6, 1.0);
    let scale = ceil(peak * 255.0) / 255.0;
    return pack4x8unorm(vec4f(e / (EMISSIVE_RANGE * scale), scale));
}

fn write_voxel(coord: vec3i, data: VoxelData) {
    let n = i32(params.resolution);
    if any(coord < vec3i(0)) || any(coord >= vec3i(n)) {
        return;
    }
    let index = u32(coord.z * n * n + coord.y * n + coord.x) + params.level_offset;
    atomicStore(&voxels[index * 3u], data.albedo);
    atomicStore(&voxels[index * 3u + 1u], data.normal);
    atomicStore(&voxels[index * 3u + 2u], data.emissive);
}

// How a cut-out triangle maps to its colour map, for `voxel_solid`.
struct VoxelMask {
    enabled: bool,
    uv0: vec2f,
    uv1: vec2f,
    uv2: vec2f,
    layer: i32,
    level: f32,
}

// Whether a voxel at grid position `coord`, and at texture position `uv`, is kept of a cut-out
// triangle. A voxel is far larger than a strand, so the texture's coverage there is compared to
// a value fixed to the voxel's place in the world: over an area the right share of voxels are
// solid, and which ones doesn't change when the volume is drawn again.
fn voxel_solid(mask: VoxelMask, uv: vec2f, coord: vec3i) -> bool {
    let world = coord + vec3i(round(params.origin / params.voxel_size));
    let hash = pcg(pcg(pcg(u32(world.x)) ^ u32(world.y)) ^ u32(world.z));
    let threshold = (f32(hash >> 16u) + 0.5) / 65536.0;
    return textureSampleLevel(color_textures, material_sampler, uv, mask.layer, mask.level).a > threshold;
}

fn voxelize_triangle(v0: vec3f, v1: vec3f, v2: vec3f, data: VoxelData, mask: VoxelMask) {
    let normal = cross(v1 - v0, v2 - v0);
    let abs_normal = abs(normal);

    // Choose the projection axis `w` that the triangle faces most directly.
    var u_axis = 0;
    var v_axis = 1;
    var w_axis = 2;
    if abs_normal.x >= abs_normal.y && abs_normal.x >= abs_normal.z {
        u_axis = 1;
        v_axis = 2;
        w_axis = 0;
    } else if abs_normal.y >= abs_normal.z {
        u_axis = 0;
        v_axis = 2;
        w_axis = 1;
    }
    if abs_normal[w_axis] < 1.0e-8 {
        return;
    }

    let a = vec2f(v0[u_axis], v0[v_axis]);
    let b = vec2f(v1[u_axis], v1[v_axis]);
    let c = vec2f(v2[u_axis], v2[v_axis]);
    let limit = f32(params.resolution);

    // Triangles wholly outside the grid need no scan at all.
    let box_min = min(min(a, b), c);
    let box_max = max(max(a, b), c);
    let w_min = min(min(v0[w_axis], v1[w_axis]), v2[w_axis]);
    let w_max = max(max(v0[w_axis], v1[w_axis]), v2[w_axis]);
    if any(box_max < vec2f(-1.0)) || any(box_min > vec2f(limit + 1.0)) || w_max < -1.0 || w_min > limit + 1.0 {
        return;
    }
    let lo = clamp(floor(box_min), vec2f(0.0), vec2f(limit - 1.0));
    let hi = clamp(floor(box_max), vec2f(0.0), vec2f(limit - 1.0));

    // Orient edges so the inside is positive, and widen them to cover any touched column.
    let orientation = select(-1.0, 1.0, (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x) >= 0.0);
    let widen = 0.5;
    let e0 = vec3f(a.y - b.y, b.x - a.x, 0.0) * orientation;
    let e1 = vec3f(b.y - c.y, c.x - b.x, 0.0) * orientation;
    let e2 = vec3f(c.y - a.y, a.x - c.x, 0.0) * orientation;
    let bias0 = -dot(e0.xy, a) + (abs(e0.x) + abs(e0.y)) * widen;
    let bias1 = -dot(e1.xy, b) + (abs(e1.x) + abs(e1.y)) * widen;
    let bias2 = -dot(e2.xy, c) + (abs(e2.x) + abs(e2.y)) * widen;

    // Depth across the plane, and how much it varies within one column.
    let dw_du = -normal[u_axis] / normal[w_axis];
    let dw_dv = -normal[v_axis] / normal[w_axis];
    let w0 = v0[w_axis] - dw_du * a.x - dw_dv * a.y;
    let spread = 0.5 * (abs(dw_du) + abs(dw_dv));
    // For texture positions, the inverse of twice the signed area in this projection.
    let inverse_area = 1.0 / ((b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x));

    for (var y = lo.y; y <= hi.y; y += 1.0) {
        let py = y + 0.5;
        // Each edge limits the column centres on this row to one side of a crossing point.
        var span = vec2f(lo.x + 0.5, hi.x + 0.5);
        span = clip_span(span, e0.x, e0.y * py + bias0);
        span = clip_span(span, e1.x, e1.y * py + bias1);
        span = clip_span(span, e2.x, e2.y * py + bias2);
        if span.x > span.y {
            continue;
        }
        let first_column = ceil(span.x - 0.5);
        let last_column = floor(span.y - 0.5);
        for (var x = first_column; x <= last_column; x += 1.0) {
            let w = w0 + dw_du * (x + 0.5) + dw_dv * py;
            let first = i32(floor(w - spread));
            let last = i32(floor(w + spread));
            var uv = vec2f(0.0);
            if mask.enabled {
                // The column's centre can lie just outside, as the edges were widened.
                let p = vec2f(x + 0.5, py);
                let weight1 = clamp(((p.x - a.x) * (c.y - a.y) - (p.y - a.y) * (c.x - a.x)) * inverse_area, 0.0, 1.0);
                let weight2 = clamp(((b.x - a.x) * (p.y - a.y) - (b.y - a.y) * (p.x - a.x)) * inverse_area, 0.0, 1.0 - weight1);
                uv = mask.uv0 * (1.0 - weight1 - weight2) + mask.uv1 * weight1 + mask.uv2 * weight2;
            }
            for (var z = first; z <= last; z++) {
                var coord = vec3i(0);
                coord[u_axis] = i32(x);
                coord[v_axis] = i32(y);
                coord[w_axis] = z;
                if !mask.enabled || voxel_solid(mask, uv, coord) {
                    write_voxel(coord, data);
                }
            }
        }
    }
}

// Narrows the span of X values that satisfy `slope * x + offset >= 0`.
fn clip_span(span: vec2f, slope: f32, offset: f32) -> vec2f {
    if slope > 0.0 {
        return vec2f(max(span.x, -offset / slope), span.y);
    }
    if slope < 0.0 {
        return vec2f(span.x, min(span.y, -offset / slope));
    }
    // A horizontal edge: the whole row is either inside or out.
    return select(vec2f(1.0, 0.0), span, offset >= 0.0);
}

// Consumes the "software" list like the software rasterizer: one workgroup per cluster, striding
// over the rest, and one thread per triangle. Matches MAX_CLUSTER_TRIANGLES.
const GROUP_SIZE: u32 = 128u;

var<workgroup> cluster_count: u32;

@compute @workgroup_size(GROUP_SIZE)
fn main(
    @builtin(workgroup_id) group: vec3u,
    @builtin(num_workgroups) groups: vec3u,
    @builtin(local_invocation_index) lane: u32,
) {
    if lane == 0u {
        cluster_count = state.sw_count;
    }
    let count = workgroupUniformLoad(&cluster_count);
    for (var item = group.x; item < count; item += groups.x) {
        voxelize_cluster_triangle(item, lane);
    }
}

// Twin of SPECK_GROUP_SIZE in vg/cull_control.wgsl.
const SPECK_GROUP_SIZE: u32 = 64u;

// The specks: clusters under a voxel across, which culling sends to the hardware end of the
// list. Each marks the voxels its bounds touch, with what its first triangle would have
// written, and a thread does each, where `main` gives a cluster a workgroup.
@compute @workgroup_size(SPECK_GROUP_SIZE)
fn specks(@builtin(global_invocation_id) id: vec3u, @builtin(num_workgroups) groups: vec3u) {
    let count = state.hw_count - state.hw_base;
    for (var item = id.x; item < count; item += groups.x * SPECK_GROUP_SIZE) {
        voxelize_speck(visible[state.opaque_end - 1u - state.hw_base - item]);
    }
}

// A triangle's albedo, from its first vertex. Ground blended of three kinds (land.rs) mixes the
// three materials' colours by the shares its texture coordinates hold, each laid by x and z.
fn voxel_albedo(instance: Instance, material: Material, vertex0: PackedVertex, world0: vec3f) -> vec3f {
    if material.texture_layer < 0 {
        return material.base_color.rgb;
    }
    let uv = decode_uv(instance, vertex0);
    if (material.flags & MATERIAL_BLENDS) == 0u {
        return material.base_color.rgb
            * textureSampleLevel(color_textures, material_sampler, uv * material.uv_scale, material.texture_layer, COLOR_MIP).rgb;
    }
    let shares = vec3f(1.0 - uv.x - uv.y, uv.x, uv.y);
    let index = instance_material(instance);
    var albedo = vec3f(0.0);
    for (var k = 0u; k < 3u; k++) {
        let m = materials[index + k];
        albedo += shares[k] * m.base_color.rgb
            * textureSampleLevel(color_textures, material_sampler, world0.xz * m.uv_scale, m.texture_layer, COLOR_MIP).rgb;
    }
    return albedo;
}

fn voxelize_speck(entry: VisibleCluster) {
    let cluster = clusters[entry.cluster];
    let instance = instances[entry.instance];
    let material = materials[instance_material(instance)];
    let packed = load_triangle(cluster, 0u);
    let vertex0 = load_vertex(cluster, packed & 0xffu);
    let vertex1 = load_vertex(cluster, (packed >> 8u) & 0xffu);
    let vertex2 = load_vertex(cluster, (packed >> 16u) & 0xffu);
    let world0 = transform_point(instance, decode_position(instance, vertex0));
    let world1 = transform_point(instance, decode_position(instance, vertex1));
    let world2 = transform_point(instance, decode_position(instance, vertex2));

    let albedo = voxel_albedo(instance, material, vertex0, world0);
    let face = cross(world1 - world0, world2 - world0);
    let face_length = length(face);
    var normal = vec3f(0.0, 1.0, 0.0);
    if face_length > 0.0 {
        normal = face / face_length;
    }
    let data = VoxelData(
        pack4x8unorm(vec4f(albedo, 1.0)),
        pack2x16snorm(oct_encode(normal)),
        encode_emissive(material.emissive * abs(instance.glow)),
    );

    let inv_voxel = 1.0 / params.voxel_size;
    let center = (transform_point(instance, cluster.center) - params.origin) * inv_voxel;
    let reach = cluster.radius * instance.max_scale * inv_voxel;
    let low = vec3i(floor(center - reach));
    // Under a voxel across, so two voxels each way at most.
    let high = min(vec3i(floor(center + reach)), low + 1);
    for (var z = low.z; z <= high.z; z++) {
        for (var y = low.y; y <= high.y; y++) {
            for (var x = low.x; x <= high.x; x++) {
                write_voxel(vec3i(x, y, z), data);
            }
        }
    }
}

fn voxelize_cluster_triangle(item: u32, t: u32) {
    let entry = visible[item];
    let cluster = clusters[entry.cluster];
    if t >= cluster_triangle_count(cluster) {
        return;
    }
    let instance = instances[entry.instance];
    let material = materials[instance_material(instance)];
    let inv_voxel = 1.0 / params.voxel_size;

    {
        let packed = load_triangle(cluster, t);
        let vertex0 = load_vertex(cluster, packed & 0xffu);
        let vertex1 = load_vertex(cluster, (packed >> 8u) & 0xffu);
        let vertex2 = load_vertex(cluster, (packed >> 16u) & 0xffu);
        let world0 = transform_point(instance, decode_position(instance, vertex0));
        let world1 = transform_point(instance, decode_position(instance, vertex1));
        let world2 = transform_point(instance, decode_position(instance, vertex2));

        let albedo = voxel_albedo(instance, material, vertex0, world0);
        let face = cross(world1 - world0, world2 - world0);
        let face_length = length(face);
        if face_length <= 0.0 {
            return;
        }
        let data = VoxelData(
            pack4x8unorm(vec4f(albedo, 1.0)),
            pack2x16snorm(oct_encode(face / face_length)),
            encode_emissive(material.emissive * abs(instance.glow)),
        );
        let p0 = (world0 - params.origin) * inv_voxel;
        let p1 = (world1 - params.origin) * inv_voxel;
        let p2 = (world2 - params.origin) * inv_voxel;
        var mask: VoxelMask;
        mask.enabled = (material.flags & MATERIAL_MASKED) != 0u;
        if mask.enabled {
            mask.uv0 = decode_uv(instance, vertex0) * material.uv_scale;
            mask.uv1 = decode_uv(instance, vertex1) * material.uv_scale;
            mask.uv2 = decode_uv(instance, vertex2) * material.uv_scale;
            mask.layer = material.texture_layer;
            // Texels per voxel, from the triangle's areas in the texture and in voxels.
            let uv_area = abs((mask.uv1.x - mask.uv0.x) * (mask.uv2.y - mask.uv0.y) - (mask.uv1.y - mask.uv0.y) * (mask.uv2.x - mask.uv0.x));
            let voxel_area = max(length(cross(p1 - p0, p2 - p0)), 1.0e-6);
            let edge = f32(textureDimensions(color_textures).x);
            let lod = 0.5 * log2(max(uv_area / voxel_area * edge * edge, 1.0e-6));
            mask.level = clamp(floor(lod + 0.5), 0.0, f32(textureNumLevels(color_textures) - 1u));
        }
        voxelize_triangle(p0, p1, p2, data, mask);
    }
}
