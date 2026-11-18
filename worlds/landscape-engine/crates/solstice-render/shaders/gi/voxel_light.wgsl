// Updates the light leaving each surface voxel: the GI system's radiance cache.
//
// Direct light is computed with a shadow ray through the volume. Indirect light gathers a
// few cosine-weighted rays per update, and each one reads radiance stored by earlier
// updates, so bounces accumulate across frames and multiple bounces arrive for free. A
// running average turns these few noisy rays into a smooth, slowly adapting result.

#include "common/environment.wgsl"
#include "common/scene.wgsl"
#include "gi/radiance_codec.wgsl"
#include "lighting/light_shadows_common.wgsl"

struct Params {
    level: u32,
    frame: u32,
    resolution: u32,
    light_count: u32,
    // Toroidal: the world voxel coordinate of the grid's corner, wrapped to resolution.
    wrap_origin: vec3i,
    _pad: u32,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> voxels: array<u32>;
@group(0) @binding(2) var<storage, read_write> radiance: array<vec2u>;
@group(0) @binding(3) var<storage, read> lights: array<Light>;
// Each light's shadow map, from lighting/light_shadows.wgsl.
@group(0) @binding(4) var light_shadows: texture_2d_array<f32>;

const EMISSIVE_RANGE: f32 = 128.0;
const INDIRECT_RAYS: u32 = 3u;
const BLEND: f32 = 0.22;

fn decode_emissive(packed: u32) -> vec3f {
    let e = unpack4x8unorm(packed);
    return e.rgb * e.a * EMISSIVE_RANGE;
}

@compute @workgroup_size(4, 4, 4)
fn main(@builtin(global_invocation_id) id: vec3u) {
    let n = params.resolution;
    if any(id >= vec3u(n)) {
        return;
    }
    // Refresh half of the voxels each update, in a checkerboard.
    if ((id.x + id.y + id.z + params.frame) & 1u) != 0u {
        return;
    }
    let level_offset = params.level * n * n * n;
    let local_index = id.z * n * n + id.y * n + id.x;
    let word = (level_offset + local_index) * 3u;
    let albedo_word = voxels[word];
    if (albedo_word & 0xff000000u) == 0u {
        return;
    }

    let grid = volume_params.levels[params.level];
    let voxel = grid.w;
    let albedo = unpack4x8unorm(albedo_word).rgb;
    let normal = oct_decode(unpack2x16snorm(voxels[word + 1u]));
    let emissive = decode_emissive(voxels[word + 2u]);
    let center = grid.xyz + (vec3f(id) + vec3f(0.5)) * voxel;
    let origin = center + normal * voxel * 1.5;

    // Sun.
    var irradiance = vec3f(0.0);
    let sun_cos = dot(normal, frame.sun_direction);
    if sun_cos > 0.0 {
        let visible = volume_visibility(origin, frame.sun_direction, voxel, 400.0, 48u);
        irradiance += sun_irradiance_at(center) * sun_cos * visible;
    }

    // Local lights, through their shadow maps so that walls contain them.
    for (var i = 0u; i < params.light_count; i++) {
        let light = lights[i];
        let to_light = light.position - origin;
        let distance = length(to_light);
        if distance < light.range {
            let direction = to_light / distance;
            let cos_light = dot(normal, direction);
            if cos_light > 0.0 {
                let falloff = sq(clamp(1.0 - sq(sq(distance / light.range)), 0.0, 1.0)) / max(distance * distance, 0.25);
                let open = light_shadow(i, -to_light, distance, voxel, normal);
                irradiance += light.intensity * falloff * cos_light * open;
            }
        }
    }

    // Indirect: light arriving from other surfaces and the sky.
    let basis = orthonormal_basis(normal);
    let seed = vec2f(hash_to_unit(local_index * 7u + params.level), hash_to_unit(local_index * 13u + 5u));
    var gathered = vec3f(0.0);
    for (var r = 0u; r < INDIRECT_RAYS; r++) {
        let direction = cosine_sample(r2_sequence(params.frame * INDIRECT_RAYS + r, seed), basis);
        let hit = trace_volume(origin, direction, voxel, 320.0, 32u);
        gathered += select(miss_radiance(hit, direction, sky_radiance(direction)), hit.radiance, hit.hit);
    }
    // With cosine-weighted rays, irradiance is pi times the mean radiance.
    irradiance += gathered / f32(INDIRECT_RAYS) * PI;

    let target_radiance = albedo / PI * irradiance + emissive;

    // Radiance is stored by world position, wrapped around the grid, so it survives the
    // clipmap following the camera.
    let world = vec3i(id) + params.wrap_origin;
    let wrapped = vec3u((world % i32(n) + vec3i(i32(n))) % i32(n));
    let slot = level_offset + wrapped.z * n * n + wrapped.y * n + wrapped.x;
    let previous = decode_radiance(radiance[slot]);
    radiance[slot] = encode_radiance(mix(previous, target_radiance, BLEND));
}
