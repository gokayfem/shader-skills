// Produces final per-pixel indirect diffuse and ambient occlusion: interpolates the nearest
// probes using the pixel's own normal, adds screen-space occlusion for detail smaller than
// the probe grid can see, and accumulates over time.

#include "common/environment.wgsl"
#include "gi/probe_common.wgsl"

@group(0) @binding(0) var normal_tex: texture_2d<f32>;
@group(0) @binding(1) var depth_tex: texture_2d<f32>;
@group(0) @binding(2) var motion_tex: texture_2d<f32>;
@group(0) @binding(3) var history_tex: texture_2d<f32>;
@group(0) @binding(4) var<storage, read> probes: array<Probe>;
@group(0) @binding(5) var output: texture_storage_2d<rgba16float, write>;
// Each pixel's distance from the eye along the view, kept with its history: last frame's to
// read, and this frame's to write.
@group(0) @binding(6) var history_depth_tex: texture_2d<f32>;
@group(0) @binding(7) var depth_output: texture_storage_2d<r32float, write>;

// Irradiance from harmonics towards `n`: radiance convolved with the cosine lobe.
fn sh_irradiance(probe: Probe, n: vec3f) -> vec3f {
    let a0 = PI;
    let a1 = TAU / 3.0;
    let a2 = PI * 0.25;
    var e = probe.sh[0].xyz * 0.282095 * a0;
    e += (probe.sh[1].xyz * n.y + probe.sh[2].xyz * n.z + probe.sh[3].xyz * n.x) * 0.488603 * a1;
    e += (probe.sh[4].xyz * n.x * n.y * 1.092548
        + probe.sh[5].xyz * n.y * n.z * 1.092548
        + probe.sh[6].xyz * (3.0 * n.z * n.z - 1.0) * 0.315392
        + probe.sh[7].xyz * n.x * n.z * 1.092548
        + probe.sh[8].xyz * (n.x * n.x - n.y * n.y) * 0.546274) * a2;
    return max(e, vec3f(0.0));
}

fn linear_distance(depth: f32) -> f32 {
    return frame.near / max(depth, 1.0e-7);
}

// How far apart, as a share of the distance to the eye, a pixel and last frame's pixel it
// reprojects to may lie and still be the same surface, besides the slope of the surface across a
// pixel. Blades of grass a hand's width apart are told apart a few metres away.
const HISTORY_DEPTH_TOLERANCE: f32 = 0.01;

// How far the surface's distance from the eye changes per pixel across and down: taken from
// the side where it changes least, and none where both sides jump, as on a blade of grass a pixel
// or two wide with the ground behind it on either side.
fn depth_gradient(pixel: vec2i, distance: f32) -> vec2f {
    let last = vec2i(frame.viewport) - vec2i(1);
    var gradient = vec2f(0.0);
    for (var axis = 0; axis < 2; axis++) {
        var along = vec2i(0);
        along[axis] = 1;
        // Off the screen counts as no surface, as the sky does: the pixel itself would say flat.
        let before_pixel = pixel - along;
        let after_pixel = pixel + along;
        let before = textureLoad(depth_tex, clamp(before_pixel, vec2i(0), last), 0).r;
        let after = textureLoad(depth_tex, clamp(after_pixel, vec2i(0), last), 0).r;
        let back = select(1.0e9, distance - linear_distance(before), before > 0.0 && before_pixel[axis] >= 0);
        let ahead = select(1.0e9, linear_distance(after) - distance, after > 0.0 && after_pixel[axis] <= last[axis]);
        let change = select(ahead, back, abs(back) < abs(ahead));
        gradient[axis] = select(0.0, change, abs(change) < distance * SURFACE_JUMP);
    }
    return gradient;
}

// A change in distance from one pixel to the next of more than this share of it is a jump from
// one surface to another, not the slope of one.
const SURFACE_JUMP: f32 = 0.1;

// How far off a surface's plane a probe may sit and still stand for it, as a share of the
// distance to the eye: the weight halves at about one part in a hundred. Any looser and light
// crosses a silhouette in blocks the size of a probe's tile, from the wall behind onto an arch
// in front of it.
const PLANE_SHARPNESS: f32 = 7000.0;
// Below this much weight among the four nearest probes, the ring of twelve around them is
// asked too.
const MATCH_FLOOR: f32 = 0.05;

// How well a probe stands for the surface at `world`: near one when each lies in the other's
// plane and they face the same way.
fn surface_weight(probe: Probe, world: vec3f, normal: vec3f, inv_distance: f32) -> f32 {
    let offset = probe.position.xyz - world;
    let from_pixel = dot(offset, normal);
    let from_probe = dot(offset, probe.normal.xyz);
    let off_plane = (from_pixel * from_pixel + from_probe * from_probe) * inv_distance * inv_distance;
    let facing = max(dot(probe.normal.xyz, normal) * 0.5 + 0.5, 0.0);
    return exp(-off_plane * PLANE_SHARPNESS) * facing * facing;
}

// Ground-truth-style occlusion in a horizon search along a few screen-space directions.
fn screen_occlusion(pixel: vec2f, world: vec3f, normal: vec3f, noise: f32) -> f32 {
    const DIRECTIONS: u32 = 3u;
    const STEPS: u32 = 4u;
    const RADIUS: f32 = 0.9;
    let distance = length(world - frame.camera_position);
    let pixel_radius = clamp(RADIUS / distance * frame.viewport.y * 0.8, 4.0, 90.0);
    let size = vec2i(frame.viewport);

    var occlusion = 0.0;
    for (var d = 0u; d < DIRECTIONS; d++) {
        let angle = (f32(d) + noise) / f32(DIRECTIONS) * PI;
        let direction = vec2f(cos(angle), sin(angle));
        for (var side = -1.0; side <= 1.0; side += 2.0) {
            var horizon = 0.0;
            for (var s = 1u; s <= STEPS; s++) {
                let offset = direction * side * pixel_radius * (f32(s) - 0.5 + noise * 0.5) / f32(STEPS);
                let sample_pixel = clamp(vec2i(pixel + offset), vec2i(0), size - vec2i(1));
                let depth = textureLoad(depth_tex, sample_pixel, 0).r;
                if depth <= 0.0 {
                    continue;
                }
                let p = reconstruct_world(frame, vec2f(sample_pixel) + vec2f(0.5), depth);
                let delta = p - world;
                let length_delta = length(delta);
                let elevation = dot(delta, normal) / max(length_delta, 1.0e-4);
                let falloff = clamp(1.0 - length_delta / RADIUS, 0.0, 1.0);
                horizon = max(horizon, elevation * falloff);
            }
            occlusion += horizon;
        }
    }
    return clamp(1.0 - occlusion / f32(DIRECTIONS * 2u) * 1.6, 0.0, 1.0);
}

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3u) {
    let size = vec2u(frame.viewport);
    if any(id.xy >= size) {
        return;
    }
    let pixel = vec2i(id.xy);
    let depth = textureLoad(depth_tex, pixel, 0).r;
    if depth <= 0.0 {
        textureStore(output, pixel, vec4f(0.0, 0.0, 0.0, 1.0));
        textureStore(depth_output, pixel, vec4f(0.0));
        return;
    }
    let center = vec2f(id.xy) + vec2f(0.5);
    let normal = oct_decode(textureLoad(normal_tex, pixel, 0).xy);
    let world = reconstruct_world(frame, center, depth);
    let grid = vec2i(probe_grid_size(frame.viewport));

    // Bilinear over the four nearest tiles, counting only probes on this pixel's surface.
    let position = center / f32(PROBE_SPACING) - vec2f(0.5);
    let base = vec2i(floor(position));
    let fraction = position - vec2f(base);
    let distance = linear_distance(depth);
    let inv_distance = 1.0 / max(distance, 0.1);

    var irradiance = vec3f(0.0);
    var total = 0.0;
    // The same four with almost no regard for the surface: what a pixel takes when no probe
    // near it lies on its surface, and nothing remembered says better.
    var loose = vec3f(0.0);
    var loose_total = 0.0;
    let loose_scale = 6.0 / max(distance, 1.0);
    for (var dy = 0; dy <= 1; dy++) {
        for (var dx = 0; dx <= 1; dx++) {
            let tile = clamp(base + vec2i(dx, dy), vec2i(0), grid - vec2i(1));
            let probe = probes[u32(tile.y * grid.x + tile.x)];
            if probe.position.w <= 0.0 {
                continue;
            }
            let bilinear = select(1.0 - fraction.x, fraction.x, dx == 1) * select(1.0 - fraction.y, fraction.y, dy == 1);
            let light = sh_irradiance(probe, normal);
            let weight = max(bilinear, 0.001) * surface_weight(probe, world, normal, inv_distance);
            irradiance += light * weight;
            total += weight;

            let plane = abs(dot(probe.position.xyz - world, normal));
            let facing = max(dot(probe.normal.xyz, normal) * 0.5 + 0.5, 0.0);
            let loose_weight = max(bilinear, 0.001) * exp(-plane * loose_scale) * facing * facing + 1.0e-5;
            loose += light * loose_weight;
            loose_total += loose_weight;
        }
    }
    // A pixel on something narrow, or just inside a silhouette, can have all four of its probes
    // on what lies behind. The ring around them is likely to hold one on its own surface.
    if total < MATCH_FLOOR {
        let wanted = 1.0 - total / MATCH_FLOOR;
        for (var dy = -1; dy <= 2; dy++) {
            for (var dx = -1; dx <= 2; dx++) {
                let offset = vec2i(dx, dy);
                if all(offset >= vec2i(0)) && all(offset <= vec2i(1)) {
                    continue;
                }
                let tile = base + offset;
                if any(tile < vec2i(0)) || any(tile >= grid) {
                    continue;
                }
                let probe = probes[u32(tile.y * grid.x + tile.x)];
                if probe.position.w <= 0.0 {
                    continue;
                }
                let away = length(vec2f(offset) - fraction);
                let weight = wanted * max(1.0 - away / 2.9, 0.02) * 0.5
                    * surface_weight(probe, world, normal, inv_distance);
                irradiance += sh_irradiance(probe, normal) * weight;
                total += weight;
            }
        }
    }
    // How far this frame's probes can be trusted for this pixel.
    let matched = clamp(total / MATCH_FLOOR, 0.0, 1.0);
    if loose_total > 0.0 {
        loose /= loose_total;
    }
    irradiance = mix(loose, irradiance / max(total, 1.0e-12), clamp(total * 1.0e6, 0.0, 1.0));

    let noise = interleaved_gradient_noise(vec2f(id.xy), frame.frame_index);
    let occlusion = screen_occlusion(center, world, normal, noise);
    var current = vec4f(irradiance, occlusion);

    // Accumulate over time wherever the previous frame saw the same surface. Where it saw
    // something else, as where a swaying blade of grass or a passing trunk uncovers what was
    // behind it, its light is not this surface's: by day the sun outshines the mistake, but at
    // dusk and by night this light is most of what is seen, and it would trail behind
    // everything that moves on screen.
    textureStore(depth_output, pixel, vec4f(distance));
    let motion = textureLoad(motion_tex, pixel, 0).xy;
    let previous_uv = center * frame.inv_viewport + motion;
    if all(previous_uv > vec2f(0.0)) && all(previous_uv < vec2f(1.0)) && frame.frame_index > 0u {
        // The four texels around where this point was, each counted only if it held a surface
        // as far away as this point was then.
        let expected = (frame.prev_view_proj * vec4f(world, 1.0)).w;
        let tolerance = expected * HISTORY_DEPTH_TOLERANCE;
        let gradient = depth_gradient(pixel, distance);
        let position = previous_uv * frame.viewport - vec2f(0.5);
        let corner = vec2i(floor(position));
        let part = position - vec2f(corner);
        let last = vec2i(frame.viewport) - vec2i(1);
        var history = vec4f(0.0);
        var found = 0.0;
        for (var dy = 0; dy <= 1; dy++) {
            for (var dx = 0; dx <= 1; dx++) {
                let texel = clamp(corner + vec2i(dx, dy), vec2i(0), last);
                let remembered = textureLoad(history_depth_tex, texel, 0).r;
                // Where this surface would lie at that texel's centre, a fraction of a pixel away.
                let there = expected + dot(gradient, vec2f(texel) + vec2f(0.5) - previous_uv * frame.viewport);
                if abs(remembered - there) <= tolerance {
                    let sample = textureLoad(history_tex, texel, 0);
                    // A single NaN otherwise grows into a larger patch every frame through
                    // this four-tap gather, even after its original source disappears.
                    if !finite4(sample) {
                        continue;
                    }
                    let side = select(vec2f(1.0) - part, part, vec2i(dx, dy) == vec2i(1));
                    let weight = max(side.x * side.y, 1.0e-3);
                    history += sample * weight;
                    found += weight;
                }
            }
        }
        if found > 0.0 {
            history /= found;
            // A moving view exposes new surfaces, which need history to fade out faster.
            let moved = length(motion * frame.viewport);
            let settled = frame_rate_blend(0.08, frame.frame_seconds);
            // The motion term and its cap are per frame, so how fast history gives way under
            // motion still depends on the frame rate.
            let blend = max(settled, min(settled + moved * 0.03, 0.5));
            // Probes move within their tiles from frame to frame, so a pixel none of them
            // matched now was matched a few frames ago: it keeps more of what it had. Occlusion
            // is measured at the pixel itself and needs no probe.
            let light_blend = blend * mix(0.2, 1.0, matched);
            current = vec4f(mix(history.rgb, current.rgb, light_blend), mix(history.a, current.a, blend));
        }
    }
    textureStore(output, pixel, current);
}
