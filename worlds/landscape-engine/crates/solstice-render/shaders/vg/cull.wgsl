// Hierarchical LOD selection and culling, one breadth-first level per dispatch.
//
// WebGPU has no way for a dispatch to grow its own workload, so traversal runs as a
// fixed series of dispatches over a shared queue: each reads the range appended by
// the previous one and appends the next level behind it (see cull_control.wgsl).
//
// Occlusion culling uses two phases. Phase 0 tests against the previous frame's
// depth, which is mostly right, and defers everything it rejects to a retry list.
// After that result is rasterized and a fresh depth pyramid built, phase 1 re-tests
// the retry list and draws whatever had been wrongly rejected.
//
// Not all of a scene need be on the GPU. Leaf nodes say whether their page is installed, and
// every view records which pages it reached, so the CPU can load them. A cluster draws, however
// coarse it is, while the set simplified into it isn't installed. So a parent keeps drawing
// until its children arrive, and since the CPU installs parents first, the cut stays watertight.

#include "common/math.wgsl"
#include "common/scene.wgsl"
#include "common/skin.wgsl"
#include "common/sway.wgsl"
#include "vg/pass_view.wgsl"

struct QueueEntry {
    instance: u32,
    item: u32, // Node index, or cluster index with CLUSTER_BIT set.
}

const CLUSTER_BIT: u32 = 0x80000000u;
// Bits of `Node.is_leaf`. Twins of the constants in solstice-asset's `gpu` module.
const NODE_INSTALLED: u32 = 0x40000000u;
const NODE_PAGE_MASK: u32 = 0x3fffffffu;
const CLUSTER_CHILDREN_MISSING: u32 = 0x10000u;
// A request that only says "seen, but occluded": keep the page, but don't load it for this.
const REQUEST_KEEP: u32 = 1u;
const REQUEST_LOWEST_LOAD: u32 = 2u;
// Requests per pixel of error. Fine enough to order pages, coarse enough never to overflow.
const REQUEST_PER_PIXEL: f32 = 16.0;
const REQUEST_MAX_PIXELS: f32 = 100000.0;
// Shadow and voxel views only affect the image indirectly, so they yield to the main camera.
// How many times coarser what is near but out of view may be than if it were seen.
const PREFETCH_RELAX: f32 = 4.0;
const REQUEST_ORTHO_WEIGHT: f32 = 0.125;
// `Instance.texture_info` of a material with no texture.
const NO_TEXTURE: u32 = 0xffffffffu;
// Clusters projecting smaller than this are rasterized in the compute shader.
const SOFTWARE_MAX_DIAMETER_PX: f32 = 40.0;
// The same for depth maps, where the compute rasterizer only pays off for the smallest
// clusters: it costs a pass to lay its depth over the map, which a few mid-sized clusters do
// not earn back, while the pixel-sized clusters of a wood's leaves do many times over.
const SOFTWARE_MAX_DIAMETER_TEXELS: f32 = 16.0;

@group(0) @binding(0) var<uniform> view: PassView;
@group(0) @binding(1) var<storage, read> instances: array<Instance>;
@group(0) @binding(2) var<storage, read> nodes: array<Node>;
@group(0) @binding(3) var<storage, read> clusters: array<Cluster>;
@group(0) @binding(4) var hzb: texture_2d<f32>;
@group(0) @binding(5) var bone_texture: texture_2d<f32>;
// Each skinned cluster's bones, four to a word, by its number plus `Instance.bone_lists`.
@group(0) @binding(6) var cluster_bone_lists: texture_2d<u32>;

@group(1) @binding(0) var<storage, read_write> state: CullState;
@group(1) @binding(1) var<storage, read_write> queue: array<QueueEntry>;
@group(1) @binding(2) var<storage, read_write> retry: array<QueueEntry>;
@group(1) @binding(3) var<storage, read_write> visible: array<VisibleCluster>;
// Per page: 0 if not reached since the array was last read, else how much it is wanted. See
// `residency.rs`. Then a bit per page, set by the views kept from frame to frame that asked.
// From the end, per texture layer: the edge a texture must have to look sharp.
@group(1) @binding(4) var<storage, read_write> page_requests: array<atomic<u32>>;

fn push_queue(entry: QueueEntry) {
    let slot = atomicAdd(&state.write_head, 1u);
    if slot < arrayLength(&queue) {
        queue[slot] = entry;
    } else {
        atomicAdd(&state.dropped, 1u);
    }
}

fn push_retry(entry: QueueEntry) {
    let slot = atomicAdd(&state.retry_count, 1u);
    if slot < arrayLength(&retry) {
        retry[slot] = entry;
    } else {
        atomicAdd(&state.dropped, 1u);
    }
}

// Software clusters fill the list from the front and hardware from the back, so
// both can append concurrently and each still forms one contiguous range. An entry is
// claimed from a single count of both before either end is written, so the two ranges can
// never meet, however many threads arrive at once. Returns false, and counts the loss, if
// the list is full.
fn emit_cluster(instance: u32, cluster: u32, software: bool) -> bool {
    let capacity = state.opaque_end;
    if atomicAdd(&state.reserved, 1u) >= capacity {
        atomicAdd(&state.dropped, 1u);
        return false;
    }
    if software {
        visible[atomicAdd(&state.sw_count, 1u)] = VisibleCluster(instance, cluster);
    } else {
        visible[capacity - 1u - atomicAdd(&state.hw_count, 1u)] = VisibleCluster(instance, cluster);
    }
    return true;
}

// Sends a cluster to be drawn as a speck. For a voxelization it goes to the hardware end of the
// list, which nothing else there uses, and gi/voxelize.wgsl marks the voxels its bounds touch.
// For a depth map, the texels its bounds touch and its depth are worked out here, where its
// bounds are to hand, and vg/speck_depth.wgsl only writes them.
fn emit_speck(instance: u32, cluster: u32, center: vec3f, radius: f32) {
    if view.allow_software == 2u {
        _ = emit_cluster(instance, cluster, false);
        return;
    }
    let clip = view.raster_view_proj * vec4f(center, 1.0);
    let at = vec2f((clip.x * 0.5 + 0.5) * view.viewport.x, (0.5 - clip.y * 0.5) * view.viewport.y);
    let reach = radius * view.lod_scale;
    let last = vec2i(view.viewport) - 1;
    let low = vec2i(floor(at - reach));
    let high = vec2i(floor(at + reach));
    // Nothing of it lands in the map. Depth runs 0 at the light's side to 1 at the far side.
    if any(high < vec2i(0)) || any(low > last) || clip.z <= 0.0 || clip.z >= 1.0 {
        return;
    }
    let slot = atomicAdd(&state.speck_count, 1u);
    if slot >= arrayLength(&retry) {
        atomicAdd(&state.dropped, 1u);
        return;
    }
    // The far side of its bounds, so that it does not shadow itself.
    let m = view.raster_view_proj;
    let depth = min(clip.z + radius * length(vec3f(m[0][2], m[1][2], m[2][2])), 1.0);
    let first = vec2u(max(low, vec2i(0)));
    let more = min(vec2u(min(high, last)) - first, vec2u(3u));
    retry[slot] = QueueEntry(
        first.x | (first.y << 13u) | (more.x << 26u) | (more.y << 28u),
        max(u32((1.0 - depth) * 16777215.0), 1u),
    );
}

// The same for cut-out clusters, in the part of the list after the opaque clusters' part.
fn emit_masked_cluster(instance: u32, cluster: u32, software: bool) -> bool {
    let end = arrayLength(&visible);
    if atomicAdd(&state.masked_reserved, 1u) >= end - state.opaque_end {
        atomicAdd(&state.dropped, 1u);
        return false;
    }
    if software {
        visible[state.opaque_end + atomicAdd(&state.masked_sw_count, 1u)] = VisibleCluster(instance, cluster);
    } else {
        visible[end - 1u - atomicAdd(&state.masked_hw_count, 1u)] = VisibleCluster(instance, cluster);
    }
    return true;
}

// Screen-space size of an error metric living on a sphere. Using the sphere's
// nearest point makes this conservative and, since parent spheres contain their
// children, monotonic up the DAG.
fn projected_error(center: vec3f, radius: f32, error: f32) -> f32 {
    if view.is_ortho != 0u {
        var pixels = error * view.lod_scale;
        // Drawn for a camera: error that camera cannot see is not worth drawing here either.
        // A caster's error shows where its shadow lands, anywhere along the line from it away
        // from the light, so the camera's distance to that line bounds how near the error can
        // be seen. A parent's line sweeps a volume holding its children's, so this stays
        // monotonic up the DAG, as the view's own measure is; the smaller of the two is too.
        if view.eye_lod_scale > 0.0 {
            let to_eye = view.eye_position - center;
            let along = max(dot(to_eye, -view.eye_light), 0.0);
            let nearest = sqrt(max(dot(to_eye, to_eye) - along * along, 0.0));
            let distance = max(nearest - radius, view.eye_near);
            pixels = min(pixels, error * view.eye_lod_scale / distance);
        }
        return pixels;
    }
    let distance = max(length(center - view.camera_position) - radius, view.near);
    return error * view.lod_scale / distance;
}

// A child below the LOD cut would return immediately on the next dispatch. Reject it
// before reserving queue space: distant instances can have many such children even
// though only a few of their coarsest clusters contribute to the image.
fn push_child(instance_index: u32, node_index: u32, instance: Instance) {
    let node = nodes[node_index];
    let center = transform_point(instance, node.lod_center);
    let radius = (node.lod_radius + instance.lod_margin) * instance.max_scale;
    let error = node.max_parent_error * instance.max_scale;
    if projected_error(center, radius, error) <= view.lod_threshold {
        return;
    }
    push_queue(QueueEntry(instance_index, node_index));
}

fn in_frustum(center: vec3f, radius: f32) -> bool {
    for (var i = 0u; i < 6u; i++) {
        let p = view.planes[i];
        if dot(p.xyz, center) + p.w < -radius {
            return false;
        }
    }
    for (var i = 0u; i < view.receiver_plane_count; i++) {
        let p = view.receiver_planes[i];
        if dot(p.xyz, center) + p.w < -radius {
            return false;
        }
    }
    return true;
}

// Tests a world-space sphere against the depth pyramid. The pyramid stores the
// farthest depth per texel, so a sphere whose nearest point is beyond that is hidden.
fn passes_occlusion(center: vec3f, radius: f32) -> bool {
    if view.occlusion_enabled == 0u {
        return true;
    }
    let c_view = (view.occlusion_view * vec4f(center, 1.0)).xyz;
    let c = vec3f(c_view.x, c_view.y, -c_view.z);
    if c.z < radius + view.near {
        return true; // Touches the near plane: cannot be projected safely.
    }

    // Tight projected bounds of a sphere (Mara & McGuire 2013).
    let cx = -c.xz;
    let vx = vec2f(sqrt(dot(cx, cx) - radius * radius), radius);
    let minx = mat2x2f(vx.x, vx.y, -vx.y, vx.x) * cx;
    let maxx = mat2x2f(vx.x, -vx.y, vx.y, vx.x) * cx;
    let cy = -c.yz;
    let vy = vec2f(sqrt(dot(cy, cy) - radius * radius), radius);
    let miny = mat2x2f(vy.x, vy.y, -vy.y, vy.x) * cy;
    let maxy = mat2x2f(vy.x, -vy.y, vy.y, vy.x) * cy;

    let x0 = minx.x / minx.y * view.proj_scale.x * 0.5 + 0.5;
    let x1 = maxx.x / maxx.y * view.proj_scale.x * 0.5 + 0.5;
    let y0 = 0.5 - miny.x / miny.y * view.proj_scale.y * 0.5;
    let y1 = 0.5 - maxy.x / maxy.y * view.proj_scale.y * 0.5;
    let uv_min = clamp(vec2f(min(x0, x1), min(y0, y1)), vec2f(0.0), vec2f(1.0));
    let uv_max = clamp(vec2f(max(x0, x1), max(y0, y1)), vec2f(0.0), vec2f(1.0));

    let extent = (uv_max - uv_min) * view.hzb_size;
    let level = u32(clamp(ceil(log2(max(max(extent.x, extent.y), 1.0))), 0.0, f32(textureNumLevels(hzb) - 1u)));
    let dims = vec2i(textureDimensions(hzb, level));
    let t0 = clamp(vec2i(uv_min * vec2f(dims)), vec2i(0), dims - 1);
    let t1 = clamp(vec2i(uv_max * vec2f(dims)), vec2i(0), dims - 1);

    var farthest = textureLoad(hzb, t0, level).r;
    farthest = min(farthest, textureLoad(hzb, vec2i(t1.x, t0.y), level).r);
    farthest = min(farthest, textureLoad(hzb, vec2i(t0.x, t1.y), level).r);
    farthest = min(farthest, textureLoad(hzb, t1, level).r);

    let nearest_depth = view.near / (c.z - radius);
    return nearest_depth >= farthest;
}

// Works out how many texels across its texture this cluster could show: its pixels per object
// unit, divided by the texture repeats per object unit.
fn request_texture_detail(cluster: Cluster, instance: Instance, distance: f32) {
    let density_code = cluster.counts >> 24u;
    // Ground blended of three kinds (land.rs) lays its textures by the world's x and z, a unit
    // of texture coordinate to the metre before its scale, whatever its mesh's coordinates say.
    let blends = (instance.material & INSTANCE_BLENDS) != 0u;
    if view.is_ortho != 0u || instance.texture_info == NO_TEXTURE || (density_code == 0u && !blends) {
        return;
    }
    let layer = instance.texture_info >> 16u;
    let log2_uv_scale = f32(instance.texture_info & 0xffffu) / 256.0 - 128.0;
    let log2_density = select(f32(density_code) / 8.0 - 16.0, 0.0, blends);
    let pixels_per_unit = view.lod_scale * instance.max_scale / max(distance, view.near);
    let edge = pixels_per_unit * exp2(-log2_density - log2_uv_scale);
    let slot = arrayLength(&page_requests) - 1u - layer;
    atomicMax(&page_requests[slot], u32(min(edge, 65536.0)));
    // And it needs the two textures after its own as sharp: their layers follow its own, at
    // about the same scale.
    if blends {
        atomicMax(&page_requests[slot - 1u], u32(min(edge, 65536.0)));
        atomicMax(&page_requests[slot - 2u], u32(min(edge, 65536.0)));
    }
}

fn try_emit(entry: QueueEntry, cluster_index: u32, instance: Instance) {
    let cluster = clusters[cluster_index];
    let sphere = posed_sphere(instance, cluster_index, cluster);
    let center = sphere.xyz;
    let radius = sphere.w;
    if !in_frustum(center, radius) {
        return;
    }
    if !passes_occlusion(center, radius) {
        if view.phase == 0u {
            push_retry(QueueEntry(entry.instance, cluster_index | CLUSTER_BIT));
        }
        return;
    }

    // A cluster under a texel of a depth map, or a voxel, across marks the same texel or two
    // whichever of its triangles is drawn, and a rasterizer gives it a hundred threads and more
    // to do so. In a scene whose coarsest clusters are small, a far cascade is nearly all such
    // clusters: they are drawn as points, by a thread each.
    if view.speck_texels > 0.0 && view.is_ortho != 0u && !instance_masked(instance)
        && 2.0 * radius * view.lod_scale < view.speck_texels {
        emit_speck(entry.instance, cluster_index, center, radius);
        return;
    }

    // 2 means the list is consumed by a compute pass, not a rasterizer: voxelization.
    var software = view.allow_software == 2u;
    if view.allow_software == 1u {
        // How big it is on screen: swaying moves a cluster about, but hardly changes its size,
        // so a swaying one is sized by its own sphere, at the nearest the wind can bring it.
        // Sized by the sphere grown for the wind, a wood's tiny leaf clusters would leave the
        // software rasterizer, where they are cheap, for the hardware one.
        let size = select(radius, cluster.radius * instance.max_scale, instance_sways(instance) && !instance_skinned(instance));
        if view.is_ortho != 0u {
            // Orthographic: the same size everywhere.
            software = 2.0 * size * view.lod_scale < SOFTWARE_MAX_DIAMETER_TEXELS;
        } else {
            let distance = length(center - view.camera_position) - radius;
            if distance > view.near {
                software = 2.0 * size * view.lod_scale / distance < SOFTWARE_MAX_DIAMETER_PX;
            }
        }
    }
    var emitted: bool;
    if view.masked_lists != 0u && instance_masked(instance) {
        emitted = emit_masked_cluster(entry.instance, cluster_index, software);
    } else {
        emitted = emit_cluster(entry.instance, cluster_index, software);
    }
    if !emitted {
        return;
    }
    request_texture_detail(cluster, instance, length(center - view.camera_position) - radius);
    atomicAdd(&state.triangle_count, cluster_triangle_count(cluster));
}

// A cluster's bounding sphere where the pose has put it, in the world. Each vertex's place is a
// blend of its place under each of its bones, so it lies inside the union of the cluster's
// sphere moved by each bone of the cluster, which is what this bounds.
fn posed_sphere(instance: Instance, cluster_index: u32, c: Cluster) -> vec4f {
    if !instance_skinned(instance) {
        // What sways can lean out of its sphere, in a view that sees it sway.
        let reach = sway_reach(instance, c.center.y, c.radius, view.is_ortho == 0u);
        return vec4f(transform_point(instance, c.center), (c.radius + reach) * instance.max_scale);
    }
    let at = u32(i32(cluster_index) + instance.bone_lists);
    let list = textureLoad(
        cluster_bone_lists,
        vec2u(at % BONE_TEXTURE_WIDTH, at / BONE_TEXTURE_WIDTH),
        0,
    );
    // Too many bones to list: the whole mesh's reach bounds it.
    if (list.x & 0xffffu) == 0xffu {
        return vec4f(
            transform_point(instance, c.center),
            (c.radius + instance.lod_margin) * instance.max_scale,
        );
    }
    // Sixteen bones in four words, ascending, with unused slots repeating the last.
    var sum = vec3f(0.0);
    var count = 0u;
    var moved: array<vec3f, 16>;
    var last = 0u;
    for (var slot = 0u; slot < 16u; slot++) {
        let bone = (list[slot / 4u] >> ((slot % 4u) * 8u)) & 0xffu;
        if slot > 0u && bone <= last {
            break;
        }
        last = bone;
        let place = skin_point(bone_rows(instance.bones, bone, false), c.center);
        moved[count] = place;
        sum += place;
        count += 1u;
    }
    let center = sum / f32(count);
    // Bent as it is posed, if it sways too.
    var reach = sway_reach(instance, c.center.y, c.radius, view.is_ortho == 0u);
    for (var i = 0u; i < count; i++) {
        reach = max(reach, distance(moved[i], center));
    }
    return vec4f(
        transform_point(instance, center),
        (reach + c.radius) * instance.max_scale,
    );
}

// Whether this view draws an instance at all.
fn view_draws(instance: Instance) -> bool {
    if instance_hidden(instance) {
        return false;
    }
    switch view.instance_filter {
        case INSTANCES_STATIC: {
            return !instance_movable(instance);
        }
        case INSTANCES_MOVABLE: {
            return instance_movable(instance);
        }
        case INSTANCES_GI: {
            return !instance_movable(instance) || instance_stays_in_gi(instance);
        }
        default: {
            return true;
        }
    }
}

// Queues the hierarchy root of every instance the view draws.
@compute @workgroup_size(64)
fn seed(@builtin(global_invocation_id) id: vec3u, @builtin(num_workgroups) groups: vec3u) {
    let index = id.x + id.y * groups.x * 64u;
    if index < arrayLength(&instances) && view_draws(instances[index]) {
        push_queue(QueueEntry(index, instances[index].root_node));
    }
}

@compute @workgroup_size(32)
fn process(@builtin(global_invocation_id) id: vec3u, @builtin(num_workgroups) groups: vec3u) {
    // A level can contain more than 65535 workgroups. Continue in Y without dropping
    // entries or increasing the local workgroup size (which hurts smaller levels).
    let slot = state.read_begin + id.x + id.y * groups.x * 32u;
    if slot >= state.read_end || slot >= arrayLength(&queue) {
        return;
    }
    let entry = queue[slot];
    let instance = instances[entry.instance];

    if (entry.item & CLUSTER_BIT) != 0u {
        try_emit(entry, entry.item & ~CLUSTER_BIT, instance);
        return;
    }

    let node = nodes[entry.item];
    let page = node.is_leaf & NODE_PAGE_MASK;

    // If even the coarsest parent below is good enough, everything below is finer
    // than required and this subtree cannot contribute to the cut.
    let lod_center = transform_point(instance, node.lod_center);
    let lod_radius = (node.lod_radius + instance.lod_margin) * instance.max_scale;
    let scaled_error = node.max_parent_error * instance.max_scale;
    let parent_pixels = projected_error(lod_center, lod_radius, scaled_error);
    if parent_pixels <= view.lod_threshold {
        return;
    }

    let center = transform_point(instance, node.center);
    let reach = sway_reach(instance, node.center.y, node.radius, view.is_ortho == 0u);
    let radius = (node.radius + instance.lod_margin + reach) * instance.max_scale;
    if !in_frustum(center, radius) {
        // Near the camera but out of its view: nothing is drawn, but pages are asked for down
        // to a coarser level, so that a turn or a fast move finds more than the coarsest
        // geometry already there. What is in view is always wanted more.
        if view.prefetch_radius > 0.0 && view.phase == 0u
            && parent_pixels > view.lod_threshold * PREFETCH_RELAX
            && distance(center, view.camera_position) - radius < view.prefetch_radius {
            if node.is_leaf == 0u {
                for (var i = 0u; i < node.count; i++) {
                    push_child(entry.instance, node.first + i, instance);
                }
            } else {
                atomicMax(&page_requests[page], REQUEST_LOWEST_LOAD);
            }
        }
        return;
    }
    if !passes_occlusion(center, radius) {
        if view.phase == 0u {
            push_retry(entry);
        }
        if node.is_leaf != 0u {
            atomicMax(&page_requests[page], REQUEST_KEEP);
        }
        return;
    }

    if node.is_leaf == 0u {
        for (var i = 0u; i < node.count; i++) {
            push_child(entry.instance, node.first + i, instance);
        }
        return;
    }

    // Leaf: this set could improve the image, so its page is wanted.
    let weight = select(1.0, REQUEST_ORTHO_WEIGHT, view.is_ortho != 0u);
    let wanted = min(parent_pixels, REQUEST_MAX_PIXELS) * weight * REQUEST_PER_PIXEL;
    atomicMax(&page_requests[page], REQUEST_LOWEST_LOAD + u32(wanted));
    if view.hold_requests != 0u {
        // A view kept from frame to frame asks only now, as it is drawn.
        atomicOr(&page_requests[view.hold_requests + (page >> 5u)], 1u << (page & 31u));
    }
    if (node.is_leaf & NODE_INSTALLED) == 0u {
        return;
    }

    // The parent is too coarse for all these siblings, so each one is in the cut when it is
    // itself fine enough, or nothing finer is here to replace it.
    for (var i = 0u; i < node.count; i++) {
        let cluster_index = node.first + i;
        let cluster = clusters[cluster_index];
        let own_center = transform_point(instance, cluster.lod_center);
        let own_radius = (cluster.lod_radius + instance.lod_margin) * instance.max_scale;
        let own_error = cluster.lod_error * instance.max_scale;
        if projected_error(own_center, own_radius, own_error) <= view.lod_threshold
            || (cluster.counts & CLUSTER_CHILDREN_MISSING) != 0u {
            try_emit(entry, cluster_index, instance);
        }
    }
}
