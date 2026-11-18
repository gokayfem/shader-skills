// Compresses each probe's rays into spherical harmonics, then shares results between
// neighbouring probes on the same surface to reduce noise.

#include "common/environment.wgsl"
#include "gi/probe_common.wgsl"

@group(0) @binding(0) var normal_tex: texture_2d<f32>;
@group(0) @binding(1) var depth_tex: texture_2d<f32>;
@group(0) @binding(2) var traced: texture_2d<f32>;
@group(0) @binding(3) var<storage, read> probes_in: array<Probe>;
@group(0) @binding(4) var<storage, read_write> probes_out: array<Probe>;

fn sh_basis(d: vec3f) -> array<f32, 9> {
    return array<f32, 9>(
        0.282095,
        0.488603 * d.y,
        0.488603 * d.z,
        0.488603 * d.x,
        1.092548 * d.x * d.y,
        1.092548 * d.y * d.z,
        0.315392 * (3.0 * d.z * d.z - 1.0),
        1.092548 * d.x * d.z,
        0.546274 * (d.x * d.x - d.y * d.y),
    );
}

@compute @workgroup_size(8, 8)
fn project(@builtin(global_invocation_id) id: vec3u) {
    let grid = probe_grid_size(frame.viewport);
    if any(id.xy >= grid) {
        return;
    }
    let tile = id.xy;
    let slot = tile.y * grid.x + tile.x;
    var probe: Probe;

    let pixel = probe_pixel(tile, frame.viewport, frame.frame_index);
    let depth = textureLoad(depth_tex, vec2i(pixel), 0).r;
    if depth <= 0.0 {
        probe.position = vec4f(0.0);
        probes_out[slot] = probe;
        return;
    }
    let normal = oct_decode(textureLoad(normal_tex, vec2i(pixel), 0).xy);
    probe.position = vec4f(reconstruct_world(frame, vec2f(pixel) + vec2f(0.5), depth), 1.0);
    probe.normal = vec4f(normal, 0.0);

    // The rays cover the upper hemisphere only. Projecting that alone would make the
    // harmonics ring strongly below the horizon, so mirror each sample into the lower
    // hemisphere carrying the average radiance. A pixel whose bumped normal leans away from
    // the probe's normal then reads something sensible.
    var mean = vec3f(0.0);
    var valid_rays = 0.0;
    for (var y = 0u; y < PROBE_RAYS; y++) {
        for (var x = 0u; x < PROBE_RAYS; x++) {
            let texel = tile * PROBE_RAYS + vec2u(x, y);
            let radiance = textureLoad(traced, vec2i(texel), 0).rgb;
            if finite3(radiance) {
                mean += radiance;
                valid_rays += 1.0;
            }
        }
    }
    let ray_count = f32(PROBE_RAYS * PROBE_RAYS);
    mean /= max(valid_rays, 1.0);

    // Each ray stands for an equal part of the 2 pi steradian hemisphere.
    let weight = TAU / ray_count;
    for (var y = 0u; y < PROBE_RAYS; y++) {
        for (var x = 0u; x < PROBE_RAYS; x++) {
            let texel = tile * PROBE_RAYS + vec2u(x, y);
            let sample = textureLoad(traced, vec2i(texel), 0).rgb;
            let radiance = select(mean, sample, finite3(sample));
            let direction = probe_ray_direction(tile, vec2u(x, y), normal, frame.frame_index);
            let mirrored = direction - 2.0 * dot(direction, normal) * normal;
            var basis_up = sh_basis(direction);
            var basis_down = sh_basis(mirrored);
            for (var i = 0; i < 9; i++) {
                let contribution = radiance * basis_up[i] + mean * basis_down[i];
                probe.sh[i] = vec4f(probe.sh[i].xyz + contribution * weight, 0.0);
            }
        }
    }
    probes_out[slot] = probe;
}

// Blends a probe with the eight around it, `step` tiles away, that lie on the same surface.
fn blend_with_neighbours(id: vec2u, step: i32) {
    let grid = probe_grid_size(frame.viewport);
    if any(id >= grid) {
        return;
    }
    let slot = id.y * grid.x + id.x;
    var probe = probes_in[slot];
    if probe.position.w <= 0.0 {
        probes_out[slot] = probe;
        return;
    }

    var total = 1.0;
    let plane_scale = 4.0 / max(length(probe.position.xyz - frame.camera_position), 1.0);
    for (var dy = -1; dy <= 1; dy++) {
        for (var dx = -1; dx <= 1; dx++) {
            if dx == 0 && dy == 0 {
                continue;
            }
            let c = vec2i(id) + vec2i(dx, dy) * step;
            if any(c < vec2i(0)) || any(c >= vec2i(grid)) {
                continue;
            }
            var other = probes_in[u32(c.y) * grid.x + u32(c.x)];
            if other.position.w <= 0.0 {
                continue;
            }
            let normal_weight = pow(max(dot(other.normal.xyz, probe.normal.xyz), 0.0), 8.0);
            let offset = other.position.xyz - probe.position.xyz;
            // Off either one's plane: two walls that meet share neither.
            let plane = max(abs(dot(offset, probe.normal.xyz)), abs(dot(offset, other.normal.xyz)));
            let weight = normal_weight * exp(-plane * plane_scale * 8.0);
            if weight > 1.0e-3 {
                for (var i = 0; i < 9; i++) {
                    probe.sh[i] += other.sh[i] * weight;
                }
                total += weight;
            }
        }
    }
    for (var i = 0; i < 9; i++) {
        probe.sh[i] /= total;
    }
    probes_out[slot] = probe;
}

// Two passes, the second reaching twice as far, come to a blend over seven probes a side for
// the cost of two over three. A room lit by a patch of sun that few rays find needs that many
// to be even.
@compute @workgroup_size(8, 8)
fn filter_spatial(@builtin(global_invocation_id) id: vec3u) {
    blend_with_neighbours(id.xy, 1);
}

@compute @workgroup_size(8, 8)
fn filter_wide(@builtin(global_invocation_id) id: vec3u) {
    blend_with_neighbours(id.xy, 2);
}
