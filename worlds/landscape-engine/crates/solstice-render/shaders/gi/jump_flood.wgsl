// Computes, for every voxel of a clipmap level, the nearest occupied voxel.
//
// Jump flooding spreads the nearest-seed information in passes of shrinking step size.
// Running each step along one axis at a time needs 2 reads per voxel per pass rather than
// the 26 of the full 3D neighbourhood, at the cost of slight inaccuracy far from surfaces.
// Far distances only ever set how far sphere tracing may leap, so that is acceptable.

struct Params {
    resolution: u32,
    level_offset: u32,
    // Axis of this pass as (1,0,0), (0,1,0) or (0,0,1), scaled by the step length.
    step_x: i32,
    step_y: i32,
    step_z: i32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
}

const NO_SEED: u32 = 0xffffffffu;

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> voxels: array<u32>;
@group(0) @binding(2) var<storage, read> seeds_in: array<u32>;
@group(0) @binding(3) var<storage, read_write> seeds_out: array<u32>;

fn index_of(c: vec3u) -> u32 {
    let n = params.resolution;
    return c.z * n * n + c.y * n + c.x;
}

fn coord_of(index: u32) -> vec3f {
    let n = params.resolution;
    return vec3f(f32(index % n), f32((index / n) % n), f32(index / (n * n)));
}

// Marks each occupied voxel as its own seed.
@compute @workgroup_size(4, 4, 4)
fn seed(@builtin(global_invocation_id) id: vec3u) {
    let n = params.resolution;
    if any(id >= vec3u(n)) {
        return;
    }
    let index = index_of(id);
    let occupied = (voxels[(index + params.level_offset) * 3u] & 0xff000000u) != 0u;
    seeds_out[index] = select(NO_SEED, index, occupied);
}

@compute @workgroup_size(4, 4, 4)
fn flood(@builtin(global_invocation_id) id: vec3u) {
    let n = i32(params.resolution);
    let c = vec3i(id);
    if any(c >= vec3i(n)) {
        return;
    }
    let step = vec3i(params.step_x, params.step_y, params.step_z);
    let here = vec3f(c);
    var best = seeds_in[index_of(id)];
    var best_distance = 1.0e20;
    if best != NO_SEED {
        let delta = coord_of(best) - here;
        best_distance = dot(delta, delta);
    }
    for (var side = -1; side <= 1; side += 2) {
        let neighbor = c + step * side;
        if any(neighbor < vec3i(0)) || any(neighbor >= vec3i(n)) {
            continue;
        }
        let candidate = seeds_in[index_of(vec3u(neighbor))];
        if candidate != NO_SEED {
            let delta = coord_of(candidate) - here;
            let distance = dot(delta, delta);
            if distance < best_distance {
                best = candidate;
                best_distance = distance;
            }
        }
    }
    seeds_out[index_of(id)] = best;
}
