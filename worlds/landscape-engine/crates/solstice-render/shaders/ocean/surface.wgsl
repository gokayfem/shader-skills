// The sea's surface, drawn into the water layer: patches of a grid fixed in the world, finer near
// the camera. Each vertex blends onto the coarser grid by its distance, then rises and shifts
// with the waves, which shrink as the water over the seabed shoals. The colour target keeps where
// each pixel's water lay on the calm sea, from which the water pass reads the waves' slopes. Mirrors crates/solstice-render/src/ocean/grid.rs.

#include "common/frame.wgsl"
#include "ocean/surface_params.wgsl"

@group(0) @binding(0) var<uniform> frame: Frame;
@group(0) @binding(1) var<uniform> surface: SurfaceParams;
@group(0) @binding(2) var displacement: texture_2d_array<f32>;
@group(0) @binding(3) var wave_sampler: sampler;
@group(0) @binding(4) var seabed_map: texture_depth_2d;

#include "ocean/seabed.wgsl"

struct VertexOutput {
    @builtin(position) position: vec4f,
    // Where this point lay on the calm sea, in metres.
    @location(0) calm: vec2f,
}

@vertex
fn vs_main(
    @location(0) grid: vec2u,
    @location(1) origin: vec2f,
    @location(2) size: f32,
    @location(3) level_and_cells: vec2u,
) -> VertexOutput {
    let level = level_and_cells.x;
    let cells = f32(level_and_cells.y);
    let spacing = size / cells;
    let g = vec2f(grid);
    let sea_level = surface.sizes.w;

    // Odd vertices slide onto their even neighbours as they near the end of their level's range,
    // so that where this patch meets a coarser one, both hold the same points.
    let unmorphed = origin + g * spacing;
    let distance = length(vec3f(unmorphed.x, sea_level, unmorphed.y) - frame.camera_position);
    let end = surface.grid.w * surface.grid.z * f32(1u << level);
    let start = end * surface.morph.x;
    let morph = clamp((distance - start) / (end - start), 0.0, 1.0);
    let odd = g - floor(g * 0.5) * 2.0;
    let calm = origin + (g - odd * morph) * spacing;

    var offset = vec3f(0.0);
    for (var cascade = 0u; cascade < 3u; cascade++) {
        let tile = surface.sizes[cascade];
        // The level whose texels are as far apart as the vertices: smaller waves would only
        // alias between them. The spacing blends to the coarser grid's with the position, so
        // where levels meet both read the same heights (grid.rs, `vertex`).
        let level_of_detail = max(log2(spacing * (1.0 + morph) / (tile / surface.grid.x)), 0.0);
        offset += textureSampleLevel(displacement, wave_sampler, calm / tile, cascade, level_of_detail).xyz;
    }
    offset = shoal(offset, water_depth_at(calm));
    let choppiness = surface.grid.y;
    let world = vec3f(calm.x + choppiness * offset.x, sea_level + offset.y, calm.y + choppiness * offset.z);

    var out: VertexOutput;
    out.position = frame.view_proj * vec4f(world, 1.0);
    out.calm = calm;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4f {
    return vec4f(in.calm, 0.0, 0.0);
}
