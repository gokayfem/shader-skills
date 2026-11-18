// One line of the inverse FFT, in workgroup memory: radix 2, in place, on a line loaded in
// bit-reversed order. Each line holds four complex numbers per texel, in two vec4s: the eight
// fields, paired. SIZE / 2 threads run each line, one butterfly each per stage.
//
// Needs ocean/common.wgsl.

var<workgroup> line_a: array<vec4f, SIZE>;
var<workgroup> line_b: array<vec4f, SIZE>;

// Both complex numbers in `v` times `w`.
fn turn_pair(v: vec4f, w: vec2f) -> vec4f {
    return vec4f(complex_mul(v.xy, w), complex_mul(v.zw, w));
}

// Where element `index` goes for the in-place transform.
fn bit_reversed(index: u32) -> u32 {
    return reverseBits(index) >> (32u - countTrailingZeros(SIZE));
}

// Loads two elements of a line, `thread` and `thread + HALF_SIZE`, in bit-reversed order.
fn load_line(thread: u32, a0: vec4f, b0: vec4f, a1: vec4f, b1: vec4f) {
    let first = bit_reversed(thread);
    let second = bit_reversed(thread + HALF_SIZE);
    line_a[first] = a0;
    line_b[first] = b0;
    line_a[second] = a1;
    line_b[second] = b1;
    workgroupBarrier();
}

// The inverse transform of the loaded line: sums of value · e^{+2πi·n·x/SIZE}.
fn transform_line(thread: u32) {
    let stages = countTrailingZeros(SIZE);
    for (var stage = 0u; stage < stages; stage++) {
        let half = 1u << stage;
        let position = thread & (half - 1u);
        let first = (thread - position) * 2u + position;
        let second = first + half;
        let angle = TAU * f32(position) / f32(half * 2u);
        let w = vec2f(cos(angle), sin(angle));
        let a = turn_pair(line_a[second], w);
        let b = turn_pair(line_b[second], w);
        let a0 = line_a[first];
        let b0 = line_b[first];
        line_a[first] = a0 + a;
        line_a[second] = a0 - a;
        line_b[first] = b0 + b;
        line_b[second] = b0 - b;
        workgroupBarrier();
    }
}
