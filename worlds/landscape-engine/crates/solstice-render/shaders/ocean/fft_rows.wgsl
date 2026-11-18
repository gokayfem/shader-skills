// The inverse FFT along each row of every cascade: one workgroup per row.

#include "ocean/common.wgsl"
#include "ocean/fft.wgsl"

@group(0) @binding(0) var in_a: texture_2d_array<f32>;
@group(0) @binding(1) var in_b: texture_2d_array<f32>;
@group(0) @binding(2) var out_a: texture_storage_2d_array<rgba32float, write>;
@group(0) @binding(3) var out_b: texture_storage_2d_array<rgba32float, write>;

@compute @workgroup_size(HALF_SIZE)
fn main(@builtin(local_invocation_index) thread: u32, @builtin(workgroup_id) group: vec3u) {
    let row = group.x;
    let cascade = group.y;
    let near = vec2u(thread, row);
    let far = vec2u(thread + HALF_SIZE, row);
    load_line(
        thread,
        textureLoad(in_a, near, cascade, 0),
        textureLoad(in_b, near, cascade, 0),
        textureLoad(in_a, far, cascade, 0),
        textureLoad(in_b, far, cascade, 0),
    );
    transform_line(thread);
    textureStore(out_a, near, cascade, line_a[thread]);
    textureStore(out_b, near, cascade, line_b[thread]);
    textureStore(out_a, far, cascade, line_a[thread + HALF_SIZE]);
    textureStore(out_b, far, cascade, line_b[thread + HALF_SIZE]);
}
