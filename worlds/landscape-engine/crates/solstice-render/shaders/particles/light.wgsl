// The light each live particle receives, worked out once per particle rather than per pixel
// (docs/particles.md, step 2): that is the main saving over shading each fragment, and on a
// sprite it looks the same.
//
// - Sun: through the atmosphere and the clouds' shadow, times the cascaded shadow at the
//   particle, sampled at four points across particles larger than a metre.
// - Ambient: the radiance arriving from every direction, averaged in the particle over frames.
//   Each frame traces one direction and its opposite through the GI volume: a hit gives the
//   scene's light there, a miss the sky. So smoke in an archway is darker than in the open, and
//   spray by a sunlit cliff takes its colour. Beyond the volume, the sky alone.
// - Local lights: each of the scene's lights in range, unshadowed but for the GI volume.
// - Haze: the same aerial perspective as everything else, between particle and eye.
// - Glow: a black body's light at the particle's temperature, in the scene's units.

#include "common/environment.wgsl"
#include "common/scene.wgsl"
#include "particles/common.wgsl"
#include "particles/light_common.wgsl"

@group(0) @binding(0) var<uniform> sim: SimParams;
@group(0) @binding(1) var<storage, read> emitters: array<Emitter, MAX_EMITTERS>;
@group(0) @binding(2) var<storage, read_write> state: array<EmitterState, MAX_EMITTERS>;
@group(0) @binding(3) var<storage, read> particles: array<Particle>;
@group(0) @binding(4) var<storage, read> alive: array<u32>;
@group(0) @binding(5) var<storage, read_write> received: array<ParticleLight>;
@group(0) @binding(6) var<storage, read> lights: array<Light>;
@group(0) @binding(7) var<uniform> shadows: Shadows;
@group(0) @binding(8) var shadow_maps: texture_depth_2d_array;
@group(0) @binding(9) var shadow_sampler: sampler_comparison;

#include "lighting/sun_shadow.wgsl"

// How far ambient rays look for the scene, in as many steps as `sim.ray_steps` allows.
const RAY_REACH: f32 = 30.0;
// Samples the ambient average holds at most: about a quarter of a second at 60 frames.
const MOST_SAMPLES: f32 = 16.0;
// What a ray looking down that finds nothing within reach sees, as a share of the sky at the
// horizon: ground or sea a long way below, lit by the same sky.
const GROUND_SHARE: f32 = 0.3;
// Sunlit ground in full daylight takes about 100,000 lux, which the scene calls the daylight
// illuminance; a black body's luminance, in candela per square metre, scales the same way.
const DAYLIGHT_LUX: f32 = 100000.0;
// How many of the lights reaching a particle are traced for walls between: the strongest.
const TRACED_LIGHTS: u32 = 2u;
const NO_LIGHT: u32 = 0xffffffffu;

// The emitter this workgroup lights, found once by its first lane.
var<workgroup> group_emitter: u32;

// The radiance arriving at `origin` from `direction`.
fn incoming(origin: vec3f, direction: vec3f, inside: bool, voxel: f32) -> vec3f {
    if inside {
        let hit = trace_volume(origin, direction, voxel * 0.5, RAY_REACH, sim.ray_steps);
        if hit.hit {
            return hit.radiance;
        }
    }
    let sky = sky_radiance(toward_horizon(direction));
    return select(sky, sky * GROUND_SHARE, direction.y < 0.0);
}

// The black body's light comes from particles/light_common.wgsl.

// The open sky's average radiance over every direction, with the ground below as a share of
// the horizon: where a new particle's ambient light starts, before it has samples of its own.
fn open_sky() -> vec3f {
    var sum = sky_radiance(vec3f(0.0, 1.0, 0.0));
    for (var k = 0u; k < 4u; k++) {
        let a = f32(k) * 1.5707963;
        sum += sky_radiance(vec3f(cos(a) * 0.866, 0.5, sin(a) * 0.866));
    }
    let horizon = sky_radiance(toward_horizon(vec3f(1.0, -1.0, 0.0)));
    // Five directions above, and as many below.
    return (sum + horizon * GROUND_SHARE * 5.0) / 10.0;
}

@compute @workgroup_size(64)
fn main(@builtin(workgroup_id) group: vec3u, @builtin(local_invocation_index) lane: u32) {
    let written = 1u - sim.parity;
    // Starts rise from emitter to emitter, so the last whose start this group has reached is
    // its own.
    if lane == 0u {
        var found = MAX_EMITTERS;
        for (var k = 0u; k < MAX_EMITTERS; k++) {
            let start = state[k].light_start;
            if start > group.x {
                break;
            }
            if group.x < start + (atomicLoad(&state[k].alive[written]) + GROUP_SIZE - 1u) / GROUP_SIZE {
                found = k;
            }
        }
        group_emitter = found;
    }
    let e = workgroupUniformLoad(&group_emitter);
    if e >= MAX_EMITTERS {
        return;
    }
    let emitter = emitters[e];
    let i = (group.x - state[e].light_start) * GROUP_SIZE + lane;
    if (emitter.flags & FLAG_LIVE) == 0u || i >= atomicLoad(&state[e].alive[written]) {
        return;
    }
    let slot = alive[written * sim.capacity + emitter.base + i];
    let p = particles[slot];

    var l = unpack_light(received[slot]);
    let fresh = l.index != p.index || l.samples <= 0.0;
    // Between turns a particle keeps the light it had; a new one is lit at once. Turns go by
    // whole workgroups, so that those not due sit idle rather than wait on one lane that is.
    // New particles head each list, so they share workgroups too. A particle's place in the
    // list moves as particles are born and die, so one that has missed its turn for as long as
    // a turn lasts is lit anyway: none waits longer than that.
    let turn = group.x - state[e].light_start + frame.frame_index;
    let overdue = frame.frame_index - l.lit_frame >= sim.light_interval;
    if !fresh && !overdue && turn % sim.light_interval != 0u {
        return;
    }
    if fresh {
        l.index = p.index;
        l.ambient = open_sky() * frame.gi_intensity;
        l.samples = 2.0;
    }
    l.lit_frame = frame.frame_index;
    l.dither = particle_random(p.index, frame.frame_index, 2u);
    let to_camera = frame.camera_position - p.position;
    let distance = max(length(to_camera), 1.0e-4);
    let view = to_camera / distance;

    if (emitter.flags & FLAG_LIT) != 0u {
        // The sun, shadowed. Large particles take the shadow across their width.
        let noise = hash_to_unit(p.index ^ (frame.frame_index * 2654435761u));
        var shade = sun_shadow(p.position, frame.sun_direction, distance, noise);
        if p.size > 1.0 {
            let right = vec3f(frame.view[0].x, frame.view[1].x, frame.view[2].x);
            let up = vec3f(frame.view[0].y, frame.view[1].y, frame.view[2].y);
            let reach = p.size * 0.6;
            shade += sun_shadow(p.position + right * reach, frame.sun_direction, distance, noise);
            shade += sun_shadow(p.position - right * reach, frame.sun_direction, distance, noise);
            shade += sun_shadow(p.position + up * reach, frame.sun_direction, distance, noise);
            shade += sun_shadow(p.position - up * reach, frame.sun_direction, distance, noise);
            shade /= 5.0;
        }
        l.sun = sun_irradiance_at(p.position) * shade;

        // Ambient: one direction and its opposite each frame, which also evens out the
        // difference between above and below.
        let level = volume_level_at(p.position, 0u);
        let inside = level < VOLUME_LEVELS;
        let voxel = volume_params.levels[min(level, VOLUME_LEVELS - 1u)].w;
        let u = vec2f(
            particle_random(p.index, frame.frame_index, 0u),
            particle_random(p.index, frame.frame_index, 1u),
        );
        let z = 2.0 * u.x - 1.0;
        let ring = sqrt(max(1.0 - z * z, 0.0));
        let direction = vec3f(ring * cos(TAU * u.y), z, ring * sin(TAU * u.y));
        // Indirect light, which the scene's GI intensity scales as it does for surfaces.
        let pair = 0.5 * (incoming(p.position, direction, inside, voxel)
            + incoming(p.position, -direction, inside, voxel)) * frame.gi_intensity;
        l.ambient = mix(l.ambient, pair, 2.0 / (l.samples + 2.0));
        l.samples = min(l.samples + 2.0, MOST_SAMPLES);

        // Local lights in range. Tracing each through the GI volume, to stop light through
        // walls, costs far more than the rest of the loop, so only the strongest few at this
        // particle are traced; the rest, each fainter, count as open.
        var nearby = vec3f(0.0);
        var strongest = array<u32, TRACED_LIGHTS>(NO_LIGHT, NO_LIGHT);
        var strength = array<f32, TRACED_LIGHTS>(0.0, 0.0);
        var arriving = array<vec3f, TRACED_LIGHTS>(vec3f(0.0), vec3f(0.0));
        for (var k = 0u; k < frame.light_count; k++) {
            let light = lights[k];
            let reach = length(light.position - p.position);
            if reach >= light.range {
                continue;
            }
            let window = sq(clamp(1.0 - sq(sq(reach / light.range)), 0.0, 1.0));
            let incoming_light = light.intensity * window / max(reach * reach, 0.04);
            let level = luminance(incoming_light);
            // Keep the strongest in order; whatever is pushed out counts as open.
            var held = incoming_light;
            var held_level = level;
            var held_index = k;
            for (var s = 0u; s < TRACED_LIGHTS; s++) {
                if held_level > strength[s] {
                    let index = strongest[s];
                    let light_level = strength[s];
                    let light_in = arriving[s];
                    strongest[s] = held_index;
                    strength[s] = held_level;
                    arriving[s] = held;
                    held_index = index;
                    held_level = light_level;
                    held = light_in;
                }
            }
            if held_index != NO_LIGHT {
                nearby += held;
            }
        }
        for (var s = 0u; s < TRACED_LIGHTS; s++) {
            if strongest[s] == NO_LIGHT {
                continue;
            }
            var open = 1.0;
            if inside {
                let to_light = lights[strongest[s]].position - p.position;
                let reach = max(length(to_light), 1.0e-4);
                open = volume_visibility(p.position, to_light / reach, voxel * 0.5, max(reach - voxel * 2.0, 0.0), 12u);
            }
            nearby += arriving[s] * open;
        }
        l.lamps = nearby / PI;
    } else {
        l.sun = vec3f(0.0);
        l.ambient = vec3f(0.0);
        l.lamps = vec3f(0.0);
    }

    // The air between: what aerial perspective adds over black, and what it leaves.
    l.haze = aerial_perspective(vec3f(0.0), p.position, view, distance);
    l.transmittance = exp(-distance * HAZE);

    l.glow = vec3f(0.0);
    if (emitter.flags & FLAG_GLOW) != 0u {
        let kelvin = particle_temperature(p, emitter);
        l.glow = black_body(kelvin) * emitter.temperature_glow.w * frame.daylight_illuminance / DAYLIGHT_LUX;
    }
    received[slot] = pack_light(l);
}
