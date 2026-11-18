// Fills the wind field: the wind at each texel of a square around the camera, this frame's and
// last frame's, so that what sways with it reads one texture rather than three noises, and can
// say where it was a frame ago. The row below the field carries what readers need besides.
// See wind.rs and common/sway.wgsl, which reads it.

#include "common/wind.wgsl"

// Twin of `FieldParams` in wind.rs.
struct FieldParams {
    now: WindSnapshot,
    previous: WindSnapshot,
    // The world's (x, z) at the centre of texel (0, 0), and metres between texel centres.
    origin: vec2f,
    texel: f32,
    // 1 if things sway this frame, 0 if they stand still.
    sway: f32,
    // Distances from the camera over which swaying fades out.
    fade: vec2f,
    _pad: vec2f,
}

// Twin of `FIELD_TEXELS` in wind.rs and common/sway.wgsl.
const FIELD_TEXELS: u32 = 128u;

@group(0) @binding(0) var<uniform> params: FieldParams;
@group(0) @binding(1) var field: texture_storage_2d<rgba32float, write>;

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3u) {
    if id.x >= FIELD_TEXELS || id.y > FIELD_TEXELS {
        return;
    }
    if id.y == FIELD_TEXELS {
        switch id.x {
            case 0u: {
                textureStore(field, id.xy, vec4f(params.origin, params.texel, params.sway));
            }
            case 1u: {
                textureStore(field, id.xy, vec4f(params.now.time, params.previous.time, params.fade));
            }
            default: {}
        }
        return;
    }
    let p = params.origin + vec2f(id.xy) * params.texel;
    textureStore(
        field,
        id.xy,
        vec4f(wind_velocity(params.now, p), wind_velocity(params.previous, p)),
    );
}
