// Spawning and ageing particles. Every workgroup belongs to one emitter, found from the starts
// `plan` laid out, so every branch on the emitter's modules is uniform across it.
//
// Each emitter has its own slice of the pool, with a free list and two alive lists there: one is
// read this frame while the other is written, and they swap each frame.

#include "particles/common.wgsl"

@group(0) @binding(0) var<uniform> sim: SimParams;
@group(0) @binding(1) var<storage, read> emitters: array<Emitter, MAX_EMITTERS>;
@group(0) @binding(2) var<storage, read_write> state: array<EmitterState, MAX_EMITTERS>;
@group(0) @binding(3) var<storage, read_write> particles: array<Particle>;
@group(0) @binding(4) var<storage, read_write> free_list: array<u32>;
// Both alive lists, the second `sim.capacity` entries after the first.
@group(0) @binding(5) var<storage, read_write> alive: array<u32>;
// Each emitter's events from last frame's hits, EVENT_CAPACITY apiece.
@group(0) @binding(6) var<storage, read_write> events: array<Event>;

// The air this frame's steps end in.
fn sim_air() -> Air {
    return Air(sim.time, sim.wind, sim.camera_position.w);
}

// Keeps a particle for the next frame, in the alive list being written.
fn keep(e: u32, base: u32, slot: u32) {
    let written = 1u - sim.parity;
    let at = atomicAdd(&state[e].alive[written], 1u);
    alive[written * sim.capacity + base + at] = slot;
}

@compute @workgroup_size(64)
fn spawn(@builtin(workgroup_id) group: vec3u, @builtin(local_invocation_index) lane: u32) {
    var e = 0u;
    for (; e < MAX_EMITTERS; e++) {
        let start = state[e].spawn_start;
        let groups = (state[e].spawn_count + GROUP_SIZE - 1u) / GROUP_SIZE;
        if group.x >= start && group.x < start + groups {
            break;
        }
    }
    if e >= MAX_EMITTERS {
        return;
    }
    let count = state[e].spawn_count;
    let i = (group.x - state[e].spawn_start) * GROUP_SIZE + lane;
    if i >= count {
        return;
    }
    let emitter = emitters[e];
    let slot = free_list[emitter.base + state[e].spawn_first + i];
    // Born part way through the frame, and moved on to its end; from last frame's events first,
    // each spawning its share.
    let index = state[e].spawn_index + i;
    var p: Particle;
    let from_events = state[e].events_in * emitter.event_spawn;
    if i < from_events {
        p = event_particle(emitter, index, events[e * EVENT_CAPACITY + i / max(emitter.event_spawn, 1u)]);
    } else {
        p = spawn_particle(emitter, index);
    }
    step_particle(&p, emitter, spawn_elapsed(i, count, sim.seconds), sim_air());
    particles[slot] = p;
    keep(e, emitter.base, slot);
}

@compute @workgroup_size(64)
fn update(@builtin(workgroup_id) group: vec3u, @builtin(local_invocation_index) lane: u32) {
    var e = 0u;
    for (; e < MAX_EMITTERS; e++) {
        let start = state[e].update_start;
        let groups = (state[e].update_count + GROUP_SIZE - 1u) / GROUP_SIZE;
        if group.x >= start && group.x < start + groups {
            break;
        }
    }
    if e >= MAX_EMITTERS {
        return;
    }
    let i = (group.x - state[e].update_start) * GROUP_SIZE + lane;
    if i >= state[e].update_count {
        return;
    }
    let emitter = emitters[e];
    let slot = alive[sim.parity * sim.capacity + emitter.base + i];
    var p = particles[slot];
    step_particle(&p, emitter, sim.seconds, sim_air());
    if is_dead(p) {
        // Its slot goes back on the free list, above the slots this frame's spawns took.
        let at = atomicAdd(&state[e].free_count, 1u);
        free_list[emitter.base + at] = slot;
        return;
    }
    particles[slot] = p;
    keep(e, emitter.base, slot);
}
