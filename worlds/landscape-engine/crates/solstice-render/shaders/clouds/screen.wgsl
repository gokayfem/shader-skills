// The clouds seen on screen, at half resolution. Each texel marches one ray, jittered in
// position and depth every frame, and blends it into last frame's clouds reprojected to where
// they now appear: over a few frames that gathers many rays' worth of detail for the cost of
// one. Texels whose pixels are all covered by geometry are skipped, since clouds only show
// where the sky does.
//
// Output: light from the clouds in RGB, and how much of the sky behind still shows in A.

@group(0) @binding(0) var shape_noise: texture_3d<f32>;
@group(0) @binding(1) var detail_noise: texture_3d<f32>;
@group(0) @binding(2) var weather_map: texture_2d<f32>;
@group(0) @binding(3) var noise_sampler: sampler;
@group(0) @binding(4) var output: texture_storage_2d<rgba16float, write>;
@group(0) @binding(5) var history: texture_2d<f32>;
@group(0) @binding(6) var depth_tex: texture_2d<f32>;

@group(0) @binding(7) var<storage, read> coverage_cdf: array<f32, 256>;

#include "clouds/common.wgsl"

const STEPS: i32 = 56;
const LIGHT_SAMPLES: i32 = 5;
// The share of each new frame in the blend, more where the history cannot be trusted.
const BLEND: f32 = 0.12;
// Transmittance stored by texels that were not marched.
const UNKNOWN: f32 = -1.0;

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3u) {
    let size = textureDimensions(output);
    if any(id.xy >= size) {
        return;
    }
    // Skip only texels with no sky anywhere near: the composite filters between neighbouring
    // texels, so a texel next to the sky must hold real clouds, or the clear sky would show
    // as a bright outline along every ridge.
    let full = vec2i(frame.viewport);
    let block = vec2i(id.xy) * 2;
    var sky_pixels = 0;
    for (var y = -1; y <= 2; y++) {
        for (var x = -1; x <= 2; x++) {
            let pixel = clamp(block + vec2i(x, y), vec2i(0), full - vec2i(1));
            if textureLoad(depth_tex, pixel, 0).r <= 0.0 {
                sky_pixels++;
            }
        }
    }
    if sky_pixels == 0 {
        // Unknown, rather than clear: when jitter or motion brings this texel into view next
        // frame, its history must not say "no cloud", or clear sky would linger at every edge.
        textureStore(output, id.xy, vec4f(0.0, 0.0, 0.0, UNKNOWN));
        return;
    }

    // A different pixel of the 2x2 block each frame, and a different start along the ray.
    let corner = vec2f(f32(frame.frame_index & 1u), f32((frame.frame_index >> 1u) & 1u));
    let center = vec2f(block) + corner + vec2f(0.5);
    let ndc = pixel_to_ndc(center, frame.inv_viewport);
    let far_point = frame.inv_view_proj * vec4f(ndc, 1.0e-6, 1.0);
    let direction = normalize(far_point.xyz / far_point.w - frame.camera_position);
    let jitter = interleaved_gradient_noise(vec2f(id.xy), frame.frame_index);
    let sample = clouds_along(direction, STEPS, LIGHT_SAMPLES, jitter);
    var result = vec4f(sample.radiance, sample.transmittance);

    // Where the clouds were on screen last frame. Clouds are far away, so where none were hit
    // the direction alone will do.
    var previous_clip: vec4f;
    if sample.transmittance > 0.995 {
        previous_clip = frame.prev_view_proj * vec4f(direction, 0.0);
    } else {
        previous_clip = frame.prev_view_proj * vec4f(frame.camera_position + direction * sample.distance, 1.0);
    }
    if previous_clip.w > 0.0 && frame.frame_index > 0u {
        let previous_ndc = previous_clip.xy / previous_clip.w;
        let previous_uv = vec2f(previous_ndc.x * 0.5 + 0.5, 0.5 - previous_ndc.y * 0.5);
        if all(previous_uv > vec2f(0.0)) && all(previous_uv < vec2f(1.0)) {
            // History is used only where all four texels it filters between were marched.
            let marched = textureGather(3, history, clamp_sampler, previous_uv);
            if all(marched >= vec4f(0.0)) {
                let past = textureSampleLevel(history, clamp_sampler, previous_uv, 0.0);
                result = mix(past, result, BLEND);
            }
        }
    }
    textureStore(output, id.xy, result);
}
