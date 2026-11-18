// Sorts the particles of each emitter that asks for it far to near along the view, so that the
// draw, which lays each over what came before, puts nearer smoke over farther. One workgroup
// sorts one emitter's whole alive list in its own memory, with a bitonic network.
//
// Each key holds a particle's depth, quantised to 20 bits on a log scale, farthest smallest,
// above its slot's place in the emitter's slice in 12 bits: a sorted emitter's budget is at most
// 4096, so the key alone says where the particle is, and the list is rewritten from the keys.
// 4096 keys of four bytes fill the 16 KiB of workgroup memory WebGPU promises.

#include "particles/common.wgsl"

@group(0) @binding(0) var<uniform> sim: SimParams;
@group(0) @binding(1) var<storage, read> emitters: array<Emitter, MAX_EMITTERS>;
@group(0) @binding(2) var<storage, read_write> state: array<EmitterState, MAX_EMITTERS>;
@group(0) @binding(3) var<storage, read_write> particles: array<Particle>;
@group(0) @binding(4) var<storage, read_write> free_list: array<u32>;
@group(0) @binding(5) var<storage, read_write> alive: array<u32>;
@group(0) @binding(6) var<storage, read_write> events: array<Event>;

const MOST_SORTED: u32 = 4096u;
const SORT_LANES: u32 = 256u;
const DEPTH_STEPS: f32 = 1048575.0;
// Depths beyond this, in metres, share the farthest key.
const FARTHEST: f32 = 20000.0;
const EMPTY: u32 = 0xffffffffu;

// The keys, which fill the whole of the workgroup memory WebGPU promises: the first also
// passes the count from the first lane to all the others before the keys are written.
var<workgroup> keys: array<u32, MOST_SORTED>;

@compute @workgroup_size(256)
fn main(@builtin(workgroup_id) group: vec3u, @builtin(local_invocation_index) lane: u32) {
    let e = group.x;
    let written = 1u - sim.parity;
    let emitter = emitters[e];
    if lane == 0u {
        var found = 0u;
        if (emitter.flags & FLAG_LIVE) != 0u && (emitter.flags & FLAG_SORT) != 0u {
            found = atomicLoad(&state[e].alive[written]);
        }
        // More than fits is left unsorted, rather than sorted wrongly: the keys hold a place
        // in the slice in 12 bits. The CPU never asks to sort so many.
        keys[0] = select(found, 0u, found > MOST_SORTED || emitter.budget > MOST_SORTED);
    }
    let count = workgroupUniformLoad(&keys[0]);
    // Every lane has read the count before the first lane's keys overwrite it.
    workgroupBarrier();
    if count < 2u {
        return;
    }
    var size = 1u;
    while size < count {
        size *= 2u;
    }
    let list = written * sim.capacity + emitter.base;

    for (var i = lane; i < size; i += SORT_LANES) {
        var key = EMPTY;
        if i < count {
            let slot = alive[list + i];
            let depth = max(dot(particles[slot].position - sim.camera_position.xyz, sim.camera_forward.xyz), 0.0);
            let near = clamp(log2(1.0 + depth) / log2(1.0 + FARTHEST), 0.0, 1.0);
            // Farthest first: the smallest keys are the farthest particles.
            let order = u32((1.0 - near) * DEPTH_STEPS);
            key = (order << 12u) | (slot - emitter.base);
        }
        keys[i] = key;
    }
    workgroupBarrier();

    // Bitonic sort, ascending, of `size` keys.
    for (var span = 2u; span <= size; span *= 2u) {
        for (var gap = span / 2u; gap > 0u; gap /= 2u) {
            for (var i = lane; i < size; i += SORT_LANES) {
                let partner = i ^ gap;
                if partner > i {
                    let a = keys[i];
                    let b = keys[partner];
                    let ascending = (i & span) == 0u;
                    if (a > b) == ascending {
                        keys[i] = b;
                        keys[partner] = a;
                    }
                }
            }
            workgroupBarrier();
        }
    }

    for (var i = lane; i < count; i += SORT_LANES) {
        alive[list + i] = emitter.base + (keys[i] & 4095u);
    }
}
