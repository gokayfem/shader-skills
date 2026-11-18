// The particle system's bookkeeping, a workgroup each: before the frame's spawn and update, what
// each emitter does and where its workgroups start (`plan`); after them, how many of each
// emitter's particles to draw (`finish`). One small pass for every emitter replaces a dispatch
// per emitter, which matters on the web, where each dispatch costs the browser CPU time.

#include "particles/common.wgsl"

@group(0) @binding(0) var<uniform> sim: SimParams;
@group(0) @binding(1) var<storage, read> emitters: array<Emitter, MAX_EMITTERS>;
@group(0) @binding(2) var<storage, read_write> state: array<EmitterState, MAX_EMITTERS>;
// Dispatch arguments for spawn, update and light, then from word 12 a draw's arguments per
// emitter: vertex count, instance count, first vertex, first instance.
@group(0) @binding(3) var<storage, read_write> indirect: array<u32>;

const DRAWS_FIRST: u32 = 12u;

var<workgroup> spawn_groups: array<u32, MAX_EMITTERS>;
var<workgroup> update_groups: array<u32, MAX_EMITTERS>;
var<workgroup> light_groups: array<u32, MAX_EMITTERS>;

@compute @workgroup_size(64)
fn plan(@builtin(local_invocation_index) e: u32) {
    let read = sim.parity;
    let written = 1u - read;
    let emitter = emitters[e];
    var spawn = 0u;
    var update = 0u;
    if (emitter.flags & FLAG_LIVE) != 0u {
        // New particles take slots from the top of the free list, as many as it holds.
        let free = atomicLoad(&state[e].free_count);
        // Last frame's events, which this emitter spawns from first.
        let events_sent = atomicLoad(&state[e].event_count);
        let events_in = min(events_sent, EVENT_CAPACITY);
        state[e].events_in = events_in;
        let wanted = emitter.spawn_request + events_in * emitter.event_spawn;
        spawn = min(wanted, free);
        // What didn't fit, and what the events past the emitter's capacity would have spawned.
        state[e].dropped += wanted - spawn + (events_sent - events_in) * emitter.event_spawn;
        state[e].spawn_first = free - spawn;
        atomicStore(&state[e].free_count, free - spawn);
        state[e].spawn_index = state[e].spawned;
        state[e].spawned += spawn;
        update = atomicLoad(&state[e].alive[read]);
    }
    // This frame's hits count afresh, after the spawn has read last frame's.
    atomicStore(&state[e].event_count, 0u);
    state[e].spawn_count = spawn;
    state[e].update_count = update;
    atomicStore(&state[e].alive[written], 0u);
    spawn_groups[e] = (spawn + GROUP_SIZE - 1u) / GROUP_SIZE;
    update_groups[e] = (update + GROUP_SIZE - 1u) / GROUP_SIZE;
    workgroupBarrier();

    if e == 0u {
        var spawn_total = 0u;
        var update_total = 0u;
        for (var i = 0u; i < MAX_EMITTERS; i++) {
            state[i].spawn_start = spawn_total;
            state[i].update_start = update_total;
            spawn_total += spawn_groups[i];
            update_total += update_groups[i];
        }
        indirect[0] = spawn_total;
        indirect[1] = 1u;
        indirect[2] = 1u;
        indirect[3] = update_total;
        indirect[4] = 1u;
        indirect[5] = 1u;
    }
}

@compute @workgroup_size(64)
fn finish(@builtin(local_invocation_index) e: u32) {
    let written = 1u - sim.parity;
    var count = 0u;
    if (emitters[e].flags & FLAG_LIVE) != 0u {
        count = atomicLoad(&state[e].alive[written]);
    }
    let at = DRAWS_FIRST + e * 4u;
    indirect[at] = 6u;
    indirect[at + 1u] = count;
    indirect[at + 2u] = 0u;
    indirect[at + 3u] = 0u;

    // The light pass covers what is alive now, a workgroup at a time per emitter.
    light_groups[e] = (count + GROUP_SIZE - 1u) / GROUP_SIZE;
    workgroupBarrier();
    if e == 0u {
        var total = 0u;
        for (var i = 0u; i < MAX_EMITTERS; i++) {
            state[i].light_start = total;
            total += light_groups[i];
        }
        indirect[6] = total;
        indirect[7] = 1u;
        indirect[8] = 1u;
    }
}
