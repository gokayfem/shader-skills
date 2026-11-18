// Temporal anti-aliasing and upscaling. Each frame's camera jitter makes every pixel sample
// a different point; reprojecting and accumulating those samples gives both anti-aliasing
// and, when the render resolution is below the output, real extra detail.

#include "common/math.wgsl"
#include "common/frame.wgsl"

@group(0) @binding(0) var<uniform> frame: Frame;
@group(0) @binding(1) var scene: texture_2d<f32>;
@group(0) @binding(2) var depth_tex: texture_2d<f32>;
@group(0) @binding(3) var motion_tex: texture_2d<f32>;
@group(0) @binding(4) var history: texture_2d<f32>;
@group(0) @binding(5) var linear_sampler: sampler;
@group(0) @binding(6) var output: texture_storage_2d<rgba16float, write>;
// How much of each scene pixel translucent particles cover. They have no motion of their own
// in the scene's motion, so the history reprojected there shows what was behind them, or them
// somewhere else: where they cover, history counts for less.
@group(0) @binding(7) var reactive_tex: texture_2d<f32>;

// The share of the current frame where particles cover a pixel fully. Not all of it: under a
// large plume of smoke, what shows through still needs some history to stay anti-aliased.
const REACTIVE_BLEND: f32 = 0.6;

// Tonemap weights keep very bright samples from dominating the filter.
fn compress(c: vec3f) -> vec3f {
    return c / (1.0 + luminance(c));
}

fn decompress(c: vec3f) -> vec3f {
    return c / max(1.0 - luminance(c), 1.0e-4);
}

fn to_ycocg(c: vec3f) -> vec3f {
    return vec3f(
        dot(c, vec3f(0.25, 0.5, 0.25)),
        dot(c, vec3f(0.5, 0.0, -0.5)),
        dot(c, vec3f(-0.25, 0.5, -0.25)),
    );
}

fn from_ycocg(c: vec3f) -> vec3f {
    return vec3f(c.x + c.y - c.z, c.x + c.z, c.x - c.y - c.z);
}

// Five-tap Catmull-Rom: nearly bicubic history sampling, which is the main thing keeping
// reprojection from blurring the image over time.
fn sample_history(uv: vec2f, fallback: vec3f) -> vec3f {
    let size = vec2f(textureDimensions(history));
    let position = uv * size;
    let center = floor(position - vec2f(0.5)) + vec2f(0.5);
    let f = position - center;
    let w0 = f * (-0.5 + f * (1.0 - 0.5 * f));
    let w1 = 1.0 + f * f * (-2.5 + 1.5 * f);
    let w2 = f * (0.5 + f * (2.0 - 1.5 * f));
    let w3 = f * f * (-0.5 + 0.5 * f);
    let w12 = w1 + w2;
    let offset12 = w2 / w12;
    let t0 = (center - vec2f(1.0)) / size;
    let t3 = (center + vec2f(2.0)) / size;
    let t12 = (center + offset12) / size;

    var taps = array<vec3f, 5>(
        vec3f(t12.x, t0.y, w12.x * w0.y),
        vec3f(t0.x, t12.y, w0.x * w12.y),
        vec3f(t12.x, t12.y, w12.x * w12.y),
        vec3f(t3.x, t12.y, w3.x * w12.y),
        vec3f(t12.x, t3.y, w12.x * w3.y),
    );
    var sum = vec3f(0.0);
    var total = 0.0;
    for (var i = 0; i < 5; i++) {
        let tap = taps[i];
        let color = textureSampleLevel(history, linear_sampler, tap.xy, 0.0).rgb;
        if finite3(color) {
            sum += color * tap.z;
            total += tap.z;
        }
    }
    if total <= 1.0e-4 || !finite3(sum) {
        return fallback;
    }
    return sum / max(total, 1.0e-4);
}

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3u) {
    let output_size = textureDimensions(output);
    if any(id.xy >= output_size) {
        return;
    }
    let uv = (vec2f(id.xy) + vec2f(0.5)) / vec2f(output_size);
    let scene_size = vec2i(frame.viewport);

    // The scene pixel nearest this output pixel, removing the jitter applied this frame.
    let scene_position = uv * frame.viewport;
    let nearest = clamp(vec2i(floor(scene_position)), vec2i(0), scene_size - vec2i(1));

    // Gather the neighbourhood. Track the closest depth, so that edges reproject with the
    // foreground's motion, and the variance, which bounds what history is believable.
    var mean = vec3f(0.0);
    var second_moment = vec3f(0.0);
    var sample_count = 0.0;
    var filtered = vec3f(0.0);
    var filter_total = 0.0;
    var closest_depth = 0.0;
    var closest_pixel = nearest;
    // At this pixel alone: taken over the neighbourhood, it would spread a hard-edged band of
    // little history around every particle.
    let reactive = textureLoad(reactive_tex, nearest, 0).r;
    let jitter_pixels = frame.jitter_ndc * 0.5 * frame.viewport * vec2f(1.0, -1.0);
    for (var dy = -1; dy <= 1; dy++) {
        for (var dx = -1; dx <= 1; dx++) {
            let p = clamp(nearest + vec2i(dx, dy), vec2i(0), scene_size - vec2i(1));
            let depth = textureLoad(depth_tex, p, 0).r;
            if depth > closest_depth {
                closest_depth = depth;
                closest_pixel = p;
            }
            let sample = textureLoad(scene, p, 0).rgb;
            if !finite3(sample) {
                continue;
            }
            let color = compress(max(sample, vec3f(0.0)));
            let ycocg = to_ycocg(color);
            mean += ycocg;
            second_moment += ycocg * ycocg;
            sample_count += 1.0;

            // A Gaussian centred on this output pixel, using where the sample really landed.
            let sample_position = vec2f(p) + vec2f(0.5) + jitter_pixels;
            let delta = sample_position - scene_position;
            let weight = exp(-2.3 * dot(delta, delta));
            filtered += color * weight;
            filter_total += weight;

        }
    }
    mean /= max(sample_count, 1.0);
    second_moment /= max(sample_count, 1.0);
    let deviation = sqrt(max(second_moment - mean * mean, vec3f(0.0)));
    let current = filtered / max(filter_total, 1.0e-4);

    // Reproject. The sky has no stored motion, so derive it from the camera alone.
    var previous_uv: vec2f;
    if closest_depth > 0.0 {
        previous_uv = uv + textureLoad(motion_tex, closest_pixel, 0).xy;
    } else {
        let ndc = vec2f(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
        let world_h = frame.inv_view_proj * vec4f(ndc, 1.0e-6, 1.0);
        let previous_clip = frame.prev_view_proj * vec4f(world_h.xyz / world_h.w, 1.0);
        let previous_ndc = previous_clip.xy / previous_clip.w;
        previous_uv = vec2f(previous_ndc.x * 0.5 + 0.5, 0.5 - previous_ndc.y * 0.5);
    }

    var result = current;
    let on_screen = all(previous_uv > vec2f(0.0)) && all(previous_uv < vec2f(1.0));
    if on_screen && frame.frame_index > 0u && frame.temporal_accumulation != 0u {
        let history_color = max(sample_history(previous_uv, decompress(current)), vec3f(0.0));
        // If the whole neighbourhood is invalid, keep valid history until lighting recovers.
        if sample_count == 0.0 {
            textureStore(output, id.xy, vec4f(min(history_color, vec3f(65504.0)), 1.0));
            return;
        }
        let previous = compress(history_color);

        // Clip history against the current neighbourhood's colour distribution: anything far
        // outside it is a disocclusion or lighting change, not a useful earlier sample.
        let tolerance = 1.25;
        let low = mean - deviation * tolerance;
        let high = mean + deviation * tolerance;
        let y = to_ycocg(previous);
        let center_box = (high + low) * 0.5;
        let extents = max((high - low) * 0.5, vec3f(1.0e-4));
        let ratio = (y - center_box) / extents;
        let largest = max(abs(ratio.x), max(abs(ratio.y), abs(ratio.z)));
        let clipped = select(y, center_box + (y - center_box) / largest, largest > 1.0);

        // More weight on the current frame when rendering below output resolution, and
        // when the scene is moving, since resampled history gets softer with distance.
        let render_ratio = frame.viewport.x / f32(output_size.x);
        let moved = length((previous_uv - uv) * frame.viewport);
        let blend = max(clamp(0.06 * render_ratio + moved * 0.012, 0.04, 0.3), reactive * REACTIVE_BLEND);
        result = mix(from_ycocg(clipped), current, blend);
    }
    textureStore(output, id.xy, vec4f(min(decompress(max(result, vec3f(0.0))), vec3f(65504.0)), 1.0));
}
