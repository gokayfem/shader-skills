// Traces each local light's shadow map: for every direction of its octahedral map, how far
// the light reaches through the GI volume before something blocks it, up to its range. A
// workgroup covers an 8×8 patch of one light's map. lighting/light_shadows_common.wgsl reads
// the maps.

#include "common/environment.wgsl"
#include "common/scene.wgsl"

@group(0) @binding(0) var<storage, read> lights: array<Light>;
@group(0) @binding(1) var light_shadows: texture_storage_2d_array<r32float, write>;

// Twin of `LIGHT_SHADOW_SIZE` in lighting.rs.
const LIGHT_SHADOW_SIZE: u32 = 64u;
// A light sits in or on its own fixture, which must not shadow everything: the trace starts
// this many voxels out, as the per-pixel trace this replaces stopped that short of the light.
const CLEAR_VOXELS: f32 = 2.5;
// Enough for a ray to skim along a surface in sub-voxel steps for several metres.
const STEPS: u32 = 48u;

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3u) {
    let light = lights[id.z];
    let direction = oct_decode((vec2f(id.xy) + 0.5) / f32(LIGHT_SHADOW_SIZE) * 2.0 - 1.0);
    // Beyond the volume nothing is known to block the light, and nothing is lit by it past
    // its range anyway.
    var reach = light.range;
    let level = volume_level_at(light.position, 0u);
    if level < VOLUME_LEVELS {
        let voxel = volume_params.levels[level].w;
        let hit = trace_volume_inwards(light.position, direction, voxel * CLEAR_VOXELS, light.range, STEPS);
        // A ray that ran out of steps while still inside the volume has seen nothing beyond
        // where it stopped, so the light is taken to stop there rather than leak through
        // whatever stands further on; one that left the volume has nothing left to meet.
        let stopped_inside = hit.t < light.range
            && volume_level_at(light.position + direction * hit.t, 0u) < VOLUME_LEVELS;
        if hit.hit || stopped_inside {
            reach = hit.t;
        }
    }
    textureStore(light_shadows, id.xy, i32(id.z), vec4f(reach, 0.0, 0.0, 0.0));
}
