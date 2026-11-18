// Software rasterization of small cut-out clusters into a depth map: for the shadow cascades.

#include "vg/raster_common.wgsl"
#include "vg/depth_write.wgsl"
#include "vg/mask.wgsl"
#include "vg/cover_mask.wgsl"
#include "vg/raster_sw_body.wgsl"
