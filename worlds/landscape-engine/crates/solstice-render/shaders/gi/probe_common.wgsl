// Screen-space radiance probes. Lighting varies slowly across a surface, so instead of
// tracing from every pixel, rays are traced from a sparse grid of probes placed on visible
// geometry, and each pixel interpolates between the nearest ones.

// Each probe covers this many pixels on a side.
const PROBE_SPACING: u32 = 16u;
// Each probe traces this many rays on a side. Set per pipeline from the quality setting.
override PROBE_RAYS: u32 = 8u;

struct Probe {
    // Spherical harmonics, order 2, RGB in xyz.
    sh: array<vec4f, 9>,
    // World position, and w > 0 when the probe landed on geometry.
    position: vec4f,
    normal: vec4f,
}

fn probe_grid_size(viewport: vec2f) -> vec2u {
    return (vec2u(viewport) + vec2u(PROBE_SPACING - 1u)) / PROBE_SPACING;
}

// The pixel a probe sits on this frame. Moving it within its tile each frame means that,
// over time, probes sample every part of the surface.
fn probe_pixel(tile: vec2u, viewport: vec2f, frame_index: u32) -> vec2u {
    let seed = tile.x * 7919u + tile.y * 104729u + (frame_index % 16u) * 31337u;
    let jitter = vec2u(pcg(seed) % PROBE_SPACING, pcg(seed + 1u) % PROBE_SPACING);
    return min(tile * PROBE_SPACING + jitter, vec2u(viewport) - vec2u(1u));
}

// Maps [0,1]^2 onto the unit hemisphere around +Z with even area.
fn hemisphere_direction(uv: vec2f, basis: mat3x3f) -> vec3f {
    let cos_theta = 1.0 - uv.x;
    let sin_theta = sqrt(max(1.0 - cos_theta * cos_theta, 0.0));
    let phi = TAU * uv.y;
    return basis * vec3f(sin_theta * cos(phi), sin_theta * sin(phi), cos_theta);
}

// A probe's ray direction. A random rotation per frame and a stratified jitter per ray make
// the fixed ray count act like a much larger one once temporal filtering averages it.
fn probe_ray_direction(tile: vec2u, ray: vec2u, normal: vec3f, frame_index: u32) -> vec3f {
    let seed = tile.x * 2957u + tile.y * 6113u + frame_index * 409u;
    let jitter = vec2f(
        hash_to_unit(seed + ray.x * 17u + ray.y * 257u),
        hash_to_unit(seed + ray.x * 131u + ray.y * 7u + 1u),
    );
    let rotation = hash_to_unit(seed + 3u);
    var uv = (vec2f(ray) + jitter) / f32(PROBE_RAYS);
    uv.y = fract(uv.y + rotation);
    return hemisphere_direction(uv, orthonormal_basis(normal));
}
