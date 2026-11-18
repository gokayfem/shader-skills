// The ocean's waves: the spectrum they are drawn from, and the grid of waves on each cascade's
// tile. Mirrors crates/solstice-render/src/ocean/spectrum.rs, which tests hold it to.

#include "common/math.wgsl"

// Texels across each cascade's tile. The renderer rewrites this line for lower quality tiers.
const SIZE: u32 = 256u;
const HALF_SIZE: u32 = SIZE / 2u;
const GRAVITY: f32 = 9.81;
const RIPPLE_WAVENUMBER: f32 = 370.0;
const RIPPLE_SPEED: f32 = 0.23;
const SWELL_PEAK_ENHANCEMENT: f32 = 6.0;
const SWELL_SPREAD: f32 = 8.0;
const REPEAT_SECONDS: f32 = 1000.0;

struct OceanParams {
    // Width of each cascade's tile, in metres.
    sizes: vec4f,
    // Wavenumbers where cascade 0 hands over to 1, and 1 to 2.
    bands: vec4f,
    // The wind sea, after Elfouhaily: wavenumber at the peak, inverse wave age, direction, and
    // the short waves' level.
    wind: vec4f,
    // The long waves' level, the peak's sharpening and width, and the peak waves' speed.
    shape: vec4f,
    // The swell, likewise.
    swell: vec4f,
    // How far through the repeat the sea is, 0..1; seconds since the last frame; choppiness.
    motion: vec4f,
    // Foam: the stretch of the surface below which it forms, how fast it forms, and how much
    // of it stays from one frame to the next.
    foam: vec4f,
    seed: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
}

fn jonswap_shape(omega: f32, peak: f32, enhancement: f32) -> f32 {
    if omega <= 0.0 {
        return 0.0;
    }
    let width = select(0.09, 0.07, omega <= peak);
    let r = exp(-(omega - peak) * (omega - peak) / (2.0 * width * width * peak * peak));
    let ratio = peak / omega;
    let ratio2 = ratio * ratio;
    return pow(enhancement, r) * exp(-1.25 * ratio2 * ratio2) / pow(omega, 5.0);
}

fn banner_spread(ratio: f32) -> f32 {
    var beta: f32;
    if ratio < 0.95 {
        beta = 2.61 * pow(ratio, 1.3);
    } else if ratio < 1.6 {
        beta = 2.28 * pow(ratio, -1.3);
    } else {
        beta = pow(10.0, -0.4 + 0.8393 * exp(-0.567 * log(ratio * ratio)));
    }
    return max(beta, 0.01);
}

fn spread(beta: f32, angle: f32) -> f32 {
    let sech = 1.0 / cosh(beta * angle);
    return beta / (2.0 * tanh(beta * PI)) * sech * sech;
}

fn wrap_angle(angle: f32) -> f32 {
    return angle + PI - TAU * floor((angle + PI) / TAU) - PI;
}

// Elfouhaily's curvature spectrum of the wind sea, B = k³·S(k): the long waves around the peak,
// and the short waves the wind's friction raises, cut off below the peak as in Bruneton et al.
// (see wind_curvature in spectrum.rs).
fn wind_curvature(params: OceanParams, k: f32) -> f32 {
    if k <= 0.0 {
        return 0.0;
    }
    let speed = sqrt(GRAVITY / k);
    let root = sqrt(k / params.wind.x);
    let below_peak = params.wind.x / k;
    let cutoff = exp(-1.25 * min(below_peak * below_peak, 80.0));
    let width = params.shape.z;
    let sharpening = exp(-(root - 1.0) * (root - 1.0) / (2.0 * width * width));
    let peak = cutoff * pow(params.shape.y, sharpening) * exp(-params.wind.y / sqrt(10.0) * (root - 1.0));
    let long = 0.5 * params.shape.x * params.shape.w / speed * peak;
    let scaled = k / RIPPLE_WAVENUMBER - 1.0;
    let ripples = exp(-0.25 * scaled * scaled);
    let short = 0.5 * params.wind.w * RIPPLE_SPEED / speed * ripples * cutoff;
    return long + short;
}

// The variance of the sea's height per unit area of wavenumber at `k`, in m^4.
fn spectrum(params: OceanParams, k: vec2f) -> f32 {
    let length_k = length(k);
    if length_k < 1.0e-6 {
        return 0.0;
    }
    let omega = sqrt(GRAVITY * length_k);
    let angle = atan2(k.y, k.x);
    let peak_omega = sqrt(GRAVITY * params.wind.x);
    let k2 = length_k * length_k;
    let wind = wind_curvature(params, length_k) / (k2 * k2)
        * spread(banner_spread(omega / peak_omega), wrap_angle(angle - params.wind.z));
    // The swell is a spectrum over frequency, carried over to wavenumber.
    let domega_dk = GRAVITY / (2.0 * omega);
    let swell = params.swell.x
        * jonswap_shape(omega, params.swell.y, SWELL_PEAK_ENHANCEMENT)
        * spread(SWELL_SPREAD, wrap_angle(angle - params.swell.z))
        * domega_dk
        / length_k;
    return wind + swell;
}

fn in_band(params: OceanParams, k: f32, cascade: u32) -> bool {
    switch cascade {
        case 0u: { return k > 0.0 && k < params.bands.x; }
        case 1u: { return k >= params.bands.x && k < params.bands.y; }
        default: { return k >= params.bands.y; }
    }
}

// Which wave a texel holds: (0, 0) at the middle of the tile.
fn wave_index(texel: vec2u) -> vec2i {
    return vec2i(texel) - vec2i(i32(HALF_SIZE));
}

fn wavenumber(params: OceanParams, n: vec2i, cascade: u32) -> vec2f {
    return vec2f(n) * (TAU / params.sizes[cascade]);
}

// Two independent standard normal numbers for a wave, by Box and Muller.
fn gaussian_pair(n: vec2i, cascade: u32, seed: u32) -> vec2f {
    let hash = pcg(bitcast<u32>(n.x) ^ pcg(bitcast<u32>(n.y) ^ pcg(cascade ^ seed)));
    let u1 = hash_to_unit(hash);
    let u2 = hash_to_unit(hash ^ 0x68bc21ebu);
    let radius = sqrt(-2.0 * log(1.0 - u1));
    let angle = TAU * u2;
    return radius * vec2f(cos(angle), sin(angle));
}

// A wave's angular frequency in steps of 2π / REPEAT_SECONDS, rounded so the sea repeats exactly.
fn frequency_steps(k: f32) -> f32 {
    return floor(sqrt(GRAVITY * k) * REPEAT_SECONDS / TAU + 0.5);
}

fn complex_mul(a: vec2f, b: vec2f) -> vec2f {
    return vec2f(a.x * b.x - a.y * b.y, a.x * b.y + a.y * b.x);
}

fn complex_conj(a: vec2f) -> vec2f {
    return vec2f(a.x, -a.y);
}
