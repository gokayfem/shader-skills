// How the software rasterizer writes a depth map: for the shadow cascades, whose pixel-sized
// foliage clusters the hardware rasterizer draws slowly. Depth alone is kept, as an atomic on
// each pixel, and vg/shadow_merge.wgsl then lays it over the cascade's own depth.

// Per pixel: the view's stamp in the top byte, over 24 bits of reversed depth, so that the
// nearest sample of the current drawing has the largest value and anything an earlier
// drawing left behind loses to it. The buffer is not cleared per drawing, only once each time
// the stamps start a new round (shadow.rs); the merge ignores other stamps.
@group(1) @binding(0) var<storage, read_write> shadow_depth: array<atomic<u32>>;

#include "vg/depth_stamp.wgsl"

// Keeps the nearest depth. The view is orthographic with depth running 0 at the light's side
// to 1 at the far side, so it is reversed here, and anything past either side is left out.
fn write_visibility(index: u32, depth: f32, id: u32) {
    let reversed = 1.0 - depth;
    if depth < 0.0 || reversed <= 0.0 {
        return;
    }
    atomicMax(&shadow_depth[index], stamped(view.stamp, reversed));
}

// The same slope-scaled bias as the hardware shadow pipeline's, in depth per pixel of the
// steepest direction, so that a surface does not shadow itself where a texel spans a long
// stretch of it. Twin of the `slope_scale` in shadow.rs.
const SLOPE_BIAS: f32 = 1.5;

fn depth_bias(a: vec3f, b: vec3f, c: vec3f) -> f32 {
    let ab = b - a;
    let ac = c - a;
    // Never zero: the rasterizer has already dropped triangles with no area. A triangle almost
    // edge-on to the light gets a huge bias and is pushed past the far side, as the hardware
    // does too, clamping to 1 and failing its depth test.
    let det = ab.x * ac.y - ac.x * ab.y;
    let dzdx = (ab.z * ac.y - ac.z * ab.y) / det;
    let dzdy = (ac.z * ab.x - ab.z * ac.x) / det;
    return SLOPE_BIAS * max(abs(dzdx), abs(dzdy));
}
