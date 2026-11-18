// Draws a scene's transparent glowing cards, such as the flames on lamps, over the lit scene.
//
// A card's light is its glow map, which is added, and its colour map's alpha is how much of what
// is behind it is covered. Nothing lights a card. Its depth is tested here against the scene's,
// which is reversed: a greater depth is nearer.
//
// A card holds one still picture, so it is moved a little to look alive: it sways more towards
// its top, and its brightness wavers.

#include "common/frame.wgsl"

@group(0) @binding(0) var<uniform> frame: Frame;
@group(0) @binding(1) var scene_depth: texture_2d<f32>;
@group(1) @binding(0) var image_sampler: sampler;
@group(1) @binding(1) var color_map: texture_2d<f32>;
@group(1) @binding(2) var glow_map: texture_2d<f32>;

const SWAY: f32 = 0.018;

struct Vertex {
    @location(0) position: vec3f,
    @location(1) opacity: f32,
    @location(2) emissive: vec3f,
    @location(3) phase: f32,
    @location(4) uv: vec2f,
}

struct Varyings {
    @builtin(position) clip: vec4f,
    @location(0) uv: vec2f,
    @location(1) @interpolate(flat) emissive: vec3f,
    @location(2) @interpolate(flat) opacity: f32,
    @location(3) @interpolate(flat) phase: f32,
}

@vertex
fn vs_main(vertex: Vertex) -> Varyings {
    var out: Varyings;
    out.clip = frame.view_proj * vec4f(vertex.position, 1.0);
    out.uv = vertex.uv;
    out.emissive = vertex.emissive;
    out.opacity = vertex.opacity;
    out.phase = vertex.phase;
    return out;
}

@fragment
fn fs_main(in: Varyings) -> @location(0) vec4f {
    if in.clip.z < textureLoad(scene_depth, vec2i(in.clip.xy), 0).r {
        discard;
    }
    let time = frame.time;
    let wave = sin(in.uv.y * 7.0 + time * 5.0 + in.phase) + 0.5 * sin(in.uv.y * 17.0 - time * 9.0 + in.phase * 2.0);
    let uv = vec2f(in.uv.x + wave * SWAY * (1.0 - saturate(in.uv.y)), in.uv.y);
    let flicker = 0.92 + 0.08 * sin(time * 11.0 + in.phase * 3.0);
    let coverage = textureSample(color_map, image_sampler, uv).a * in.opacity;
    let light = textureSample(glow_map, image_sampler, uv).rgb * in.emissive * flicker;
    return vec4f(light, coverage);
}
