// Lights the rain: the light a falling drop receives, in each cell of a box around the camera,
// worked out once a frame so that every streak needs one look-up (docs/weather-effects.md).
//
// A drop is a tiny lens: it gathers light from all around and from behind, and a streak shows
// roughly the average of what surrounds it (Garg and Nayar 2006), with sunlight scattered
// strongly onward. So each cell keeps two things:
//   ambient: the sky's light from overhead and around the horizon, and the scene's lamps,
//            scattered evenly;
//   sun:     sunlight through the atmosphere and the clouds, shadowed by the sun's cascades,
//            which the drawing pass scatters onward by its phase function.

#include "common/environment.wgsl"
#include "common/scene.wgsl"
#include "rain/params.wgsl"

@group(0) @binding(0) var<uniform> rain: RainParams;
@group(0) @binding(1) var<uniform> shadows: Shadows;
@group(0) @binding(2) var shadow_maps: texture_depth_2d_array;
@group(0) @binding(3) var shadow_sampler: sampler_comparison;
@group(0) @binding(4) var<storage, read> lights: array<Light>;
@group(0) @binding(5) var ambient_out: texture_storage_3d<rgba16float, write>;
@group(0) @binding(6) var sun_out: texture_storage_3d<rgba16float, write>;

#include "lighting/sun_shadow.wgsl"

// A lamp's light, scattered evenly by a drop: its irradiance over 4π.
const LAMP_SCATTER: f32 = 0.0796;

@compute @workgroup_size(4, 4, 4)
fn main(@builtin(global_invocation_id) id: vec3u) {
    let size = textureDimensions(ambient_out);
    if any(id >= size) {
        return;
    }
    let p = rain.light.xyz + (vec3f(id) + vec3f(0.5)) * rain.light.w;

    // The sky overhead counts twice: rain falls through open air, and most of what lights it
    // comes from above.
    var sky = sky_radiance(vec3f(0.0, 1.0, 0.0)) * 2.0;
    for (var i = 0u; i < 4u; i++) {
        let a = f32(i) * 1.5707963 + 0.4;
        sky += sky_radiance(toward_horizon(vec3f(cos(a), 0.0, sin(a))));
    }
    var ambient = sky / 6.0;
    for (var i = 0u; i < frame.light_count; i++) {
        let light = lights[i];
        let away = length(light.position - p);
        if away >= light.range {
            continue;
        }
        let window = sq(clamp(1.0 - sq(sq(away / light.range)), 0.0, 1.0));
        ambient += light.intensity * window / max(away * away, 0.04) * LAMP_SCATTER;
    }

    let from_eye = distance(p, frame.camera_position);
    let shadow = sun_shadow(p, frame.sun_direction, from_eye, 0.5);
    let sun = sun_irradiance_at(p) * shadow;

    textureStore(ambient_out, id, vec4f(ambient, 1.0));
    textureStore(sun_out, id, vec4f(sun, 1.0));
}
