// Generates the three atmosphere lookup tables. Transmittance and multiple scattering
// depend only on the atmosphere, so they are built once. The sky view depends on the
// sun and viewer radius, and is rebuilt when either changes. Radiance is per unit of sun illuminance.

#include "common/math.wgsl"
#include "common/atmosphere.wgsl"

struct SkyParams {
    sun_direction: vec3f,
    viewer_radius: f32,
}

@group(0) @binding(0) var<uniform> params: SkyParams;
@group(0) @binding(1) var transmittance_in: texture_2d<f32>;
@group(0) @binding(2) var multi_scatter_in: texture_2d<f32>;
@group(0) @binding(3) var lut_sampler: sampler;
@group(0) @binding(4) var output: texture_storage_2d<rgba16float, write>;

fn optical_depth_to_top(origin: vec3f, direction: vec3f) -> vec3f {
    if ray_sphere(origin, direction, GROUND_RADIUS) > 0.0 {
        return vec3f(1.0e9);
    }
    let length = ray_sphere(origin, direction, TOP_RADIUS);
    if length <= 0.0 {
        return vec3f(0.0);
    }
    const STEPS: i32 = 40;
    var depth = vec3f(0.0);
    let dt = length / f32(STEPS);
    for (var i = 0; i < STEPS; i++) {
        let p = origin + direction * (f32(i) + 0.5) * dt;
        depth += sample_medium(sqrt(dot(p, p)) - GROUND_RADIUS).extinction * dt;
    }
    return depth;
}

@compute @workgroup_size(8, 8)
fn build_transmittance(@builtin(global_invocation_id) id: vec3u) {
    let size = textureDimensions(output);
    if any(id.xy >= size) {
        return;
    }
    let uv = (vec2f(id.xy) + vec2f(0.5)) / vec2f(size);
    let params_rm = transmittance_params(uv);
    let origin = vec3f(0.0, params_rm.x, 0.0);
    let direction = vec3f(sqrt(max(1.0 - params_rm.y * params_rm.y, 0.0)), params_rm.y, 0.0);
    let t = exp(-optical_depth_to_top(origin, direction));
    textureStore(output, id.xy, vec4f(t, 1.0));
}

fn sample_transmittance(p: vec3f, sun: vec3f) -> vec3f {
    let r = length(p);
    let mu = dot(p / r, sun);
    return textureSampleLevel(transmittance_in, lut_sampler, transmittance_uv(r, mu), 0.0).rgb;
}

const GROUND_ALBEDO: vec3f = vec3f(0.3);

// Second-order scattering towards a point, and how much of a uniform input it re-emits.
// Their ratio gives the sum of all higher orders as a geometric series (Hillaire, eq. 10).
@compute @workgroup_size(8, 8)
fn build_multi_scatter(@builtin(global_invocation_id) id: vec3u) {
    let size = textureDimensions(output);
    if any(id.xy >= size) {
        return;
    }
    let uv = (vec2f(id.xy) + vec2f(0.5)) / vec2f(size);
    let cos_sun = uv.x * 2.0 - 1.0;
    let radius = mix(GROUND_RADIUS + 0.01, TOP_RADIUS - 0.01, uv.y);
    let origin = vec3f(0.0, radius, 0.0);
    let sun = vec3f(sqrt(max(1.0 - cos_sun * cos_sun, 0.0)), cos_sun, 0.0);

    const SQRT_DIRECTIONS: u32 = 8u;
    const STEPS: i32 = 20;
    var second_order = vec3f(0.0);
    var transfer = vec3f(0.0);
    for (var a = 0u; a < SQRT_DIRECTIONS; a++) {
        for (var b = 0u; b < SQRT_DIRECTIONS; b++) {
            // Uniform directions over the sphere.
            let u = (f32(a) + 0.5) / f32(SQRT_DIRECTIONS);
            let v = (f32(b) + 0.5) / f32(SQRT_DIRECTIONS);
            let cos_theta = 1.0 - 2.0 * u;
            let sin_theta = sqrt(max(1.0 - cos_theta * cos_theta, 0.0));
            let phi = TAU * v;
            let direction = vec3f(sin_theta * cos(phi), cos_theta, sin_theta * sin(phi));

            var t_max = ray_sphere(origin, direction, TOP_RADIUS);
            let ground_t = ray_sphere(origin, direction, GROUND_RADIUS);
            let hits_ground = ground_t > 0.0;
            if hits_ground {
                t_max = ground_t;
            }
            if t_max <= 0.0 {
                continue;
            }
            let dt = t_max / f32(STEPS);
            var through = vec3f(1.0);
            var radiance = vec3f(0.0);
            var scatter_transfer = vec3f(0.0);
            for (var i = 0; i < STEPS; i++) {
                let p = origin + direction * (f32(i) + 0.5) * dt;
                let medium = sample_medium(length(p) - GROUND_RADIUS);
                let scattering = medium.rayleigh + vec3f(medium.mie);
                let step_transmittance = exp(-medium.extinction * dt);
                let integrate = (vec3f(1.0) - step_transmittance) / medium.extinction;

                let in_sun = sample_transmittance(p, sun);
                // Isotropic phase: this LUT assumes scattering has lost its direction.
                radiance += through * scattering * in_sun / (4.0 * PI) * integrate;
                scatter_transfer += through * scattering * integrate;
                through *= step_transmittance;
            }
            if hits_ground {
                let ground_point = origin + direction * t_max;
                let n = normalize(ground_point);
                radiance += through * sample_transmittance(ground_point, sun)
                    * max(dot(n, sun), 0.0) * GROUND_ALBEDO / PI;
            }
            second_order += radiance;
            transfer += scatter_transfer;
        }
    }
    let count = f32(SQRT_DIRECTIONS * SQRT_DIRECTIONS);
    second_order /= count;
    transfer /= count;
    textureStore(output, id.xy, vec4f(second_order / (vec3f(1.0) - transfer), 1.0));
}

fn sample_multi_scatter(p: vec3f, sun: vec3f) -> vec3f {
    let r = length(p);
    let cos_sun = dot(p / r, sun);
    let uv = vec2f(cos_sun * 0.5 + 0.5, (r - GROUND_RADIUS) / (TOP_RADIUS - GROUND_RADIUS));
    return textureSampleLevel(multi_scatter_in, lut_sampler, uv, 0.0).rgb;
}

@compute @workgroup_size(8, 8)
fn build_sky_view(@builtin(global_invocation_id) id: vec3u) {
    let size = textureDimensions(output);
    if any(id.xy >= size) {
        return;
    }
    let uv = (vec2f(id.xy) + vec2f(0.5)) / vec2f(size);
    let direction = sky_view_direction(uv);
    let origin = vec3f(0.0, params.viewer_radius, 0.0);
    let sun = params.sun_direction;

    var t_max = ray_sphere(origin, direction, TOP_RADIUS);
    let ground_t = ray_sphere(origin, direction, GROUND_RADIUS);
    if ground_t > 0.0 {
        t_max = ground_t;
    }

    const STEPS: i32 = 32;
    let cos_theta = dot(direction, sun);
    let phase_r = rayleigh_phase(cos_theta);
    let phase_m = mie_phase(cos_theta);
    var through = vec3f(1.0);
    var radiance = vec3f(0.0);
    // Quadratic spacing clusters samples near the viewer, where most in-scatter happens.
    var previous_t = 0.0;
    for (var i = 0; i < STEPS; i++) {
        let f = (f32(i) + 0.5) / f32(STEPS);
        let t = f * f * t_max;
        let dt = t - previous_t;
        previous_t = t;
        let p = origin + direction * t;
        let medium = sample_medium(length(p) - GROUND_RADIUS);
        let step_transmittance = exp(-medium.extinction * dt);
        let integrate = (vec3f(1.0) - step_transmittance) / medium.extinction;

        let in_sun = sample_transmittance(p, sun);
        let single = medium.rayleigh * phase_r + vec3f(medium.mie * phase_m);
        let scattering = medium.rayleigh + vec3f(medium.mie);
        let multi = sample_multi_scatter(p, sun) * scattering;
        radiance += through * (single * in_sun + multi) * integrate;
        through *= step_transmittance;
    }
    // Below the horizon the ray ends on the planet, which should look like lit ground and
    // not black. This is what shows past the edge of a scene that has no terrain of its own.
    if ground_t > 0.0 {
        let p = origin + direction * t_max;
        let lit = max(dot(normalize(p), sun), 0.0);
        radiance += through * sample_transmittance(p, sun) * lit * GROUND_ALBEDO / PI;
    }
    textureStore(output, id.xy, vec4f(radiance, 1.0));
}
