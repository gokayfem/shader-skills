// Moves every wave on to this moment, and makes the spectra of the eight fields the waves are
// drawn from: the displacement in x, y and z, its cross derivative, the two slopes, and the two
// other derivatives that say how much the surface stretches. Each field is real across the tile,
// so two fit in one complex number, A + iB, and one inverse transform gives both.

#include "ocean/common.wgsl"

@group(0) @binding(0) var<uniform> params: OceanParams;
@group(0) @binding(1) var amplitudes: texture_2d_array<f32>;
@group(0) @binding(2) var out_a: texture_storage_2d_array<rgba32float, write>;
@group(0) @binding(3) var out_b: texture_storage_2d_array<rgba32float, write>;

// A + iB, for two real fields A and B.
fn pair(a: vec2f, b: vec2f) -> vec2f {
    return a + vec2f(-b.y, b.x);
}

@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) id: vec3u) {
    if any(id.xy >= vec2u(SIZE)) {
        return;
    }
    let cascade = id.z;
    let n = wave_index(id.xy);
    let k = wavenumber(params, n, cascade);
    let length_k = length(k);

    // h(k, t) = h0(k) e^{iωt} + conj(h0(-k)) e^{-iωt}, which keeps the height real.
    let amplitude = textureLoad(amplitudes, id.xy, cascade, 0).xy;
    let mirror = (vec2u(SIZE) - id.xy) % vec2u(SIZE);
    let mirrored = textureLoad(amplitudes, mirror, cascade, 0).xy;
    let phase = TAU * fract(frequency_steps(length_k) * params.motion.x);
    let turn = vec2f(cos(phase), sin(phase));
    let h = complex_mul(amplitude, turn) + complex_mul(complex_conj(mirrored), complex_conj(turn));

    // Displacement towards each crest, i·(k/|k|)·h, sharpens crests and widens troughs.
    let inv_k = select(0.0, 1.0 / length_k, length_k > 0.0);
    let dx = complex_mul(vec2f(0.0, k.x * inv_k), h);
    let dz = complex_mul(vec2f(0.0, k.y * inv_k), h);
    let dx_dz = -h * (k.x * k.y * inv_k);
    let slope_x = complex_mul(vec2f(0.0, k.x), h);
    let slope_z = complex_mul(vec2f(0.0, k.y), h);
    let dx_dx = -h * (k.x * k.x * inv_k);
    let dz_dz = -h * (k.y * k.y * inv_k);

    textureStore(out_a, id.xy, cascade, vec4f(pair(dx, h), pair(dz, dx_dz)));
    textureStore(out_b, id.xy, cascade, vec4f(pair(slope_x, slope_z), pair(dx_dx, dz_dz)));
}
