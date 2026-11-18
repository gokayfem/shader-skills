// Updates the water-line map (ocean/waterline.rs) for one frame: where the water stands now at
// each spot, from the waves shrunk over the seabed exactly as the drawn surface is, and the
// highest it has stood there lately, which falls back towards it as the spot dries.

#include "ocean/surface_params.wgsl"
#include "ocean/waterline_params.wgsl"

@group(0) @binding(0) var<uniform> surface: SurfaceParams;
@group(0) @binding(1) var<uniform> waterline: WaterlineParams;
@group(0) @binding(2) var displacement: texture_2d_array<f32>;
@group(0) @binding(3) var wave_sampler: sampler;
@group(0) @binding(4) var seabed_map: texture_depth_2d;
// Last frame's map, and this frame's.
@group(0) @binding(5) var previous: texture_2d<f32>;
@group(0) @binding(6) var current: texture_storage_2d<rg32float, write>;

#include "ocean/seabed.wgsl"

// The waves' displacement of the calm sea at `calm`: every cascade at the level whose texels
// match the map's, shrunk over the seabed as the drawn surface is.
fn displaced(calm: vec2f) -> vec3f {
    var offset = vec3f(0.0);
    for (var cascade = 0u; cascade < 3u; cascade++) {
        let tile = surface.sizes[cascade];
        let level = max(log2(waterline.grid.x / (tile / surface.grid.x)), 0.0);
        offset += textureSampleLevel(displacement, wave_sampler, calm / tile, cascade, level).xyz;
    }
    return shoal(offset, water_depth_at(calm));
}

// The water's height over `xz` now, as a height in the world. The drawn surface carries each
// point of the calm sea sideways by the waves' sway, so the water over `xz` came from about
// `xz` less that sway: one step of that correction is enough at the map's 25 cm.
fn water_height(xz: vec2f) -> f32 {
    let choppiness = surface.grid.y;
    let first = displaced(xz);
    let source = xz - choppiness * first.xz;
    return surface.sizes.w + displaced(source).y;
}

// The world texel that the map texel `index` holds within this frame's window. Indices wrap, so
// one past either edge of the map is the texel at the window's other side.
fn world_texel(index: vec2f) -> vec2f {
    let first = waterline.window.xy;
    let n = waterline.grid.y;
    return first + (index - first - floor((index - first) / n) * n);
}

// The water's height at each texel of this workgroup's 8×8 and the ring of texels around it,
// worked out once and shared, as each texel looks at its neighbours'.
const TILE: u32 = 10u;
var<workgroup> tile: array<f32, 100>;

@compute @workgroup_size(8, 8)
fn main(
    @builtin(global_invocation_id) id: vec3u,
    @builtin(workgroup_id) group: vec3u,
    @builtin(local_invocation_index) local_index: u32,
) {
    let size = waterline.grid.x;
    // Heights in the world, not above the calm sea: when the sea falls, as a tide going out,
    // the line it reached stays behind and dries.
    let corner = vec2f(group.xy * 8u) - vec2f(1.0);
    for (var i = local_index; i < TILE * TILE; i += 64u) {
        let cell = corner + vec2f(f32(i % TILE), f32(i / TILE));
        tile[i] = water_height((world_texel(cell) + vec2f(0.5)) * size);
    }
    workgroupBarrier();

    let n = u32(waterline.grid.y);
    if any(id.xy >= vec2u(n)) {
        return;
    }
    let index = vec2f(id.xy);
    let texel = world_texel(index);
    let local = id.xy - group.xy * 8u + vec2u(1u);
    let now = tile[local.y * TILE + local.x];

    // What the water reaches here: the highest of the water here and beside it. Where this
    // texel holds a rock's face or a sea wall, the ground under it stands above the sea, so no
    // wave stands over it; the waves beside it break against the face and wet it as high as
    // they stand.
    var reaches = now;
    let sides = array<vec2i, 4>(vec2i(1, 0), vec2i(-1, 0), vec2i(0, 1), vec2i(0, -1));
    for (var s = 0u; s < 4u; s++) {
        let side = sides[s];
        var height: f32;
        // The neighbouring map texel holds the neighbouring spot, except across the seam where
        // the map wraps; there the spot beside is worked out on its own.
        if all(world_texel(index + vec2f(side)) == texel + vec2f(side)) {
            let at = vec2i(local) + side;
            height = tile[u32(at.y) * TILE + u32(at.x)];
        } else {
            height = water_height((texel + vec2f(side) + vec2f(0.5)) * size);
        }
        reaches = max(reaches, height);
    }

    // The highest it has stood here lately, if the map held this spot last frame; otherwise
    // the spot has only just come into view, and all that is known is where the water is now.
    var high = reaches;
    if waterline.grid.w > 0.5 && in_waterline_window(texel, waterline.window.zw) {
        let remembered = textureLoad(previous, vec2i(id.xy), 0).r;
        high = max(reaches, reaches + (remembered - reaches) * waterline.grid.z);
    }
    textureStore(current, vec2i(id.xy), vec4f(high, now, 0.0, 0.0));
}
