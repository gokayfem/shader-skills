// Bind group 1 of every lighting pass: frame constants, sky LUTs, the GI volume and the clouds.
// Includes the functions that sample them.

#include "common/math.wgsl"
#include "common/frame.wgsl"
#include "common/atmosphere.wgsl"
#include "clouds/params.wgsl"

const VOLUME_LEVELS: u32 = 5u;

struct VolumeParams {
    // Per clipmap level: world position of the grid's corner, and the voxel size.
    levels: array<vec4f, 5>,
    resolution: f32,
    inv_resolution: f32,
    _pad0: f32,
    _pad1: f32,
}

@group(1) @binding(0) var<uniform> frame: Frame;
@group(1) @binding(1) var<uniform> volume_params: VolumeParams;
@group(1) @binding(2) var volume_atlas: texture_3d<f32>;
@group(1) @binding(3) var clamp_sampler: sampler;
@group(1) @binding(4) var transmittance_lut: texture_2d<f32>;
@group(1) @binding(5) var sky_view_lut: texture_2d<f32>;
@group(1) @binding(6) var repeat_sampler: sampler;
// The clouds over the whole sky (light in RGB, how much sky shows through in A), and how much
// sunlight they let through to the ground around the camera. Cloud passes, which make these,
// see placeholders: no cloud and no shadow.
@group(1) @binding(7) var cloud_sky: texture_2d<f32>;
@group(1) @binding(8) var cloud_shadow_map: texture_2d<f32>;
@group(1) @binding(9) var<uniform> clouds: CloudParams;
// One texel: the shadow map's mean.
@group(1) @binding(10) var cloud_shadow_mean: texture_2d<f32>;

// Height of the viewer above the planet centre, in kilometres.
fn viewer_radius() -> f32 {
    return GROUND_RADIUS + max(frame.camera_position.y, 1.0) * 0.001;
}

// The colour of the night sky's own glow, from airglow and scattered starlight: deep blue
// overhead, paler towards the horizon.
const NIGHT_GLOW_ZENITH: vec3f = vec3f(0.3, 0.45, 1.0);
const NIGHT_GLOW_HORIZON: vec3f = vec3f(0.62, 0.7, 0.9);

// Radiance of the cloudless sky towards `direction`, excluding the sun disc and the stars.
fn clear_sky_radiance(direction: vec3f) -> vec3f {
    let color = textureSampleLevel(sky_view_lut, repeat_sampler, sky_view_uv(direction), 0.0).rgb;
    // The night glow is several times brighter towards the horizon, where the line of sight
    // crosses more of the glowing air, and dark overhead, where the stars stand out.
    let horizon = exp(-clamp(direction.y, 0.0, 1.0) * 5.0);
    let glow = mix(NIGHT_GLOW_ZENITH, NIGHT_GLOW_HORIZON, horizon) * frame.night_sky * (0.4 + 2.0 * horizon);
    return color * frame.sun_illuminance * frame.sky_intensity + glow * frame.sky_intensity;
}

// `direction`, raised to just above the horizon if it points below it. Far beyond the scene,
// land and sea fade into the air until they match the sky just above them, as the sky at the
// horizon is itself the glow of a long path of air: so anything seen or reflected past the edge
// of the scene takes the horizon's colour, and sea and sky meet without a seam.
fn toward_horizon(direction: vec3f) -> vec3f {
    return normalize(vec3f(direction.x, max(direction.y, 0.003), direction.z));
}

// Distance over which distant objects dissolve into the air, as extinction per metre.
const HAZE: f32 = 0.0011;
// How much thicker a downpour makes the air: heavy rain cuts how far one sees to a kilometre or
// two, and greys the distance.
const RAIN_HAZE: f32 = 2.5;

// `color` of a surface at `world`, `distance` from the camera along `view` (towards the camera),
// seen through the air between: distant things fade towards the sky colour behind them.
fn aerial_perspective(color: vec3f, world: vec3f, view: vec3f, distance: f32) -> vec3f {
    return aerial_perspective_lit(color, world, view, distance, 1.0);
}

// The same, where only `lit` of the air between is lit: under a roof the air scatters almost no
// light towards the eye, and a room would otherwise fill with the glow of the open sky.
fn aerial_perspective_lit(color: vec3f, world: vec3f, view: vec3f, distance: f32, lit: f32) -> vec3f {
    let transmittance = exp(-distance * HAZE * (1.0 + RAIN_HAZE * clouds.rain));
    // How far into the haze this is: nearby haze glows with light from all around, but over a
    // long path the air takes the colour of the sky at the horizon behind the object.
    let far = sq(1.0 - transmittance);
    // Sampled above the horizon and partly desaturated: the air between here and a distant
    // object scatters light from all around, not only the long path to the horizon line.
    let haze_direction = normalize(vec3f(-view.x, max(-view.y, 0.2), -view.z));
    // Haze is the air's own glow, so it comes from the clear sky, not the clouds in it. Where
    // clouds shade the air between here and the surface, less sunlight scatters into it.
    let shade = mix(0.35, 1.0, cloud_shadow((world + frame.camera_position) * 0.5));
    let haze_sky = clear_sky_radiance(haze_direction) * shade;
    var haze_color = mix(vec3f(luminance(haze_sky)), haze_sky, 0.55);
    // That direction can land on the glow around the sun even when looking down at nearby ground,
    // whose radiance is so high that this small share of it shows as smoke across shadows. So the
    // haze is limited relative to the open sky on the side away from the sun.
    let away_from_sun = normalize(vec3f(-frame.sun_direction.x, 0.2, -frame.sun_direction.z));
    let haze_limit = luminance(clear_sky_radiance(away_from_sun)) * shade * 2.0;
    haze_color *= min(haze_limit / max(luminance(haze_color), 1.0e-4), 1.0);
    haze_color = mix(haze_color, clear_sky_radiance(toward_horizon(-view)) * shade, far);
    return mix(haze_color * lit, color, transmittance);
}

// Lightning's light: bluish white, and how much of a flash's illuminance the glowing cloud
// deck sends towards each direction of the sky, as radiance: about its illuminance over π,
// gathered towards the flash.
const FLASH_COLOR: vec3f = vec3f(0.82, 0.88, 1.0);
const FLASH_SKY: f32 = 0.3;

// The glow of the clouds lit from within by a flash of lightning, seen towards `direction`:
// brightest towards the flash, and a little all over the cloud deck, which it lights from
// above. Nothing between flashes.
fn flash_glow(direction: vec3f) -> vec3f {
    if clouds.flash.w <= 0.0 {
        return vec3f(0.0);
    }
    let toward = max(dot(direction, clouds.flash.xyz), 0.0);
    let lobe = 0.15 + 1.5 * pow(toward, 8.0);
    let above = smoothstep(-0.1, 0.2, direction.y);
    return FLASH_COLOR * clouds.flash.w * FLASH_SKY * lobe * above;
}

// Radiance of the sky towards `direction`, clouds included, excluding the sun disc.
fn sky_radiance(direction: vec3f) -> vec3f {
    let cloud = textureSampleLevel(cloud_sky, repeat_sampler, sky_view_uv(direction), 0.0);
    return clear_sky_radiance(direction) * cloud.a + cloud.rgb + flash_glow(direction);
}

// How much of the sun is above the horizon seen from `radius` (km from the planet's centre):
// the horizon dips the higher one is, so clouds and peaks keep the sun after the ground loses it.
// The transmittance table does not know the ground is there, so this blocks the light instead.
fn above_horizon(radius: f32, mu: f32) -> f32 {
    let dip = -sqrt(max(1.0 - (GROUND_RADIUS / radius) * (GROUND_RADIUS / radius), 0.0));
    // Across the sun's own width, so that it sets gradually rather than switching off.
    return smoothstep(dip - 0.006, dip + 0.006, mu);
}

// Sunlight above the clouds, after the atmosphere has filtered it.
fn sun_irradiance() -> vec3f {
    let r = viewer_radius();
    let uv = transmittance_uv(r, frame.sun_direction.y);
    let transmittance = textureSampleLevel(transmittance_lut, clamp_sampler, uv, 0.0).rgb;
    return transmittance * frame.sun_color * frame.sun_illuminance
        * above_horizon(r, frame.sun_direction.y);
}

// How much sunlight the clouds let through to `p`: 1 in the open, less in a cloud's shadow.
fn cloud_shadow(p: vec3f) -> f32 {
    if clouds.enabled < 0.5 {
        return 1.0;
    }
    // The map holds light at the ground. A point above it is lit along the same line to the
    // sun, from where that line meets the ground.
    let ground = p.xz - frame.sun_direction.xz * (p.y / max(frame.sun_direction.y, 0.08));
    let uv = (ground - clouds.shadow_origin) / clouds.shadow_extent;
    let mapped = textureSampleLevel(cloud_shadow_map, clamp_sampler, clamp(uv, vec2f(0.0), vec2f(1.0)), 0.0).r;
    // Beyond the map, its mean stands in, blended in across its outer band. From a height the
    // horizon lies far past the map's edge, and that edge's texels, held out to the horizon,
    // would stretch into streaks across the sea. The far field is the same in every direction:
    // one mean for the whole horizon, as a stand-in for clouds the map does not reach.
    let edge = max(abs(uv.x - 0.5), abs(uv.y - 0.5)) * 2.0;
    let mean = textureLoad(cloud_shadow_mean, vec2i(0), 0).r;
    return mix(mapped, mean, smoothstep(0.75, 1.0, edge));
}

// Sunlight reaching `p`, after the atmosphere and the clouds.
fn sun_irradiance_at(p: vec3f) -> vec3f {
    return sun_irradiance() * cloud_shadow(p);
}

// The finest clipmap level containing `p` with some margin, or VOLUME_LEVELS if none.
fn volume_level_at(p: vec3f, first_level: u32) -> u32 {
    for (var level = first_level; level < VOLUME_LEVELS; level++) {
        let grid = volume_params.levels[level];
        let local = (p - grid.xyz) / grid.w;
        let margin = 1.5;
        let upper = volume_params.resolution - margin;
        if all(local > vec3f(margin)) && all(local < vec3f(upper)) {
            return level;
        }
    }
    return VOLUME_LEVELS;
}

// Radiance in RGB, distance to the nearest surface in A (world units).
fn sample_volume(p: vec3f, level: u32) -> vec4f {
    let grid = volume_params.levels[level];
    let local = (p - grid.xyz) / grid.w * volume_params.inv_resolution;
    // Each level occupies one slab of the atlas. Stay half a texel inside, so that
    // filtering cannot read a neighbouring level.
    let half_texel = 0.5 * volume_params.inv_resolution;
    let z = clamp(local.z, half_texel, 1.0 - half_texel);
    let atlas_z = (z + f32(level)) / f32(VOLUME_LEVELS);
    return textureSampleLevel(volume_atlas, clamp_sampler, vec3f(local.xy, atlas_z), 0.0);
}

struct VolumeHit {
    hit: bool,
    t: f32,
    // Of what was hit. Of a ray that ran out of steps, the nearest surface to where it stopped.
    radiance: vec3f,
    // For a miss, how far the ray got away: 1 if it left the volume or went its whole length.
    // One that ran out of steps did so among surfaces, and got away less the nearer one it
    // stopped. See `miss_radiance`.
    escaped: f32,
}

// How close to a surface counts as a hit for lighting, in voxels: generous, so that thin walls
// stop light.
const LIGHTING_HIT_VOXELS: f32 = 0.85;

// Sphere traces the GI volume. `start` skips self-intersection near the ray's origin.
fn trace_volume(origin: vec3f, direction: vec3f, start: f32, max_t: f32, max_steps: u32) -> VolumeHit {
    return trace_volume_with(origin, direction, start, max_t, max_steps, LIGHTING_HIT_VOXELS);
}

// `hit_voxels` is how close counts as a hit, in voxels. Reflections can use less than lighting,
// so that thin objects look thin.
fn trace_volume_with(origin: vec3f, direction: vec3f, start: f32, max_t: f32, max_steps: u32, hit_voxels: f32) -> VolumeHit {
    return trace_volume_levels(origin, direction, start, max_t, max_steps, hit_voxels, false);
}

// For rays that may head towards the camera, as a lamp's do when it stands further out than
// the camera. Each step looks for the finest level from the top: a ray keeping the level it
// started in would judge everything near the camera by that level's coarse voxels, and its
// threshold would stop it well short of a floor it only skims. A ray still in the coarser level
// is judged by it, so a low lamp's light can stop a little short just inside a level's edge.
fn trace_volume_inwards(origin: vec3f, direction: vec3f, start: f32, max_t: f32, max_steps: u32) -> VolumeHit {
    return trace_volume_levels(origin, direction, start, max_t, max_steps, LIGHTING_HIT_VOXELS, true);
}

// `any_level` looks for the finest level at every step; otherwise a ray only ever moves out to
// coarser ones, which is all a ray starting by the camera needs.
fn trace_volume_levels(origin: vec3f, direction: vec3f, start: f32, max_t: f32, max_steps: u32, hit_voxels: f32, any_level: bool) -> VolumeHit {
    var t = start;
    var level = 0u;
    var measured_level = VOLUME_LEVELS;
    var previous_distance = 0.0;
    // The last place looked at: its radiance, and its distance to a surface in voxels.
    var last = vec4f(0.0);
    for (var step = 0u; step < max_steps && t < max_t; step++) {
        let p = origin + direction * t;
        level = volume_level_at(p, select(level, 0u, any_level));
        if level >= VOLUME_LEVELS {
            break;
        }
        let voxel = volume_params.levels[level].w;
        let s = sample_volume(p, level);
        last = vec4f(s.rgb, s.a / voxel);
        // A hit needs the ray to be closing in on a surface, measured within a single level:
        // distances from different levels are not comparable. Without this, a ray skimming away
        // from flat ground falsely hits it on changing to a coarser level, where the threshold
        // doubles, which draws the boundaries of the levels onto glossy surfaces as lines.
        let approaching = level == measured_level && s.a <= previous_distance;
        measured_level = level;
        previous_distance = s.a;
        if s.a < voxel * hit_voxels && approaching {
            return VolumeHit(true, t, s.rgb, 0.0);
        }
        t += max(s.a - voxel * 0.5, voxel * 0.6);
    }
    if level >= VOLUME_LEVELS || t >= max_t {
        return VolumeHit(false, t, vec3f(0.0), 1.0);
    }
    // Out of steps. A ray takes its smallest steps beside a surface, so it has spent them
    // skimming one, or threading through many: it is among surfaces still, and likelier to end
    // on one than to reach the sky. In the open, a few voxels from anything, it may yet.
    return VolumeHit(false, t, last.rgb, smoothstep(1.0, 3.0, last.a));
}

// How far below the horizon a ray that left the volume points, as its direction's height, before
// it is taken to have got under the ground, and by where it certainly has.
const UNDERGROUND_FROM: f32 = -0.1;
const UNDERGROUND_BY: f32 = -0.35;

// What a ray that hit nothing brings back, where `sky` is what the open sky gives in its
// direction. The sky is the answer only for a ray that got away. One that ran out of steps
// among surfaces brings the light of the nearest: in a hall, calling that sky paints daylight
// onto every floor a ray skims. And one that left the volume pointing well down has passed
// under the ground, where there is no sky, unless there is a sea below to mirror it.
fn miss_radiance(hit: VolumeHit, direction: vec3f, sky: vec3f) -> vec3f {
    var beyond = sky;
    if frame.sea_level <= -1.0e29 {
        beyond *= smoothstep(UNDERGROUND_BY, UNDERGROUND_FROM, direction.y);
    }
    return mix(hit.radiance, beyond, hit.escaped);
}

// How far inside the outermost level `p` is: 0 at its edge, reaching 1 well inside. Rays from
// beyond the edge see only sky, so anything that blends by this avoids a visible boundary there.
fn volume_edge_fade(p: vec3f) -> f32 {
    let grid = volume_params.levels[VOLUME_LEVELS - 1u];
    let local = (p - grid.xyz) / grid.w * volume_params.inv_resolution;
    let edge = min(min(local.x, 1.0 - local.x), min(local.z, 1.0 - local.z));
    return smoothstep(0.02, 0.2, edge);
}

// True if nothing in the volume blocks the path towards `direction`.
fn volume_visibility(origin: vec3f, direction: vec3f, start: f32, max_t: f32, max_steps: u32) -> f32 {
    return select(1.0, 0.0, trace_volume(origin, direction, start, max_t, max_steps).hit);
}
