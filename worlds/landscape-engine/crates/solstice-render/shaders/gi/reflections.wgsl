// Traces one glossy reflection ray per half-resolution pixel: first through the screen's depth
// pyramid for sharp detail, then through the GI volume for what the screen cannot see, then the sky.
// Very rough surfaces skip tracing, since diffuse GI already approximates them well.

#include "common/environment.wgsl"
#include "gi/screen_trace.wgsl"
#include "gi/screen_history.wgsl"
#include "lighting/specular_aa.wgsl"

@group(0) @binding(0) var normal_tex: texture_2d<f32>;
@group(0) @binding(1) var depth_tex: texture_2d<f32>;
@group(0) @binding(2) var motion_tex: texture_2d<f32>;
@group(0) @binding(3) var scene_history: texture_2d<f32>;
@group(0) @binding(4) var reflection_history: texture_2d<f32>;
@group(0) @binding(5) var output: texture_storage_2d<rgba16float, write>;
@group(0) @binding(6) var depth_pyramid: texture_2d<f32>;
// This frame's diffuse irradiance, from gi/probe_integrate.wgsl.
@group(0) @binding(7) var irradiance_tex: texture_2d<f32>;

const MAX_TRACED_ROUGHNESS: f32 = 0.65;
const SCREEN_DISTANCE: f32 = 200.0;
const SCREEN_STEPS: u32 = 64u;
const SCREEN_THICKNESS: f32 = 0.25;
const REFINE_ATTEMPTS: u32 = 6u;
// How many times the step past a hit the camera sees through doubles: half a voxel, then one,
// two and four, and four from then on.
const REFINE_MOST_DOUBLINGS: u32 = 3u;
// The smoothest a surface is taken to be when it reflects the GI volume.
const VOLUME_MIN_ROUGHNESS: f32 = 0.25;
// How many times what has built up at a pixel one sample may be, on a smooth and a rough surface.
const SMOOTH_REACH: f32 = 16.0;
const ROUGH_REACH: f32 = 3.0;
// The share of itself a sample is always allowed, so that a history in complete darkness can
// still brighten. Very small: one ray in many finds the sky through a window, two thousand
// times brighter than the room, and even a five hundredth of that is a spark.
const FIREFLY_SHARE: f32 = 0.0003;
// How many times the light arriving at a point from all around a sample may add at once. That
// light is what a reflection there is made of, so a room lit dimly has dim reflections, give or
// take what one direction holds over the average. No more than that average and a half: a
// polished door mirrors nearly all of what its sample holds, and several times the light
// around it is a white dot on it. What is truly brighter builds up over a few frames.
const AMBIENT_REACH: f32 = 1.5;
// How close to a surface a reflection's ray counts as having hit it, in voxels.
const HIT_VOXELS: f32 = 0.55;

// Samples the GGX distribution of visible normals (Heitz 2018).
fn sample_vndf(view_local: vec3f, a: f32, u: vec2f) -> vec3f {
    let vh = normalize(vec3f(a * view_local.x, a * view_local.y, view_local.z));
    let len2 = vh.x * vh.x + vh.y * vh.y;
    let t1 = select(vec3f(1.0, 0.0, 0.0), vec3f(-vh.y, vh.x, 0.0) / sqrt(len2), len2 > 1.0e-7);
    let t2 = cross(vh, t1);
    let r = sqrt(u.x);
    let phi = TAU * u.y;
    let p1 = r * cos(phi);
    let s = 0.5 * (1.0 + vh.z);
    let p2 = (1.0 - s) * sqrt(max(1.0 - p1 * p1, 0.0)) + s * r * sin(phi);
    let nh = t1 * p1 + t2 * p2 + vh * sqrt(max(1.0 - p1 * p1 - p2 * p2, 0.0));
    return normalize(vec3f(a * nh.x, a * nh.y, max(nh.z, 0.0)));
}

// What a ray is already beside where it starts, if anything. The trace takes a ray to hit only
// what it is closing in on, which the first place it looks cannot tell. So a ray from the foot
// of a step, starting among the voxels of the floor and below the middle of them, sees itself
// only leaving them: it passes through the floor and out under the scene, and brings back the
// sky. The ray's own surface is a voxel and a half behind it, so this is another.
fn met_at_once(origin: vec3f, direction: vec3f, start: f32) -> VolumeHit {
    let p = origin + direction * start;
    let level = volume_level_at(p, 0u);
    if level < VOLUME_LEVELS {
        let s = sample_volume(p, level);
        if s.a < volume_params.levels[level].w * HIT_VOXELS {
            return VolumeHit(true, start, s.rgb, 0.0);
        }
    }
    return VolumeHit(false, start, vec3f(0.0), 1.0);
}

// Traces the volume, then checks each hit against what the camera can see. Voxels are coarse, so
// thin geometry such as rigging swells into solid blobs. Where the camera sees past a hit point,
// that point is really empty, and the ray carries on. Where the camera sees a surface there, its
// colour replaces the voxel's, since it is sharper and fully shaded.
fn trace_refined(origin: vec3f, direction: vec3f, start: f32) -> VolumeHit {
    var from_t = start;
    var seen_past = VolumeHit(false, start, vec3f(0.0), 1.0);
    for (var attempt = 0u; attempt < REFINE_ATTEMPTS; attempt++) {
        var hit = met_at_once(origin, direction, start);
        if attempt > 0u || !hit.hit {
            hit = trace_volume_with(origin, direction, from_t, 320.0, 48u, HIT_VOXELS);
        }
        if !hit.hit {
            return hit;
        }
        let point = origin + direction * hit.t;
        let p = project_to_screen(point);
        if any(p.xy < vec2f(0.0)) || any(p.xy >= vec2f(1.0)) || p.z <= 0.0 {
            return hit;
        }
        let level = min(volume_level_at(point, 0u), VOLUME_LEVELS - 1u);
        let margin = volume_params.levels[level].w * 1.5;
        let pixel = min(vec2i(p.xy * frame.viewport), vec2i(frame.viewport) - vec2i(1));
        let scene_depth = textureLoad(depth_tex, pixel, 0).r;
        let ray_distance = frame.near / p.z;
        if scene_depth <= 0.0 || frame.near / scene_depth > ray_distance + margin {
            // Further each time: a ray that skims a floor stays within the floor's swollen
            // voxels for a long way, and short steps would all end inside them.
            seen_past = hit;
            from_t = hit.t + margin * 0.33 * f32(1u << min(attempt, REFINE_MOST_DOUBLINGS));
            continue;
        }
        if frame.near / scene_depth < ray_distance - margin {
            return hit; // Something nearer hides this point, so the voxel is all there is.
        }
        let surface = oct_decode(textureLoad(normal_tex, pixel, 0).xy);
        if dot(surface, direction) >= 0.0 {
            return hit;
        }
        let edge = min(min(p.x, 1.0 - p.x), min(p.y, 1.0 - p.y));
        let seen = screen_hit_radiance(p.xy);
        return VolumeHit(true, hit.t, mix(hit.radiance, seen.rgb, seen.a * smoothstep(0.0, SCREEN_EDGE_FADE, edge)), 0.0);
    }
    // Every hit was somewhere the camera sees past, so the ray is still skimming the voxels of
    // something. That thing's light is a far better guess than the sky's, which it would be
    // given as a miss: indoors that paints daylight along every glossy floor.
    return seen_past;
}

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3u) {
    let dims = textureDimensions(output);
    if any(id.xy >= dims) {
        return;
    }
    // Each output pixel covers a 2x2 block. Sample a different corner per frame.
    let corner = vec2u(frame.frame_index & 1u, (frame.frame_index >> 1u) & 1u);
    let pixel = min(id.xy * 2u + corner, vec2u(frame.viewport) - vec2u(1u));
    let depth = textureLoad(depth_tex, vec2i(pixel), 0).r;
    if depth <= 0.0 {
        textureStore(output, id.xy, vec4f(0.0));
        return;
    }
    let packed = textureLoad(normal_tex, vec2i(pixel), 0);
    let normal = oct_decode(packed.xy);
    // As the lighting pass takes it, so that the two agree on what is too rough to trace.
    let roughness = max(antialiased_roughness(normal_tex, depth_tex, vec2i(pixel), normal, depth, packed.z), 0.045);
    if roughness > MAX_TRACED_ROUGHNESS {
        textureStore(output, id.xy, vec4f(0.0));
        return;
    }
    let center = vec2f(pixel) + vec2f(0.5);
    let world = reconstruct_world(frame, center, depth);
    let view = normalize(frame.camera_position - world);

    // Build the reflection direction in the normal's tangent frame.
    let basis = orthonormal_basis(normal);
    let view_local = transpose(basis) * view;
    let noise = vec2f(
        interleaved_gradient_noise(vec2f(id.xy), frame.frame_index),
        interleaved_gradient_noise(vec2f(id.yx) + vec2f(47.0, 17.0), frame.frame_index),
    );
    let half_vector = basis * sample_vndf(view_local, roughness * roughness, noise);
    var direction = reflect(-view, half_vector);
    if dot(direction, normal) <= 0.0 {
        direction = reflect(-view, normal);
    }

    let screen = trace_screen(world + normal * 0.02, direction, SCREEN_DISTANCE, SCREEN_STEPS, SCREEN_THICKNESS);
    var radiance = vec3f(0.0);
    var screen_weight = 0.0;
    if screen.hit {
        let seen = screen_hit_radiance(screen.uv);
        radiance = seen.rgb;
        screen_weight = screen.weight * seen.a;
    }
    if screen_weight < 1.0 {
        let level = min(volume_level_at(world, 0u), VOLUME_LEVELS - 1u);
        let voxel = volume_params.levels[level].w;
        // Voxels are coarse, so a sharp reflection of them shows their blocks. A smooth surface
        // sees them spread as a rougher one would.
        if roughness < VOLUME_MIN_ROUGHNESS {
            let spread = basis * sample_vndf(view_local, VOLUME_MIN_ROUGHNESS * VOLUME_MIN_ROUGHNESS, noise);
            let wider = reflect(-view, spread);
            if dot(wider, normal) > 0.0 {
                direction = wider;
            }
        }
        let hit = trace_refined(world + normal * voxel * 1.5, direction, voxel * 0.5);
        let sky = sky_radiance(toward_horizon(direction));
        // Fade by where the ray ended up. A ray leaving the volume before reaching something sees
        // only sky, so hits near the edge must already be blending towards that, or the edge shows
        // as a hard line across glossy surfaces.
        let hit_point = world + direction * hit.t;
        let found = select(miss_radiance(hit, direction, sky), hit.radiance, hit.hit);
        let far = mix(sky, found, volume_edge_fade(hit_point) * volume_edge_fade(world));
        radiance = mix(far, radiance, screen_weight);
    }

    // Fireflies would otherwise persist through the temporal filter. At night the limit stays
    // where exposure stops brightening, or a dim or absent moon would black out the sky and
    // lamps seen in reflections.
    let peak = max(radiance.x, max(radiance.y, radiance.z));
    let limit = max(frame.sun_illuminance, frame.daylight_illuminance / 128.0) * 4.0;
    if peak > limit {
        radiance *= limit / peak;
    }

    var result = vec4f(radiance, 1.0);
    let motion = textureLoad(motion_tex, vec2i(pixel), 0).xy;
    let previous_uv = center * frame.inv_viewport + motion;
    var history = vec4f(0.0);
    var remembered = all(previous_uv > vec2f(0.0)) && all(previous_uv < vec2f(1.0)) && frame.frame_index > 0u;
    if remembered {
        history = textureSampleLevel(reflection_history, clamp_sampler, previous_uv, 0.0);
        if !finite4(history) {
            history = vec4f(0.0);
            remembered = false;
        }
    }
    {
        // A rough surface's rays scatter, and one in many finds something far brighter than the
        // rest: the sky through a gap, a sunlit patch. Such a sample is held to a few times what
        // has built up here, plus the light around it and a small share of itself, so that a
        // dark history can still brighten. A smooth surface's rays agree with each other, and
        // are trusted further. A pixel seen for the first time has nothing built up, and is held
        // the same way: it would otherwise start as a spark and take many frames to fade.
        let known = select(0.0, luminance(history.rgb / max(history.a, 1.0e-4)), history.a > 0.5);
        // What the screen shows is exact. The volume is coarse, with gaps where walls are
        // thin, so what comes from it or from the sky beyond is trusted as a rough
        // surface's rays are, however smooth this one is.
        let smooth_reach = mix(ROUGH_REACH, SMOOTH_REACH, screen_weight);
        let reach = mix(smooth_reach, ROUGH_REACH, smoothstep(0.08, 0.4, roughness));
        let brightest = max(result.x, max(result.y, result.z));
        let ambient = luminance(textureLoad(irradiance_tex, vec2i(pixel), 0).rgb) * frame.gi_intensity / PI;
        let allowed = known * reach + ambient * AMBIENT_REACH + brightest * FIREFLY_SHARE;
        if brightest > allowed {
            result = vec4f(result.rgb * (allowed / brightest), result.a);
        }
    }
    if remembered {
        // Rough reflections are noisier and depend less on the view, so they keep more history.
        let blend = frame_rate_blend(mix(0.25, 0.07, clamp(roughness / MAX_TRACED_ROUGHNESS, 0.0, 1.0)), frame.frame_seconds);
        let moved = length(motion * frame.viewport);
        // The motion term and its cap are per frame, so how fast history gives way under
        // motion still depends on the frame rate.
        result = mix(history, result, max(blend, min(blend + moved * 0.04, 0.6)));
    }
    textureStore(output, id.xy, result);
}
