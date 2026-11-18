//! A car, stepped: what the ground, the air and its own engine do to it.
//!
//! Each fixed step is cut into [`SUBSTEPS`] shorter ones, as a tyre and a damper answer in
//! milliseconds. Each of those:
//!
//! 1. **Finds the ground** under each wheel with one ray down the suspension's axis, and from
//!    how far the wheel has risen, the spring, damper, anti-roll bar and bump stop give the load
//!    on its tyre.
//! 2. **Solves the drivetrain**: the engine, clutch, gearbox, differential, brakes and the two
//!    driven wheels, with the tyres' grip on the road, all at once (see [`drivetrain`]). A
//!    four-wheel drive solves all four wheels, with its transfer case and both differentials.
//!
//! Off the road the ground is soft. A tyre sinks into it by what the surface says
//! ([`crate::surface::SurfaceGrip`]), further the harder it is pressed and the more it spins,
//! and the rut it ploughs holds the car back by how deep it is.
//! 3. **Sums the forces** on the body: each tyre's, gravity, drag and downforce.
//! 4. **Moves the body**: its velocity and spin, then its place and turn.
//!
//! The body is a rigid one with its own mass and inertia. The game's physics engine moves the
//! same body and deals with what it hits: it is handed where this step ended (see
//! [`Body`]), and adds what the collision did.

use crate::drivetrain::{self, Coupling, Driveline, DrivenAxle};
use crate::rut::Press;
use crate::spec::{VehicleSpec, AIR_DENSITY, RPM_TO_RADIANS};
use crate::surface::{bump, Surface};
use crate::tyre::TyreCurves;
use glam::{Quat, Vec3};

/// How many shorter steps one fixed step is cut into.
pub const SUBSTEPS: u32 = 10;
/// Below this speed, in metres a second, slip is measured against this speed instead of the
/// wheel's own: slip is a ratio of speeds, and has no meaning at a standstill.
const LOW_SPEED: f32 = 2.0;
/// How far above its highest point a wheel's ray starts, so that a wheel pushed past its bump
/// stop still finds the ground.
const RAY_MARGIN: f32 = 0.15;
/// The bump stop's rate, in newtons a metre: what the wheel meets at the top of its travel.
const BUMP_STOP: f32 = 600_000.0;
/// The damper's knee: above this speed, in metres a second, it stiffens only this much as fast.
const DAMPER_KNEE: f32 = 0.13;
const DAMPER_FAST: f32 = 0.45;
/// The most a tyre can be loaded, as a multiple of its load at rest: a landing, not a number.
const MAX_LOAD: f32 = 9.0;
/// The slip ratio at which the anti-lock brakes let go, and at which traction control cuts in:
/// just past where the tyre's grip peaks.
const ABS_SLIP: f32 = 0.13;
const TRACTION_SLIP: f32 = 0.11;
/// How much more slip than that takes the throttle away altogether.
const TRACTION_BAND: f32 = 0.12;
/// Stability control: how far the rear tyres' slip angle may pass the angle they grip best at
/// before it steps in, as a multiple of that angle, and at how much it is doing all it can.
const STABILITY_FROM: f32 = 0.95;
const STABILITY_FULL: f32 = 1.9;
/// How far ahead it looks, in seconds, at the rate the tail is stepping out: a tail that is
/// swinging fast is caught before it has gone far.
const STABILITY_AHEAD: f32 = 0.16;
/// The most of the front brake it puts on the outside front wheel to pull the nose back.
const STABILITY_BRAKE: f32 = 0.6;
/// Under braking, the rear brakes begin to be let go at this multiple of that angle: a tail
/// that the brakes are holding at the edge of its grip has none left to hold the car straight.
const STABILITY_RELEASE: f32 = 0.55;
/// Below this speed, in metres a second, there is nothing for it to do.
const STABILITY_SPEED: f32 = 7.0;
/// How long, in seconds, a tyre takes to sink into soft ground, and to climb back out of it.
const SINK_SECONDS: f32 = 0.22;
const RISE_SECONDS: f32 = 0.5;
/// The drag of ploughing a rut, as a share of the tyre's load, for a rut as deep as the tyre's
/// radius (Bekker's compaction resistance, near enough a straight line over the depths here).
const RUT_DRAG: f32 = 0.35;
/// A wheel that spins goes on digging where it stands: how far below the floor of its own
/// rut it reaches for, as a multiple of what the ground says a spinning wheel digs.
const DIG_ON: f32 = 0.25;
/// The deepest a wheel can dig itself into ground, as multiples of how far it sinks there
/// and of how far spinning digs it, and as a share of its own radius: by then it is on its
/// axle.
const DIG_SOFT: f32 = 1.5;
const DIG_SPUN: f32 = 2.5;
const DIG_RADIUS: f32 = 0.7;
/// How many lugs a tyre's tread has round it: what it prints in soft ground.
pub const TREAD_LUGS: f32 = 32.0;
/// A tyre's width, as a multiple of its radius.
const TYRE_WIDTH: f32 = 0.7;
/// How far a tyre rolls, in metres, before the water it came out of a creek with is mostly
/// off it, and the mud it came out of a bog with; and how far through water before that
/// mud is washed off.
const CARRIES_WATER: f32 = 18.0;
const CARRIES_MUD: f32 = 30.0;
const WASHES_MUD: f32 = 4.0;
/// The most of a wheel's brake a four-wheel drive's traction control puts on to stop it
/// spinning: enough to send a good share of the drive across an open differential, and less
/// than locking it does.
const BRAKE_TRACTION: f32 = 0.3;
/// Hill descent control: the speed it holds a car to, in metres a second, and the most of the
/// brakes it uses to do it.
const DESCENT_SPEED: f32 = 1.7;
const DESCENT_BRAKE: f32 = 0.75;
/// Tyres let down for soft ground, to about half their road pressure: the tread lies longer
/// and wider on the ground. On soil it grips this much better and sinks this much less; on a
/// hard road it squirms, gripping a little worse and rolling this much harder.
const AIRED_GRIP: f32 = 1.17;
const AIRED_SINK: f32 = 0.62;
const AIRED_ROAD_GRIP: f32 = 0.93;
const AIRED_ROLLING: f32 = 1.6;
/// How far above idle, in rpm, the clutch starts to bite when pulling away flat out.
const LAUNCH_REVS: f32 = 2600.0;
/// Within this many rpm of the gearbox's speed, the engine counts as turning with it.
const CLUTCH_MATCHED: f32 = 150.0;

/// What the ground is under a wheel.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Contact {
    /// How far along the ray it was met, in metres.
    pub distance: f32,
    /// Which way the ground faces there: a unit vector.
    pub normal: Vec3,
    pub surface: Surface,
}

/// The world, as a wheel finds it.
pub trait Ground {
    /// What a ray from `from` along the unit vector `direction` meets first within `reach`.
    fn cast(&self, from: Vec3, direction: Vec3, reach: f32) -> Option<Contact>;

    /// How far the ground at a place has been pressed down since it was made, in metres:
    /// the depth of the rut there, below zero on the berm beside one ([`crate::rut`]).
    /// Ground that keeps no ruts has none.
    fn rut(&self, _at: Vec3) -> f32 {
        0.0
    }

    /// Takes a tyre's mark: where its contact has been over a step, and how deep. Ground
    /// that keeps no ruts does nothing with it.
    fn press(&self, _press: &Press) {}
}

/// Flat ground at a height, of one surface: for tests, and for tuning a car.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Flat {
    pub height: f32,
    pub surface: Surface,
}

impl Ground for Flat {
    fn cast(&self, from: Vec3, direction: Vec3, reach: f32) -> Option<Contact> {
        if direction.y >= -1.0e-6 {
            return None;
        }
        let distance = (self.height - from.y) / direction.y;
        (0.0..=reach).contains(&distance).then_some(Contact {
            distance,
            normal: Vec3::Y,
            surface: self.surface,
        })
    }
}

/// Where the car's body is and how it is moving.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Body {
    /// The car's origin: on the ground under its middle, when it stands level.
    pub position: Vec3,
    pub rotation: Quat,
    /// Of its centre of mass, in metres a second.
    pub velocity: Vec3,
    /// How it is turning, in radians a second about each of the world's axes.
    pub spin: Vec3,
}

impl Body {
    /// Standing still at a place, turned by `yaw` radians about +Y. Zero looks along -Z.
    pub fn at_rest(position: Vec3, yaw: f32) -> Self {
        Self {
            position,
            rotation: Quat::from_rotation_y(yaw),
            velocity: Vec3::ZERO,
            spin: Vec3::ZERO,
        }
    }

    /// The way the car points.
    pub fn forward(&self) -> Vec3 {
        self.rotation * Vec3::NEG_Z
    }

    pub fn right(&self) -> Vec3 {
        self.rotation * Vec3::X
    }

    pub fn up(&self) -> Vec3 {
        self.rotation * Vec3::Y
    }
}

/// What a four-wheel drive's transfer case is set to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DriveMode {
    /// Two-wheel drive, high range: the rear axle alone, for the road.
    #[default]
    TwoHigh,
    /// Four-wheel drive, high range: both axles, locked together.
    FourHigh,
    /// Four-wheel drive, low range: the same, geared down for crawling.
    FourLow,
}

impl DriveMode {
    /// The setting after this one, as its lever goes: 2H, 4H, 4L, and round again.
    pub fn next(self) -> Self {
        match self {
            Self::TwoHigh => Self::FourHigh,
            Self::FourHigh => Self::FourLow,
            Self::FourLow => Self::TwoHigh,
        }
    }

    /// As it is written on the lever.
    pub fn name(self) -> &'static str {
        match self {
            Self::TwoHigh => "2H",
            Self::FourHigh => "4H",
            Self::FourLow => "4L",
        }
    }

    /// Whether both axles are driven.
    pub fn four(self) -> bool {
        self != Self::TwoHigh
    }
}

/// What the driver is doing with the car, after any help (see [`crate::driver`]).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Controls {
    /// From 0 to 1.
    pub throttle: f32,
    pub brake: f32,
    /// From -1, full left, to 1, full right.
    pub steer: f32,
    /// From 0 to 1: locks the rear wheels.
    pub handbrake: f32,
    /// Asks for the next gear up or down, once: set for one step.
    pub shift_up: bool,
    pub shift_down: bool,
}

/// One wheel, as it is now.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Wheel {
    /// How fast it turns, in radians a second: above zero rolling forward.
    pub speed: f32,
    /// How far it has turned, in radians, kept within one turn.
    pub angle: f32,
    /// How far it has risen from where it rests, in metres: below zero, hanging.
    pub travel: f32,
    /// How far its tyre has sunk into soft ground, in metres.
    pub sink: f32,
    /// How far it is steered, in radians: above zero to the right.
    pub steer: f32,
    /// How hard its tyre is pressed to the ground, in newtons.
    pub load: f32,
    /// Its slip ratio, and the tangent of its slip angle, as the tyre feels them.
    pub slip_ratio: f32,
    pub slide: f32,
    /// The force the ground puts on it: along the way it points, and to its right.
    pub force: (f32, f32),
    pub on_ground: bool,
    pub surface: Surface,
    /// The share of the brake's pressure the anti-lock system is letting through.
    pub brake_share: f32,
}

/// What the instruments show, and what a game wants to know of how a car is doing.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Telemetry {
    /// Along the car, in metres a second: below zero going backwards.
    pub speed: f32,
    pub rpm: f32,
    /// -1 is reverse, 0 neutral.
    pub gear: i32,
    /// What the engine is being asked for, from 0 to 1, after the limiter and traction control.
    pub throttle: f32,
    /// The body's acceleration over the last step, in g: forward, to the right, and up.
    pub g: Vec3,
    /// Whether the anti-lock brakes, traction control or stability control are at work.
    pub abs_active: bool,
    pub traction_active: bool,
    pub stability_active: bool,
    /// Whether the engine is at its limiter.
    pub limiter: bool,
    /// What a four-wheel drive's transfer case is set to, and whether the front and the rear
    /// differentials are locked.
    pub drive: DriveMode,
    pub front_locked: bool,
    pub rear_locked: bool,
    /// Whether its tyres are let down for soft ground, and whether hill descent control is
    /// holding it back.
    pub aired_down: bool,
    pub descent_active: bool,
}

/// A gear change under way.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Shift {
    seconds_left: f32,
    /// Whether it is to a lower gear, where the engine is blipped up to the wheels' speed.
    down: bool,
}

/// A car: its numbers, and the state of everything that turns.
#[derive(Clone, Debug)]
pub struct Vehicle {
    spec: VehicleSpec,
    /// What it weighs with nothing in it, in kilograms.
    unladen: f32,
    curves: TyreCurves,
    gravity: f32,
    /// The load on a front wheel and on a rear one, standing still.
    static_loads: (f32, f32),
    pub wheels: [Wheel; 4],
    /// The engine's speed, in radians a second.
    engine_speed: f32,
    gear: i32,
    shift: Option<Shift>,
    /// The torque through the clutch in the last short step, which sets how hard the
    /// differential locks.
    clutch_torque: f32,
    /// The share of the throttle traction control is letting through.
    traction_share: f32,
    /// How far the tail was stepping out a short step ago: the rear axle's slip angle, in
    /// radians, above zero when it slides to the right.
    rear_slide: f32,
    limiting: bool,
    /// A four-wheel drive's transfer case, and whether each differential is locked: front,
    /// then rear.
    drive: DriveMode,
    lockers: (bool, bool),
    /// How wet the ground is, from 0, dry, to 1, soaked: rain makes clay greasy and mud deep.
    wetness: f32,
    /// Whether its tyres are let down for soft ground.
    aired_down: bool,
    /// Where each tyre met the ground in the last short step, if it did.
    touching: [Option<Vec3>; 4],
    /// Where each tyre's last mark on the ground ended, and where in its pattern each
    /// tyre's tread is, in radians.
    trail: [Option<glam::Vec2>; 4],
    tread: [f32; 4],
    /// What each tyre carries from where it has been, from 0 to 1: water, and mud.
    carried: [(f32, f32); 4],
    telemetry: Telemetry,
}

impl Vehicle {
    /// A car of these numbers, standing still with its engine idling, in neutral.
    ///
    /// # Panics
    /// If the numbers fail [`VehicleSpec::check`]: check what comes from a file first.
    pub fn new(spec: VehicleSpec, gravity: f32) -> Self {
        if let Err(message) = spec.check() {
            panic!("an unsound vehicle: {message}");
        }
        let wheel = Wheel {
            brake_share: 1.0,
            ..Wheel::default()
        };
        Self {
            unladen: spec.mass,
            curves: spec.tyre.curves(),
            static_loads: spec.static_loads(gravity),
            engine_speed: spec.engine.idle * RPM_TO_RADIANS,
            gravity,
            wheels: [wheel; 4],
            gear: 0,
            shift: None,
            clutch_torque: 0.0,
            traction_share: 1.0,
            rear_slide: 0.0,
            limiting: false,
            // A four-wheel drive starts with both axles driven: it is what it is for.
            drive: if spec.transfer.is_some() {
                DriveMode::FourHigh
            } else {
                DriveMode::TwoHigh
            },
            lockers: (false, false),
            wetness: 0.0,
            aired_down: false,
            touching: [None; 4],
            trail: [None; 4],
            tread: [0.0; 4],
            carried: [(0.0, 0.0); 4],
            spec,
            telemetry: Telemetry::default(),
        }
    }

    /// What the transfer case is set to. A car without one is always in two-wheel drive.
    pub fn drive_mode(&self) -> DriveMode {
        self.drive
    }

    /// Sets the transfer case, on a car that has one.
    pub fn set_drive_mode(&mut self, mode: DriveMode) {
        if self.spec.transfer.is_some() {
            self.drive = mode;
        }
    }

    /// Whether the front and the rear differentials are locked.
    pub fn lockers(&self) -> (bool, bool) {
        self.lockers
    }

    /// Locks or frees each differential, on a car with a transfer case. The front one only
    /// locks in four-wheel drive, as its axle is only driven then.
    pub fn set_lockers(&mut self, front: bool, rear: bool) {
        if self.spec.transfer.is_some() {
            self.lockers = (front, rear);
        }
    }

    /// Says how wet the ground is, from 0, dry, to 1, soaked.
    pub fn set_wetness(&mut self, wetness: f32) {
        self.wetness = if wetness.is_finite() {
            wetness.clamp(0.0, 1.0)
        } else {
            0.0
        };
    }

    pub fn wetness(&self) -> f32 {
        self.wetness
    }

    /// Lets the tyres down for soft ground, or pumps them back up for the road.
    pub fn set_aired_down(&mut self, aired_down: bool) {
        self.aired_down = aired_down;
    }

    pub fn aired_down(&self) -> bool {
        self.aired_down
    }

    pub fn spec(&self) -> &VehicleSpec {
        &self.spec
    }

    /// Loads it: what is carried in it, in kilograms, is that much more for its engine to
    /// move, its brakes to stop and its springs to hold up.
    pub fn set_payload(&mut self, kilograms: f32) {
        self.spec.mass = self.unladen + kilograms.max(0.0);
    }

    /// What it is carrying, in kilograms.
    pub fn payload(&self) -> f32 {
        self.spec.mass - self.unladen
    }

    pub fn telemetry(&self) -> &Telemetry {
        &self.telemetry
    }

    pub fn gear(&self) -> i32 {
        self.gear
    }

    pub fn rpm(&self) -> f32 {
        self.engine_speed / RPM_TO_RADIANS
    }

    /// Whether a gear change is under way.
    pub fn shifting(&self) -> bool {
        self.shift.is_some()
    }

    /// Puts the car in a gear at once: -1 for reverse, 0 for neutral. For starting a race, and
    /// for tests.
    pub fn set_gear(&mut self, gear: i32) {
        self.gear = gear.clamp(-1, self.spec.gearbox.gears.len() as i32);
        self.shift = None;
    }

    /// Stops every wheel and sets the engine idling: for putting a car back on the track.
    pub fn settle(&mut self) {
        for wheel in &mut self.wheels {
            *wheel = Wheel {
                angle: wheel.angle,
                brake_share: 1.0,
                ..Wheel::default()
            };
        }
        self.engine_speed = self.spec.engine.idle * RPM_TO_RADIANS;
        self.gear = 0;
        self.shift = None;
        self.clutch_torque = 0.0;
        self.traction_share = 1.0;
        self.rear_slide = 0.0;
        self.touching = [None; 4];
        self.trail = [None; 4];
    }

    /// What each tyre carries from where it has been, from 0 to 1: water, then mud. A
    /// tyre out of a creek leaves wet tracks; one out of a bog, muddy ones.
    pub fn carried(&self) -> [(f32, f32); 4] {
        self.carried
    }

    /// The deepest a wheel of this radius can dig itself into a surface, in metres: as far
    /// as it sinks there and a spinning wheel digs, and no further than its axle.
    fn deepest(&self, surface: Surface, radius: f32) -> f32 {
        let grip = surface.grip();
        let soft = grip.sink_at(self.wetness) * if self.aired_down { AIRED_SINK } else { 1.0 };
        (soft * DIG_SOFT + grip.dig * DIG_SPUN).min(DIG_RADIUS * radius)
    }

    /// Leaves each tyre's mark on the ground it rolled over in the step just taken: a rut
    /// as deep as the tyre has sunk, printed with its tread, and as wet and as churned as
    /// the ground there is and as the tyre came to it.
    fn mark(&mut self, body: &Body, ground: &dyn Ground, seconds: f32) {
        use std::f32::consts::TAU;
        for index in 0..4 {
            let wheel = self.wheels[index];
            let radius = self.spec.axle(index).radius;
            let turned = wheel.speed * seconds * TREAD_LUGS;
            let phase = self.tread[index];
            self.tread[index] = (phase + turned).rem_euclid(TAU);
            let rolled = (wheel.speed * radius * seconds).abs();
            let (mut water, mut mud) = self.carried[index];
            water *= (-rolled / CARRIES_WATER).exp();
            mud *= (-rolled / CARRIES_MUD).exp();
            let Some(at) = self.touching[index].filter(|_| wheel.load > 0.0) else {
                self.carried[index] = (water, mud);
                self.trail[index] = None;
                continue;
            };
            let soil = wheel.surface.soil(self.wetness);
            // No deeper than this ground can be pressed: a tyre still climbing out of a
            // bog as it comes onto rock leaves the rock as it was.
            let depth = wheel.sink.clamp(0.0, self.deepest(wheel.surface, radius));
            // What the tyre picks up: water from a creek, which washes the mud off it, and
            // mud from ground it has sunk into.
            let sunk = (depth / 0.03).min(1.0);
            if wheel.surface == Surface::Ford {
                water = 1.0;
                mud *= (-rolled / WASHES_MUD).exp();
            } else {
                water = water.max(soil.wet * sunk);
                mud = mud.max(soil.churn * soil.wet * sunk);
            }
            self.carried[index] = (water, mud);
            let to = glam::Vec2::new(at.x, at.z);
            // A car put somewhere else between two steps leaves no strip from where it was.
            let from = self.trail[index]
                .filter(|from| from.distance(to) < 1.5)
                .unwrap_or(to);
            self.trail[index] = Some(to);
            let heading = body.rotation * (Quat::from_rotation_y(-wheel.steer) * Vec3::NEG_Z);
            let along = glam::Vec2::new(heading.x, heading.z).normalize_or_zero();
            if along == glam::Vec2::ZERO {
                continue;
            }
            // A tyre rolling prints its tread; one spinning or locked smears it.
            let print = soil.print.max(0.5 * mud) * (1.0 - (wheel.slip_ratio.abs() - 0.06) / 0.3);
            ground.press(&Press {
                from,
                to,
                along,
                half_width: 0.5 * TYRE_WIDTH * radius,
                // The chord of the tyre below the ground's surface, and its own flat.
                half_length: (2.0 * radius * (depth + 0.02)).sqrt().min(0.75 * radius),
                depth,
                berm: soil.berm,
                phase: (phase, phase + turned),
                print: print.clamp(0.0, 1.0),
                wet: soil.wet.max(0.85 * water),
                stays: soil.stays,
                churn: soil.churn.max(0.7 * mud),
            });
        }
    }

    /// Sets every wheel rolling at the speed the car is going: for starting a car that is
    /// already moving, with the engine turning to match in the gear it is in.
    pub fn roll_with(&mut self, body: &Body) {
        let speed = body.velocity.dot(body.forward());
        for (index, wheel) in self.wheels.iter_mut().enumerate() {
            wheel.speed = speed / self.spec.axle(index).radius;
        }
        if let Some(ratio) = self.ratio() {
            let wheels = self.shaft_speed();
            self.engine_speed = (wheels * ratio).max(self.spec.engine.idle * RPM_TO_RADIANS);
        }
    }

    /// The gear's ratio with the final drive, from the engine to the wheels: below zero in
    /// reverse, and none in neutral.
    pub fn ratio(&self) -> Option<f32> {
        let gearbox = &self.spec.gearbox;
        let range = self.range();
        match self.gear {
            0 => None,
            gear if gear < 0 => Some(-gearbox.reverse * gearbox.final_drive * range),
            gear => Some(gearbox.gears[gear as usize - 1] * gearbox.final_drive * range),
        }
    }

    /// The transfer case's reduction: 1 in high range.
    pub fn range(&self) -> f32 {
        match (&self.spec.transfer, self.drive) {
            (Some(transfer), DriveMode::FourLow) => transfer.low,
            _ => 1.0,
        }
    }

    /// How much of the drive each wheel is sent, which is also how much of the propshaft's
    /// speed each is: a half each on the one driven axle, or shared between both.
    fn shares(&self) -> [f32; 4] {
        match &self.spec.transfer {
            Some(transfer) if self.drive.four() => {
                let front = transfer.front_share * 0.5;
                [front, front, 0.5 - front, 0.5 - front]
            }
            Some(_) => [0.0, 0.0, 0.5, 0.5],
            None if self.spec.front.driven => [0.5, 0.5, 0.0, 0.0],
            None => [0.0, 0.0, 0.5, 0.5],
        }
    }

    /// How fast the driven wheels turn, as the gearbox's output sees them.
    fn shaft_speed(&self) -> f32 {
        let shares = self.shares();
        (0..4)
            .map(|index| shares[index] * self.wheels[index].speed)
            .sum()
    }

    fn driven_wheels(&self) -> [usize; 2] {
        if self.spec.front.driven && self.spec.transfer.is_none() {
            [0, 1]
        } else {
            [2, 3]
        }
    }

    /// Where a wheel is drawn, from the car's origin, and how it is turned: steered, and rolled.
    pub fn wheel_pose(&self, index: usize) -> (Vec3, Quat) {
        let axle = self.spec.axle(index);
        let wheel = &self.wheels[index];
        let travel = wheel.travel.clamp(-axle.travel_down, axle.travel_up + 0.02);
        (
            self.spec.hubs()[index] + Vec3::Y * travel,
            Quat::from_rotation_y(-wheel.steer) * Quat::from_rotation_x(-wheel.angle),
        )
    }

    /// How far each front wheel is steered, in radians, for this much of the lock: left, right.
    ///
    /// The wheel on the inside of the turn follows a tighter circle, so it turns further.
    fn steer_angles(&self, steer: f32) -> (f32, f32) {
        let middle = steer.clamp(-1.0, 1.0) * self.spec.steering_lock;
        if middle.abs() < 1.0e-5 {
            return (middle, middle);
        }
        let wheelbase = self.spec.wheelbase();
        let half_track = self.spec.front.track * 0.5;
        // The turn's radius to the car's middle line: above zero turning right.
        let radius = wheelbase / middle.tan();
        let ideal = |offset: f32| (wheelbase / (radius - offset)).atan();
        let blend = |ideal: f32| middle + self.spec.ackermann * (ideal - middle);
        (blend(ideal(-half_track)), blend(ideal(half_track)))
    }

    /// Asks for a gear change, if one can be made: not while another is under way, not into a
    /// gear that would over-rev the engine, and into reverse only when nearly stopped.
    fn request_shift(&mut self, up: bool, speed: f32) {
        if self.shift.is_some() {
            return;
        }
        let top = self.spec.gearbox.gears.len() as i32;
        let target = self.gear + if up { 1 } else { -1 };
        if !(-1..=top).contains(&target) {
            return;
        }
        if target == -1 && speed > 1.5 {
            return;
        }
        if target >= 1 && speed < -1.5 {
            return;
        }
        if !up && target >= 1 {
            let gearbox = &self.spec.gearbox;
            let ratio = gearbox.gears[target as usize - 1] * gearbox.final_drive * self.range();
            let wheels = self.shaft_speed();
            if wheels * ratio > (self.spec.engine.limit + 300.0) * RPM_TO_RADIANS {
                return;
            }
        }
        let seconds = self.spec.gearbox.shift_seconds * if up { 1.0 } else { 2.0 };
        self.gear = target;
        self.shift = (seconds > 0.0).then_some(Shift {
            seconds_left: seconds,
            down: !up,
        });
    }

    /// Moves the car through one fixed step of `seconds`.
    pub fn step(
        &mut self,
        body: &mut Body,
        controls: &Controls,
        ground: &dyn Ground,
        seconds: f32,
    ) {
        let speed = body.velocity.dot(body.forward());
        if controls.shift_up {
            self.request_shift(true, speed);
        } else if controls.shift_down {
            self.request_shift(false, speed);
        }
        let (left, right) = self.steer_angles(controls.steer);
        self.wheels[0].steer = left;
        self.wheels[1].steer = right;

        // Hill descent control: in low range, with no foot on either pedal, the brakes hold
        // the car to a walking pace whichever way it is rolling.
        let mut held = *controls;
        let descending = self.spec.hill_descent
            && self.drive == DriveMode::FourLow
            && controls.throttle < 0.05
            && speed.abs() > DESCENT_SPEED;
        if descending {
            held.brake = held
                .brake
                .max(((speed.abs() - DESCENT_SPEED) * 0.55).min(DESCENT_BRAKE));
        }
        let controls = &held;

        let before = body.velocity;
        let dt = seconds / SUBSTEPS as f32;
        let mut aids = (false, false, false);
        for _ in 0..SUBSTEPS {
            let (abs, traction, stability) = self.substep(body, controls, ground, dt);
            aids = (aids.0 | abs, aids.1 | traction, aids.2 | stability);
        }
        self.mark(body, ground, seconds);

        let acceleration = (body.velocity - before) / seconds;
        // What the driver feels: the acceleration, less gravity's.
        let felt = acceleration + Vec3::Y * self.gravity;
        self.telemetry = Telemetry {
            speed: body.velocity.dot(body.forward()),
            rpm: self.rpm(),
            gear: self.gear,
            throttle: self.telemetry.throttle,
            g: Vec3::new(
                felt.dot(body.forward()),
                felt.dot(body.right()),
                felt.dot(body.up()),
            ) / self.gravity,
            abs_active: aids.0,
            traction_active: aids.1,
            stability_active: aids.2,
            limiter: self.limiting,
            drive: self.drive,
            front_locked: self.lockers.0 && self.drive.four(),
            rear_locked: self.lockers.1,
            aired_down: self.aired_down,
            descent_active: descending,
        };
    }

    /// One short step. Returns whether the anti-lock brakes, traction control and stability
    /// control were at work.
    fn substep(
        &mut self,
        body: &mut Body,
        controls: &Controls,
        ground: &dyn Ground,
        dt: f32,
    ) -> (bool, bool, bool) {
        let shares = self.shares();
        let range = self.range();
        let four = self.drive.four() && self.spec.transfer.is_some();
        let lockers = self.lockers;
        let wetness = self.wetness;
        let aired = self.aired_down;
        let spec = &self.spec;
        let hubs = spec.hubs();
        let (forward, up) = (body.forward(), body.up());
        let centre = body.position + body.rotation * spec.centre;

        // 1. The ground under each wheel, and how far each has risen.
        let mut found: [Option<Found>; 4] = [None; 4];
        let mut travel = [0.0_f32; 4];
        for index in 0..4 {
            let axle = spec.axle(index);
            let top_offset = axle.travel_up + RAY_MARGIN;
            let top = body.position + body.rotation * hubs[index] + up * top_offset;
            let reach = top_offset + axle.travel_down + axle.radius;
            travel[index] = -axle.travel_down;
            let rest = if index < 2 {
                self.static_loads.0
            } else {
                self.static_loads.1
            };
            let wheel = &mut self.wheels[index];
            self.touching[index] = None;
            // A wall, or the underside of something, is not ground to stand on.
            let contact = ground
                .cast(top, -up, reach + wheel.sink.max(0.0))
                .filter(|contact| contact.normal.dot(up) >= 0.3 && contact.normal.is_finite());
            let Some(contact) = contact else {
                // In the air, the tyre is out of whatever it had sunk into.
                wheel.sink *= (-dt / RISE_SECONDS).exp();
                continue;
            };
            let point = top - up * contact.distance;
            // What has been pressed into the ground here already: the floor of a rut, which
            // the tyre stands on, or the berm beside one, which it rides over.
            let floor = ground.rut(point);
            let floor = if floor.is_finite() { floor } else { 0.0 };
            let grip = contact.surface.grip();
            // Soft ground gives until it carries the tyre: deeper the harder the tyre is
            // pressed, and deeper again under a wheel that spins and digs. It takes a moment
            // to sink, and longer to climb out.
            let soft = grip.sink_at(wetness) * if aired { AIRED_SINK } else { 1.0 };
            let wanted = if soft > 0.0 || grip.dig > 0.0 {
                let pressed = (wheel.load / rest).clamp(0.0, 2.5);
                let spinning = (wheel.slip_ratio.abs() - 0.15).clamp(0.0, 1.0);
                let sunk = soft * pressed.sqrt() + grip.dig * spinning * pressed.min(1.0);
                // A wheel that goes on spinning goes on digging: always a little below the
                // floor of the hole it has made, until it is as deep as this ground lets a
                // wheel dig itself.
                let deepest = (soft * DIG_SOFT + grip.dig * DIG_SPUN).min(DIG_RADIUS * axle.radius);
                let dug = floor + grip.dig * spinning * pressed.min(1.0) * DIG_ON;
                sunk.max(dug.min(deepest))
            } else {
                0.0
            };
            // The tyre stands on the floor of whatever rut is here, and sinks below it only
            // as far as it would have sunk in ground nobody had pressed.
            let below = (wheel.sink - floor).max(0.0);
            let wanted_below = (wanted - floor).max(0.0);
            let seconds = if wanted_below > below {
                SINK_SECONDS
            } else {
                RISE_SECONDS
            };
            wheel.sink = floor + below + (wanted_below - below) * (1.0 - (-dt / seconds).exp());
            self.touching[index] = Some(point);
            let rough = if grip.bump_height > 0.0 {
                grip.bump_height * bump(point.x, point.z, grip.bump_length)
            } else {
                0.0
            };
            let risen = top_offset - (contact.distance + wheel.sink - rough - axle.radius);
            if risen <= -axle.travel_down {
                continue;
            }
            travel[index] = risen;
            found[index] = Some(Found {
                point,
                normal: contact.normal,
                surface: contact.surface,
            });
        }

        // The load on each tyre: spring, damper, anti-roll bar and bump stop.
        let mut loads = [0.0_f32; 4];
        for index in 0..4 {
            let Some(contact) = &found[index] else {
                continue;
            };
            let axle = spec.axle(index);
            let rest = if index < 2 {
                self.static_loads.0
            } else {
                self.static_loads.1
            };
            let arm = contact.point - centre;
            let moving = body.velocity + body.spin.cross(arm);
            let rising = -moving.dot(contact.normal);
            let damper = if rising > 0.0 {
                axle.bump
            } else {
                axle.rebound
            };
            let damped = damper
                * if rising.abs() < DAMPER_KNEE {
                    rising
                } else {
                    rising.signum() * (DAMPER_KNEE + DAMPER_FAST * (rising.abs() - DAMPER_KNEE))
                };
            // The bar pushes down on the wheel that has risen more than its neighbour.
            let other = index ^ 1;
            let bar = axle.bar * (travel[index] - travel[other]);
            let stop = BUMP_STOP * (travel[index] - axle.travel_up).max(0.0);
            let load = rest + axle.spring * travel[index] + damped + bar + stop;
            loads[index] = load.clamp(0.0, rest * MAX_LOAD);
        }

        // 2. Each tyre's slip, and the force it makes at the speed its wheel turns now.
        let mut patches: [Option<Patch>; 4] = [None; 4];
        for index in 0..4 {
            let wheel = &mut self.wheels[index];
            wheel.travel = travel[index];
            wheel.load = loads[index];
            wheel.on_ground = found[index].is_some();
            let Some(contact) = found[index].filter(|_| loads[index] > 0.0) else {
                wheel.slide = 0.0;
                wheel.slip_ratio = 0.0;
                wheel.force = (0.0, 0.0);
                continue;
            };
            wheel.surface = contact.surface;
            let axle = spec.axle(index);
            let heading = body.rotation * (Quat::from_rotation_y(-wheel.steer) * Vec3::NEG_Z);
            let along =
                (heading - contact.normal * heading.dot(contact.normal)).normalize_or_zero();
            if along == Vec3::ZERO {
                continue;
            }
            let across = along.cross(contact.normal);
            let arm = contact.point - centre;
            let moving = body.velocity + body.spin.cross(arm);
            let (rolling, sliding) = (moving.dot(along), moving.dot(across));
            let against = rolling.abs().max(LOW_SPEED);
            // The side force lags the slip angle: the tyre must roll a way to take up its set.
            let slide = -sliding / against;
            let catch_up = 1.0 - (-dt * (rolling.abs() + 3.0) / spec.tyre.relaxation).exp();
            wheel.slide += (slide - wheel.slide) * catch_up;
            let surface = contact.surface.grip();
            // What holds a tyre back: the ground's own drag, and the rut it is ploughing.
            // Let down, a tyre grips soil better, and squirms and drags on anything hard.
            let loose = surface.slip > 1.5;
            let (footprint, squirm) = match (aired, loose) {
                (false, _) => (1.0, 1.0),
                (true, true) => (AIRED_GRIP, 1.0),
                (true, false) => (AIRED_ROAD_GRIP, AIRED_ROLLING),
            };
            let drag =
                surface.rolling_at(rolling) * squirm + RUT_DRAG * wheel.sink.max(0.0) / axle.radius;
            patches[index] = Some(Patch {
                point: contact.point,
                normal: contact.normal,
                along,
                across,
                rolling,
                against,
                grip: surface.grip_at(wetness)
                    * axle.grip
                    * footprint
                    * spec.tyre.tread_on(surface.sink),
                slip: surface.slip,
                hold: surface.hold,
                resistance: -drag * loads[index] * (rolling / 0.5).tanh(),
            });
        }

        // The tyre's force along, as the drivetrain needs it: at the wheel's speed now, and how
        // it changes with that speed.
        let tyre = spec.tyre;
        let curves = self.curves;
        // On loose ground a tyre must slip further to grip its best: its slips are measured
        // against that.
        let grip_force = |patch: &Patch, load: f32, slide: f32, radius: f32, speed: f32| {
            let mut ratio = (speed * radius - patch.rolling) / patch.against;
            // Soil goes on pushing back past the slip it grips best at: so much of the slip
            // beyond that counts for nothing.
            let best = tyre.peak_ratio * patch.slip;
            if ratio.abs() > best {
                ratio = ratio.signum() * (best + (ratio.abs() - best) * (1.0 - patch.hold));
            }
            tyre.force(
                &curves,
                load,
                ratio / patch.slip,
                slide / patch.slip,
                patch.grip,
            )
        };
        let mut along_now = [0.0_f32; 4];
        let mut stiffness = [0.0_f32; 4];
        for index in 0..4 {
            let Some(patch) = &patches[index] else {
                continue;
            };
            let (wheel, radius) = (&self.wheels[index], spec.axle(index).radius);
            const NUDGE: f32 = 0.25;
            let (now, _) = grip_force(patch, loads[index], wheel.slide, radius, wheel.speed);
            let (more, _) = grip_force(
                patch,
                loads[index],
                wheel.slide,
                radius,
                wheel.speed + NUDGE,
            );
            along_now[index] = now;
            // Past its peak a tyre grips less the more it slips. Solving ahead with that would
            // feed the slip; holding the force steady there is safe, and corrected next step.
            stiffness[index] = ((more - now) / NUDGE).max(0.0);
        }

        // The driver's aids, from the slip of the step before.
        let mut abs_active = false;
        for (wheel, patch) in self.wheels.iter_mut().zip(&patches) {
            // Locking: turning slower than the road passes, whichever way the car is going.
            let slipping = spec.abs
                && controls.brake > 0.0
                && wheel.on_ground
                && patch.is_some_and(|p| {
                    // On loose ground a tyre grips best slipping further: it is let.
                    p.rolling.abs() > 2.5
                        && wheel.slip_ratio * p.rolling.signum() < -ABS_SLIP * p.slip
                });
            wheel.brake_share = if slipping {
                abs_active = true;
                (wheel.brake_share - dt * 22.0).max(0.05)
            } else {
                (wheel.brake_share + dt * 9.0).min(1.0)
            };
        }
        // Traction control eases the throttle by how far past the tyre's peak the faster
        // driven wheel is spinning, quickly, and gives it back more slowly.
        let driven = self.driven_wheels();
        // How far each driven wheel is spinning past the speed the car is going, measured
        // against what the ground under it grips best at. A wheel in the air counts: it is
        // the one that spins.
        let onward = body.velocity.dot(forward);
        let mut spin = [0.0_f32; 4];
        for index in 0..4 {
            if shares[index] > 0.0 {
                let slip = patches[index].map_or(1.0, |patch| patch.slip);
                let passing = self.wheels[index].speed * spec.axle(index).radius - onward;
                spin[index] =
                    passing * self.gear.signum() as f32 / onward.abs().max(LOW_SPEED) / slip;
            }
        }
        // A four-wheel drive's traction control works on each wheel: one that runs away from
        // the wheel across its axle is braked, which sends the drive across an open
        // differential to the wheel that grips, and the engine is eased only for the wheels
        // that are not being braked so. A car with one driven axle eases it for the faster.
        let per_wheel = spec.transfer.is_some();
        let runaway = |index: usize| {
            per_wheel
                && (self.wheels[index].speed - self.wheels[index ^ 1].speed)
                    * spec.axle(index).radius
                    * self.gear.signum() as f32
                    > 0.5
        };
        let wheelspin = (0..4)
            .filter(|&index| shares[index] > 0.0 && self.wheels[index].on_ground)
            .filter(|&index| !runaway(index))
            .map(|index| {
                if per_wheel {
                    spin[index]
                } else {
                    self.wheels[index].slip_ratio * self.gear.signum() as f32
                        / patches[index].map_or(1.0, |patch| patch.slip)
                }
            })
            .fold(0.0, f32::max);
        // It lets a tyre slip as far as that tyre grips best at, and a little more: a slick
        // peaks sooner than a tyre with tread.
        let allowed = TRACTION_SLIP.max(spec.tyre.peak_ratio * 1.15);
        let spinning = spec.traction_control && controls.throttle > 0.0 && wheelspin > allowed;
        // At once, no more throttle than the slip allows; and for as long as the wheel goes on
        // spinning, less and less, until it is back at the edge of its grip. Without the
        // second, it would settle part-way, spinning a little for good.
        self.traction_share = if spinning {
            let excess = (wheelspin - allowed) / TRACTION_BAND;
            (self.traction_share - dt * 14.0 * excess)
                .min(1.0 - excess)
                .max(0.0)
        } else {
            (self.traction_share + dt * 3.5).min(1.0)
        };

        // Stability control watches the tail. The rear tyres' slip angle is how far the rear
        // axle's own path points from the way the car does; past the angle they grip best at,
        // the tail is on its way round. It looks a little ahead at the rate that is growing,
        // then eases the engine, lets go of the rear brakes, and brakes the outside front
        // wheel, which pulls the nose back into line.
        let behind = body.rotation * (Vec3::new(0.0, 0.0, spec.rear.at) - spec.centre);
        let passing = body.velocity + body.spin.cross(behind);
        let (sideways, onward) = (passing.dot(body.right()), passing.dot(forward));
        let rear_slide = if onward > STABILITY_SPEED {
            sideways.atan2(onward)
        } else {
            0.0
        };
        let growing = ((rear_slide.abs() - self.rear_slide.abs()) / dt).max(0.0);
        self.rear_slide = rear_slide;
        let peak = spec.tyre.peak_angle;
        let slide = rear_slide.abs() + growing * STABILITY_AHEAD;
        let over =
            |from: f32| ((slide - peak * from) / (peak * (STABILITY_FULL - from))).clamp(0.0, 1.0);
        let (catching, releasing) = if spec.stability {
            (over(STABILITY_FROM), over(STABILITY_RELEASE))
        } else {
            (0.0, 0.0)
        };
        // The front wheel on the side the tail is sliding towards: the outside one.
        let outside = if rear_slide > 0.0 { 1 } else { 0 };

        // The engine: what it is asked for, and the torque it makes.
        let engine = &spec.engine;
        let rpm = self.engine_speed / RPM_TO_RADIANS;
        self.limiting = if self.limiting {
            rpm > engine.limit - 150.0
        } else {
            rpm > engine.limit
        };
        let gearbox = &spec.gearbox;
        let ratio = match self.gear {
            0 => None,
            gear if gear < 0 => Some(-gearbox.reverse * gearbox.final_drive * range),
            gear => Some(gearbox.gears[gear as usize - 1] * gearbox.final_drive * range),
        };
        let wheels_speed: f32 = (0..4)
            .map(|index| shares[index] * self.wheels[index].speed)
            .sum();
        // How fast the gearbox's input shaft turns: what the engine would turn at, in gear.
        let shaft = ratio.map_or(0.0, |ratio| wheels_speed * ratio);
        let mut throttle =
            controls.throttle.clamp(0.0, 1.0) * self.traction_share * (1.0 - catching);
        if let Some(shift) = &self.shift {
            // A lower gear wants the engine faster: blip it up to the wheels' speed.
            throttle = if shift.down && self.engine_speed < shaft * 0.97 {
                1.0
            } else {
                0.0
            };
        }
        if self.limiting {
            throttle = 0.0;
        }
        // Idling: enough throttle to keep turning.
        let idling = ((engine.idle * 1.03 - rpm) / 150.0).clamp(0.0, 1.0) * 0.6;
        throttle = throttle.max(idling);
        self.telemetry.throttle = throttle;
        let engine_torque =
            engine.full_torque(rpm) * throttle - engine.drag_torque(rpm) * (1.0 - throttle);

        // The clutch: open in a gear change, slipping to pull away, shut once rolling.
        let shaft_rpm = shaft / RPM_TO_RADIANS;
        let engaged = if self.shift.is_some() || ratio.is_none() {
            0.0
        } else if shaft_rpm > engine.idle * 1.15 && shaft_rpm >= rpm - CLUTCH_MATCHED {
            // Rolling, with the engine and the gearbox turning together, or the wheels
            // driving the engine: shut.
            1.0
        } else {
            // Pulling away, or the engine still coming down to the gearbox's speed: the clutch
            // bites as the revs rise past a point that goes up with the throttle. A gentle
            // start slips it just above idle; a hard one holds the engine up in its torque
            // while the car catches up. Traction control eases it with the throttle, as the
            // engine's own momentum would spin the wheels whatever the throttle did.
            // A gearbox that cruises takes up the drive low in the revs, as a road car's
            // does, where a racing car's is held up in its power to leave the line.
            let launch = if spec.gearbox.cruise < 1.0 {
                LAUNCH_REVS * 0.4
            } else {
                LAUNCH_REVS
            };
            let bite = engine.idle * 1.05 + controls.throttle.clamp(0.0, 1.0) * launch;
            ((rpm - bite) / 1500.0).clamp(0.0, 1.0) * self.traction_share
        };
        let diff = &spec.differential;
        let carried = self.clutch_torque * ratio.unwrap_or(0.0).abs() * spec.gearbox.efficiency;
        let lock = diff.preload
            + carried.abs()
                * if self.clutch_torque >= 0.0 {
                    diff.power
                } else {
                    diff.coast
                };

        // The brakes: each wheel's, as far as the anti-lock system lets it.
        let brake = controls.brake.clamp(0.0, 1.0);
        let handbrake = controls.handbrake.clamp(0.0, 1.0);
        let mut brakes = [0.0_f32; 4];
        for (index, torque) in brakes.iter_mut().enumerate() {
            let axle = spec.axle(index);
            let share = self.wheels[index].brake_share;
            *torque = brake * axle.brake * share;
            // A four-wheel drive's wheel that runs away from the one across its axle is
            // braked, by how far it has run away: metres a second at the tread. Wheels that
            // a locked differential holds together never do.
            if per_wheel && spec.traction_control && controls.throttle > 0.0 && shares[index] > 0.0
            {
                let ahead = (self.wheels[index].speed - self.wheels[index ^ 1].speed)
                    * axle.radius
                    * self.gear.signum() as f32;
                let over = ((ahead - 0.5) / 1.5).clamp(0.0, 1.0);
                *torque += over * axle.brake * BRAKE_TRACTION;
            }
            if index >= 2 {
                *torque *= 1.0 - 0.92 * releasing;
                *torque += handbrake * 3000.0;
            } else if index == outside {
                *torque =
                    (*torque + catching * STABILITY_BRAKE * axle.brake * share).min(axle.brake);
            }
        }

        // Solve the turning parts for their speeds at the end of this short step.
        let axle = spec.axle(driven[0]);
        let side = |index: usize| drivetrain::Wheel {
            speed: self.wheels[index].speed,
            inertia: spec.axle(index).inertia,
            radius: spec.axle(index).radius,
            force: along_now[index],
            stiffness: stiffness[index],
            brake: brakes[index],
        };
        debug_assert!(axle.driven);
        let mut speeds = [0.0_f32; 4];
        if let Some(transfer) = &spec.transfer {
            // A four-wheel drive: both axles, and what joins them. In two-wheel drive the
            // front is sent nothing and the lock between the propshafts is open. A locked
            // differential carries whatever its lock can; the front's own differential shares
            // what its axle is sent as the rear's does.
            let front_share = if four { transfer.front_share } else { 0.0 };
            let front_diff = &spec.front_differential;
            let front_lock = if lockers.0 && four {
                transfer.locker
            } else {
                front_diff.preload
                    + carried.abs()
                        * front_share
                        * if self.clutch_torque >= 0.0 {
                            front_diff.power
                        } else {
                            front_diff.coast
                        }
            };
            let rear_lock = if lockers.1 {
                transfer.locker
            } else {
                diff.preload + (lock - diff.preload) * (1.0 - front_share)
            };
            let solved = drivetrain::solve_all(
                &Driveline {
                    engine_speed: self.engine_speed,
                    engine_inertia: engine.inertia,
                    engine_torque,
                    ratio: ratio.unwrap_or(0.0),
                    efficiency: spec.gearbox.efficiency,
                    clutch: Coupling::new(spec.gearbox.clutch * engaged),
                    front_share,
                    centre: Coupling::new(if four { transfer.lock } else { 0.0 }),
                    front: Coupling::new(front_lock),
                    rear: Coupling::new(rear_lock),
                    wheels: [side(0), side(1), side(2), side(3)],
                },
                dt,
            );
            speeds = solved.wheels;
            self.engine_speed = solved.engine.max(0.0);
            self.clutch_torque = solved.clutch_torque;
        } else {
            let solved = drivetrain::solve(
                &DrivenAxle {
                    engine_speed: self.engine_speed,
                    engine_inertia: engine.inertia,
                    engine_torque,
                    ratio: ratio.unwrap_or(0.0),
                    efficiency: spec.gearbox.efficiency,
                    clutch: Coupling::new(spec.gearbox.clutch * engaged),
                    differential: Coupling::new(lock),
                    left: side(driven[0]),
                    right: side(driven[1]),
                },
                dt,
            );
            speeds[driven[0]] = solved.left;
            speeds[driven[1]] = solved.right;
            for index in (0..4).filter(|index| !driven.contains(index)) {
                speeds[index] = drivetrain::solve_free(&side(index), dt);
            }
            self.engine_speed = solved.engine.max(0.0);
            self.clutch_torque = solved.clutch_torque;
        }

        // 3. The forces on the body.
        let mut force = Vec3::new(0.0, -spec.mass * self.gravity, 0.0);
        let mut torque = Vec3::ZERO;
        let mut push = |at: Vec3, pushed: Vec3| {
            force += pushed;
            torque += (at - centre).cross(pushed);
        };
        for index in 0..4 {
            let wheel = &mut self.wheels[index];
            let axle = spec.axle(index);
            let was = wheel.speed;
            wheel.speed = speeds[index];
            wheel.angle = (wheel.angle + wheel.speed * dt) % std::f32::consts::TAU;
            let Some(patch) = &patches[index] else {
                continue;
            };
            let load = loads[index];
            wheel.slip_ratio = (wheel.speed * axle.radius - patch.rolling) / patch.against;
            // Along: what the drivetrain was solved with, so that the wheel and the road agree.
            // Across: what the tyre gives at its new slip. Both within what the tyre can do.
            let limit = load * tyre.load_factor(load) * patch.grip;
            let mut along = along_now[index] + stiffness[index] * (wheel.speed - was);
            let (_, mut across) = grip_force(patch, load, wheel.slide, axle.radius, wheel.speed);
            let used = (along / (limit * tyre.grip_long)).hypot(across / (limit * tyre.grip_lat));
            if used > 1.0 {
                along /= used;
                across /= used;
            }
            wheel.force = (along, across);

            push(patch.point, patch.normal * load);
            // The suspension's links carry a share of the weight transfer straight to the
            // body: as if the tyre's force reached it higher up.
            let links = if index < 2 || along < 0.0 {
                spec.anti_dive
            } else {
                spec.anti_squat
            };
            push(
                patch.point + up * (links * spec.centre.y),
                patch.along * (along + patch.resistance),
            );
            push(patch.point + up * axle.roll_centre, patch.across * across);
        }

        // The air: drag against the way it goes, and downforce on each axle.
        let air_speed = body.velocity.length();
        push(
            centre,
            -body.velocity * (0.5 * AIR_DENSITY * spec.aero.drag * air_speed),
        );
        let ahead = body.velocity.dot(forward);
        let downforce = 0.5 * AIR_DENSITY * spec.aero.downforce * ahead * ahead;
        let axle_at = |z: f32| body.position + body.rotation * Vec3::new(0.0, spec.centre.y, z);
        push(
            axle_at(-spec.front.at),
            -up * (downforce * spec.aero.balance),
        );
        push(
            axle_at(spec.rear.at),
            -up * (downforce * (1.0 - spec.aero.balance)),
        );

        // 4. Move the body: velocity and spin first, then place and turn with the new ones.
        body.velocity += force / spec.mass * dt;
        let inverse = body.rotation.inverse();
        let mut spin = inverse * body.spin;
        let turning = inverse * torque - spin.cross(spec.inertia * spin);
        spin += turning / spec.inertia * dt;
        body.spin = body.rotation * spin;
        let centre = centre + body.velocity * dt;
        body.rotation = (Quat::from_scaled_axis(body.spin * dt) * body.rotation).normalize();
        body.position = centre - body.rotation * spec.centre;

        if let Some(shift) = &mut self.shift {
            shift.seconds_left -= dt;
            if shift.seconds_left <= 0.0 {
                self.shift = None;
            }
        }
        (
            abs_active,
            spinning,
            catching > 0.02 || (brake > 0.0 && releasing > 0.02),
        )
    }
}

/// Where a wheel's ray met the ground.
#[derive(Clone, Copy)]
struct Found {
    point: Vec3,
    normal: Vec3,
    surface: Surface,
}

/// A tyre's contact with the ground.
#[derive(Clone, Copy)]
struct Patch {
    point: Vec3,
    normal: Vec3,
    /// The way the tyre points, along the ground, and to its right.
    along: Vec3,
    across: Vec3,
    /// How fast the ground passes under it, along the way it points.
    rolling: f32,
    /// The speed slip is measured against.
    against: f32,
    /// The surface's grip, with the axle's.
    grip: f32,
    /// How many times further the tyre must slip here to grip its best.
    slip: f32,
    /// How much of the fall in grip past its best a spinning tyre is spared here.
    hold: f32,
    /// Rolling resistance, along the way it points.
    resistance: f32,
}
