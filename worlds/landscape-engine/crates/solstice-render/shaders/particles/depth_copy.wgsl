// Copies the scene's depth, which compute passes write as an r32float texture, into a depth
// attachment, so that the hardware can reject particle fragments hidden behind the scene before
// shading them.

@group(0) @binding(0) var scene_depth: texture_2d<f32>;

@vertex
fn vs_fullscreen(@builtin(vertex_index) vertex: u32) -> @builtin(position) vec4f {
    let uv = vec2f(f32((vertex << 1u) & 2u), f32(vertex & 2u));
    return vec4f(uv * 2.0 - 1.0, 0.0, 1.0);
}

@fragment
fn fs_main(@builtin(position) position: vec4f) -> @builtin(frag_depth) f32 {
    return textureLoad(scene_depth, vec2i(position.xy), 0).r;
}
