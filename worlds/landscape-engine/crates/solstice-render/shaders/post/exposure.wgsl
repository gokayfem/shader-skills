// Automatic exposure: a luminance histogram, whose central range gives the average scene
// brightness. Ignoring the extremes keeps a bright sun or a dark shadow from swinging it.

struct Params {
    delta_seconds: f32,
    first_frame: u32,
    compensation: f32,
    // Exposure adapts only between these averages, so that a bright scene stays bright and a
    // night scene stays dark.
    brightest_average: f32,
    darkest_average: f32,
}

struct Exposure {
    average_luminance: f32,
    exposure: f32,
    _pad0: f32,
    _pad1: f32,
}

// Matches EXPOSURE_BINS in post.rs.
const BINS: u32 = 192u;
// The dimmest luminance told apart, as its logarithm, and how many stops the bins span. It
// reaches well under the darkest average exposure adapts to, a room by day, twelve stops below
// sunlit ground: a bin's worth of pixels piled up at the bottom would hold the average above it.
const MIN_LOG_LUMINANCE: f32 = -17.0;
const LOG_LUMINANCE_RANGE: f32 = 27.0;

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var scene: texture_2d<f32>;
@group(0) @binding(2) var<storage, read_write> histogram: array<atomic<u32>, BINS>;
@group(0) @binding(3) var<storage, read_write> exposure: Exposure;

// Samples a sparse grid of pixels. The average does not need every one.
@compute @workgroup_size(16, 16)
fn build_histogram(@builtin(global_invocation_id) id: vec3u) {
    let size = textureDimensions(scene);
    let pixel = id.xy * 4u;
    if any(pixel >= size) {
        return;
    }
    let c = textureLoad(scene, vec2i(pixel), 0).rgb;
    let luminance = dot(c, vec3f(0.2126, 0.7152, 0.0722));
    if luminance < 1.0e-5 {
        return;
    }
    let position = (log2(luminance) - MIN_LOG_LUMINANCE) / LOG_LUMINANCE_RANGE;
    let bin = u32(clamp(position, 0.0, 1.0) * f32(BINS - 1u));
    atomicAdd(&histogram[bin], 1u);
}

@compute @workgroup_size(1)
fn resolve() {
    var total = 0u;
    for (var i = 0u; i < BINS; i++) {
        total += atomicLoad(&histogram[i]);
    }
    // Average over the band between these percentiles. The top is well below the brightest pixels
    // because open sky can fill half the frame, and metering it leaves the land underexposed.
    let low_cut = f32(total) * 0.2;
    let high_cut = f32(total) * 0.7;
    var running = 0.0;
    var sum = 0.0;
    var weight = 0.0;
    for (var i = 0u; i < BINS; i++) {
        let count = f32(atomicExchange(&histogram[i], 0u));
        let next = running + count;
        let used = max(0.0, min(next, high_cut) - max(running, low_cut));
        let log_luminance = MIN_LOG_LUMINANCE + (f32(i) + 0.5) / f32(BINS) * LOG_LUMINANCE_RANGE;
        sum += log_luminance * used;
        weight += used;
        running = next;
    }
    let measured = exp2(sum / max(weight, 1.0));

    var average = measured;
    if params.first_frame == 0u {
        // Eyes adapt with exponential smoothing, and faster to light than to dark.
        let rate = select(1.1, 2.4, measured > exposure.average_luminance);
        let keep = exp(-params.delta_seconds * rate);
        average = mix(measured, exposure.average_luminance, keep);
    }
    exposure.average_luminance = average;
    // Maps the average to middle grey.
    exposure.exposure = params.compensation * 0.18
        / clamp(average, max(params.darkest_average, 1.0e-6), params.brightest_average);
}
