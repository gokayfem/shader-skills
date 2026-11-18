// The water pass: where the sea's surface is nearer than the opaque scene, shades it and puts it
// over the lit scene, and gives the water's own depth and motion to everything after. Elsewhere
// the lit scene passes through unchanged.
//
// The scene and sky reflected by Fresnel's law off a surface as rough as its unresolved ripples,
// the sun's highlight, and what is seen through the water: the bed, refracted, dimmed by the
// water's absorption over the path down to it and back, and the glow of the water's own body,
// all under the sun's shadows and through the same haze as everything else (docs/ocean.md,
// step 3). Foam comes in a later step.

#include "common/environment.wgsl"
#include "common/brdf.wgsl"
#include "ocean/surface_params.wgsl"
#include "lighting/sun_shadow.wgsl"
#include "gi/screen_trace.wgsl"
#include "ocean/seabed.wgsl"
#include "ocean/optics.wgsl"
#include "rain/rings.wgsl"
#include "common/wind.wgsl"

// The opaque scene, lit, with its depth and motion.
@group(0) @binding(0) var lit: texture_2d<f32>;
// On a GPU allowing 16 sampled textures per stage, the depth pyramid's mip 0, the same depth,
// is read in its place.
// opaque-depth:begin
@group(0) @binding(1) var opaque_depth: texture_2d<f32>;
// opaque-depth:end
@group(0) @binding(2) var opaque_motion: texture_2d<f32>;
// The water layer: its depth, and where its water lay on the calm sea.
@group(0) @binding(3) var water_depth: texture_depth_2d;
@group(0) @binding(4) var water_calm: texture_2d<f32>;
// The waves' slopes and stretch, and their displacement (its shear in w), per cascade.
@group(0) @binding(5) var derivatives: texture_2d_array<f32>;
@group(0) @binding(6) var displacement: texture_2d_array<f32>;
@group(0) @binding(7) var wave_sampler: sampler;
@group(0) @binding(8) var<uniform> surface: SurfaceParams;
@group(0) @binding(9) var out_color: texture_storage_2d<rgba16float, write>;
@group(0) @binding(10) var out_depth: texture_storage_2d<r32float, write>;
@group(0) @binding(11) var out_motion: texture_storage_2d<rg32float, write>;
// The sun's shadows, as the lighting pass has them.
@group(0) @binding(12) var<uniform> shadows: Shadows;
@group(0) @binding(13) var shadow_maps: texture_depth_2d_array;
@group(0) @binding(14) var shadow_sampler: sampler_comparison;
// For reflections of the scene: the opaque scene's closest-depth pyramid, and its normals.
@group(0) @binding(15) var depth_pyramid: texture_2d<f32>;
@group(0) @binding(16) var normal_tex: texture_2d<f32>;
// The seabed under the sea, seen from above, for how deep the water is.
@group(0) @binding(17) var seabed_map: texture_depth_2d;

const DEBUG_OCEAN: u32 = 10u;
// Depth of water over which the sea fades into the shore, in metres.
const SHORE_FADE: f32 = 0.3;

// The surface's normal at `calm`, from every cascade's slopes at the level whose texels match
// `footprint` metres, over water `depth` deep: as the waves shrink in the shallows (`shoal`), so
// do their slopes.
fn wave_normal(calm: vec2f, footprint: f32, depth: f32) -> vec3f {
    var d = vec4f(0.0);
    var shear = 0.0;
    var height = 0.0;
    for (var cascade = 0u; cascade < 3u; cascade++) {
        let tile = surface.sizes[cascade];
        let level = max(log2(footprint / (tile / surface.grid.x)), 0.0);
        d += textureSampleLevel(derivatives, wave_sampler, calm / tile, cascade, level);
        let moved = textureSampleLevel(displacement, wave_sampler, calm / tile, cascade, level);
        shear += moved.w;
        height += moved.y;
    }
    let sway = clamp(depth / SWAY_DEPTH, 0.0, 1.0);
    d = vec4f(d.xy * shoal_slope(height, depth), d.zw * sway);
    shear *= sway;
    // The displaced surface's tangents along x and z, with the horizontal displacement's
    // stretch and shear (∂Dz/∂x = ∂Dx/∂z), and their cross product. Where a crest folds over,
    // the surface turns back on itself; it is held upright rather than flipped.
    let choppiness = surface.grid.y;
    let along_x = vec3f(1.0 + choppiness * d.z, d.x, choppiness * shear);
    let along_z = vec3f(choppiness * shear, d.y, 1.0 + choppiness * d.w);
    var normal = cross(along_z, along_x);
    normal.y = max(normal.y, 0.05);
    return normalize(normal);
}

// The surface's roughness, as GGX alpha, for a pixel `footprint` metres across: the slopes of
// every wave too short to show in it, whether finer than the pixel or than the cascades draw,
// from the spectrum (spectrum.rs, `unresolved_slopes`). Further off, more waves fall below a
// pixel, so the water turns rougher and the sun's glitter spreads towards the horizon.
fn surface_roughness(footprint: f32) -> f32 {
    let shown = min(PI / max(footprint, 1.0e-4), surface.slope_range.z);
    let at = clamp((log2(shown) / log2(10.0) - surface.slope_range.x) / surface.slope_range.y, 0.0, 31.0);
    let i = u32(floor(at));
    let j = min(i + 1u, 31u);
    let below = surface.slopes[i / 4u][i % 4u];
    let above = surface.slopes[j / 4u][j % 4u];
    let mean_square_slope = mix(below, above, at - f32(i));
    let smoothest = surface.slope_range.w;
    return sqrt(smoothest * smoothest + mean_square_slope);
}


@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3u) {
    let size = vec2u(frame.viewport);
    if any(id.xy >= size) {
        return;
    }
    let pixel = vec2i(id.xy);
    let behind = textureLoad(lit, pixel, 0);
    let scene_depth = textureLoad(opaque_depth, pixel, 0).r;
    let water = textureLoad(water_depth, pixel, 0);
    // Reversed depth: nearer is larger. No water here, or something stands in front of it; or a
    // debug view of the opaque scene, which water would hide.
    let other_view = frame.debug_mode != DEBUG_NONE && frame.debug_mode != DEBUG_OCEAN;
    if water <= scene_depth || other_view {
        textureStore(out_color, pixel, behind);
        textureStore(out_depth, pixel, vec4f(scene_depth, 0.0, 0.0, 0.0));
        textureStore(out_motion, pixel, textureLoad(opaque_motion, pixel, 0));
        return;
    }

    let center = vec2f(id.xy) + vec2f(0.5);
    let world = reconstruct_world(frame, center, water);
    let to_camera = frame.camera_position - world;
    let distance = length(to_camera);
    let view = to_camera / distance;
    // How much sea one pixel covers: wider at grazing angles.
    let next = reconstruct_world(frame, center + vec2f(1.0, 0.0), water);
    let footprint = length(next - world) / sqrt(max(abs(view.y), 0.02));
    let calm = textureLoad(water_calm, pixel, 0).xy;
    var normal = wave_normal(calm, footprint, water_depth_at(calm));
    // Rain rings the water and roughens it (rain/rings.wgsl).
    let rained = rain_rings(world.xz, frame.time, clouds.rain, footprint);
    normal = normalize(normal + vec3f(-rained.x, 0.0, -rained.y));
    // Waves turned away from the camera show their backs only at grazing angles; lean them back
    // into view, as the waves in front would hide them.
    let facing = dot(normal, view);
    if facing < 0.02 {
        normal = normalize(normal + view * (0.02 - facing));
    }

    var color: vec3f;
    if frame.debug_mode == DEBUG_OCEAN {
        color = normal * 0.5 + vec3f(0.5);
    } else {
        let n_dot_v = max(dot(normal, view), 1.0e-3);
        let fresnel = WATER_F0 + (1.0 - WATER_F0) * pow(1.0 - n_dot_v, 5.0);
        // Gusts roughen the short waves as they cross, more than the long ones can follow:
        // cat's paws, dark patches running downwind. Cox and Munk found the waves' mean square
        // slope grows with the wind's speed as 0.003 + 0.00512 U.
        let air = unpack_wind(surface.wind[0], surface.wind[1]);
        let here = length(wind_velocity(air, world.xz));
        let paws = (0.003 + 0.00512 * here) / (0.003 + 0.00512 * length(air.mean));
        let roughness = sqrt(sq(surface_roughness(footprint)) * paws + rained.z);
        // One reflected ray per pixel, off a facet the ripples tilt at random by the surface's
        // roughness; temporal accumulation gathers them into the blurred reflection.
        let jitter = vec2f(
            interleaved_gradient_noise(vec2f(id.xy), frame.frame_index),
            interleaved_gradient_noise(vec2f(id.yx) + vec2f(31.0, 17.0), frame.frame_index),
        );
        let spread = sqrt(-2.0 * log(max(1.0 - jitter.x, 1.0e-4))) * roughness * 0.7071;
        let basis = orthonormal_basis(normal);
        let facet = normalize(normal + (basis[0] * cos(TAU * jitter.y) + basis[1] * sin(TAU * jitter.y)) * spread);
        var direction = reflect(-view, facet);
        if dot(direction, normal) <= 0.0 {
            direction = reflect(-view, normal);
        }
        let reflected = reflection(world, normal, direction);

        // Sunlight at the water, after the clouds, and the shadows of what stands in its way:
        // cliffs, the fort, the ship. Shadows show on water by dimming its highlight and the
        // light it scatters from within; the sky it reflects is untouched.
        let noise = interleaved_gradient_noise(vec2f(id.xy) + vec2f(5.3, 11.7), frame.frame_index);
        let shadow = sun_shadow(world, vec3f(0.0, 1.0, 0.0), distance, noise);
        let sun_light = sun_irradiance_at(world) * shadow;

        // The sun's highlight off the waves.
        let sun = frame.sun_direction;
        let n_dot_l = dot(normal, sun);
        var highlight = vec3f(0.0);
        if n_dot_l > 0.0 {
            let a = roughness;
            let h = normalize(view + sun);
            let specular = d_ggx(max(dot(normal, h), 0.0), a) * v_smith(n_dot_v, n_dot_l, a)
                * f_schlick(vec3f(WATER_F0), max(dot(view, h), 0.0));
            highlight = specular * n_dot_l * sun_light;
        }

        // Light scattered back out of the water's body: its share of what is not absorbed, of
        // the light from the sky and sun that gets in.
        let extinction = surface.absorption.rgb + surface.scattering.rgb;
        let albedo = surface.scattering.rgb / max(extinction, vec3f(1.0e-4));
        let skylight = clear_sky_radiance(vec3f(0.0, 1.0, 0.0)) * PI;
        let sunlight = sun_light * max(sun.y, 0.0);
        let body = albedo * (skylight + sunlight) / PI * 0.5;

        let underwater = through_water(center, world, view, normal, water, scene_depth, body, extinction);
        color = mix(underwater, reflected, fresnel) + highlight;
        color = aerial_perspective(color, world, view, distance);
        // At the very edge, where the water thins to nothing over the sand, it fades into the
        // shore rather than ending in a line: a sheet a few centimetres deep barely shows. The
        // shore behind has its own haze already.
        if scene_depth > 0.0 {
            let bed = reconstruct_world(frame, center, scene_depth);
            let depth_here = max(world.y - bed.y, 0.0);
            color = mix(behind.rgb, color, smoothstep(0.0, SHORE_FADE, depth_here));
        }
    }

    let prev_clip = frame.prev_view_proj * vec4f(world, 1.0);
    let current_clip = frame.unjittered_view_proj * vec4f(world, 1.0);
    let motion = (prev_clip.xy / prev_clip.w - current_clip.xy / current_clip.w) * vec2f(0.5, -0.5);

    textureStore(out_color, pixel, vec4f(color, 1.0));
    textureStore(out_depth, pixel, vec4f(water, 0.0, 0.0, 0.0));
    textureStore(out_motion, pixel, vec4f(motion, 0.0, 0.0));
}
