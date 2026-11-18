// Generates the noise that cloud shapes are carved from, once, on the GPU. Everything tiles, so
// a volume can repeat across the sky without a seam. After Schneider, "The Real-time Volumetric
// Cloudscapes of Horizon: Zero Dawn" (2015), and Hillaire's Unreal implementation.
//
// shape (128^3): R is Perlin-Worley, the billowy base shape. G, B and A are Worley noise of
//   rising frequency, which erode it into separate puffs.
// detail (32^3): R, G and B are Worley noise of rising frequency, which carve wisps and
//   cauliflower edges into the base shape.
// weather (512^2): R is where cloud can form, in clumps. G varies the kind of cloud from place to
//   place. B is long, fibrous streaks for cirrus.

#include "common/math.wgsl"

@group(0) @binding(0) var volume_out: texture_storage_3d<rgba8unorm, write>;
@group(0) @binding(1) var map_out: texture_storage_2d<rgba8unorm, write>;

fn hash(p: vec3i, seed: u32) -> u32 {
    var h = (u32(p.x) * 0x8da6b343u) ^ (u32(p.y) * 0xd8163841u) ^ (u32(p.z) * 0xcb1ab31fu) ^ (seed * 0x9e3779b1u);
    h ^= h >> 15u;
    h *= 0x2c1b3c6du;
    h ^= h >> 12u;
    h *= 0x297a2d39u;
    return h ^ (h >> 15u);
}

fn unit(h: u32) -> f32 {
    return f32(h >> 8u) / 16777216.0;
}

fn gradient(h: u32) -> vec3f {
    switch h % 12u {
        case 0u: { return vec3f(1.0, 1.0, 0.0); }
        case 1u: { return vec3f(-1.0, 1.0, 0.0); }
        case 2u: { return vec3f(1.0, -1.0, 0.0); }
        case 3u: { return vec3f(-1.0, -1.0, 0.0); }
        case 4u: { return vec3f(1.0, 0.0, 1.0); }
        case 5u: { return vec3f(-1.0, 0.0, 1.0); }
        case 6u: { return vec3f(1.0, 0.0, -1.0); }
        case 7u: { return vec3f(-1.0, 0.0, -1.0); }
        case 8u: { return vec3f(0.0, 1.0, 1.0); }
        case 9u: { return vec3f(0.0, -1.0, 1.0); }
        case 10u: { return vec3f(0.0, 1.0, -1.0); }
        default: { return vec3f(0.0, -1.0, -1.0); }
    }
}

fn wrap(c: vec3i, period: vec3i) -> vec3i {
    return ((c % period) + period) % period;
}

// Perlin noise in roughly -1..1, tiling every `period` cells along each axis.
fn perlin(p: vec3f, period: vec3i, seed: u32) -> f32 {
    let cell = vec3i(floor(p));
    let f = p - floor(p);
    let fade = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);
    var corners: array<f32, 8>;
    for (var i = 0; i < 8; i++) {
        let offset = vec3i(i & 1, (i >> 1) & 1, (i >> 2) & 1);
        let g = gradient(hash(wrap(cell + offset, period), seed));
        corners[i] = dot(g, f - vec3f(offset));
    }
    let x0 = mix(corners[0], corners[1], fade.x);
    let x1 = mix(corners[2], corners[3], fade.x);
    let x2 = mix(corners[4], corners[5], fade.x);
    let x3 = mix(corners[6], corners[7], fade.x);
    return mix(mix(x0, x1, fade.y), mix(x2, x3, fade.y), fade.z);
}

fn perlin_fbm(p: vec3f, period: vec3i, octaves: i32, seed: u32) -> f32 {
    var sum = 0.0;
    var amplitude = 0.5;
    var total = 0.0;
    var scale = 1;
    for (var i = 0; i < octaves; i++) {
        sum += amplitude * perlin(p * f32(scale), period * scale, seed + u32(i));
        total += amplitude;
        amplitude *= 0.5;
        scale *= 2;
    }
    return sum / total;
}

// Worley noise in 0..1: 1 at the middle of a cell, falling to 0 a cell away. Tiles every
// `period` cells.
fn worley(p: vec3f, period: i32, seed: u32) -> f32 {
    let cell = vec3i(floor(p));
    let f = p - floor(p);
    var nearest = 9.0;
    for (var z = -1; z <= 1; z++) {
        for (var y = -1; y <= 1; y++) {
            for (var x = -1; x <= 1; x++) {
                let offset = vec3i(x, y, z);
                let h = hash(wrap(cell + offset, vec3i(period)), seed);
                let point = vec3f(offset) + vec3f(unit(h), unit(h * 747796405u + 1u), unit(h * 2891336453u + 7u));
                nearest = min(nearest, length(point - f));
            }
        }
    }
    return clamp(1.0 - nearest, 0.0, 1.0);
}

// Three octaves of Worley noise, starting at `period` cells across the tile.
fn worley_fbm(uvw: vec3f, period: i32, seed: u32) -> f32 {
    return worley(uvw * f32(period), period, seed) * 0.625
        + worley(uvw * f32(period * 2), period * 2, seed + 1u) * 0.25
        + worley(uvw * f32(period * 4), period * 4, seed + 2u) * 0.125;
}

// Spreads a value whose 2nd and 98th percentiles are `low` and `high` over 0..1. Each kind of
// noise here bunches in a narrow band; measured by the cloud_noise example, these bands are
// stretched to fill the range, so that thresholds in the cloud model mean what they say.
fn spread(x: f32, low: f32, high: f32) -> f32 {
    return clamp((x - low) / (high - low), 0.0, 1.0);
}

@compute @workgroup_size(4, 4, 4)
fn shape(@builtin(global_invocation_id) id: vec3u) {
    let size = textureDimensions(volume_out);
    if any(id >= size) {
        return;
    }
    let uvw = (vec3f(id) + 0.5) / vec3f(size);
    // Billowy Perlin noise, pinched into puffs by Worley noise.
    let perlin_value = perlin_fbm(uvw * 4.0, vec3i(4), 6, 11u) * 0.5 + 0.5;
    let cells = worley_fbm(uvw, 4, 21u);
    let perlin_worley = clamp(remap(perlin_value, 0.0, 1.0, cells, 1.0), 0.0, 1.0);
    let value = vec4f(
        spread(perlin_worley, 0.58, 0.88),
        spread(worley_fbm(uvw, 4, 31u), 0.2, 0.74),
        spread(worley_fbm(uvw, 8, 41u), 0.2, 0.74),
        spread(worley_fbm(uvw, 16, 51u), 0.22, 0.74),
    );
    textureStore(volume_out, id, value);
}

@compute @workgroup_size(4, 4, 4)
fn detail(@builtin(global_invocation_id) id: vec3u) {
    let size = textureDimensions(volume_out);
    if any(id >= size) {
        return;
    }
    let uvw = (vec3f(id) + 0.5) / vec3f(size);
    textureStore(volume_out, id, vec4f(
        spread(worley_fbm(uvw, 2, 61u), 0.24, 0.74),
        spread(worley_fbm(uvw, 4, 71u), 0.24, 0.74),
        spread(worley_fbm(uvw, 8, 81u), 0.2, 0.72),
        1.0,
    ));
}

@compute @workgroup_size(8, 8)
fn weather(@builtin(global_invocation_id) id: vec3u) {
    let size = textureDimensions(map_out);
    if any(id.xy >= size) {
        return;
    }
    let uv = vec3f((vec2f(id.xy) + 0.5) / vec2f(size), 0.37);
    // Where cloud can form: irregular Perlin fields at several scales, gathered a little into
    // clumps by Worley cells. The cloud passes rank these values (clouds/coverage.wgsl), so only
    // their order matters, not their spread.
    let field = perlin_fbm(uv * vec3f(8.0, 8.0, 1.0), vec3i(8, 8, 1), 5, 91u) * 0.5 + 0.5;
    let clumps = worley(uv * vec3f(12.0, 12.0, 1.0), 12, 101u);
    let coverage = spread(field * 0.75 + clumps * 0.25, 0.32, 0.68);
    let kind = spread(perlin_fbm(uv * vec3f(3.0, 3.0, 1.0), vec3i(3, 3, 1), 3, 111u) * 0.5 + 0.5, 0.3, 0.7);
    // Cirrus: streaks drawn out along X, warped so they curve and fray.
    let warp = perlin_fbm(uv * vec3f(4.0, 4.0, 1.0), vec3i(4, 4, 1), 3, 121u);
    let streaks = perlin_fbm(vec3f(uv.x * 3.0, (uv.y + warp * 0.06) * 24.0, 0.37), vec3i(3, 24, 1), 5, 131u);
    let cirrus = clamp(streaks * 1.3 + 0.5, 0.0, 1.0) * (perlin_fbm(uv * vec3f(5.0, 5.0, 1.0), vec3i(5, 5, 1), 3, 141u) * 0.5 + 0.6);
    textureStore(map_out, id.xy, vec4f(coverage, kind, clamp(cirrus, 0.0, 1.0), 1.0));
}
