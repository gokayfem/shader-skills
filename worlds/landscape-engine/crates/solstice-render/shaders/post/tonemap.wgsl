// The final image: exposure, bloom, sharpening, a filmic tone curve, display encoding, and a
// touch of grain to break up banding.

struct Exposure {
    average_luminance: f32,
    exposure: f32,
    _pad0: f32,
    _pad1: f32,
}

struct Params {
    bloom_strength: f32,
    sharpen: f32,
    vignette: f32,
    grain: f32,
    frame_index: u32,
    debug_passthrough: u32,
    _pad0: u32,
    _pad1: u32,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var hdr: texture_2d<f32>;
@group(0) @binding(2) var bloom: texture_2d<f32>;
@group(0) @binding(3) var linear_sampler: sampler;
@group(0) @binding(4) var<storage, read> exposure: Exposure;

// Matches BLOOM_LEVELS in post.rs.
const BLOOM_LEVELS: f32 = 6.0;

struct VertexOutput {
    @builtin(position) position: vec4f,
    @location(0) uv: vec2f,
}

@vertex
fn vs_fullscreen(@builtin(vertex_index) index: u32) -> VertexOutput {
    let uv = vec2f(f32((index << 1u) & 2u), f32(index & 2u));
    return VertexOutput(vec4f(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.0, 1.0), uv);
}

// ACES filmic curve, using Stephen Hill's fit of the reference transforms.
fn aces(color: vec3f) -> vec3f {
    let to_rrt = mat3x3f(
        vec3f(0.59719, 0.07600, 0.02840),
        vec3f(0.35458, 0.90834, 0.13383),
        vec3f(0.04823, 0.01566, 0.83777),
    );
    let from_odt = mat3x3f(
        vec3f(1.60475, -0.10208, -0.00327),
        vec3f(-0.53108, 1.10813, -0.07276),
        vec3f(-0.07367, -0.00605, 1.07602),
    );
    let v = to_rrt * color;
    let a = v * (v + vec3f(0.0245786)) - vec3f(0.000090537);
    let b = v * (0.983729 * v + vec3f(0.4329510)) + vec3f(0.238081);
    return clamp(from_odt * (a / b), vec3f(0.0), vec3f(1.0));
}

fn linear_to_srgb(c: vec3f) -> vec3f {
    let low = c * 12.92;
    let high = 1.055 * pow(c, vec3f(1.0 / 2.4)) - vec3f(0.055);
    return select(high, low, c <= vec3f(0.0031308));
}

fn hash(p: vec2f) -> f32 {
    return fract(sin(dot(p, vec2f(12.9898, 78.233))) * 43758.5453);
}

// `color` from the scene, after exposure (`scale`, with vignetting), bloom and the tone curve.
fn toned(color: vec3f, bloom_color: vec3f, scale: f32) -> vec3f {
    return aces(mix(color, bloom_color, params.bloom_strength) * scale);
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4f {
    let pixel = vec2i(in.position.xy);
    let last = vec2i(textureDimensions(hdr)) - vec2i(1);
    let center = textureLoad(hdr, pixel, 0).rgb;
    if params.debug_passthrough != 0u {
        return vec4f(linear_to_srgb(clamp(center, vec3f(0.0), vec3f(1.0))), 1.0);
    }

    // Exposure, bloom and vignette, then the tone curve, for this pixel and its four neighbours.
    // Bloom is smooth, so the neighbours share this pixel's.
    let bloom_color = textureSampleLevel(bloom, linear_sampler, in.uv, 0.0).rgb / BLOOM_LEVELS;
    let from_center = (in.uv - vec2f(0.5)) * vec2f(1.0, 0.8);
    let scale = exposure.exposure * (1.0 - params.vignette * dot(from_center, from_center) * 2.0);
    let c = toned(center, bloom_color, scale);
    let n = toned(textureLoad(hdr, clamp(pixel + vec2i(0, -1), vec2i(0), last), 0).rgb, bloom_color, scale);
    let s = toned(textureLoad(hdr, clamp(pixel + vec2i(0, 1), vec2i(0), last), 0).rgb, bloom_color, scale);
    let w = toned(textureLoad(hdr, clamp(pixel + vec2i(-1, 0), vec2i(0), last), 0).rgb, bloom_color, scale);
    let e = toned(textureLoad(hdr, clamp(pixel + vec2i(1, 0), vec2i(0), last), 0).rgb, bloom_color, scale);

    // Contrast-adaptive sharpening, which restores the crispness that temporal filtering
    // softens. It works on the displayed values, from 0 to 1, as it is designed to: strong where
    // the image is flat and gentle at high contrast. The result stays within its neighbours'
    // range, so no edge, such as a roof against the sky or the moon's rim, gets a dark halo.
    let low = min(c, min(n, min(s, min(w, e))));
    let high = max(c, max(n, max(s, max(w, e))));
    let amount = sqrt(clamp(min(low, vec3f(1.0) - high) / max(high, vec3f(1.0e-4)), vec3f(0.0), vec3f(1.0)));
    let weight = -amount * params.sharpen * 0.2;
    let toned_color = clamp((c + (n + s + w + e) * weight) / (1.0 + 4.0 * weight), low, high);

    var display = linear_to_srgb(toned_color);
    let noise = hash(in.position.xy + vec2f(f32(params.frame_index % 64u) * 3.17));
    display += (noise - 0.5) * params.grain;
    return vec4f(display, 1.0);
}
