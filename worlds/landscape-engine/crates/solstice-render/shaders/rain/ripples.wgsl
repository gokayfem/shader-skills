// Rings spreading where drops strike standing water, over a tile a metre across that repeats:
// what puddles, ponds and the sea read while it rains (Lagarde, "Water drop 3b", 2013). Each
// texel holds the water's slope in x and z, and its height.
//
// Two staggered grids of cells each start a ring now and then, at a random spot in the cell. A
// ring runs outward, a crest with a trough either side, and dies away as it spreads. How many
// cells start one follows how hard it rains.

#include "common/math.wgsl"
#include "rain/params.wgsl"

@group(0) @binding(0) var<uniform> rain: RainParams;
@group(0) @binding(1) var ripples: texture_storage_2d<rgba16float, write>;

// Twin of `RIPPLE_TEXELS` in rain/mod.rs.
const RIPPLE_TEXELS: u32 = 256u;
// Cells along the tile in each grid, and the farthest a ring spreads, in metres.
const CELLS: f32 = 7.0;
const RING_REACH: f32 = 0.12;
// How wide a ring's crest is, in metres, and how high it starts.
const RING_WIDTH: f32 = 0.009;
const RING_HEIGHT: f32 = 0.0012;
// Rings a cell starts a second, at most.
const RINGS_PER_SECOND: f32 = 1.6;

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3u) {
    if id.x >= RIPPLE_TEXELS || id.y >= RIPPLE_TEXELS {
        return;
    }
    let uv = (vec2f(id.xy) + vec2f(0.5)) / f32(RIPPLE_TEXELS);
    // How many cells start rings: none without rain, every one in a downpour.
    let density = clamp(rain.rain.y * 1.1, 0.0, 1.0);
    let time = rain.camera.w;
    var slope = vec2f(0.0);
    var height = 0.0;
    for (var grid = 0u; grid < 2u; grid++) {
        let shift = f32(grid) * 0.5;
        let at = uv * CELLS + vec2f(shift);
        let base = floor(at);
        for (var dy = -1; dy <= 1; dy++) {
            for (var dx = -1; dx <= 1; dx++) {
                let cell = base + vec2f(f32(dx), f32(dy));
                // The tile repeats, so cells wrap around it.
                let wrapped = vec2u((cell % CELLS + CELLS) % CELLS);
                let seed = hash4(wrapped.x, wrapped.y, grid);
                let period = 1.0 / (RINGS_PER_SECOND * (0.5 + 0.5 * seed.x));
                let cycles = time / period + seed.y;
                let cycle = u32(floor(cycles));
                let age = fract(cycles);
                let ring = hash4(wrapped.x + cycle * 7919u, wrapped.y, grid + 2u);
                if ring.x > density {
                    continue;
                }
                let centre = cell + vec2f(0.15) + ring.yz * 0.7;
                let offset = (at - centre) / CELLS;
                let d = length(offset);
                let radius = age * RING_REACH;
                let x = (d - radius) / RING_WIDTH;
                if abs(x) > 3.0 || d < 1.0e-5 {
                    continue;
                }
                let fade = sq(1.0 - age) * RING_HEIGHT * (0.6 + 0.8 * ring.w);
                let envelope = exp(-x * x);
                height += sin(x * PI) * envelope * fade;
                let along = (PI * cos(x * PI) - 2.0 * x * sin(x * PI)) * envelope * fade / RING_WIDTH;
                slope += offset / d * along;
            }
        }
    }
    textureStore(ripples, id.xy, vec4f(slope, height, 0.0));
}
