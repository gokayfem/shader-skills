// Draws each wave's starting amplitude from the spectrum. Runs only when the sea changes: the
// waves keep their random phases, so a changing wind grows and calms them rather than replacing
// them.

#include "ocean/common.wgsl"

@group(0) @binding(0) var<uniform> params: OceanParams;
@group(0) @binding(1) var amplitudes: texture_storage_2d_array<rg32float, write>;

@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) id: vec3u) {
    if any(id.xy >= vec2u(SIZE)) {
        return;
    }
    let cascade = id.z;
    let n = wave_index(id.xy);
    let k = wavenumber(params, n, cascade);
    var amplitude = vec2f(0.0);
    // The Nyquist row and column have no mirror on the tile, so they carry nothing.
    let half = i32(HALF_SIZE);
    if n.x != -half && n.y != -half && in_band(params, length(k), cascade) {
        let dk = TAU / params.sizes[cascade];
        let scale = sqrt(spectrum(params, k) * dk * dk / 4.0);
        amplitude = gaussian_pair(n, cascade, params.seed) * scale;
    }
    textureStore(amplitudes, id.xy, cascade, vec4f(amplitude, 0.0, 0.0));
}
