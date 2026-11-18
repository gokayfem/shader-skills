// The light each particle receives, worked out once per particle by particles/light.wgsl and
// applied by particles/draw.wgsl. Each colour is one word in the shared-exponent RGB9E5 form,
// which keeps about three significant figures across the whole range of light, using only
// integer operations:
//   a: sun, ambient, local lights, the particle's spawn index
//   b: haze, glow, (transmittance as 16 bits, samples), the frame it was last lit
// The spawn index says whose light it is: a new particle in a reused slot starts afresh. A
// slice given to a new emitter has its light cleared, so that no stale light matches.

struct ParticleLight {
    a: vec4<u32>,
    b: vec4<u32>,
}

struct Received {
    // Sunlight arriving, after the atmosphere, the clouds and the shadows: irradiance.
    sun: vec3f,
    // The average radiance arriving from every direction: sky and light bounced off the scene.
    ambient: vec3f,
    // Light from the scene's local lights, as the radiance a white surface facing them would
    // give: irradiance over pi.
    lamps: vec3f,
    // The air's own glow between the particle and the eye, and how much of the particle shows
    // through the air.
    haze: vec3f,
    transmittance: f32,
    // How many samples the ambient light has averaged, up to a limit.
    samples: f32,
    // Light the particle gives off itself, from its temperature.
    glow: vec3f,
    index: u32,
    // The frame it was last lit in.
    lit_frame: u32,
    // A number in 0..1 that rounds the ambient light up or down when it is stored, so that
    // small changes average out rather than stall on its precision. Not stored.
    dither: f32,
}

// The largest value RGB9E5 holds.
const RGB9E5_MAX: f32 = 65408.0;

fn pack_rgb9e5(color: vec3f) -> u32 {
    return pack_rgb9e5_rounding(color, 0.5);
}

// `bias` is added before truncating: 0.5 rounds to nearest, a random number in 0..1 rounds
// up or down in proportion, so that values between steps keep their average.
fn pack_rgb9e5_rounding(color: vec3f, bias: f32) -> u32 {
    let c = clamp(color, vec3f(0.0), vec3f(RGB9E5_MAX));
    let largest = max(max(c.x, c.y), max(c.z, 1.0e-9));
    // The shared exponent, biased by 15, so that the largest channel fills nine bits.
    var exponent = max(-16, i32(floor(log2(largest)))) + 16;
    var unit = exp2(f32(exponent - 24));
    if floor(largest / unit + 0.5) >= 512.0 {
        exponent += 1;
        unit *= 2.0;
    }
    let m = vec3<u32>(min(floor(c / unit + bias), vec3f(511.0)));
    return m.x | (m.y << 9u) | (m.z << 18u) | (u32(exponent) << 27u);
}

fn unpack_rgb9e5(v: u32) -> vec3f {
    let unit = exp2(f32(i32(v >> 27u) - 24));
    return vec3f(f32(v & 511u), f32((v >> 9u) & 511u), f32((v >> 18u) & 511u)) * unit;
}

fn pack_light(l: Received) -> ParticleLight {
    var out: ParticleLight;
    out.a = vec4<u32>(
        pack_rgb9e5(l.sun),
        pack_rgb9e5_rounding(l.ambient, l.dither),
        pack_rgb9e5(l.lamps),
        l.index,
    );
    let through = u32(round(clamp(l.transmittance, 0.0, 1.0) * 65535.0));
    let samples = u32(clamp(l.samples, 0.0, 65535.0));
    out.b = vec4<u32>(pack_rgb9e5(l.haze), pack_rgb9e5(l.glow), through | (samples << 16u), l.lit_frame);
    return out;
}

fn unpack_light(p: ParticleLight) -> Received {
    var l: Received;
    l.sun = unpack_rgb9e5(p.a.x);
    l.ambient = unpack_rgb9e5(p.a.y);
    l.lamps = unpack_rgb9e5(p.a.z);
    l.index = p.a.w;
    l.haze = unpack_rgb9e5(p.b.x);
    l.glow = unpack_rgb9e5(p.b.y);
    l.transmittance = f32(p.b.z & 65535u) / 65535.0;
    l.samples = f32(p.b.z >> 16u);
    l.lit_frame = p.b.w;
    l.dither = 0.5;
    return l;
}

// Wyman, Sloan and Shirley's fit of the CIE 1931 colour matching functions (JCGT 2013).
fn lobe(x: f32, mean: f32, below: f32, above: f32) -> f32 {
    let t = (x - mean) / select(above, below, x < mean);
    return exp(-0.5 * t * t);
}

fn cie_xyz(nm: f32) -> vec3f {
    return vec3f(
        1.056 * lobe(nm, 599.8, 37.9, 31.0) + 0.362 * lobe(nm, 442.0, 16.0, 26.7)
            - 0.065 * lobe(nm, 501.1, 20.4, 26.2),
        0.821 * lobe(nm, 568.8, 46.9, 40.5) + 0.286 * lobe(nm, 530.9, 16.3, 31.1),
        1.217 * lobe(nm, 437.0, 11.8, 36.0) + 0.681 * lobe(nm, 459.0, 26.0, 13.8),
    );
}

const LUMINOUS_EFFICACY: f32 = 683.0;

// A black body's light at `kelvin` as CIE XYZ, with Y its luminance in candela per square
// metre: Planck's law integrated against the colour matching functions in 20 nm bands.
fn black_body_xyz(kelvin: f32) -> vec3f {
    if kelvin < 500.0 {
        return vec3f(0.0);
    }
    var xyz = vec3f(0.0);
    let step_nm = 20.0;
    for (var nm = 390.0; nm < 790.0; nm += step_nm) {
        let metres = nm * 1.0e-9;
        // 2hc² / λ⁵ / (exp(hc / λkT) - 1), in W / (m² sr m), times the band's width in metres.
        let exponent = 0.0143877735 / (metres * kelvin);
        let radiance = 1.191042972e-16 / (pow(metres, 5.0) * (exp(min(exponent, 80.0)) - 1.0));
        xyz += cie_xyz(nm) * radiance * step_nm * 1.0e-9;
    }
    return xyz * LUMINOUS_EFFICACY;
}

// The same as linear Rec. 709, with any colour outside it clipped to it.
fn black_body(kelvin: f32) -> vec3f {
    let xyz = black_body_xyz(kelvin);
    let rgb = vec3f(
        3.2406 * xyz.x - 1.5372 * xyz.y - 0.4986 * xyz.z,
        -0.9689 * xyz.x + 1.8758 * xyz.y + 0.0415 * xyz.z,
        0.0557 * xyz.x - 0.2040 * xyz.y + 1.0570 * xyz.z,
    );
    return max(rgb, vec3f(0.0));
}

// How much of the sun's light a particle of asymmetry `g` sends towards the eye, `cos_theta`
// being the cosine between the sunlight's travel and the line to the eye: Henyey and Greenstein's
// phase function, times 4 pi so that it averages 1 over every direction. A particle is taken as
// thick enough to scatter all it does not pass on, so with even scattering (g = 0) a white one
// in the sun is as bright as a white surface facing it.
fn phase(g: f32, cos_theta: f32) -> f32 {
    let g2 = g * g;
    return (1.0 - g2) / pow(max(1.0 + g2 - 2.0 * g * cos_theta, 1.0e-4), 1.5);
}
