// Traces all probe rays. Each ray tries the screen's depth pyramid first, where geometry is exact and last
// frame's lighting is sharp, and on leaving it continues through the GI volume. Screen hits
// recover the contact detail that the volume's voxels are too coarse to represent.

#include "common/environment.wgsl"
#include "gi/probe_common.wgsl"
#include "gi/screen_trace.wgsl"
#include "gi/screen_history.wgsl"

@group(0) @binding(0) var normal_tex: texture_2d<f32>;
@group(0) @binding(1) var depth_tex: texture_2d<f32>;
@group(0) @binding(2) var scene_history: texture_2d<f32>;
@group(0) @binding(3) var output: texture_storage_2d<rgba16float, write>;
@group(0) @binding(4) var motion_tex: texture_2d<f32>;
@group(0) @binding(5) var depth_pyramid: texture_2d<f32>;

// Probe rays only need the screen near their origin: farther lighting comes from the volume,
// which is stable, while long screen rays flicker as surfaces enter and leave the view.
const SCREEN_DISTANCE: f32 = 8.0;
const SCREEN_STEPS: u32 = 32u;
const SCREEN_THICKNESS: f32 = 0.15;

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3u) {
    let dims = textureDimensions(output);
    if any(id.xy >= dims) {
        return;
    }
    let tile = id.xy / PROBE_RAYS;
    let ray = id.xy % PROBE_RAYS;

    let pixel = probe_pixel(tile, frame.viewport, frame.frame_index);
    let depth = textureLoad(depth_tex, vec2i(pixel), 0).r;
    if depth <= 0.0 {
        textureStore(output, id.xy, vec4f(0.0, 0.0, 0.0, -1.0));
        return;
    }
    let packed_normal = textureLoad(normal_tex, vec2i(pixel), 0);
    let normal = oct_decode(packed_normal.xy);
    let world = reconstruct_world(frame, vec2f(pixel) + vec2f(0.5), depth);
    let direction = probe_ray_direction(tile, ray, normal, frame.frame_index);

    let screen = trace_screen(world + normal * 0.03, direction, SCREEN_DISTANCE, SCREEN_STEPS, SCREEN_THICKNESS);
    var seen = vec4f(0.0);
    if screen.hit {
        seen = screen_hit_radiance(screen.uv);
    }
    let screen_weight = screen.weight * seen.a;
    if screen_weight >= 1.0 {
        textureStore(output, id.xy, vec4f(seen.rgb, screen.t));
        return;
    }

    // The volume is coarse near its surfaces, so start clear of the local voxel.
    let level = min(volume_level_at(world, 0u), VOLUME_LEVELS - 1u);
    let voxel = volume_params.levels[level].w;
    let origin = world + normal * voxel * 1.5;
    let hit = trace_volume(origin, direction, voxel * 0.5, 320.0, 40u);
    var radiance = select(miss_radiance(hit, direction, sky_radiance(direction)), hit.radiance, hit.hit);
    var t = select(1000.0, hit.t, hit.hit);
    if screen.hit {
        // Near the screen edge, blend towards the volume so probes do not pop.
        radiance = mix(radiance, seen.rgb, screen_weight);
        t = mix(t, screen.t, screen_weight);
    }
    textureStore(output, id.xy, vec4f(radiance, t));
}
