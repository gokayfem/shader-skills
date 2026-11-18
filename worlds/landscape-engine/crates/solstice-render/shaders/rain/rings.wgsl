// Rings where raindrops strike open water, worked out where they are seen: the sea and ponds
// call this rather than reading a texture, as the sea's pass already binds as many textures as
// the smallest devices allow (docs/weather-effects.md). Puddles read rain/ripples.wgsl's tile,
// which draws the same rings.
//
// Each cell of two staggered grids, fixed in the world, starts a ring now and then at a random
// spot. A ring runs outward, a crest with a trough either side, and dies away as it spreads.
// Heavy rain also roughens the water between the rings it can show, and further off, where
// rings are smaller than a pixel, only that roughening is left.

#include "common/math.wgsl"

// Cells per metre, the farthest a ring spreads, its crest's width and its starting height, all
// in metres; how many rings a cell starts a second at most.
const RINGS_CELLS: f32 = 7.0;
const RINGS_REACH: f32 = 0.12;
const RINGS_WIDTH: f32 = 0.009;
const RINGS_HEIGHT: f32 = 0.0012;
const RINGS_PER_SECOND: f32 = 1.6;
// The mean square slope heavy rain gives water too finely to draw: splashes, pits and the rings'
// own crowns, on top of what the wind raises.
const RAIN_ROUGHENING: f32 = 0.012;

fn rings_hash(a: u32, b: u32, c: u32) -> vec4f {
    let h = pcg(a + pcg(b + pcg(c)));
    return vec4f(
        hash_to_unit(h),
        hash_to_unit(h ^ 0x9e3779b9u),
        hash_to_unit(h ^ 0x85ebca6bu),
        hash_to_unit(h ^ 0xc2b2ae35u),
    );
}

// The water's slope from rings at `xz`, in x and z, at `time`, when `density` of the cells start
// them (0 none, 1 a downpour), for a pixel `footprint` metres across; and in z, the mean square
// slope to add to its roughness.
fn rain_rings(xz: vec2f, time: f32, density: f32, footprint: f32) -> vec3f {
    if density <= 0.0 {
        return vec3f(0.0);
    }
    let rough = RAIN_ROUGHENING * density;
    // Rings show while a pixel spans under about a crest's width.
    let shown = 1.0 - smoothstep(RINGS_WIDTH * 1.5, RINGS_WIDTH * 4.0, footprint);
    if shown <= 0.0 {
        return vec3f(0.0, 0.0, rough);
    }
    var slope = vec2f(0.0);
    for (var grid = 0u; grid < 2u; grid++) {
        let at = xz * RINGS_CELLS + vec2f(f32(grid) * 0.5);
        let base = floor(at);
        for (var dy = -1; dy <= 1; dy++) {
            for (var dx = -1; dx <= 1; dx++) {
                let cell = base + vec2f(f32(dx), f32(dy));
                let cx = u32(i32(cell.x));
                let cy = u32(i32(cell.y));
                let seed = rings_hash(cx, cy, grid + 40u);
                let period = 1.0 / (RINGS_PER_SECOND * (0.5 + 0.5 * seed.x));
                let cycles = time / period + seed.y;
                let cycle = u32(floor(cycles));
                let age = fract(cycles);
                let ring = rings_hash(cx + cycle * 7919u, cy, grid + 42u);
                if ring.x > density {
                    continue;
                }
                let centre = cell + vec2f(0.15) + ring.yz * 0.7;
                let offset = (at - centre) / RINGS_CELLS;
                let d = length(offset);
                let x = (d - age * RINGS_REACH) / RINGS_WIDTH;
                if abs(x) > 3.0 || d < 1.0e-5 {
                    continue;
                }
                let fade = sq(1.0 - age) * RINGS_HEIGHT * (0.6 + 0.8 * ring.w);
                let along = (PI * cos(x * PI) - 2.0 * x * sin(x * PI)) * exp(-x * x) * fade / RINGS_WIDTH;
                slope += offset / d * along;
            }
        }
    }
    return vec3f(slope * shown, rough);
}
