// Single-invocation steps that turn the counters in the cull state into indirect
// arguments. They run between the wider dispatches of cull.wgsl.

#include "vg/pass_view.wgsl"

// Must match the workgroup size of `process`.
const PROCESS_GROUP_SIZE: u32 = 32u;
// The most workgroups one dispatch dimension may hold. Consumers of the software list take
// one workgroup per cluster and stride over the rest when there are more.
const MAX_GROUPS: u32 = 65535u;

struct Args {
    // Dispatch: traversal of the next queue level.
    queue_x: u32,
    queue_y: u32,
    queue_z: u32,
    // Dispatch: software rasterization of the newly visible clusters.
    software_x: u32,
    software_y: u32,
    software_z: u32,
    // Draw: hardware rasterization of the newly visible clusters.
    draw_vertex_count: u32,
    draw_instance_count: u32,
    draw_first_vertex: u32,
    draw_first_instance: u32,
    // Draw: one triangle laying software-rasterized depth over a depth map, or nothing when
    // no cluster went to software.
    merge_vertex_count: u32,
    merge_instance_count: u32,
    merge_first_vertex: u32,
    merge_first_instance: u32,
    // The same two rasterizations again, for the newly visible cut-out clusters.
    masked_software_x: u32,
    masked_software_y: u32,
    masked_software_z: u32,
    masked_draw_vertex_count: u32,
    masked_draw_instance_count: u32,
    masked_draw_first_vertex: u32,
    masked_draw_first_instance: u32,
    // Dispatch: a depth map's specks, a thread each.
    speck_x: u32,
    speck_y: u32,
    speck_z: u32,
    // Dispatch: a voxelization's specks, which are the hardware end of its list.
    voxel_speck_x: u32,
    voxel_speck_y: u32,
    voxel_speck_z: u32,
}

// Must match the workgroup size of the passes that draw specks.
const SPECK_GROUP_SIZE: u32 = 64u;

const MAX_CLUSTER_TRIANGLES: u32 = 128u;

@group(0) @binding(0) var<storage, read_write> state: CullState;
@group(0) @binding(1) var<storage, read_write> args: Args;

fn groups_for(count: u32, size: u32) -> u32 {
    return (count + size - 1u) / size;
}

// Makes everything appended by the previous level the next range to read.
@compute @workgroup_size(1)
fn advance() {
    state.read_begin = state.read_end;
    state.read_end = atomicLoad(&state.write_head);
    let groups = groups_for(state.read_end - state.read_begin, PROCESS_GROUP_SIZE);
    args.queue_y = max(groups_for(groups, MAX_GROUPS), 1u);
    // Balance rows so crossing the X limit does not launch a nearly empty second row.
    args.queue_x = groups_for(groups, args.queue_y);
    args.queue_z = 1u;
}

// Publishes the clusters selected since the last call for rasterization.
@compute @workgroup_size(1)
fn finish() {
    args.software_x = min(atomicLoad(&state.sw_count) - state.sw_base, MAX_GROUPS);
    args.software_y = 1u;
    args.software_z = 1u;
    args.draw_vertex_count = MAX_CLUSTER_TRIANGLES * 3u;
    args.draw_instance_count = atomicLoad(&state.hw_count) - state.hw_base;
    args.draw_first_vertex = 0u;
    args.draw_first_instance = 0u;
    args.masked_software_x = min(atomicLoad(&state.masked_sw_count) - state.masked_sw_base, MAX_GROUPS);
    args.masked_software_y = 1u;
    args.masked_software_z = 1u;
    args.masked_draw_vertex_count = MAX_CLUSTER_TRIANGLES * 3u;
    args.masked_draw_instance_count = atomicLoad(&state.masked_hw_count) - state.masked_hw_base;
    args.masked_draw_first_vertex = 0u;
    args.masked_draw_first_instance = 0u;
    args.speck_x = min(groups_for(atomicLoad(&state.speck_count), SPECK_GROUP_SIZE), MAX_GROUPS);
    args.speck_y = 1u;
    args.speck_z = 1u;
    args.voxel_speck_x = min(groups_for(args.draw_instance_count, SPECK_GROUP_SIZE), MAX_GROUPS);
    args.voxel_speck_y = 1u;
    args.voxel_speck_z = 1u;
    args.merge_vertex_count = 3u;
    args.merge_instance_count = min(args.software_x + args.masked_software_x + args.speck_x, 1u);
    args.merge_first_vertex = 0u;
    args.merge_first_instance = 0u;
}

// Starts occlusion phase 1. The bind group swaps queue and retry buffers, so the
// retry list written during phase 0 is now the queue.
@compute @workgroup_size(1)
fn begin_second_phase() {
    let retried = atomicLoad(&state.retry_count);
    state.read_begin = 0u;
    state.read_end = 0u;
    atomicStore(&state.write_head, retried);
    atomicStore(&state.retry_count, 0u);
    state.sw_base = atomicLoad(&state.sw_count);
    state.hw_base = atomicLoad(&state.hw_count);
    state.masked_sw_base = atomicLoad(&state.masked_sw_count);
    state.masked_hw_base = atomicLoad(&state.masked_hw_count);
}
