// Collisions, after particles have moved this frame: each live particle of an emitter that
// collides is tested against what it asked for, and a hit bounces it, sticks it, or kills it,
// and may send an event for another emitter to spawn from next frame (docs/particles.md, step 4).
//
// - Depth: this frame's opaque depth and normals. Exact where it is drawn, blind off screen and
//   behind what is drawn.
// - Field: the GI volume's distance field, everywhere within the volume, on screen or not, to
//   within a voxel (0.2 m near the camera).
// - Water: the water's height now, from the water-line map near the camera and the calm level
//   beyond it.
//
// A particle counts as touching once its centre is within its radius of the surface, so it rests
// on a surface rather than sinking into it. It must have begun the frame on the surface's open
// side, so that one passing behind a thin thing drawn in front, or moving under the water, is
// left alone.
//
// A bouncing particle moving into the surface slower than two frames' worth of what pushes it
// there is resting: it is held at the surface, sends no event, and slides on it, losing speed to
// friction as Coulomb's law has it, the friction times the speed the surface took from it.
// Sticking and dying particles do so at any speed into the surface.

#include "common/environment.wgsl"
#include "common/wetness.wgsl"
#include "particles/common.wgsl"

@group(0) @binding(0) var<uniform> sim: SimParams;
@group(0) @binding(1) var<storage, read> emitters: array<Emitter, MAX_EMITTERS>;
@group(0) @binding(2) var<storage, read_write> state: array<EmitterState, MAX_EMITTERS>;
@group(0) @binding(3) var<storage, read_write> particles: array<Particle>;
@group(0) @binding(4) var<storage, read> alive: array<u32>;
@group(0) @binding(5) var<storage, read_write> events: array<Event>;
// The opaque scene's depth (reversed Z, zero for sky) and normals, octahedral in RG.
@group(2) @binding(0) var scene_depth: texture_2d<f32>;
@group(2) @binding(1) var scene_normal: texture_2d<f32>;
@group(3) @binding(0) var<uniform> waterline: WaterlineParams;
@group(3) @binding(1) var waterline_map: texture_2d<f32>;

// How far under the water's surface a particle may have begun the frame and still land on it.
const WATER_DEPTH_TOLERANCE: f32 = 0.25;

// Normals that agree this well (about 26 degrees, either sign, as the gradient flips under a
// level's ridge) are one surface curving. Past that it is another, or a corner's blur.
const SAME_SURFACE: f32 = 0.9;
// A gradient this far up is off a floor, on the open side of it whatever the field's ridge does.
const OFF_A_FLOOR: f32 = 0.5;
// The gradient agreeing less than this with the surface known, what is in reach is beside it.
const BESIDE: f32 = 0.3;
// Levels above this have voxels too large to search three of them off a surface.
const FINEST_LEVEL_SEARCHED: u32 = 1u;
// The shallowest view of a surface allowed for in how far along the view a particle may be from it.
const GRAZING: f32 = 0.05;

var<workgroup> group_emitter: u32;

// A surface the particle touches: where, which way it faces, and whether it was found.
struct Contact {
    hit: bool,
    point: vec3f,
    normal: vec3f,
    // No hit, but near enough to a surface to keep remembering it.
    within_reach: bool,
}

fn no_contact() -> Contact {
    return Contact(false, vec3f(0.0), vec3f(0.0, 1.0, 0.0), false);
}

fn against_water(start: vec3f, x: vec3f, radius: f32) -> Contact {
    // With no sea, its level lies far below everything.
    if frame.sea_level < -1.0e20 {
        return no_contact();
    }
    let height = water_heights(x.xz).y;
    // Under the water when the frame began, by more than the water rises in a frame, it stays
    // under: a bubble, or a basin below the sea's level.
    if x.y - radius >= height || start.y < height - radius - WATER_DEPTH_TOLERANCE {
        return no_contact();
    }
    return Contact(true, vec3f(x.x, height, x.z), vec3f(0.0, 1.0, 0.0), false);
}

// The surface the field puts `distance` from `x`. Where the field has no direction, as exactly on
// a ridge, it is the side remembered, or nothing.
fn field_contact(x: vec3f, level: u32, distance: f32, remembered: vec3f) -> Contact {
    // The field's gradient points away from the nearest surface.
    let d = volume_params.levels[level].w;
    let gradient = vec3f(
        sample_volume(x + vec3f(d, 0.0, 0.0), level).a - sample_volume(x - vec3f(d, 0.0, 0.0), level).a,
        sample_volume(x + vec3f(0.0, d, 0.0), level).a - sample_volume(x - vec3f(0.0, d, 0.0), level).a,
        sample_volume(x + vec3f(0.0, 0.0, d), level).a - sample_volume(x - vec3f(0.0, 0.0, d), level).a,
    );
    if length(gradient) < 1.0e-6 {
        if dot(remembered, remembered) == 0.0 {
            return no_contact();
        }
        return Contact(true, x - remembered * distance, remembered, false);
    }
    let normal = normalize(gradient);
    return Contact(true, x - normal * distance, normal, false);
}

// The field holds the distance to the nearest surface, which is no help in telling which side
// of a thin surface a point is on. So the particle's path this frame is marched from where it
// started, in steps no longer than that distance, which cannot pass through a surface, and it
// touches where the distance first falls within its reach.
//
// One that began the frame within reach, and knows which side it is on, is resting or sliding on
// that surface. It keeps its movement along the surface and loses any into it, by the normal on
// the side it knows. It isn't moved nearer: to a voxel's resolution, the field's distance may put
// the surface below the true one, and under that the gradient would send it further down. If
// what is within reach is something beside it instead, such as a wall by a ledge it has slid off,
// nothing holds it up.
//
// Its side is the surface it remembers touching. One that remembers nothing was born within
// reach, and can only be trusted when the gradient points up, off a floor: anywhere else it may
// be under the field's ridge, where the gradient points the wrong way. Then it is left alone
// until it has come out of reach, and what it meets after that counts. (So a particle born under
// the ridge of a floor and moving down passes through it, and one born between a ceiling and
// its ridge is taken to be on a floor there. Particles spawned by an event remember the surface
// they were born on.)
//
// On the two finest levels the sliding particle's path is still marched for any other surface
// ahead of it, offset three voxels from the one it slides on, so that what is found is
// something else, with a clean normal: nearer the surface the field is too blurred in a
// corner to tell the two apart. What stands lower than that offset, or sticks out from a wall
// less than it, isn't seen.
fn against_field(start: vec3f, end: vec3f, radius: f32, remembered: vec3f) -> Contact {
    let path = end - start;
    let length_of_path = length(path);
    let direction = select(vec3f(0.0, -1.0, 0.0), path / length_of_path, length_of_path > 1.0e-6);
    // The normal of the surface the particle began the frame on, if it knows it.
    var resting_on = vec3f(0.0);
    // Whether that surface is what is within reach, so that it can hold the particle.
    var holding = false;
    // Where the rest of the path is marched from, relative to the particle's own.
    var offset = vec3f(0.0);
    // Born within reach, not knowing its side, and not yet out of reach.
    var unsure = false;
    var t = 0.0;
    for (var step = 0u; step < 12u; step++) {
        let x = start + offset + direction * t;
        let level = volume_level_at(x, 0u);
        if level >= VOLUME_LEVELS {
            break;
        }
        let voxel = volume_params.levels[level].w;
        let distance = sample_volume(x, level).a;
        let reach = max(radius, voxel * 0.5);
        if distance < reach {
            let contact = field_contact(x, level, distance, remembered);
            if step == 0u {
                var side = remembered;
                if contact.hit && dot(side, side) == 0.0 && contact.normal.y > OFF_A_FLOOR {
                    side = contact.normal;
                }
                if dot(side, side) == 0.0 {
                    unsure = true;
                } else {
                    // The gradient follows the surface as it curves, on the side known. A sudden
                    // change is the blur of a corner, not this surface.
                    let facing = contact.normal * select(1.0, -1.0, dot(contact.normal, side) < 0.0);
                    resting_on = select(side, facing, dot(facing, side) > SAME_SURFACE);
                    holding = abs(dot(contact.normal, resting_on)) > BESIDE;
                    if level > FINEST_LEVEL_SEARCHED {
                        break;
                    }
                    // Along the side known, not the gradient, which a corner tilts.
                    offset = side * voxel * 3.0;
                }
            } else if !unsure && contact.hit && abs(dot(contact.normal, resting_on)) < SAME_SURFACE {
                // On the surface's own plane, at the particle's path rather than the offset one.
                let point = contact.point - offset + contact.normal * dot(offset, contact.normal);
                return Contact(true, point, contact.normal, false);
            }
        } else {
            unsure = false;
        }
        if t >= length_of_path {
            break;
        }
        t = min(t + max(distance - reach * 0.5, voxel * 0.25), length_of_path);
    }
    let n = resting_on;
    if dot(n, n) == 0.0 {
        return no_contact();
    }
    let away = dot(path, n);
    if away >= 0.0 || !holding {
        // Not held, but still within reach: it may come back, and will need to know its side.
        var leaving = no_contact();
        leaving.within_reach = true;
        return leaving;
    }
    return Contact(true, end - n * away - n * radius, n, false);
}

fn against_depth(start: vec3f, x: vec3f, radius: f32) -> Contact {
    let clip = frame.unjittered_view_proj * vec4f(x, 1.0);
    if clip.w <= frame.near {
        return no_contact();
    }
    let ndc = clip.xy / clip.w;
    let uv = vec2f(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
    if any(uv < vec2f(0.0)) || any(uv >= vec2f(1.0)) {
        return no_contact();
    }
    let pixel = vec2i(uv * frame.viewport);
    let depth = textureLoad(scene_depth, pixel, 0).r;
    if depth <= 0.0 {
        return no_contact();
    }
    // Near what is drawn there, along the view? Only roughly: the depth is the pixel's centre's,
    // which on a slanted surface is some way from the particle's own ray.
    let surface = frame.near / depth;
    let thickness = max(0.3, radius * 4.0);
    if abs(clip.w - surface) > thickness / GRAZING {
        return no_contact();
    }
    // A height above a surface seen at a shallow angle is much further than that along the view.
    let normal = oct_decode(textureLoad(scene_normal, pixel, 0).xy);
    let view = normalize(x - frame.camera_position);
    if abs(clip.w - surface) > thickness / max(abs(dot(normal, view)), GRAZING) {
        return no_contact();
    }
    // Exactly, then, against the plane of the surface at the pixel's centre, where its depth is
    // exact (it was drawn with the jitter).
    let drawn = reconstruct_world(frame, vec2f(pixel) + 0.5, depth);
    let above = dot(x - drawn, normal);
    if above >= radius || above < -thickness {
        return no_contact();
    }
    // A particle that began the frame behind that plane passed behind what is drawn, such as a
    // post or a branch, rather than into it. (With a little slack, for surfaces that aren't flat
    // from pixel to pixel.)
    if dot(start - drawn, normal) < -radius - 0.05 {
        return no_contact();
    }
    // The particle's foot on the plane, so that one resting there is held where it is.
    let point = x - normal * above;
    return Contact(true, point, normal, false);
}

@compute @workgroup_size(64)
fn main(@builtin(workgroup_id) group: vec3u, @builtin(local_invocation_index) lane: u32) {
    let written = 1u - sim.parity;
    // The same workgroups as the light pass: the last emitter whose start this group reached.
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
    if (emitter.flags & FLAG_LIVE) == 0u || (emitter.flags & FLAG_COLLIDE) == 0u {
        return;
    }
    let i = (group.x - state[e].light_start) * GROUP_SIZE + lane;
    if i >= atomicLoad(&state[e].alive[written]) {
        return;
    }
    let slot = alive[written * sim.capacity + emitter.base + i];
    var p = particles[slot];
    if is_dead(p) || (p.flags & PARTICLE_STUCK) != 0u {
        return;
    }
    let touching = (p.flags & PARTICLE_IN_CONTACT) != 0u;
    let radius = p.size * 0.5;
    let remembered = select(vec3f(0.0), unpack_contact(p.flags), touching);
    var contact = no_contact();
    if (emitter.collide_against & COLLIDE_WATER) != 0u {
        contact = against_water(p.previous, p.position, radius);
    }
    if !contact.hit && (emitter.collide_against & COLLIDE_DEPTH) != 0u {
        contact = against_depth(p.previous, p.position, radius);
    }
    if !contact.hit && (emitter.collide_against & COLLIDE_FIELD) != 0u {
        contact = against_field(p.previous, p.position, radius, remembered);
    }
    if !contact.hit {
        if touching && !contact.within_reach {
            p.flags &= PARTICLE_STUCK;
            particles[slot] = p;
        }
        return;
    }
    let n = contact.normal;
    p.flags = (p.flags & PARTICLE_STUCK) | pack_contact(n);
    let into = dot(p.velocity, n);
    let resting = contact.point + n * radius;
    let rf = emitter.restitution_friction;
    // Slower into the surface than two frames of what pushes it there: resting, not landing.
    var push = dot(emitter.acceleration_drag.xyz, n);
    if (emitter.flags & FLAG_FORCES) != 0u && into < 0.0 {
        let air = Air(sim.time, sim.wind, sim.camera_position.w);
        let forces = particle_forces(p, emitter, air, sim.time);
        push += dot(forces[1] + forces[0] * emitter.acceleration_drag.w, n);
    }
    let rest_speed = 2.0 * abs(push) * sim.seconds + 0.05;
    if into >= 0.0 || (emitter.response == RESPONSE_BOUNCE && into > -rest_speed) {
        // Kept out of the surface, and sliding on it with friction.
        if dot(p.position - contact.point, n) < radius {
            p.position = resting;
        }
        if into < 0.0 {
            let along = p.velocity - into * n;
            let speed = length(along);
            let slowed = max(speed + rf.y * into, 0.0);
            p.velocity = along * (slowed / max(speed, 1.0e-6));
        }
        particles[slot] = p;
        return;
    }
    if emitter.response == RESPONSE_KILL {
        p.age = p.lifetime;
    } else if emitter.response == RESPONSE_STICK {
        p.position = resting;
        p.velocity = vec3f(0.0);
        p.flags |= PARTICLE_STUCK;
    } else {
        let normal_part = into * n;
        let along = p.velocity - normal_part;
        p.velocity = along * (1.0 - rf.y) - normal_part * rf.x;
        p.position = resting;
    }
    particles[slot] = p;
    let to = emitter.event_target;
    if to != NO_TARGET && to < MAX_EMITTERS {
        let k = atomicAdd(&state[to].event_count, 1u);
        if k < EVENT_CAPACITY {
            events[to * EVENT_CAPACITY + k] = Event(contact.point, 0.0, n, 0.0);
        }
    }
}
