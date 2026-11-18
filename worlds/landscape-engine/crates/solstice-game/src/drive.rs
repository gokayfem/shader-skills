//! Driving: the player's car, what is drawn of it, the camera that follows it, and its
//! instruments.
//!
//! * The input actions `throttle`, `brake`, `steer` (above zero to the right), `handbrake`,
//!   `shift_up` and `shift_down` drive the player's car. `reset` puts it back on the track, and
//!   `camera` changes the view; `look_back` looks behind while held.
//! * A car's wheels are instances of its mesh, found by where they are: an instance whose own
//!   origin is at one of the hubs its `.vehicle` file gives is that wheel, and is drawn where
//!   the suspension has it, steered and rolled. One whose origin is a few centimetres from a
//!   hub is on that wheel's upright, as a brake caliper is: it rides and steers with the wheel,
//!   and does not turn with it.

use crate::race::Lap;
use crate::track::Track;
use glam::{EulerRot, Mat4, Quat, Vec3};
use solstice_physics::vehicle::{Asked, Telemetry, Vehicle};
use solstice_physics::Car;
use solstice_world::{Entity, Input, Transform, World, STEP_SECONDS};

/// How near a hub an instance's origin must be, in metres, to be carried by that wheel.
pub const HUB_REACH: f32 = 0.06;
/// How near it must be to turn with the wheel. Further out than this, and within
/// [`HUB_REACH`], it is on the upright: it rides and steers, and stays the right way up.
pub const SPIN_REACH: f32 = 0.012;

/// One wheel, as it is drawn.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DrawnWheel {
    /// Where its middle is, from the car's origin.
    pub position: Vec3,
    /// How far it is steered, in radians, above zero to the right.
    pub steer: f32,
    /// How far it has rolled, in radians, and how fast it is rolling, in radians a second.
    pub angle: f32,
    pub speed: f32,
}

impl DrawnWheel {
    /// Where the wheel is drawn, relative to the car, `alpha` of the way from the step before
    /// to the latest.
    pub fn between(before: &Self, now: &Self, alpha: f32) -> Mat4 {
        let position = before.position.lerp(now.position, alpha);
        let steer = before.steer + (now.steer - before.steer) * alpha;
        // Rolled back from where it is now by how fast it is rolling: a wheel turns more than
        // half way round in a step at speed, and the short way between two angles is then the
        // wrong way.
        let angle = now.angle - now.speed * STEP_SECONDS as f32 * (1.0 - alpha);
        Mat4::from_rotation_translation(
            Quat::from_rotation_y(-steer) * Quat::from_rotation_x(-angle),
            position,
        )
    }

    /// Where the wheel's upright is drawn: where the wheel is, steered, and not rolled.
    pub fn upright(before: &Self, now: &Self, alpha: f32) -> Mat4 {
        let position = before.position.lerp(now.position, alpha);
        let steer = before.steer + (now.steer - before.steer) * alpha;
        Mat4::from_rotation_translation(Quat::from_rotation_y(-steer), position)
    }
}

/// A car's wheels as of the latest step and the one before, for drawing between them.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Wheels {
    pub now: [DrawnWheel; 4],
    pub before: [DrawnWheel; 4],
}

impl Wheels {
    /// Takes the wheels as the vehicle has them after a step.
    pub fn follow(&mut self, vehicle: &Vehicle) {
        self.before = self.now;
        for (index, drawn) in self.now.iter_mut().enumerate() {
            let (position, _) = vehicle.wheel_pose(index);
            let wheel = &vehicle.wheels[index];
            *drawn = DrawnWheel {
                position,
                steer: wheel.steer,
                angle: wheel.angle,
                speed: wheel.speed,
            };
        }
    }

    /// The wheels of a car standing still, both steps the same.
    pub fn at_rest(vehicle: &Vehicle) -> Self {
        let mut wheels = Self::default();
        wheels.follow(vehicle);
        wheels.before = wheels.now;
        wheels
    }
}

/// What the player asks of the car this step, from the input actions.
pub fn asked(input: &Input) -> Asked {
    Asked {
        throttle: input.value("throttle"),
        brake: input.value("brake"),
        steer: input.value("steer"),
        handbrake: input.value("handbrake"),
        shift_up: input.pressed("shift_up"),
        shift_down: input.pressed("shift_down"),
    }
}

/// What is wrong with a car, as its driver feels it: what damage has done to how it drives.
/// Whoever keeps the car's damage says ([`crate::Play::set_handicap`]); a sound car has none.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Handicap {
    /// How far its steering pulls to one side, as a share of full lock: above zero to the
    /// right. A bent wheel or a buckled arm does this, and it is held against.
    pub pull: f32,
    /// The share of its engine's pull it still has, from 0 to 1: an engine that has lost
    /// its cooling is nursed.
    pub power: f32,
}

impl Default for Handicap {
    fn default() -> Self {
        Self {
            pull: 0.0,
            power: 1.0,
        }
    }
}

impl Handicap {
    /// What the car is asked for, with this wrong with it.
    pub fn apply(&self, mut asked: Asked) -> Asked {
        asked.steer = (asked.steer + self.pull).clamp(-1.0, 1.0);
        asked.throttle *= self.power.clamp(0.0, 1.0);
        asked
    }
}

/// How hard a car that drives itself corners and brakes, in metres a second squared, and how
/// fast it will go, in metres a second.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pace {
    pub cornering: f32,
    pub braking: f32,
    pub top_speed: f32,
    /// How far ahead along the track it steers for, in metres: so far, and so much more for
    /// each metre a second it is going.
    pub looks: (f32, f32),
}

impl Pace {
    /// Round a circuit: within what a racing car on slicks does, with room to spare.
    pub const RACING: Self = Self {
        cornering: 13.0,
        braking: 12.0,
        top_speed: 80.0,
        looks: (7.0, 0.55),
    };
    /// Along a bush track: what loose clay lets a truck do, at a pace to see the country by.
    pub const TOURING: Self = Self {
        cornering: 2.4,
        braking: 2.8,
        top_speed: 11.0,
        // A track a car's width wide is followed closely, or its bends are cut through trees.
        looks: (3.6, 0.42),
    };
}

/// What a car driving itself round a track asks for, for showing a circuit off and for trying
/// one: it steers for the middle of the road a way ahead, further the faster it goes, and
/// keeps to a speed the bends coming can be taken at, braking for them in good time. `forward`
/// is the way the car points and `speed` how fast it goes along that. Gives back the sample
/// of the track it is at too, to find it by next time.
pub fn autopilot(
    track: &Track,
    position: Vec3,
    forward: Vec3,
    speed: f32,
    hint: Option<usize>,
    pace: &Pace,
    caution: Option<&dyn Fn(Vec3) -> f32>,
) -> (Asked, usize) {
    let place = track.locate(position, hint);
    let flat = |v: Vec3| Vec3::new(v.x, 0.0, v.z);
    let (target, _, _) = track.place(place.along + pace.looks.0 + speed.abs() * pace.looks.1);
    let to = flat(target - position).normalize_or_zero();
    // Above zero when the target is to the right, which is the way steering above zero turns.
    let turn = -flat(forward).normalize_or_zero().cross(to).y;

    // How fast each stretch of road ahead can be taken, and so how fast the car may be going
    // now and still slow for it in the distance there is.
    let mut limit = pace.top_speed;
    let mut ahead = 0.0_f32;
    while ahead < 260.0 {
        let at = |d: f32| flat(track.place(place.along + ahead + d).0);
        let (a, b, c) = (at(-6.0), at(0.0), at(6.0));
        let bend = 2.0 * (b - a).cross(c - b).length()
            / ((b - a).length() * (c - b).length() * (c - a).length()).max(1.0e-6);
        let through = (pace.cornering / bend.max(1.0e-4)).sqrt();
        let room = (ahead - 10.0).max(0.0);
        limit = limit.min((through * through + 2.0 * pace.braking * room).sqrt());
        ahead += 8.0;
    }
    // And for what the ground ahead is: `caution` says how fast a place may be crossed, a ford
    // or a bog at a crawl, and a track that drops away and comes back, as at a washout, is
    // crossed at one too.
    if let Some(caution) = caution {
        let mut ahead = 0.0_f32;
        while ahead <= 24.0 {
            let at = |d: f32| track.place(place.along + ahead + d).0;
            let (before, here, after) = (at(-3.0), at(0.0), at(3.0));
            let dip = (before.y + after.y - 2.0 * here.y).abs();
            let mut slow = caution(here);
            if dip > 0.22 {
                slow = slow.min(2.2);
            } else if dip > 0.1 {
                slow = slow.min(4.0);
            }
            let room = (ahead - 5.0).max(0.0);
            limit = limit.min((slow * slow + 2.0 * pace.braking * room).sqrt());
            ahead += 3.0;
        }
    }
    let asked = Asked {
        throttle: ((limit - speed) * 0.6).clamp(0.0, 1.0),
        brake: ((speed - limit) * 0.35).clamp(0.0, 1.0),
        steer: (turn * 3.2).clamp(-1.0, 1.0),
        ..Asked::default()
    };
    (asked, place.sample)
}

/// How far below the nearest stretch of its track a car is before it counts as lost, in
/// metres, and is put back.
pub const FALLEN: f32 = 14.0;

/// Puts a car back on the track: on the centre line nearest where it is, a little back, facing
/// the way the lap runs, at rest.
pub fn put_back(world: &mut World, entity: Entity, track: &Track) {
    // On a lap, in the section of it that is being driven: at its start, if the nearest of
    // the track is in another, so that a section is not got out of by being put back.
    let section = world
        .entities
        .get::<&Lap>(entity)
        .ok()
        .filter(|lap| lap.current.is_some())
        .map(|lap| lap.section(track));
    let Ok(mut transform) = world.entities.get::<&mut Transform>(entity) else {
        return;
    };
    let place = track.locate(transform.translation, None);
    let along = match section {
        Some((from, to)) if !(from..to).contains(&place.along) => from + 11.0,
        _ => place.along,
    };
    let (centre, forward, left) = track.place(along - 8.0);
    let up = forward.cross(left).normalize_or(Vec3::Y);
    // The car's -Z along the track, its +X to the track's right, and standing on its slope.
    transform.rotation = Quat::from_mat3(&glam::Mat3::from_cols(-left, up, -forward)).normalize();
    transform.translation = centre + up * 0.05;
    drop(transform);
    if let Ok(mut car) = world.entities.get::<&mut Car>(entity) {
        car.vehicle.settle();
        car.asked = Asked::default();
    }
    if let Ok(mut lap) = world.entities.get::<&mut Lap>(entity) {
        lap.interrupt();
    }
}

/// Puts a car somewhere, facing a way (a yaw in degrees: zero looks along -z), at rest: where
/// it was when a game was last left.
pub fn put_at(world: &mut World, entity: Entity, position: Vec3, yaw_degrees: f32) {
    let Ok(mut transform) = world.entities.get::<&mut Transform>(entity) else {
        return;
    };
    transform.rotation = Quat::from_rotation_y(yaw_degrees.to_radians());
    transform.translation = position + Vec3::Y * 0.08;
    drop(transform);
    if let Ok(mut car) = world.entities.get::<&mut Car>(entity) {
        car.vehicle.settle();
        car.asked = Asked::default();
    }
    if let Ok(mut lap) = world.entities.get::<&mut Lap>(entity) {
        lap.interrupt();
    }
}

/// The views of a car the `camera` action steps through: from close behind it out to high
/// above, then from on the car itself.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CameraView {
    /// Behind and above, following the car round.
    #[default]
    Chase,
    /// The same, from further back and higher.
    Far,
    /// Further again: the car and the road round it.
    Wide,
    /// From high above and well behind: the corner coming, and the one after.
    High,
    /// From the bonnet, looking down the road.
    Bonnet,
    /// From the nose, low, with nothing of the car in view.
    Bumper,
}

impl CameraView {
    pub fn next(self) -> Self {
        match self {
            Self::Chase => Self::Far,
            Self::Far => Self::Wide,
            Self::Wide => Self::High,
            Self::High => Self::Bonnet,
            Self::Bonnet => Self::Bumper,
            Self::Bumper => Self::Chase,
        }
    }

    /// For a view that follows the car: how far behind it the camera is, how far above, and
    /// how many seconds it takes to come round behind the car when the car turns.
    fn following(self) -> Option<(f32, f32, f32)> {
        match self {
            Self::Chase => Some((5.2, 1.5, 0.11)),
            Self::Far => Some((7.8, 2.3, 0.16)),
            Self::Wide => Some((12.5, 4.0, 0.2)),
            Self::High => Some((22.0, 11.0, 0.26)),
            Self::Bonnet | Self::Bumper => None,
        }
    }
}

/// Where the player is looking, besides the way the car is going.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Look {
    /// Behind: what the mirror shows.
    pub back: bool,
    /// Round the car: from -1, to the left, to 1, to the right. A stick held right over looks
    /// all but behind.
    pub around: f32,
    /// Over it: at 1 the camera is tipped up high above the car, at -1 down low behind it.
    pub over: f32,
    /// How far the mouse or the stick has swung the view round the car this frame, in radians,
    /// above zero to the left, and how far it has tipped it up over the car.
    pub orbit: f32,
    pub orbit_up: f32,
}

/// How far round the car a stick held right over looks, in radians, and how far up and down
/// one pulled right back or pushed right away tips the camera.
const LOOK_AROUND: f32 = 2.9;
const LOOK_OVER: f32 = 0.55;
const LOOK_UNDER: f32 = 0.2;
/// How long, in seconds, the camera takes to follow the stick.
const LOOK_EASE: f32 = 0.09;

/// The camera that follows a car.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Chase {
    pub view: CameraView,
    /// The way the camera looks round the car, eased behind the car's own heading: radians
    /// about +Y, zero looking along -Z.
    yaw: f32,
    /// The height it follows the car at, eased, so that kerbs don't shake the picture.
    height: f32,
    /// How far round the car the player is looking, and how far over it, eased: radians.
    around: f32,
    over: f32,
    started: bool,
    /// Where the mouse or the stick has swung the view to, round the car and over it, in
    /// radians: it stays there until it is swung again.
    orbit: f32,
    tilt: f32,
    /// How far the camera is lifted to keep the land out of the way, in metres, eased.
    lift: f32,
    /// How much bigger than a racing car what it follows is: a truck is followed from further
    /// back and higher. Zero stands for one.
    pub reach: f32,
    /// Where the driver's eyes are, and the front of the car, from its origin, if they are
    /// not a racing car's: for the views from on the car.
    pub eyes: Option<[Vec3; 2]>,
}

/// Where a camera is, and the way it looks.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shot {
    pub position: Vec3,
    pub yaw: f32,
    pub pitch: f32,
}

/// The angle from `from` to `to`, the short way round.
fn turn(from: f32, to: f32) -> f32 {
    (to - from + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI
}

impl Chase {
    /// The camera for a frame `seconds` long, for a car drawn at `car`, going at `speed` metres
    /// a second, with the player looking as `look` says.
    ///
    /// `land` gives the land's height under a place, if it is known: the camera is kept above
    /// it, and lifted as far as it takes to see the car over a rise between them.
    pub fn frame(
        &mut self,
        car: Mat4,
        speed: f32,
        look: Look,
        seconds: f32,
        land: Option<&dyn Fn(Vec3) -> Option<f32>>,
    ) -> Shot {
        let (_, rotation, origin) = car.to_scale_rotation_translation();
        let forward = rotation * Vec3::NEG_Z;
        let heading = (-forward.x).atan2(-forward.z);
        let (yaw, pitch, _) = rotation.to_euler(EulerRot::YXZ);
        let behind = if look.back { std::f32::consts::PI } else { 0.0 };
        // The stick, eased: a little of it looks a little way, and more of it a lot more.
        let asked = look.around.clamp(-1.0, 1.0);
        let around = asked.abs().powf(1.4) * asked.signum() * LOOK_AROUND;
        let tipped = look.over.clamp(-1.0, 1.0);
        let over = tipped * if tipped > 0.0 { LOOK_OVER } else { LOOK_UNDER };
        let follow = 1.0 - (-seconds / LOOK_EASE).exp();
        self.around += (around - self.around) * follow;
        self.over += (over - self.over) * follow;
        // The mouse and the stick swing the view freely: all the way round, and from low
        // behind the car to high over it. It stays where it is put.
        if look.orbit != 0.0 || look.orbit_up != 0.0 {
            self.orbit = turn(0.0, self.orbit + look.orbit);
            self.tilt = (self.tilt + look.orbit_up).clamp(-0.3, 1.05);
        }
        let reach = if self.reach > 0.0 { self.reach } else { 1.0 };
        let Some((back, up, ease)) = self.view.following() else {
            self.started = false;
            let eyes = self
                .eyes
                .unwrap_or([Vec3::new(0.0, 1.02, -0.45), Vec3::new(0.0, 0.52, -2.05)]);
            let eye = if self.view == CameraView::Bonnet {
                eyes[0]
            } else {
                eyes[1]
            };
            // From on the car, the stick turns the driver's head: to the side, and no further.
            let head = self.around.clamp(-1.9, 1.9);
            return Shot {
                position: origin + rotation * eye,
                yaw: yaw + behind - head,
                pitch: if look.back { -pitch } else { pitch } - self.over * 0.5,
            };
        };
        if !self.started {
            self.yaw = heading;
            self.height = origin.y;
            self.started = true;
        }
        // Eased round behind the car, so that a slide shows as the car turning in the
        // picture; and a little further back the faster it goes.
        let follow = 1.0 - (-seconds / ease).exp();
        self.yaw += turn(self.yaw, heading) * follow;
        self.height += (origin.y - self.height) * (1.0 - (-seconds / 0.09).exp());
        // Looking to the right is from over the car's left shoulder.
        let round = self.yaw + behind - self.around + self.orbit;
        let away = Vec3::new(round.sin(), 0.0, round.cos());
        let (back, up) = (back * reach, up * reach);
        let back = back + (speed.abs() / 85.0).min(1.0) * 0.9;
        // Tipped up over the car, or down behind it: the camera's arm swings about the car.
        let (sin, cos) = (self.over + self.tilt).sin_cos();
        let (back, up) = (back * cos - up * sin, (up * cos + back * sin).max(0.35));
        let mut position = Vec3::new(origin.x, self.height, origin.z) + away * back + Vec3::Y * up;
        // Looking at a point above the car and a little ahead of it.
        let target = origin + Vec3::Y * (0.85 * reach) - away * 1.2;
        // The land comes first: the camera is never under it, and where a rise stands
        // between it and the car it climbs until it sees over. Up quickly, down slowly.
        if let Some(land) = land {
            let mut needed = 0.0_f32;
            for (share, clear) in [(0.35, 0.25), (0.6, 0.3), (0.82, 0.4), (1.0, 0.7)] {
                let at = target.lerp(position, share);
                if let Some(ground) = land(at + Vec3::Y * 40.0) {
                    needed = needed.max((ground + clear - at.y) / share);
                }
            }
            let pace = if needed > self.lift { 0.07 } else { 0.6 };
            self.lift += (needed - self.lift) * (1.0 - (-seconds / pace).exp());
            position.y += self.lift.max(0.0);
        }
        let to = target - position;
        Shot {
            position,
            yaw: (-to.x).atan2(-to.z),
            pitch: to.y.atan2(to.x.hypot(to.z)),
        }
    }
}

/// The player's car's instruments, for a viewer to show.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Hud {
    /// Whether the driver has got out and is walking, and where they are: the car's
    /// instruments are then as it was left.
    pub afoot: bool,
    pub walker: Vec3,
    /// The way the driver faces, on foot: a yaw in radians (zero looks along -z).
    pub walker_yaw: f32,
    /// Where the car is and how it is turned: what is carried in it goes with it.
    pub origin: Vec3,
    pub rotation: Quat,
    /// Its tray, if it has one, in its own space: its size inside, across, deep and long,
    /// and the middle of its floor.
    pub tray: Option<(Vec3, Vec3)>,
    pub telemetry: Telemetry,
    /// Where the engine's limiter is, in rpm.
    pub limit: f32,
    /// The pedals and wheel as they are after easing: from 0 to 1, and from -1 to 1.
    pub throttle: f32,
    pub brake: f32,
    pub steer: f32,
    pub lap: Option<Lap>,
    pub track: String,
    pub view: CameraView,
    /// Each wheel, where it meets the ground: for what it throws up.
    pub wheels: [HudWheel; 4],
    /// The way the car points, and how fast it is going, in the world.
    pub forward: Vec3,
    pub velocity: Vec3,
    /// Whether the car has a transfer case: only then is its setting shown.
    pub four_wheel_drive: bool,
    /// How far its nose is up, and how far it leans to its right, in degrees.
    pub pitch: f32,
    pub roll: f32,
}

/// One wheel of the player's car, where it meets the ground.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct HudWheel {
    /// Where its tyre touches, in the world.
    pub at: Vec3,
    /// Where its middle is, in the car's own space, as its suspension has it: at rest it is
    /// its radius above the car's origin.
    pub hub: Vec3,
    pub on_ground: bool,
    pub surface: solstice_physics::vehicle::Surface,
    /// How fast its tread passes over the ground, in metres a second: above zero spinning
    /// faster than it rolls, below it locked or dragging.
    pub slip: f32,
    /// How far it has sunk into the ground, in metres, and how hard it is pressed to it, as a
    /// share of its load standing still.
    pub sink: f32,
    pub load: f32,
    /// What its tyre carries from where it has been, from 0 to 1: water, then mud.
    pub carried: (f32, f32),
    /// Its tyre's radius, in metres, and what its tread makes of soft ground: times its
    /// grip there, more for a tyre with deep lugs.
    pub radius: f32,
    pub tread: f32,
}

impl Hud {
    /// One line of it, for a window's title or a log.
    pub fn line(&self) -> String {
        use crate::race::format_time;
        let t = &self.telemetry;
        let gear = match t.gear {
            -1 => "R".to_owned(),
            0 => "N".to_owned(),
            gear => gear.to_string(),
        };
        let mut line = format!(
            "{:3.0} km/h   {gear}   {:4.0} rpm",
            t.speed.abs() * 3.6,
            t.rpm
        );
        if t.abs_active {
            line.push_str("   ABS");
        }
        if t.traction_active {
            line.push_str("   TC");
        }
        if t.stability_active {
            line.push_str("   ESC");
        }
        if self.four_wheel_drive {
            line.push_str("   ");
            line.push_str(t.drive.name());
            match (t.front_locked, t.rear_locked) {
                (true, true) => line.push_str("  diffs locked"),
                (false, true) => line.push_str("  rear diff locked"),
                (true, false) => line.push_str("  front diff locked"),
                (false, false) => {}
            }
            if t.descent_active {
                line.push_str("  HDC");
            }
            if t.aired_down {
                line.push_str("  tyres down");
            }
            // The inclinometer: a truck goes over somewhere past forty degrees of lean.
            line.push_str(&format!(
                "   pitch {:+3.0}°  roll {:+3.0}°",
                self.pitch, self.roll
            ));
        }
        if let Some(lap) = &self.lap {
            let time = |seconds: Option<f64>| seconds.map_or("-:--.---".to_owned(), format_time);
            line.push_str(&format!(
                "   |   lap {}  {}{}   last {}   best {}",
                lap.count + 1,
                time(lap.current),
                if lap.valid { "" } else { " (off track)" },
                time(lap.last),
                time(lap.best)
            ));
        }
        line
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wheel_is_drawn_rolled_back_from_where_it_is() {
        let before = DrawnWheel {
            position: Vec3::new(0.8, 0.33, -1.3),
            steer: 0.0,
            angle: 1.0,
            speed: 240.0,
        };
        let now = DrawnWheel {
            position: Vec3::new(0.8, 0.35, -1.3),
            steer: 0.2,
            angle: 1.0 + 240.0 / 60.0,
            speed: 240.0,
        };
        // At the latest step, as it is; at the one before, as it was then, though that is
        // more than half a turn back.
        let at = |alpha: f32| DrawnWheel::between(&before, &now, alpha);
        let turned = |angle: f32, steer: f32, position: Vec3| {
            Mat4::from_rotation_translation(
                Quat::from_rotation_y(-steer) * Quat::from_rotation_x(-angle),
                position,
            )
        };
        assert!(at(1.0).abs_diff_eq(turned(now.angle, 0.2, now.position), 1.0e-5));
        assert!(at(0.0).abs_diff_eq(turned(before.angle, 0.0, before.position), 1.0e-4));
        let half = turned(3.0, 0.1, Vec3::new(0.8, 0.34, -1.3));
        assert!(at(0.5).abs_diff_eq(half, 1.0e-4));
        // Its upright goes where it goes and steers with it, and doesn't roll.
        let held = DrawnWheel::upright(&before, &now, 0.5);
        assert!(held.abs_diff_eq(turned(0.0, 0.1, Vec3::new(0.8, 0.34, -1.3)), 1.0e-5));
    }

    #[test]
    fn the_chase_camera_sits_behind_the_car_and_looks_at_it() {
        let mut chase = Chase::default();
        // A car at (10, 2, 5) heading +X: yaw of -90 degrees.
        let car = Mat4::from_rotation_translation(
            Quat::from_rotation_y(-std::f32::consts::FRAC_PI_2),
            Vec3::new(10.0, 2.0, 5.0),
        );
        let ahead = Look::default();
        let shot = chase.frame(car, 0.0, ahead, 1.0 / 60.0, None);
        assert!(shot.position.x < 10.0 - 4.0, "behind: {}", shot.position);
        assert!((shot.position.z - 5.0).abs() < 1.0e-3 && shot.position.y > 3.0);
        assert!(
            (shot.yaw + std::f32::consts::FRAC_PI_2).abs() < 1.0e-3,
            "{}",
            shot.yaw
        );
        assert!(
            shot.pitch < 0.0 && shot.pitch > -0.4,
            "looking down a little"
        );
        // Looking back, it is ahead of the car, looking the other way.
        let back = Look {
            back: true,
            ..Look::default()
        };
        let shot = chase.frame(car, 0.0, back, 1.0 / 60.0, None);
        assert!(shot.position.x > 14.0);
        assert!((turn(shot.yaw, std::f32::consts::FRAC_PI_2)).abs() < 1.0e-3);
        // When the car turns, the camera comes round after it, not at once.
        let turned = Mat4::from_rotation_translation(Quat::IDENTITY, Vec3::new(10.0, 2.0, 5.0));
        let first = chase.frame(turned, 30.0, ahead, 1.0 / 60.0, None);
        assert!(
            first.position.x < 9.0 && first.position.z < 6.5,
            "{}",
            first.position
        );
        let mut shot = first;
        for _ in 0..120 {
            shot = chase.frame(turned, 30.0, ahead, 1.0 / 60.0, None);
        }
        assert!((shot.position.x - 10.0).abs() < 0.05 && shot.position.z > 9.0);
        assert!(shot.yaw.abs() < 0.01);
    }

    #[test]
    fn the_bonnet_camera_rides_with_the_car() {
        let mut chase = Chase {
            view: CameraView::Bonnet,
            ..Chase::default()
        };
        let car = Mat4::from_rotation_translation(
            Quat::from_rotation_y(0.5) * Quat::from_rotation_x(0.1),
            Vec3::new(1.0, 2.0, 3.0),
        );
        let shot = chase.frame(car, 20.0, Look::default(), 1.0 / 60.0, None);
        assert!((shot.yaw - 0.5).abs() < 1.0e-4 && (shot.pitch - 0.1).abs() < 1.0e-4);
        assert!(shot.position.distance(Vec3::new(1.0, 2.0, 3.0)) < 1.3);
        // The stick turns the driver's head: to the right is a lower yaw.
        let right = Look {
            around: 1.0,
            ..Look::default()
        };
        let mut turned = shot;
        for _ in 0..60 {
            turned = chase.frame(car, 20.0, right, 1.0 / 60.0, None);
        }
        assert!(
            turned.yaw < 0.5 - 1.5 && turned.yaw > 0.5 - 2.0,
            "{}",
            turned.yaw
        );
        assert_eq!(turned.position, shot.position);
    }

    #[test]
    fn the_views_step_out_from_the_car_and_then_onto_it() {
        let car = Mat4::from_translation(Vec3::new(0.0, 0.0, 0.0));
        let mut view = CameraView::Chase;
        let mut last = 0.0;
        // Each view that follows the car is further from it than the one before.
        for _ in 0..4 {
            let mut chase = Chase {
                view,
                ..Chase::default()
            };
            let shot = chase.frame(car, 0.0, Look::default(), 1.0 / 60.0, None);
            let away = shot.position.length();
            assert!(away > last * 1.3, "{view:?} is {away} m away, after {last}");
            // And looks down at the car from behind it.
            assert!(shot.position.z > 0.0 && shot.pitch < 0.0 && shot.yaw.abs() < 1.0e-4);
            last = away;
            view = view.next();
        }
        assert!(last > 20.0, "the last is well back: {last} m");
        assert_eq!(view, CameraView::Bonnet);
        assert_eq!(view.next().next(), CameraView::Chase);
    }

    #[test]
    fn the_stick_swings_the_camera_round_the_car() {
        // A car at the origin heading -Z: the camera starts behind it, at +Z.
        let car = Mat4::IDENTITY;
        let mut chase = Chase::default();
        let settle = |chase: &mut Chase, look: Look| {
            let mut shot = chase.frame(car, 0.0, look, 1.0 / 60.0, None);
            for _ in 0..90 {
                shot = chase.frame(car, 0.0, look, 1.0 / 60.0, None);
            }
            shot
        };
        let ahead = settle(&mut chase, Look::default());
        assert!(ahead.position.x.abs() < 1.0e-3 && ahead.position.z > 4.0);
        // Half-way to the right: from over the car's left shoulder, looking across it.
        let right = settle(
            &mut chase,
            Look {
                around: 0.5,
                ..Look::default()
            },
        );
        assert!(
            right.position.x < -2.0,
            "to the left of the car: {}",
            right.position
        );
        assert!(right.yaw < -0.3, "looking to the right: {}", right.yaw);
        // Right over: all but in front of the car, looking back at it.
        let over = settle(
            &mut chase,
            Look {
                around: 1.0,
                ..Look::default()
            },
        );
        assert!(
            over.position.z < -4.0,
            "ahead of the car: {}",
            over.position
        );
        // It does not jump there: the first frame of a push is a small move.
        let mut fresh = Chase::default();
        settle(&mut fresh, Look::default());
        let first = fresh.frame(
            car,
            0.0,
            Look {
                around: 1.0,
                ..Look::default()
            },
            1.0 / 60.0,
            None,
        );
        assert!(first.position.distance(ahead.position) < 3.5);
        // Let go, and it comes back behind.
        let back = settle(&mut chase, Look::default());
        assert!(back.position.distance(ahead.position) < 0.01);
        // Pulled back, it rises over the car; pushed away, it drops, but not into the road.
        let high = settle(
            &mut chase,
            Look {
                over: 1.0,
                ..Look::default()
            },
        );
        assert!(high.position.y > ahead.position.y + 2.0 && high.pitch < ahead.pitch - 0.3);
        let low = settle(
            &mut chase,
            Look {
                over: -1.0,
                ..Look::default()
            },
        );
        assert!(low.position.y < ahead.position.y && low.position.y > 0.3);
    }

    #[test]
    fn a_handicap_pulls_the_wheel_and_holds_the_engine_back() {
        let asked = Asked {
            throttle: 1.0,
            steer: 0.2,
            ..Asked::default()
        };
        // A sound car is asked for what its driver asks.
        assert_eq!(Handicap::default().apply(asked), asked);
        let wrecked = Handicap {
            pull: -0.3,
            power: 0.6,
        };
        let felt = wrecked.apply(asked);
        assert!((felt.steer + 0.1).abs() < 1.0e-6 && (felt.throttle - 0.6).abs() < 1.0e-6);
        // It pulls no further than the wheel turns.
        let hard = Asked {
            steer: -0.9,
            ..asked
        };
        assert_eq!(wrecked.apply(hard).steer, -1.0);
    }

    #[test]
    fn the_instruments_read_as_one_line() {
        let hud = Hud {
            telemetry: Telemetry {
                speed: 50.0,
                rpm: 6543.0,
                gear: 3,
                abs_active: true,
                ..Telemetry::default()
            },
            ..Hud::default()
        };
        assert_eq!(hud.line(), "180 km/h   3   6543 rpm   ABS");
    }
}
