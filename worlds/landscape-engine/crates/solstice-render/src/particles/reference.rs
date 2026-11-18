//! The records the particle shaders read and write, and a CPU reference for what they do.
//!
//! The reference runs the same spawn and step as `particles/common.wgsl`, for one particle at a
//! time, from the same emitter record. Tests compare the GPU's particles against it, as the
//! waves' tests compare against `spectrum.rs`.

use bytemuck::{Pod, Zeroable};
use glam::{Mat3, Mat4, Vec3};
use std::f32::consts::TAU;

/// Bits of [`EmitterRecord::flags`]. Twins of the `FLAG_` constants in `particles/common.wgsl`.
pub const FLAG_LIVE: u32 = 1;
pub const FLAG_ADD: u32 = 2;
pub const FLAG_SURFACE: u32 = 4;
pub const FLAG_LIT: u32 = 8;
pub const FLAG_GLOW: u32 = 16;
pub const FLAG_SOLID: u32 = 32;
pub const FLAG_SORT: u32 = 64;
/// The emitter has forces that vary through space or time, which take substeps.
pub const FLAG_FORCES: u32 = 128;
/// Its particles collide with something.
pub const FLAG_COLLIDE: u32 = 256;

/// Bits of [`Particle::flags`]: stuck where it hit, for the rest of its life.
pub const PARTICLE_STUCK: u32 = 1;

/// Values of [`EmitterRecord::response`].
pub const RESPONSE_BOUNCE: u32 = 0;
pub const RESPONSE_STICK: u32 = 1;
pub const RESPONSE_KILL: u32 = 2;
/// An [`EmitterRecord::event_target`] that sends nothing.
pub const NO_TARGET: u32 = u32::MAX;

/// The longest step forces that vary are integrated over, in seconds.
pub const LONGEST_SUBSTEP: f32 = 1.0 / 30.0;
/// Roughness length of the ground the wind blows over, in metres: open, grassy land.
pub const WIND_ROUGHNESS: f32 = 0.03;
/// How much the wind gusts above and below its mean.
pub const GUSTINESS: f32 = 0.3;

/// The air's temperature, which glowing particles cool towards, in kelvin.
pub const AIR_TEMPERATURE: f32 = 300.0;

/// Values of [`EmitterRecord::shape_kind`].
pub const SHAPE_POINT: u32 = 0;
pub const SHAPE_SPHERE: u32 = 1;
pub const SHAPE_BOX: u32 = 2;

/// Below this drag times time step, the step uses series for its exponentials, which are
/// otherwise lost to rounding.
const SERIES_BELOW: f32 = 1.0e-2;

/// One emitter, as the GPU sees it this frame, with every value already worked out from its
/// effect's parameters. Twin of `Emitter` in `particles/common.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct EmitterRecord {
    /// The effect's space to the world.
    pub transform: [[f32; 4]; 4],
    /// The shape's size: a box's full sizes, or a sphere's radius in x.
    pub shape_size: [f32; 4],
    /// Where the shape is centred in the effect's space, and in w the cone's half-angle in
    /// radians.
    pub offset_cone: [f32; 4],
    /// Initial velocity in the effect's space, and in w the spread of speed.
    pub velocity_spread: [f32; 4],
    /// Acceleration in the world, and in w the linear drag per second.
    pub acceleration_drag: [f32; 4],
    /// Lifetime's range, then size's.
    pub lifetime_size: [f32; 4],
    /// Linear colour times brightness, and opacity.
    pub color: [f32; 4],
    /// The emitter's slice of the pool.
    pub base: u32,
    pub budget: u32,
    /// Particles to spawn this frame, before the budget limits them.
    pub spawn_request: u32,
    pub seed: u32,
    pub shape_kind: u32,
    pub flags: u32,
    /// Distance over which particles fade into the scene behind them.
    pub soft: f32,
    /// The effect's scale, which particle sizes are multiplied by.
    pub scale: f32,
    /// Temperature's range at birth in kelvin, then cooling per second and emissivity.
    pub temperature_glow: [f32; 4],
    /// Henyey-Greenstein asymmetry in x; the rest is spare.
    pub phase: [f32; 4],
    /// The weather's wind's share; curl noise's speed, eddy size and rate of change.
    pub wind_curl: [f32; 4],
    /// Buoyancy, attraction and vortex; then spare.
    pub buoyancy_attract_vortex: [f32; 4],
    /// Restitution and friction of bounces; then spare.
    pub restitution_friction: [f32; 4],
    /// What its particles collide with, as `COLLIDE_` bits, and what they do then.
    pub collide_against: u32,
    pub response: u32,
    /// The emitter slot each hit sends an event to, or [`NO_TARGET`].
    pub event_target: u32,
    /// Particles this emitter spawns for each event it hears.
    pub event_spawn: u32,
}

/// One particle. Twin of `Particle` in `particles/common.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct Particle {
    pub position: [f32; 3],
    pub age: f32,
    pub velocity: [f32; 3],
    pub lifetime: f32,
    /// Where it was at the start of its last step, for motion vectors.
    pub previous: [f32; 3],
    /// Radius at birth, in metres.
    pub size: f32,
    /// Which of its emitter's particles it is, counting every one spawned: its random values
    /// come from this, not from the slot it happens to occupy.
    pub index: u32,
    pub rotation: f32,
    /// Temperature at birth, in kelvin; 0 for particles that do not glow.
    pub temperature: f32,
    /// `PARTICLE_` bits.
    pub flags: u32,
}

/// The GPU's running state for one emitter. Twin of `EmitterState` in `particles/common.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Pod, Zeroable)]
pub struct EmitterState {
    /// How many slots the free list holds.
    pub free_count: u32,
    /// How many particles each of the two alive lists holds.
    pub alive: [u32; 2],
    /// Every particle ever spawned, and every one that could not be for lack of budget.
    pub spawned: u32,
    pub dropped: u32,
    /// This frame: how many are spawned, where they come from on the free list, and the index
    /// of the first.
    pub spawn_count: u32,
    pub spawn_first: u32,
    pub spawn_index: u32,
    /// This frame: how many particles the update ages.
    pub update_count: u32,
    /// This frame: the first workgroup of this emitter in the spawn, update and light
    /// dispatches.
    pub spawn_start: u32,
    pub update_start: u32,
    pub light_start: u32,
    /// Events sent to this emitter this frame, which it spawns from next frame.
    pub event_count: u32,
    /// Events it spawns from this frame.
    pub events_in: u32,
    pub _pad: [u32; 2],
}

/// PCG hash: twin of `pcg` in `common/math.wgsl`.
pub fn pcg(v: u32) -> u32 {
    let state = v.wrapping_mul(747_796_405).wrapping_add(2_891_336_453);
    let word = ((state >> ((state >> 28) + 4)) ^ state).wrapping_mul(277_803_737);
    (word >> 22) ^ word
}

/// Twin of `hash_to_unit` in `common/math.wgsl`.
pub fn hash_to_unit(v: u32) -> f32 {
    (pcg(v) >> 8) as f32 * (1.0 / 16_777_216.0)
}

/// A particle's random number for `stream`, in `0..1`. Twin of `particle_random`.
pub fn random(seed: u32, index: u32, stream: u32) -> f32 {
    hash_to_unit(seed ^ pcg(index.wrapping_mul(16).wrapping_add(stream)))
}

/// How long the `i`th of `n` particles spawned over a step of `seconds` has lived by its end:
/// they are born evenly across it. Twin of `spawn_elapsed`.
pub fn spawn_elapsed(i: u32, n: u32, seconds: f32) -> f32 {
    seconds * (1.0 - (i as f32 + 0.5) / n.max(1) as f32)
}

/// A basis around `n`, as tangent, bitangent and `n`: twin of `orthonormal_basis`.
fn orthonormal_basis(n: Vec3) -> Mat3 {
    let s = if n.z >= 0.0 { 1.0 } else { -1.0 };
    let a = -1.0 / (s + n.z);
    let b = n.x * n.y * a;
    Mat3::from_cols(
        Vec3::new(1.0 + s * n.x * n.x * a, s * b, -s * n.x),
        Vec3::new(b, s + n.y * n.y * a, -n.y),
        n,
    )
}

/// The particle numbered `index` of an emitter, as it is born. Twin of `spawn_particle`.
pub fn spawn(e: &EmitterRecord, index: u32) -> Particle {
    let r = |stream| random(e.seed, index, stream);
    let local = match e.shape_kind {
        SHAPE_SPHERE => {
            let z = 2.0 * r(0) - 1.0;
            let phi = TAU * r(1);
            let ring = (1.0 - z * z).max(0.0).sqrt();
            let radius = if e.flags & FLAG_SURFACE != 0 {
                e.shape_size[0]
            } else {
                e.shape_size[0] * r(2).powf(1.0 / 3.0)
            };
            Vec3::new(ring * phi.cos(), ring * phi.sin(), z) * radius
        }
        SHAPE_BOX => {
            Vec3::new(r(0) - 0.5, r(1) - 0.5, r(2) - 0.5) * Vec3::from_slice(&e.shape_size[..3])
        }
        _ => Vec3::ZERO,
    } + Vec3::from_slice(&e.offset_cone[..3]);
    let transform = Mat4::from_cols_array_2d(&e.transform);
    let position = transform.transform_point3(local);

    let base = Vec3::from_slice(&e.velocity_spread[..3]);
    let speed = base.length() * (1.0 + e.velocity_spread[3] * (2.0 * r(5) - 1.0));
    let axis = if speed > 0.0 {
        base.normalize()
    } else {
        Vec3::Y
    };
    let cos_theta = 1.0 - r(3) * (1.0 - e.offset_cone[3].cos());
    let sin_theta = (1.0 - cos_theta * cos_theta).max(0.0).sqrt();
    let phi = TAU * r(4);
    let direction = orthonormal_basis(axis)
        * Vec3::new(phi.cos() * sin_theta, phi.sin() * sin_theta, cos_theta);
    let velocity = transform.transform_vector3(direction * speed);

    let [life_low, life_high, size_low, size_high] = e.lifetime_size;
    let [hot_low, hot_high, _, _] = e.temperature_glow;
    Particle {
        position: position.into(),
        age: 0.0,
        velocity: velocity.into(),
        lifetime: life_low + (life_high - life_low) * r(7),
        previous: position.into(),
        size: (size_low + (size_high - size_low) * r(6)) * e.scale,
        index,
        rotation: TAU * r(8),
        temperature: hot_low + (hot_high - hot_low) * r(9),
        flags: 0,
    }
}

/// A particle's temperature now: from its temperature at birth, cooling exponentially towards
/// the air's. Twin of `particle_temperature`.
pub fn temperature(p: &Particle, e: &EmitterRecord) -> f32 {
    if p.temperature <= 0.0 {
        return 0.0;
    }
    AIR_TEMPERATURE + (p.temperature - AIR_TEMPERATURE) * (-e.temperature_glow[2] * p.age).exp()
}

/// The air particles move through this step: the time at its end, the weather's wind 10 m up,
/// as (east, south), and the height the wind's profile starts from. Twin of the matching
/// fields of `SimParams`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Air {
    pub time: f32,
    pub wind: [f32; 2],
    pub floor: f32,
}

/// One of the hashed gradients of the noise lattice. Twin of `lattice_gradient`.
fn lattice_gradient(cell: [i32; 3], seed: u32) -> Vec3 {
    let h = pcg((cell[0] as u32).wrapping_add(pcg(
        (cell[1] as u32).wrapping_add(pcg((cell[2] as u32).wrapping_add(seed)))
    )));
    Vec3::new(
        hash_to_unit(h),
        hash_to_unit(pcg(h)),
        hash_to_unit(pcg(pcg(h))),
    ) * 2.0
        - Vec3::ONE
}

/// Gradient noise at `p`, and its gradient, from lattice `seed`, with quintic smoothing so the
/// gradient is continuous. Twin of `gradient_noise`.
pub fn gradient_noise(p: Vec3, seed: u32) -> (f32, Vec3) {
    let i = p.floor();
    let f = p - i;
    let u = f * f * f * (f * (f * 6.0 - Vec3::splat(15.0)) + Vec3::splat(10.0));
    let du = f * f * 30.0 * (f * (f - Vec3::splat(2.0)) + Vec3::ONE);
    let c = [i.x as i32, i.y as i32, i.z as i32];
    let g = |dx: i32, dy: i32, dz: i32| lattice_gradient([c[0] + dx, c[1] + dy, c[2] + dz], seed);
    let (ga, gb, gc, gd) = (g(0, 0, 0), g(1, 0, 0), g(0, 1, 0), g(1, 1, 0));
    let (ge, gf, gg, gh) = (g(0, 0, 1), g(1, 0, 1), g(0, 1, 1), g(1, 1, 1));
    let va = ga.dot(f);
    let vb = gb.dot(f - Vec3::new(1.0, 0.0, 0.0));
    let vc = gc.dot(f - Vec3::new(0.0, 1.0, 0.0));
    let vd = gd.dot(f - Vec3::new(1.0, 1.0, 0.0));
    let ve = ge.dot(f - Vec3::new(0.0, 0.0, 1.0));
    let vf = gf.dot(f - Vec3::new(1.0, 0.0, 1.0));
    let vg = gg.dot(f - Vec3::new(0.0, 1.0, 1.0));
    let vh = gh.dot(f - Vec3::new(1.0, 1.0, 1.0));
    let k1 = vb - va;
    let k2 = vc - va;
    let k3 = ve - va;
    let k4 = va - vb - vc + vd;
    let k5 = va - vc - ve + vg;
    let k6 = va - vb - ve + vf;
    let k7 = -va + vb + vc - vd + ve - vf - vg + vh;
    let value = va
        + u.x * k1
        + u.y * k2
        + u.z * k3
        + u.x * u.y * k4
        + u.y * u.z * k5
        + u.z * u.x * k6
        + u.x * u.y * u.z * k7;
    let gradient = ga
        + u.x * (gb - ga)
        + u.y * (gc - ga)
        + u.z * (ge - ga)
        + u.x * u.y * (ga - gb - gc + gd)
        + u.y * u.z * (ga - gc - ge + gg)
        + u.z * u.x * (ga - gb - ge + gf)
        + u.x * u.y * u.z * (-ga + gb + gc - gd + ge - gf - gg + gh)
        + du * (Vec3::new(k1, k2, k3)
            + Vec3::new(u.y, u.z, u.x) * Vec3::new(k4, k5, k6)
            + Vec3::new(u.z, u.x, u.y) * Vec3::new(k6, k4, k5)
            + Vec3::new(u.y * u.z, u.z * u.x, u.x * u.y) * k7);
    (value, gradient)
}

/// The curl of a field of three noises, which has no divergence: air that swirls without
/// gathering or spreading. Twin of `curl_noise`.
pub fn curl_noise(p: Vec3) -> Vec3 {
    let (_, a) = gradient_noise(p, 0x68bc_21eb);
    let (_, b) = gradient_noise(p, 0x02e5_be93);
    let (_, c) = gradient_noise(p, 0x967a_889b);
    Vec3::new(c.y - b.z, a.z - c.x, b.x - a.y)
}

/// The velocity of the air at `x` at `time`, and any other acceleration on a particle there,
/// from its emitter's forces. Twin of `particle_forces`.
fn forces(p: &Particle, e: &EmitterRecord, air: &Air, time: f32) -> (Vec3, Vec3) {
    let x = Vec3::from(p.position);
    let transform = Mat4::from_cols_array_2d(&e.transform);
    let origin = transform.w_axis.truncate();
    let mut moving = Vec3::ZERO;
    let mut accel = Vec3::ZERO;
    let [wind, curl_speed, curl_scale, curl_change] = e.wind_curl;
    if wind != 0.0 {
        // The wind 10 m up, brought to the particle's height by the log profile, gusting.
        let height = (x.y - air.floor).max(0.5);
        let profile = (height / WIND_ROUGHNESS).ln() / (10.0 / WIND_ROUGHNESS).ln();
        let (gust, _) = gradient_noise(x * 0.02 + Vec3::new(time * 0.3, 0.0, 0.0), 0x1b87_3593);
        let speed = profile * (1.0 + GUSTINESS * gust) * wind;
        moving += Vec3::new(air.wind[0], 0.0, air.wind[1]) * speed;
    }
    if curl_speed != 0.0 {
        let q = x / curl_scale.max(1.0e-3) + Vec3::splat(time * curl_change);
        moving += curl_noise(q) * curl_speed;
    }
    let [buoyancy, attract, vortex, _] = e.buoyancy_attract_vortex;
    if vortex != 0.0 {
        let axis = transform.y_axis.truncate().normalize_or(Vec3::Y);
        moving += axis.cross(x - origin) * vortex;
    }
    if attract != 0.0 {
        accel += (origin - x).normalize_or_zero() * attract;
    }
    if buoyancy != 0.0 {
        let kelvin = temperature(p, e);
        if kelvin > 0.0 {
            accel.y += 9.81 * buoyancy * (kelvin - AIR_TEMPERATURE) / kelvin;
        }
    }
    (moving, accel)
}

/// Gravity and linear drag over `h` seconds towards terminal velocity `a / k`, exactly.
fn exact_step(p: &mut Particle, a: Vec3, k: f32, h: f32) {
    let kh = k * h;
    // With e = exp(-k h): velocity becomes v e + a F and position moves by v F + a G, where
    // F = (1 - e) / k and G = (h - F) / k, the integrals of e over the step.
    let (decay, f, g) = if kh < SERIES_BELOW {
        (
            1.0 - kh + 0.5 * kh * kh,
            h * (1.0 - 0.5 * kh + kh * kh / 6.0),
            h * h * (0.5 - kh / 6.0 + kh * kh / 24.0),
        )
    } else {
        let decay = (-kh).exp();
        let f = (1.0 - decay) / k;
        (decay, f, (h - f) / k)
    };
    let v = Vec3::from(p.velocity);
    let x = Vec3::from(p.position);
    p.position = (x + v * f + a * g).into();
    p.velocity = (v * decay + a * f).into();
}

/// Moves a particle on by `seconds` in still air. See [`step_in`].
pub fn step(p: &mut Particle, e: &EmitterRecord, seconds: f32) {
    step_in(p, e, seconds, &Air::default());
}

/// Moves a particle on by `seconds`, ending at `air.time`, under its emitter's forces. Twin of
/// `step_particle`.
///
/// Gravity and linear drag are integrated exactly, so with them alone the result does not
/// depend on how time is split into steps: a slow frame lands a particle where two fast ones
/// would have. Moving air pulls a particle through its drag, as an extra acceleration of drag
/// times the air's velocity, which keeps the step exact for air that is steady; forces that vary
/// through space or time take substeps of at most [`LONGEST_SUBSTEP`].
pub fn step_in(p: &mut Particle, e: &EmitterRecord, seconds: f32, air: &Air) {
    let start = p.position;
    let g = Vec3::from_slice(&e.acceleration_drag[..3]);
    let k = e.acceleration_drag[3];
    if p.flags & PARTICLE_STUCK != 0 {
        p.velocity = [0.0; 3];
    } else if e.flags & FLAG_FORCES == 0 {
        exact_step(p, g, k, seconds);
    } else {
        let steps = (seconds / LONGEST_SUBSTEP).ceil().max(1.0);
        let h = seconds / steps;
        let mut time = air.time - seconds;
        for _ in 0..steps as u32 {
            let (moving, accel) = forces(p, e, air, time);
            exact_step(p, g + accel + moving * k, k, h);
            time += h;
        }
    }
    p.previous = start;
    p.age += seconds;
}

/// Whether a particle has lived its life. Twin of `is_dead`.
pub fn is_dead(p: &Particle) -> bool {
    p.age >= p.lifetime
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record() -> EmitterRecord {
        EmitterRecord {
            transform: Mat4::IDENTITY.to_cols_array_2d(),
            velocity_spread: [1.0, 4.0, 0.0, 0.1],
            acceleration_drag: [0.0, -9.81, 0.0, 0.4],
            lifetime_size: [5.0, 5.0, 0.1, 0.1],
            offset_cone: [0.0, 0.0, 0.0, 0.3],
            scale: 1.0,
            seed: 7,
            flags: FLAG_LIVE,
            ..Default::default()
        }
    }

    /// The step matches the closed form for gravity and linear drag, and does not depend on how
    /// time is split into steps.
    #[test]
    fn steps_follow_the_exact_motion_however_time_is_split() {
        let e = record();
        let born = spawn(&e, 3);
        let mut once = born;
        step(&mut once, &e, 1.2);
        let mut many = born;
        for h in [0.1, 0.05, 0.3, 0.25, 0.2, 0.3] {
            step(&mut many, &e, h);
        }
        let k = e.acceleration_drag[3];
        let a = Vec3::new(0.0, -9.81, 0.0);
        let v0 = Vec3::from(born.velocity);
        let t: f32 = 1.2;
        let terminal = a / k;
        let exact =
            Vec3::from(born.position) + terminal * t + (v0 - terminal) * (1.0 - (-k * t).exp()) / k;
        for p in [once, many] {
            assert!((Vec3::from(p.position) - exact).length() < 1.0e-4, "{p:?}");
            assert!((p.age - t).abs() < 1.0e-6);
        }
    }

    /// Without drag the series branch is plain projectile motion.
    #[test]
    fn without_drag_a_particle_flies_as_a_projectile() {
        let mut e = record();
        e.acceleration_drag[3] = 0.0;
        let mut p = spawn(&e, 0);
        let v0 = Vec3::from(p.velocity);
        step(&mut p, &e, 0.5);
        let expected = v0 * 0.5 + Vec3::new(0.0, -9.81, 0.0) * 0.125;
        assert!((Vec3::from(p.position) - expected).length() < 1.0e-5);
    }

    /// Initial velocities stay inside the cone and speeds inside their spread.
    #[test]
    fn spawns_stay_inside_their_cone_and_spread() {
        let e = record();
        let axis = Vec3::new(1.0, 4.0, 0.0).normalize();
        let speed = Vec3::new(1.0, 4.0, 0.0).length();
        let mut widest: f32 = 0.0;
        for index in 0..2000 {
            let p = spawn(&e, index);
            let v = Vec3::from(p.velocity);
            let angle = v.normalize().dot(axis).clamp(-1.0, 1.0).acos();
            widest = widest.max(angle);
            assert!(angle <= 0.3 + 1.0e-4, "{angle}");
            assert!((v.length() / speed - 1.0).abs() <= 0.1 + 1.0e-5);
        }
        // And they fill it, rather than hugging the axis.
        assert!(widest > 0.28, "{widest}");
    }

    #[test]
    fn shapes_hold_their_particles() {
        let mut e = record();
        e.shape_kind = SHAPE_SPHERE;
        e.shape_size = [2.0, 0.0, 0.0, 0.0];
        e.offset_cone = [0.0, 10.0, 0.0, 0.0];
        for index in 0..500 {
            let p = Vec3::from(spawn(&e, index).position);
            assert!((p - Vec3::new(0.0, 10.0, 0.0)).length() <= 2.0 + 1.0e-4);
        }
        e.flags |= FLAG_SURFACE;
        for index in 0..500 {
            let p = Vec3::from(spawn(&e, index).position);
            assert!(((p - Vec3::new(0.0, 10.0, 0.0)).length() - 2.0).abs() < 1.0e-4);
        }
        e.shape_kind = SHAPE_BOX;
        e.shape_size = [1.0, 2.0, 4.0, 0.0];
        e.offset_cone = [0.0; 4];
        e.transform = Mat4::from_translation(Vec3::new(5.0, 0.0, 0.0)).to_cols_array_2d();
        for index in 0..500 {
            let p = Vec3::from(spawn(&e, index).position) - Vec3::new(5.0, 0.0, 0.0);
            assert!(p.abs().cmple(Vec3::new(0.5, 1.0, 2.0)).all(), "{p}");
        }
    }

    #[test]
    fn record_layouts_match_the_shaders() {
        // Sizes and alignment the WGSL structs have.
        assert_eq!(std::mem::size_of::<EmitterRecord>(), 288);
        assert_eq!(std::mem::size_of::<Particle>(), 64);
        assert_eq!(std::mem::size_of::<EmitterState>(), 64);
    }

    /// A glowing particle cools exponentially towards the air, and one with no temperature
    /// stays at none.
    #[test]
    fn glowing_particles_cool_towards_the_air() {
        let mut e = record();
        e.temperature_glow = [1300.0, 1300.0, (2.0f32).ln(), 1.0];
        let mut p = spawn(&e, 0);
        assert_eq!(temperature(&p, &e), 1300.0);
        p.age = 1.0;
        assert!((temperature(&p, &e) - 800.0).abs() < 1.0e-3);
        p.temperature = 0.0;
        assert_eq!(temperature(&p, &e), 0.0);
    }

    /// The noise's gradient is its true derivative, and the curl of three noises has no
    /// divergence: air that swirls without gathering or spreading.
    #[test]
    fn curl_noise_neither_gathers_nor_spreads() {
        let d = 1.0e-3;
        let mut largest: f32 = 0.0;
        for i in 0..200u32 {
            let p = Vec3::new(
                hash_to_unit(i * 3) * 20.0 - 10.0,
                hash_to_unit(i * 3 + 1) * 20.0 - 10.0,
                hash_to_unit(i * 3 + 2) * 20.0 - 10.0,
            );
            // The analytic gradient against central differences.
            let (_, gradient) = gradient_noise(p, 5);
            let numeric = Vec3::new(
                gradient_noise(p + Vec3::X * d, 5).0 - gradient_noise(p - Vec3::X * d, 5).0,
                gradient_noise(p + Vec3::Y * d, 5).0 - gradient_noise(p - Vec3::Y * d, 5).0,
                gradient_noise(p + Vec3::Z * d, 5).0 - gradient_noise(p - Vec3::Z * d, 5).0,
            ) / (2.0 * d);
            assert!(
                (gradient - numeric).length() < 0.02,
                "{gradient} against {numeric}"
            );
            let c = curl_noise(p);
            largest = largest.max(c.length());
            let divergence = (curl_noise(p + Vec3::X * d).x - curl_noise(p - Vec3::X * d).x
                + curl_noise(p + Vec3::Y * d).y
                - curl_noise(p - Vec3::Y * d).y
                + curl_noise(p + Vec3::Z * d).z
                - curl_noise(p - Vec3::Z * d).z)
                / (2.0 * d);
            assert!(divergence.abs() < 0.05, "divergence {divergence} at {p}");
        }
        // The field is not trivially zero.
        assert!(largest > 0.5, "{largest}");
    }

    /// Steady wind pulls a particle with drag up to the wind's speed at its height.
    #[test]
    fn wind_carries_particles_up_to_its_speed() {
        let mut e = record();
        e.acceleration_drag = [0.0, 0.0, 0.0, 2.0];
        e.velocity_spread = [0.0; 4];
        e.offset_cone = [0.0, 10.0, 0.0, 0.0];
        e.wind_curl = [1.0, 0.0, 1.0, 0.0];
        e.flags |= FLAG_FORCES;
        let mut p = spawn(&e, 0);
        let mut air = Air {
            time: 0.0,
            wind: [6.0, 0.0],
            floor: 0.0,
        };
        for _ in 0..300 {
            air.time += 1.0 / 60.0;
            step_in(&mut p, &e, 1.0 / 60.0, &air);
        }
        // 10 m up, the profile is 1; gusts vary it by up to 30%.
        let speed = p.velocity[0];
        assert!((speed / 6.0 - 1.0).abs() < GUSTINESS + 0.02, "{speed}");
        assert!(p.velocity[2].abs() < 1.0e-4);
        // A stuck particle does not move.
        let mut stuck = p;
        stuck.flags = PARTICLE_STUCK;
        let at = stuck.position;
        step_in(&mut stuck, &e, 0.1, &air);
        assert_eq!(stuck.position, at);
    }

    #[test]
    fn spawns_are_spread_evenly_over_the_step() {
        let ages: Vec<f32> = (0..4).map(|i| spawn_elapsed(i, 4, 1.0)).collect();
        assert_eq!(ages, vec![0.875, 0.625, 0.375, 0.125]);
    }
}
