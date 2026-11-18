// Which local lights can reach each 16×16 tile of the screen, so that the lighting pass loops
// over those alone rather than every light in the scene. A light reaches a tile if its sphere
// of range meets the tile's frustum: the four planes through the camera and the tile's edges,
// cut to the nearest and farthest depth the tile holds.
//
// One workgroup per tile, one thread per pixel: the threads find the tile's depth range
// together, then share the lights out between them for the test.

#include "common/environment.wgsl"
#include "common/scene.wgsl"

@group(0) @binding(0) var depth_tex: texture_2d<f32>;
@group(0) @binding(1) var<storage, read> lights: array<Light>;
// Per tile: how many lights reach it, then their indices. See `light_tile_lights` below.
@group(0) @binding(2) var<storage, read_write> tiles: array<u32>;

#include "lighting/light_tiles_common.wgsl"

var<workgroup> nearest_bits: atomic<u32>;
var<workgroup> farthest_bits: atomic<u32>;
var<workgroup> found: atomic<u32>;
var<workgroup> list: array<u32, TILE_MAX_LIGHTS>;

// A plane through the camera and the two far points, facing the inside of the tile's frustum,
// which is on the side of `inside`.
fn side_plane(a: vec3f, b: vec3f, inside: vec3f) -> vec4f {
    var normal = normalize(cross(a - frame.camera_position, b - frame.camera_position));
    if dot(normal, inside - frame.camera_position) < 0.0 {
        normal = -normal;
    }
    return vec4f(normal, -dot(normal, frame.camera_position));
}

@compute @workgroup_size(TILE_SIZE, TILE_SIZE)
fn main(
    @builtin(workgroup_id) group: vec3u,
    @builtin(global_invocation_id) id: vec3u,
    @builtin(local_invocation_index) lane: u32,
) {
    if lane == 0u {
        // Reversed depth: nearer is larger, and the sky is 0. Positive floats order as their
        // bits do, so the atomics work on the bits.
        atomicStore(&nearest_bits, 0u);
        atomicStore(&farthest_bits, 0xffffffffu);
        atomicStore(&found, 0u);
    }
    workgroupBarrier();

    let size = vec2u(frame.viewport);
    if all(id.xy < size) {
        let depth = textureLoad(depth_tex, vec2i(id.xy), 0).r;
        if depth > 0.0 {
            atomicMax(&nearest_bits, bitcast<u32>(depth));
            atomicMin(&farthest_bits, bitcast<u32>(depth));
        }
    }
    workgroupBarrier();

    let nearest = bitcast<f32>(atomicLoad(&nearest_bits));
    let farthest = bitcast<f32>(atomicLoad(&farthest_bits));
    let base = light_tile_base(group.xy, size);
    // Nothing but sky: no light reaches it.
    let empty = nearest <= 0.0;
    if !empty {
        // The tile's frustum: its corner pixels, pushed out to a far depth, bound the sides; the
        // depth range, along the camera's axis, bounds the ends.
        let corner = vec2f(group.xy * TILE_SIZE);
        let far_depth = 1.0e-5;
        let c00 = reconstruct_world(frame, corner, far_depth);
        let c10 = reconstruct_world(frame, corner + vec2f(f32(TILE_SIZE), 0.0), far_depth);
        let c01 = reconstruct_world(frame, corner + vec2f(0.0, f32(TILE_SIZE)), far_depth);
        let c11 = reconstruct_world(frame, corner + vec2f(f32(TILE_SIZE)), far_depth);
        let middle = (c00 + c11) * 0.5;
        let planes = array<vec4f, 4>(
            side_plane(c00, c10, middle),
            side_plane(c10, c11, middle),
            side_plane(c11, c01, middle),
            side_plane(c01, c00, middle),
        );
        // The camera looks along -z in view space.
        let forward = -vec3f(frame.view[0].z, frame.view[1].z, frame.view[2].z);
        let centre = corner + vec2f(f32(TILE_SIZE) * 0.5);
        let near_depth = dot(reconstruct_world(frame, centre, nearest) - frame.camera_position, forward);
        let far_depth_along = dot(reconstruct_world(frame, centre, farthest) - frame.camera_position, forward);

        for (var i = lane; i < frame.light_count; i += TILE_SIZE * TILE_SIZE) {
            let light = lights[i];
            let along = dot(light.position - frame.camera_position, forward);
            var reaches = along + light.range >= near_depth && along - light.range <= far_depth_along;
            for (var p = 0u; p < 4u && reaches; p++) {
                reaches = dot(planes[p].xyz, light.position) + planes[p].w >= -light.range;
            }
            if reaches {
                let slot = atomicAdd(&found, 1u);
                if slot < TILE_MAX_LIGHTS {
                    list[slot] = i;
                }
            }
        }
    }
    workgroupBarrier();

    let count = min(atomicLoad(&found), TILE_MAX_LIGHTS);
    if lane == 0u {
        tiles[base] = count;
    }
    for (var i = lane; i < count; i += TILE_SIZE * TILE_SIZE) {
        tiles[base + 1u + i] = list[i];
    }
}
