// The colour of a screen-trace hit, from last frame's final image.
//
// The including shader declares, in group 0:
//   motion_tex: texture_2d<f32>      G-buffer motion, to find a hit in last frame's image
//   scene_history: texture_2d<f32>   last frame's final colour

// Last frame's lit colour at a hit, found through the hit pixel's motion so that it stays
// put while the camera moves.
// Alpha is validity. A missing/invalid history sample must fall back to the volume,
// not seed the radiance probes with black or non-finite light.
fn screen_hit_radiance(uv: vec2f) -> vec4f {
    let pixel = min(vec2i(uv * frame.viewport), vec2i(frame.viewport) - vec2i(1));
    let previous = uv + textureLoad(motion_tex, pixel, 0).xy;
    if frame.frame_index == 0u || !all(previous >= vec2f(0.0)) || !all(previous <= vec2f(1.0)) {
        return vec4f(0.0);
    }
    let color = textureSampleLevel(scene_history, clamp_sampler, previous, 0.0).rgb;
    if !finite3(color) {
        return vec4f(0.0);
    }
    return vec4f(color, 1.0);
}
