// Physically motivated bloom: no threshold, just a wide, energy-conserving blur of the whole
// image, mixed in at a low level. Filters are from Jimenez's "Next Generation Post Processing
// in Call of Duty: Advanced Warfare".

struct Params {
    source_texel: vec2f,
    is_first: u32,
    _pad: u32,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var source: texture_2d<f32>;
@group(0) @binding(2) var lower: texture_2d<f32>;
@group(0) @binding(3) var clamp_sampler: sampler;
@group(0) @binding(4) var output: texture_storage_2d<rgba16float, write>;

fn tap(uv: vec2f, dx: f32, dy: f32) -> vec3f {
    return textureSampleLevel(source, clamp_sampler, uv + vec2f(dx, dy) * params.source_texel, 0.0).rgb;
}

// Weights samples by inverse brightness, so that one extreme pixel cannot flicker the blur.
fn karis_weight(c: vec3f) -> f32 {
    return 1.0 / (1.0 + dot(c, vec3f(0.2126, 0.7152, 0.0722)) * 0.25);
}

// 13-tap downsample that avoids the pulsing of naive box filtering.
@compute @workgroup_size(8, 8)
fn downsample(@builtin(global_invocation_id) id: vec3u) {
    let size = textureDimensions(output);
    if any(id.xy >= size) {
        return;
    }
    let uv = (vec2f(id.xy) + vec2f(0.5)) / vec2f(size);
    let a = tap(uv, -2.0, 2.0);
    let b = tap(uv, 0.0, 2.0);
    let c = tap(uv, 2.0, 2.0);
    let d = tap(uv, -2.0, 0.0);
    let e = tap(uv, 0.0, 0.0);
    let f = tap(uv, 2.0, 0.0);
    let g = tap(uv, -2.0, -2.0);
    let h = tap(uv, 0.0, -2.0);
    let i = tap(uv, 2.0, -2.0);
    let j = tap(uv, -1.0, 1.0);
    let k = tap(uv, 1.0, 1.0);
    let l = tap(uv, -1.0, -1.0);
    let m = tap(uv, 1.0, -1.0);

    var color: vec3f;
    if params.is_first != 0u {
        // Five overlapping boxes, each Karis-weighted, at the first level only.
        var boxes = array<vec3f, 5>(
            (a + b + d + e) * 0.25,
            (b + c + e + f) * 0.25,
            (d + e + g + h) * 0.25,
            (e + f + h + i) * 0.25,
            (j + k + l + m) * 0.25,
        );
        var box_weights = array<f32, 5>(0.125, 0.125, 0.125, 0.125, 0.5);
        var sum = vec3f(0.0);
        var total = 0.0;
        for (var n = 0; n < 5; n++) {
            let w = box_weights[n] * karis_weight(boxes[n]);
            sum += boxes[n] * w;
            total += w;
        }
        color = sum / max(total, 1.0e-4);
    } else {
        color = e * 0.125 + (a + c + g + i) * 0.03125 + (b + d + f + h) * 0.0625 + (j + k + l + m) * 0.125;
    }
    textureStore(output, id.xy, vec4f(color, 1.0));
}

// Adds the 3x3 tent-filtered lower level to this level of the down chain.
@compute @workgroup_size(8, 8)
fn upsample(@builtin(global_invocation_id) id: vec3u) {
    let size = textureDimensions(output);
    if any(id.xy >= size) {
        return;
    }
    let uv = (vec2f(id.xy) + vec2f(0.5)) / vec2f(size);
    let t = 1.0 / vec2f(size);
    var sum = vec3f(0.0);
    for (var y = -1; y <= 1; y++) {
        for (var x = -1; x <= 1; x++) {
            let weight = f32(2 - abs(x)) * f32(2 - abs(y)) / 16.0;
            sum += textureSampleLevel(lower, clamp_sampler, uv + vec2f(f32(x), f32(y)) * t, 0.0).rgb * weight;
        }
    }
    let here = textureSampleLevel(source, clamp_sampler, uv, 0.0).rgb;
    textureStore(output, id.xy, vec4f(here + sum, 1.0));
}
