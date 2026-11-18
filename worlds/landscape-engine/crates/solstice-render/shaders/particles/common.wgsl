// The particle system's records, and how particles are born and move (docs/particles.md).
// Twins of the structs and functions in particles/reference.rs, which tests compare against.

#include "common/math.wgsl"
#include "common/noise.wgsl"

const FLAG_LIVE: u32 = 1u;
const FLAG_ADD: u32 = 2u;
const FLAG_SURFACE: u32 = 4u;
const FLAG_LIT: u32 = 8u;
const FLAG_GLOW: u32 = 16u;
const FLAG_SOLID: u32 = 32u;
const FLAG_SORT: u32 = 64u;
const FLAG_FORCES: u32 = 128u;
const FLAG_COLLIDE: u32 = 256u;

const PARTICLE_STUCK: u32 = 1u;

const COLLIDE_DEPTH: u32 = 1u;
const COLLIDE_FIELD: u32 = 2u;
const COLLIDE_WATER: u32 = 4u;
const RESPONSE_BOUNCE: u32 = 0u;
const RESPONSE_STICK: u32 = 1u;
const RESPONSE_KILL: u32 = 2u;
const NO_TARGET: u32 = 0xffffffffu;
// Events each emitter can take from one frame's hits.
const EVENT_CAPACITY: u32 = 256u;

const LONGEST_SUBSTEP: f32 = 1.0 / 30.0;
const WIND_ROUGHNESS: f32 = 0.03;
const GUSTINESS: f32 = 0.3;
const AIR_TEMPERATURE: f32 = 300.0;

const SHAPE_SPHERE: u32 = 1u;
const SHAPE_BOX: u32 = 2u;

const MAX_EMITTERS: u32 = 64u;
const GROUP_SIZE: u32 = 64u;
const SERIES_BELOW: f32 = 1.0e-2;

// One emitter this frame, its values worked out from its effect's parameters.
struct Emitter {
    transform: mat4x4f,
    shape_size: vec4f,
    // xyz where the shape is centred; w the cone's half-angle in radians.
    offset_cone: vec4f,
    // xyz initial velocity in the effect's space; w the spread of speed.
    velocity_spread: vec4f,
    // xyz acceleration in the world; w linear drag per second.
    acceleration_drag: vec4f,
    // Lifetime's range, then size's.
    lifetime_size: vec4f,
    color: vec4f,
    base: u32,
    budget: u32,
    spawn_request: u32,
    seed: u32,
    shape_kind: u32,
    flags: u32,
    soft: f32,
    scale: f32,
    // Temperature's range at birth in kelvin, then cooling per second and emissivity.
    temperature_glow: vec4f,
    // Henyey-Greenstein asymmetry in x.
    phase: vec4f,
    // The weather's wind's share; curl noise's speed, eddy size and rate of change.
    wind_curl: vec4f,
    buoyancy_attract_vortex: vec4f,
    restitution_friction: vec4f,
    collide_against: u32,
    response: u32,
    // The emitter slot each hit sends an event to, and particles spawned per event heard.
    event_target: u32,
    event_spawn: u32,
}

struct Particle {
    position: vec3f,
    age: f32,
    velocity: vec3f,
    lifetime: f32,
    previous: vec3f,
    size: f32,
    index: u32,
    rotation: f32,
    // Temperature at birth, in kelvin; 0 for particles that do not glow.
    temperature: f32,
    // PARTICLE_ bits.
    flags: u32,
}

// The GPU's running state for one emitter.
struct EmitterState {
    free_count: atomic<u32>,
    alive: array<atomic<u32>, 2>,
    spawned: u32,
    dropped: u32,
    spawn_count: u32,
    spawn_first: u32,
    spawn_index: u32,
    update_count: u32,
    spawn_start: u32,
    update_start: u32,
    light_start: u32,
    // Events sent to this emitter this frame, which it spawns from next frame, and how many it
    // spawns from this frame.
    event_count: atomic<u32>,
    events_in: u32,
    _pad: array<u32, 2>,
}

// Where a hit happened and which way the surface faced, for emitters that spawn on it.
struct Event {
    position: vec3f,
    _pad0: f32,
    normal: vec3f,
    _pad1: f32,
}

struct SimParams {
    // Seconds since the last simulated frame.
    seconds: f32,
    // The alive lists read this frame; the other is written.
    parity: u32,
    // Slots in the pool, and so in each alive list.
    capacity: u32,
    // Each particle's light is worked out afresh once in this many frames, staggered.
    light_interval: u32,
    // Steps each ambient ray takes through the GI volume.
    ray_steps: u32,
    // Seconds since the renderer started, at the end of this step.
    time: f32,
    // The weather's wind 10 m up, as (east, south).
    wind: vec2f,
    // Where the camera is and which way it looks, for sorting by distance along the view; in
    // the position's w, the height the wind's profile starts from.
    camera_position: vec4f,
    camera_forward: vec4f,
}

fn particle_random(seed: u32, index: u32, stream: u32) -> f32 {
    return hash_to_unit(seed ^ pcg(index * 16u + stream));
}

fn spawn_elapsed(i: u32, n: u32, seconds: f32) -> f32 {
    return seconds * (1.0 - (f32(i) + 0.5) / f32(max(n, 1u)));
}

// A new particle's velocity in its effect's space: its emitter's, spread over the cone and in
// speed.
fn local_velocity(e: Emitter, index: u32) -> vec3f {
    let base = e.velocity_spread.xyz;
    let speed = length(base) * (1.0 + e.velocity_spread.w * (2.0 * particle_random(e.seed, index, 5u) - 1.0));
    var axis = vec3f(0.0, 1.0, 0.0);
    if speed > 0.0 {
        axis = normalize(base);
    }
    let cos_theta = 1.0 - particle_random(e.seed, index, 3u) * (1.0 - cos(e.offset_cone.w));
    let sin_theta = sqrt(max(1.0 - cos_theta * cos_theta, 0.0));
    let phi = TAU * particle_random(e.seed, index, 4u);
    let direction = orthonormal_basis(axis) * vec3f(cos(phi) * sin_theta, sin(phi) * sin_theta, cos_theta);
    return direction * speed;
}

// A particle in touch with a surface remembers so in its flags, with that surface's normal in the
// top 24 bits: the distance field holds no sign, and under the ridge of a coarser level its
// gradient points down, the way a particle leaves.
const PARTICLE_IN_CONTACT: u32 = 2u;
const NORMAL_STEPS: f32 = 4095.0;

fn pack_contact(n: vec3f) -> u32 {
    let e = vec2u(round((oct_encode(n) * 0.5 + 0.5) * NORMAL_STEPS));
    return PARTICLE_IN_CONTACT | (e.x << 8u) | (e.y << 20u);
}

fn unpack_contact(flags: u32) -> vec3f {
    let e = vec2f(f32((flags >> 8u) & 0xfffu), f32(flags >> 20u)) / NORMAL_STEPS;
    return oct_decode(e * 2.0 - 1.0);
}

// A particle born from an event: at the hit, leaving the surface, with the emitter's velocity
// turned so that its up is the surface's normal.
fn event_particle(e: Emitter, index: u32, event: Event) -> Particle {
    var p = spawn_particle(e, index);
    let up = normalize(event.normal);
    let v = local_velocity(e, index);
    p.position = event.position + up * 0.01;
    p.previous = p.position;
    p.velocity = orthonormal_basis(up) * vec3f(v.x, v.z, v.y);
    p.flags = pack_contact(up);
    return p;
}

fn spawn_particle(e: Emitter, index: u32) -> Particle {
    var local = vec3f(0.0);
    if e.shape_kind == SHAPE_SPHERE {
        let z = 2.0 * particle_random(e.seed, index, 0u) - 1.0;
        let phi = TAU * particle_random(e.seed, index, 1u);
        let ring = sqrt(max(1.0 - z * z, 0.0));
        var radius = e.shape_size.x;
        if (e.flags & FLAG_SURFACE) == 0u {
            radius *= pow(particle_random(e.seed, index, 2u), 1.0 / 3.0);
        }
        local = vec3f(ring * cos(phi), ring * sin(phi), z) * radius;
    } else if e.shape_kind == SHAPE_BOX {
        let u = vec3f(
            particle_random(e.seed, index, 0u),
            particle_random(e.seed, index, 1u),
            particle_random(e.seed, index, 2u),
        );
        local = (u - vec3f(0.5)) * e.shape_size.xyz;
    }
    local += e.offset_cone.xyz;
    let position = (e.transform * vec4f(local, 1.0)).xyz;

    let velocity = (e.transform * vec4f(local_velocity(e, index), 0.0)).xyz;

    let ls = e.lifetime_size;
    var p: Particle;
    p.position = position;
    p.age = 0.0;
    p.velocity = velocity;
    p.lifetime = ls.x + (ls.y - ls.x) * particle_random(e.seed, index, 7u);
    p.previous = position;
    p.size = (ls.z + (ls.w - ls.z) * particle_random(e.seed, index, 6u)) * e.scale;
    p.index = index;
    p.rotation = TAU * particle_random(e.seed, index, 8u);
    let hot = e.temperature_glow;
    p.temperature = hot.x + (hot.y - hot.x) * particle_random(e.seed, index, 9u);
    return p;
}

fn particle_temperature(p: Particle, e: Emitter) -> f32 {
    if p.temperature <= 0.0 {
        return 0.0;
    }
    return AIR_TEMPERATURE + (p.temperature - AIR_TEMPERATURE) * exp(-e.temperature_glow.z * p.age);
}

// The curl of a field of three noises, which has no divergence: air that swirls without
// gathering or spreading.
fn curl_noise(p: vec3f) -> vec3f {
    let a = gradient_noise(p, 0x68bc21ebu).yzw;
    let b = gradient_noise(p, 0x02e5be93u).yzw;
    let c = gradient_noise(p, 0x967a889bu).yzw;
    return vec3f(c.y - b.z, a.z - c.x, b.x - a.y);
}

// The air a particle moves through this step: when it ends, the weather's wind 10 m up, and the
// height the wind's profile starts from.
struct Air {
    time: f32,
    wind: vec2f,
    floor: f32,
}

// The velocity of the air at the particle at `time`, and any other acceleration on it, from its
// emitter's forces: [air velocity, acceleration].
fn particle_forces(p: Particle, e: Emitter, air: Air, time: f32) -> array<vec3f, 2> {
    let x = p.position;
    let origin = e.transform[3].xyz;
    var moving = vec3f(0.0);
    var accel = vec3f(0.0);
    let wind = e.wind_curl.x;
    if wind != 0.0 {
        // The wind 10 m up, brought to the particle's height by the log profile, gusting.
        let height = max(x.y - air.floor, 0.5);
        let profile = log(height / WIND_ROUGHNESS) / log(10.0 / WIND_ROUGHNESS);
        let gust = gradient_noise(x * 0.02 + vec3f(time * 0.3, 0.0, 0.0), 0x1b873593u).x;
        let speed = profile * (1.0 + GUSTINESS * gust) * wind;
        moving += vec3f(air.wind.x, 0.0, air.wind.y) * speed;
    }
    let curl_speed = e.wind_curl.y;
    if curl_speed != 0.0 {
        let q = x / max(e.wind_curl.z, 1.0e-3) + vec3f(time * e.wind_curl.w);
        moving += curl_noise(q) * curl_speed;
    }
    let vortex = e.buoyancy_attract_vortex.z;
    if vortex != 0.0 {
        var axis = vec3f(0.0, 1.0, 0.0);
        if length(e.transform[1].xyz) > 0.0 {
            axis = normalize(e.transform[1].xyz);
        }
        moving += cross(axis, x - origin) * vortex;
    }
    let attract = e.buoyancy_attract_vortex.y;
    if attract != 0.0 {
        let to_origin = origin - x;
        if length(to_origin) > 0.0 {
            accel += normalize(to_origin) * attract;
        }
    }
    let buoyancy = e.buoyancy_attract_vortex.x;
    if buoyancy != 0.0 {
        let kelvin = particle_temperature(p, e);
        if kelvin > 0.0 {
            accel.y += 9.81 * buoyancy * (kelvin - AIR_TEMPERATURE) / kelvin;
        }
    }
    return array<vec3f, 2>(moving, accel);
}

// Gravity and linear drag over `h` seconds, towards terminal velocity `a / k`, integrated
// exactly: with d = exp(-k h), velocity becomes v d + a F and position moves by v F + a G, where
// F = (1 - d) / k and G = (h - F) / k.
fn exact_step(p: ptr<function, Particle>, a: vec3f, k: f32, h: f32) {
    let kh = k * h;
    var decay: f32;
    var f: f32;
    var g: f32;
    if kh < SERIES_BELOW {
        decay = 1.0 - kh + 0.5 * kh * kh;
        f = h * (1.0 - 0.5 * kh + kh * kh / 6.0);
        g = h * h * (0.5 - kh / 6.0 + kh * kh / 24.0);
    } else {
        decay = exp(-kh);
        f = (1.0 - decay) / k;
        g = (h - f) / k;
    }
    let v = (*p).velocity;
    let x = (*p).position;
    (*p).position = x + v * f + a * g;
    (*p).velocity = v * decay + a * f;
}

// Moves a particle on by `seconds`, ending at `air.time`. Gravity and drag alone are exact;
// moving air pulls a particle through its drag, and forces that vary take substeps of at most
// LONGEST_SUBSTEP. Stuck particles only age.
fn step_particle(p: ptr<function, Particle>, e: Emitter, seconds: f32, air: Air) {
    let start = (*p).position;
    let g = e.acceleration_drag.xyz;
    let k = e.acceleration_drag.w;
    if ((*p).flags & PARTICLE_STUCK) != 0u {
        (*p).velocity = vec3f(0.0);
    } else if (e.flags & FLAG_FORCES) == 0u {
        exact_step(p, g, k, seconds);
    } else {
        let steps = max(ceil(seconds / LONGEST_SUBSTEP), 1.0);
        let h = seconds / steps;
        var time = air.time - seconds;
        for (var s = 0u; s < u32(steps); s++) {
            let forces = particle_forces(*p, e, air, time);
            exact_step(p, g + forces[1] + forces[0] * k, k, h);
            time += h;
        }
    }
    (*p).previous = start;
    (*p).age += seconds;
}

fn is_dead(p: Particle) -> bool {
    return p.age >= p.lifetime;
}
