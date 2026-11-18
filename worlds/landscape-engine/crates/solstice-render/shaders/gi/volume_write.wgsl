// Writes one clipmap level of the sampled atlas from the compute-side buffers: distance
// to the nearest surface in alpha, and that surface's outgoing radiance in colour.
//
// Near-surface empty voxels copy the radiance of their nearest occupied voxel. Otherwise
// filtered lookups at a ray's hit point would average in the black of empty space, and
// surfaces would darken as rays approach them.

#include "gi/radiance_codec.wgsl"

struct Params {
    level: u32,
    resolution: u32,
    voxel_size: f32,
    _pad0: u32,
    wrap_origin: vec3i,
    _pad1: u32,
}

const NO_SEED: u32 = 0xffffffffu;
// Far enough for filtered lookups to read valid colour, in voxels.
const COLOR_BAND: f32 = 3.0;
const FAR_DISTANCE_VOXELS: f32 = 64.0;

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> seeds: array<u32>;
@group(0) @binding(2) var<storage, read> radiance: array<vec2u>;
@group(0) @binding(3) var atlas: texture_storage_3d<rgba16float, write>;

@compute @workgroup_size(4, 4, 4)
fn main(@builtin(global_invocation_id) id: vec3u) {
    let n = params.resolution;
    if any(id >= vec3u(n)) {
        return;
    }
    let level_offset = params.level * n * n * n;
    let seed = seeds[level_offset + id.z * n * n + id.y * n + id.x];

    var distance_voxels = FAR_DISTANCE_VOXELS;
    var color = vec3f(0.0);
    if seed != NO_SEED {
        let seed_coord = vec3u(seed % n, (seed / n) % n, seed / (n * n));
        distance_voxels = length(vec3f(seed_coord) - vec3f(id));
        if distance_voxels <= COLOR_BAND {
            let world = vec3i(seed_coord) + params.wrap_origin;
            let wrapped = vec3u((world % i32(n) + vec3i(i32(n))) % i32(n));
            color = decode_radiance(radiance[level_offset + wrapped.z * n * n + wrapped.y * n + wrapped.x]);
        }
    }
    let atlas_coord = vec3u(id.x, id.y, id.z + params.level * n);
    textureStore(atlas, atlas_coord, vec4f(color, distance_voxels * params.voxel_size));
}
