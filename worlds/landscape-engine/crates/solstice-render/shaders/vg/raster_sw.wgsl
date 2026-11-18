// Software rasterization of small opaque clusters into the visibility buffer: depth and triangle
// ID.

#include "vg/raster_common.wgsl"
#include "vg/vis_write.wgsl"
#include "vg/cover_none.wgsl"

// The visibility buffer takes depth as it is.
fn depth_bias(a: vec3f, b: vec3f, c: vec3f) -> f32 {
    return 0.0;
}

#include "vg/raster_sw_body.wgsl"
