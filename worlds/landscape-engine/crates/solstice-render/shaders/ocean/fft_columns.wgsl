// The inverse FFT down each column of every cascade, which finishes the transform, then the
// waves themselves: displacement, slopes, and the foam where crests fold. One workgroup per
// column.

#include "ocean/common.wgsl"
#include "ocean/fft.wgsl"

@group(0) @binding(0) var<uniform> params: OceanParams;
@group(0) @binding(1) var in_a: texture_2d_array<f32>;
@group(0) @binding(2) var in_b: texture_2d_array<f32>;
@group(0) @binding(3) var previous_foam: texture_2d_array<f32>;
// Displacement x, y, z, and ∂Dx/∂z.
@group(0) @binding(4) var displacement: texture_storage_2d_array<rgba16float, write>;
// Slopes ∂h/∂x and ∂h/∂z, then ∂Dx/∂x and ∂Dz/∂z.
@group(0) @binding(5) var derivatives: texture_storage_2d_array<rgba16float, write>;
@group(0) @binding(6) var foam: texture_storage_2d_array<r32float, write>;

fn finish(texel: vec2u, cascade: u32, a: vec4f, b: vec4f) {
    // The spectrum was centred on the tile, which flips the sign of every other texel.
    let flip = select(1.0, -1.0, ((texel.x + texel.y) & 1u) == 1u);
    // Each complex pair holds two real fields: the real part one, the imaginary part the other.
    let fields_a = a * flip;
    let fields_b = b * flip;
    textureStore(displacement, texel, cascade, fields_a);
    textureStore(derivatives, texel, cascade, fields_b);

    // How much the displaced surface stretches: below one it is squeezed, and where it would
    // fold over itself the crest breaks.
    let choppiness = params.motion.z;
    let stretch = (1.0 + choppiness * fields_b.z) * (1.0 + choppiness * fields_b.w)
        - choppiness * choppiness * fields_a.w * fields_a.w;
    let breaking = clamp((params.foam.x - stretch) * params.foam.y, 0.0, 1.0);
    let kept = textureLoad(previous_foam, texel, cascade, 0).r * params.foam.z;
    textureStore(foam, texel, cascade, vec4f(max(kept, breaking), 0.0, 0.0, 0.0));
}

@compute @workgroup_size(HALF_SIZE)
fn main(@builtin(local_invocation_index) thread: u32, @builtin(workgroup_id) group: vec3u) {
    let column = group.x;
    let cascade = group.y;
    let near = vec2u(column, thread);
    let far = vec2u(column, thread + HALF_SIZE);
    load_line(
        thread,
        textureLoad(in_a, near, cascade, 0),
        textureLoad(in_b, near, cascade, 0),
        textureLoad(in_a, far, cascade, 0),
        textureLoad(in_b, far, cascade, 0),
    );
    transform_line(thread);
    finish(near, cascade, line_a[thread], line_b[thread]);
    finish(far, cascade, line_a[thread + HALF_SIZE], line_b[thread + HALF_SIZE]);
}
