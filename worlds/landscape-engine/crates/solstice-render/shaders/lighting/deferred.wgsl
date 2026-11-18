// Combines everything into the HDR scene colour: direct sun with shadow maps and contact
// shadows, local lights, indirect diffuse and specular from the GI system, and sky.

#include "common/environment.wgsl"
#include "common/brdf.wgsl"
#include "common/scene.wgsl"
#include "sky/bodies.wgsl"
#include "lighting/sun_shadow.wgsl"
#include "lighting/specular_aa.wgsl"

@group(0) @binding(0) var albedo_tex: texture_2d<f32>;
@group(0) @binding(1) var normal_tex: texture_2d<f32>;
@group(0) @binding(2) var emissive_tex: texture_2d<f32>;
@group(0) @binding(3) var depth_tex: texture_2d<f32>;
@group(0) @binding(4) var gi_tex: texture_2d<f32>;
@group(0) @binding(5) var reflection_tex: texture_2d<f32>;
@group(0) @binding(6) var output: texture_storage_2d<rgba16float, write>;
@group(0) @binding(7) var<uniform> shadows: Shadows;
@group(0) @binding(8) var shadow_maps: texture_depth_2d_array;
@group(0) @binding(9) var shadow_sampler: sampler_comparison;
@group(0) @binding(10) var<storage, read> lights: array<Light>;
// The clouds drawn for this view, at half resolution: their light in RGB and the sky showing
// through them in A.
@group(0) @binding(11) var cloud_tex: texture_2d<f32>;
// The Milky Way over the celestial sphere, worked out once by sky/milky_way_bake.wgsl.
@group(0) @binding(12) var milky_way_tex: texture_2d<f32>;
// Which lights reach each tile of the screen, from lighting/light_tiles.wgsl.
@group(0) @binding(13) var<storage, read> light_tiles: array<u32>;
// Each light's shadow map, from lighting/light_shadows.wgsl.
@group(0) @binding(14) var light_shadows: texture_2d_array<f32>;

#include "lighting/light_tiles_common.wgsl"
#include "lighting/light_shadows_common.wgsl"

// Fraction of light a leaf or blade passes on to its far side, relative to what it reflects.
// Measured leaf spectra transmit about as much as they reflect across visible light (Jacquemoud
// and Ustin 2019), slightly less towards blue.
const FOLIAGE_TRANSMITTANCE: vec3f = vec3f(0.8, 0.8, 0.6);
// The share of a flash's illuminance that the whole lit cloud deck sends down onto a surface,
// besides what comes straight from the flash, as a Lambertian albedo-weighted term.
const FLASH_DECK: f32 = 0.12;

// A short ray through the depth buffer towards the sun. Catches the small-scale shadowing
// of dense geometry, which is finer than any shadow map texel.
fn contact_shadow(world: vec3f, normal: vec3f, view: vec3f, distance: f32, noise: f32) -> f32 {
    const STEPS: u32 = 10u;
    const LENGTH: f32 = 0.35;
    const MAX_GAP: f32 = 0.12;
    // The depth that one pixel of this surface spans: small when facing the camera, large at
    // grazing angles. A closer depth than that may just be the neighbouring pixel of the same
    // surface, so it proves nothing. Where this exceeds MAX_GAP, the test cannot tell the two
    // apart, and it is right to do nothing.
    let pixel_angle = 1.04 * frame.inv_viewport.y;
    let tolerance = max(0.002, 2.0 * distance * pixel_angle / max(abs(dot(normal, view)), 0.02));
    if tolerance >= MAX_GAP {
        return 1.0;
    }
    let start = world + normal * 0.01;
    let size = vec2i(frame.viewport);
    for (var i = 0u; i < STEPS; i++) {
        let t = (f32(i) + noise) / f32(STEPS) * LENGTH;
        let clip = frame.view_proj * vec4f(start + frame.sun_direction * t, 1.0);
        let ndc = clip.xyz / clip.w;
        let uv = vec2f(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
        if any(uv < vec2f(0.0)) || any(uv >= vec2f(1.0)) {
            return 1.0;
        }
        let scene_depth = textureLoad(depth_tex, min(vec2i(uv * frame.viewport), size - vec2i(1)), 0).r;
        if scene_depth > ndc.z {
            let gap = frame.near / ndc.z - frame.near / scene_depth;
            if gap > tolerance && gap < MAX_GAP {
                return 1.0 - clamp(1.0 - t / LENGTH, 0.0, 1.0) * 0.85;
            }
        }
    }
    return 1.0;
}

// The lamps of what is driven: one beam, brightest along its middle and gone at its edge,
// falling off with the square of the distance. It casts no shadows.
fn lamp_light(s: Surface, view: vec3f, world: vec3f) -> vec3f {
    let to_lamp = frame.lamp_position - world;
    let distance = length(to_lamp);
    if distance >= frame.lamp_range {
        return vec3f(0.0);
    }
    let direction = to_lamp / distance;
    if dot(s.normal, direction) <= 0.0 {
        return vec3f(0.0);
    }
    let along = dot(-direction, frame.lamp_direction);
    let beam = smoothstep(frame.lamp_spread, mix(frame.lamp_spread, 1.0, 0.85), along);
    if beam <= 0.0 {
        return vec3f(0.0);
    }
    let window = sq(clamp(1.0 - sq(sq(distance / frame.lamp_range)), 0.0, 1.0));
    // Two lamps a little apart, each with a reflector, are not a point: close to, their light
    // is spread, and what is under the car's nose is not burnt white.
    let falloff = window / (distance * distance + 55.0);
    return shade_light(s, view, direction) * frame.lamp_intensity * falloff * beam;
}

fn local_light(index: u32, s: Surface, view: vec3f, world: vec3f) -> vec3f {
    let light = lights[index];
    let to_light = light.position - world;
    let distance = length(to_light);
    if distance >= light.range {
        return vec3f(0.0);
    }
    let direction = to_light / distance;
    if dot(s.normal, direction) <= 0.0 {
        return vec3f(0.0);
    }
    let window = sq(clamp(1.0 - sq(sq(distance / light.range)), 0.0, 1.0));
    let falloff = window / max(distance * distance, 0.04);
    // Looked up a voxel and a half off the surface along its normal, as the trace this
    // replaced started, where the volume's own account of the surface no longer shadows it.
    let level = min(volume_level_at(world, 0u), VOLUME_LEVELS - 1u);
    let voxel = volume_params.levels[level].w;
    let from_light = world + s.normal * voxel * 1.5 - light.position;
    let open = light_shadow(index, from_light, length(from_light), voxel, s.normal);
    return shade_light(s, view, direction) * light.intensity * falloff * open;
}

// How fast a reflection texel stops counting as it leaves the pixel's surface, by its distance
// off that surface over the distance from the eye, squared. As gi/probe_integrate.wgsl has it.
const REFLECTION_PLANE_SHARPNESS: f32 = 7000.0;

// The reflection at a pixel, from the half-resolution texels around it. Each texel stands for
// the two by two pixels under it, and filtering between them as they lie would carry what a
// column reflects two pixels out onto the wall behind it. So each counts by how near the pixel's
// surface its nearest pixel lies. The texel over this pixel always counts in full.
fn reflection_on_surface(pixel: vec2i, world: vec3f, normal: vec3f, distance: f32) -> vec4f {
    let last = vec2i(textureDimensions(reflection_tex)) - vec2i(1);
    let last_pixel = vec2i(frame.viewport) - vec2i(1);
    let at = (vec2f(pixel) + vec2f(0.5)) * 0.5 - vec2f(0.5);
    let first = vec2i(floor(at));
    let part = at - floor(at);
    let sharpness = REFLECTION_PLANE_SHARPNESS / (distance * distance);
    var sum = vec4f(0.0);
    var total = 0.0;
    for (var i = 0; i < 4; i++) {
        let corner = vec2i((i & 1), (i >> 1));
        let texel = clamp(first + corner, vec2i(0), last);
        let side = mix(vec2f(1.0) - part, part, vec2f(corner));
        var weight = side.x * side.y;
        let nearest = clamp(clamp(pixel, texel * 2, texel * 2 + vec2i(1)), vec2i(0), last_pixel);
        if any(nearest != pixel) {
            let other_depth = textureLoad(depth_tex, nearest, 0).r;
            if other_depth <= 0.0 {
                continue;
            }
            let other = reconstruct_world(frame, vec2f(nearest) + vec2f(0.5), other_depth);
            let off = dot(other - world, normal);
            weight *= exp(-off * off * sharpness);
        }
        sum += textureLoad(reflection_tex, texel, 0) * weight;
        total += weight;
    }
    return sum / max(total, 1.0e-6);
}

// The most of the haze's glow that skylight alone gives the air, where no sun reaches it.
const AIR_SKY_SHARE: f32 = 0.25;

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3u) {
    let size = vec2u(frame.viewport);
    if any(id.xy >= size) {
        return;
    }
    let pixel = vec2i(id.xy);
    let center = vec2f(id.xy) + vec2f(0.5);
    let depth = textureLoad(depth_tex, pixel, 0).r;
    let emissive_model = textureLoad(emissive_tex, pixel, 0);
    let emissive = emissive_model.rgb;
    let foliage = emissive_model.a > 0.5;

    if depth <= 0.0 {
        let ndc = pixel_to_ndc(center, frame.inv_viewport);
        let far_point = frame.inv_view_proj * vec4f(ndc, 1.0e-6, 1.0);
        let direction = normalize(far_point.xyz / far_point.w - frame.camera_position);
        // The clear sky, seen through the clouds drawn for this view, with the sun behind them.
        var cloud = textureSampleLevel(cloud_tex, clamp_sampler, center * frame.inv_viewport, 0.0);
        // Texels hidden behind geometry hold -1, and are never next to a sky pixel; keep any
        // trace of them out regardless.
        cloud.a = clamp(cloud.a, 0.0, 1.0);
        var sky = clear_sky_radiance(toward_horizon(direction));
        if frame.debug_mode == DEBUG_NONE {
            // The angle one pixel spans, so stars stay a pixel across.
            let next = frame.inv_view_proj * vec4f(pixel_to_ndc(center + vec2f(1.0, 0.0), frame.inv_viewport), 1.0e-6, 1.0);
            let pixel_angle = length(direction - normalize(next.xyz / next.w - frame.camera_position));
            sky += sky_bodies(direction, pixel_angle);
        }
        sky = sky * cloud.a + cloud.rgb + flash_glow(direction);
        textureStore(output, pixel, vec4f(sky, 1.0));
        return;
    }

    // Views that the resolve pass wrote directly to emissive are shown as they are.
    if (frame.debug_mode >= DEBUG_CLUSTERS && frame.debug_mode <= DEBUG_NORMALS) || frame.debug_mode == DEBUG_WETNESS || frame.debug_mode == DEBUG_MOTION {
        textureStore(output, pixel, vec4f(emissive, 1.0));
        return;
    }

    let albedo_cavity = textureLoad(albedo_tex, pixel, 0);
    let packed = textureLoad(normal_tex, pixel, 0);
    let normal = oct_decode(packed.xy);
    let roughness = antialiased_roughness(normal_tex, depth_tex, pixel, normal, depth, packed.z);
    let metallic = packed.w;
    let world = reconstruct_world(frame, center, depth);
    let to_camera = frame.camera_position - world;
    let distance = length(to_camera);
    let view = to_camera / distance;
    let surface = make_surface(albedo_cavity.rgb, metallic, roughness, normal);

    let noise = interleaved_gradient_noise(vec2f(id.xy), frame.frame_index);

    // Sun.
    var color = vec3f(0.0);
    let sun_facing = dot(normal, frame.sun_direction);
    if sun_facing > 0.0 {
        var shadow = sun_shadow(world, normal, distance, noise);
        if shadow > 0.0 && distance < 40.0 {
            shadow *= contact_shadow(world, normal, view, distance, noise);
        }
        color += shade_light(surface, view, frame.sun_direction) * sun_irradiance_at(world) * shadow;
    } else if foliage {
        // Sunlight through a leaf from behind. The shadow is looked up for the lit side.
        let shadow = sun_shadow(world, -normal, distance, noise);
        let transmitted = surface.diffuse * FOLIAGE_TRANSMITTANCE / PI * -sun_facing;
        color += transmitted * sun_irradiance_at(world) * shadow;
    }

    // A flash of lightning: its light comes from the cloud it lit, that way, through whatever
    // stands between, which a short trace through the GI volume's distance field finds; and
    // from the whole lit cloud deck above, as the sky's light does. Only during a flash.
    if clouds.flash.w > 0.0 {
        let toward = clouds.flash.xyz;
        let flash = FLASH_COLOR * clouds.flash.w;
        if dot(normal, toward) > 0.0 {
            // A ray that runs out of steps among surfaces got away only partly.
            let blocked = trace_volume(world + normal * 0.25, toward, 0.1, 80.0, 24u);
            let open = select(blocked.escaped, 0.0, blocked.hit);
            color += shade_light(surface, view, toward) * flash * open;
        }
        let gi_open = textureLoad(gi_tex, pixel, 0).a;
        color += surface.diffuse * flash * FLASH_DECK * (normal.y * 0.5 + 0.5) * gi_open;
    }

    // Local lights: those that reach this pixel's tile.
    let tile = light_tile_base(id.xy / TILE_SIZE, size);
    let tile_lights = light_tiles[tile];
    for (var i = 0u; i < tile_lights; i++) {
        color += local_light(light_tiles[tile + 1u + i], surface, view, world);
    }
    if frame.lamp_range > 0.0 {
        color += lamp_light(surface, view, world);
    }

    // Indirect diffuse. Cavity darkens crevices the probes are too coarse to resolve.
    let gi = textureLoad(gi_tex, pixel, 0);
    let occlusion = gi.a * mix(1.0, albedo_cavity.a, 0.7);
    let irradiance = gi.rgb * frame.gi_intensity;
    color += surface.diffuse / PI * irradiance * occlusion;
    if foliage {
        // Skylight through the leaf. Probes only know the light arriving at its front, so the
        // back is estimated from the same, as it is for most foliage in a canopy.
        color += surface.diffuse * FOLIAGE_TRANSMITTANCE / PI * irradiance * occlusion * 0.5;
    }

    // Indirect specular. Rough surfaces did not trace, and their blurry reflection is
    // reasonably well estimated from the same irradiance.
    let reflection = reflection_on_surface(pixel, world, normal, distance);
    let n_dot_v = max(dot(normal, view), 0.0);
    let specular_scale = environment_brdf(surface.f0, roughness, n_dot_v);
    let rough_estimate = irradiance / PI;
    let traced_weight = clamp((0.65 - roughness) / 0.1, 0.0, 1.0) * clamp(reflection.a * 4.0, 0.0, 1.0);
    // Texels whose sampled pixel was too rough to trace hold no light and no weight, so the
    // accumulated light is divided by its weight rather than read as is, which would darken it.
    let traced = reflection.rgb / max(reflection.a, 1.0e-3);
    let incoming = mix(rough_estimate, traced * frame.gi_intensity, traced_weight);
    color += incoming * specular_scale * occlusion;

    color += emissive;

    // The air between glows only as far as light reaches it: sunlight by the shadow maps, and
    // skylight by how much of the open sky's light arrives here.
    let air_sun = air_lit(view, distance, noise);
    let open_sky = PI * luminance(clear_sky_radiance(vec3f(0.0, 1.0, 0.0)));
    let air_sky = clamp(luminance(irradiance) / max(open_sky, 1.0e-4), 0.0, 1.0) * AIR_SKY_SHARE;
    color = aerial_perspective_lit(color, world, view, distance, mix(air_sky, 1.0, air_sun));

    switch frame.debug_mode {
        case DEBUG_DIFFUSE_GI: {
            color = irradiance * occlusion / PI;
        }
        case DEBUG_REFLECTIONS: {
            color = incoming;
        }
        case DEBUG_VOXELS: {
            let hit = trace_volume(frame.camera_position, -view, 0.0, 320.0, 96u);
            color = select(sky_radiance(-view), hit.radiance, hit.hit);
        }
        default: {}
    }
    textureStore(output, pixel, vec4f(color, 1.0));
}
