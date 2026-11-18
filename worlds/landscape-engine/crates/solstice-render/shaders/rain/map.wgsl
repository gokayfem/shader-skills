// Turns the cover map, the scene seen from straight above, into the rain map. Each texel says:
//   x: how high the first surface the rain meets there stands, in the world;
//   y: how deep a puddle can gather there, in metres: how far the ground lies below the ground
//      around it, where it is near level;
//   z: how high an eave stands just beside it, which drips onto it, or 0 for none;
//   w: 1 where anything was drawn.
// Drawn only when the cover map is. See rain/mod.rs.

#include "rain/params.wgsl"

struct MapParams {
    // The cover map's top, in the world, and how far down it reaches.
    top: f32,
    reach: f32,
    _pad: vec2f,
}

@group(0) @binding(0) var<uniform> params: MapParams;
@group(0) @binding(1) var cover: texture_depth_2d;
@group(0) @binding(2) var rain_map: texture_storage_2d<rgba32float, write>;

// How far around a texel the ground is compared with, in texels: about a metre.
const HOLLOW_RADIUS: i32 = 4;
// Ground more than this above a texel is something standing on it, not the ground around.
const STANDING: f32 = 0.25;
// A puddle gathers only where the ground is about level, and is never deeper than this.
const LEVEL_SLOPE: f32 = 0.12;
const DEEPEST: f32 = 0.06;
// An edge this much higher right beside a texel, and no more than the second, drips onto it.
const EAVE_LEAST: f32 = 1.2;
const EAVE_MOST: f32 = 25.0;

fn height_at(t: vec2i) -> f32 {
    let size = i32(RAIN_MAP_TEXELS);
    let c = clamp(t, vec2i(0), vec2i(size - 1));
    let depth = textureLoad(cover, c, 0);
    if depth >= 1.0 {
        return NOTHING_BELOW;
    }
    return params.top - depth * params.reach;
}

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3u) {
    if id.x >= RAIN_MAP_TEXELS || id.y >= RAIN_MAP_TEXELS {
        return;
    }
    let t = vec2i(id.xy);
    let h = height_at(t);
    if h <= NOTHING_BELOW {
        textureStore(rain_map, id.xy, vec4f(NOTHING_BELOW, 0.0, 0.0, 0.0));
        return;
    }

    // The ground around, leaving out what stands on it: a wall beside a path is no hollow.
    var sum = 0.0;
    var count = 0.0;
    for (var dy = -HOLLOW_RADIUS; dy <= HOLLOW_RADIUS; dy++) {
        for (var dx = -HOLLOW_RADIUS; dx <= HOLLOW_RADIUS; dx++) {
            if dx * dx + dy * dy > HOLLOW_RADIUS * HOLLOW_RADIUS {
                continue;
            }
            let n = height_at(t + vec2i(dx, dy));
            if n > NOTHING_BELOW && n < h + STANDING {
                sum += n;
                count += 1.0;
            }
        }
    }
    let around = sum / max(count, 1.0);
    let texel = 0.25;
    let slope = max(
        abs(height_at(t + vec2i(1, 0)) - height_at(t - vec2i(1, 0))),
        abs(height_at(t + vec2i(0, 1)) - height_at(t - vec2i(0, 1))),
    ) / (2.0 * texel);
    let level = 1.0 - smoothstep(LEVEL_SLOPE * 0.5, LEVEL_SLOPE, slope);
    let hollow = clamp(around - h, 0.0, DEEPEST) * level;

    // An edge right beside: a roof's, a wall's top, a tree's crown.
    var eave = 0.0;
    for (var i = 0; i < 8; i++) {
        let angle = f32(i) * 0.785398;
        let step = vec2i(round(vec2f(cos(angle), sin(angle))));
        let n = height_at(t + step);
        let rise = n - h;
        if rise > EAVE_LEAST && rise < EAVE_MOST {
            eave = max(eave, n);
        }
    }
    textureStore(rain_map, id.xy, vec4f(h, hollow, eave, 1.0));
}
