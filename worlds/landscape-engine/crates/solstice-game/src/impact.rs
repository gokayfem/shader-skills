//! Blows: what a car hits, where on it, which way, and how hard.
//!
//! Nothing here decides what a blow breaks: that is whoever plays the game's to say (the
//! viewer's `damage.rs`). This is the measuring. Two things are measured, both of them what
//! the physics already works out each step:
//!
//! * **What its body meets.** A car's body is a box among the scene's walls, its ground and
//!   its blockers, and each step the physics says what meeting them did to the car's velocity
//!   and its spin ([`solstice_physics::Knock`]). Times its mass and its inertia that is the
//!   blow's impulse, and its turning impulse about the centre of mass; and the two together
//!   say where the blow landed, for a push `J` at `r` from the centre turns the car by
//!   `r × J`. A blow is seldom over in one step, so the steps of one blow are summed, and it
//!   is told once it is over. The physics says too where the body is touching what it met
//!   (`Physics::car_touches`): between them those places are the patch the blow landed on,
//!   the whole of a nose against a wall or one corner of it against a post.
//! * **What comes up through a wheel.** A wheel is not in that box: it is a spring, and what
//!   it lands on or runs into loads it. What the four of them push the car up with over a
//!   step is a force already, in newtons: the car's mass times how hard it is pushed up
//!   (`Telemetry::g`, which is over the whole step, bump stops and all), shared among the
//!   wheels as they are loaded ([`wheel_forces`]). Several times what a wheel carries
//!   standing still is a blow to that wheel, and its impulse is what it pushed with above
//!   that, summed until the load has come back down. (Its peak is no measure: a tyre's load
//!   is held to so many times its own, so every landing from over a metre peaks alike.)
//!
//! A blow's hardness is told as a force. The physics resolves a collision in a step, which
//! says nothing of how long the real thing would take: steel folding takes about a tenth of
//! a second, so a blow is its impulse over [`CRUSH_SECONDS`], to the body or through a wheel.

use glam::{Quat, Vec3};
use solstice_physics::vehicle::VehicleSpec;
use solstice_physics::Knock;
use solstice_world::STEP_SECONDS;

/// How long a blow is taken to last, in seconds: its force is its impulse over this. A
/// car's nose folds for about a tenth of a second against a wall.
pub const CRUSH_SECONDS: f32 = 0.1;
/// The least a step's blow changes the car's velocity by, in metres a second, to be a blow
/// at all, once what merely holds the car up has been taken off it.
pub const LEAST_KNOCK: f32 = 0.12;
/// The most steps one blow is summed over: a car grinding along a wall is hit again and
/// again, not once for as long as it grinds.
pub const MOST_STEPS: u32 = 12;
/// How many times what it carries standing still a wheel must be loaded with to be struck,
/// and how far back down the load must come for the blow to be over.
pub const WHEEL_STRUCK: f32 = 3.0;
pub const WHEEL_OVER: f32 = 1.6;

/// One blow to a car.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Impact {
    /// Where on the car it landed, in the car's own space: from the ground under its middle,
    /// +X to its right, +Y up, -Z ahead.
    pub at: Vec3,
    /// The way it pushed the car, in the car's own space: one long.
    pub push: Vec3,
    /// The patch of the car it landed on, in the car's own space: the least and the
    /// greatest corners of a box round every place its body touched what it hit. For a
    /// wheel, or where the physics names no place, it is the one point [`Self::at`].
    pub low: Vec3,
    pub high: Vec3,
    /// Its impulse, in newton seconds: for a wheel, of the load above what it carries at rest.
    pub impulse: f32,
    /// How hard it was, in newtons: its impulse over [`CRUSH_SECONDS`].
    pub force: f32,
    /// The wheel it came up through, if it did: front left, front right, rear left, rear
    /// right.
    pub wheel: Option<usize>,
}

/// A blow to the body that is not over yet.
#[derive(Clone, Copy, Debug, Default)]
struct Gathering {
    /// The impulse so far, and the turning impulse about the centre of mass, in the car's
    /// own space.
    push: Vec3,
    turn: Vec3,
    steps: u32,
    /// The box round every place the body has touched what it hit, if it has any.
    patch: Option<(Vec3, Vec3)>,
}

/// A wheel that is being struck: the impulse so far, of what it pushes with above what it
/// carries at rest.
#[derive(Clone, Copy, Debug, Default)]
struct Struck {
    impulse: f32,
}

/// Gathers one car's blows, step by step.
#[derive(Clone, Debug, Default)]
pub struct Blows {
    body: Option<Gathering>,
    wheels: [Option<Struck>; 4],
}

/// Where a push that turned the car so landed on its body: on the line the push acts along,
/// where that line comes into the car's box. `push` and `turn` are the impulse and the
/// turning impulse about the centre of mass, in the car's own space.
pub fn landed(spec: &VehicleSpec, push: Vec3, turn: Vec3) -> Vec3 {
    let along = push.normalize_or_zero();
    // The nearest point of the line to the centre of mass: `r × J = L` gives `r` across `J`.
    let across = push.cross(turn) / push.length_squared().max(1.0e-9);
    let from = spec.centre + across;
    let (low, high) = (
        spec.body_at - spec.body_size * 0.5,
        spec.body_at + spec.body_size * 0.5,
    );
    // Where the line is inside the box, if it is: between the last face it comes in by and
    // the first it goes out by.
    let (mut enters, mut leaves) = (f32::MIN, f32::MAX);
    for axis in 0..3 {
        if along[axis].abs() < 1.0e-6 {
            if from[axis] < low[axis] || from[axis] > high[axis] {
                enters = f32::MAX;
            }
            continue;
        }
        let (a, b) = (
            (low[axis] - from[axis]) / along[axis],
            (high[axis] - from[axis]) / along[axis],
        );
        enters = enters.max(a.min(b));
        leaves = leaves.min(a.max(b));
    }
    if enters <= leaves {
        return from + along * enters;
    }
    // Several things met at once need not sum to a push through the box: then it is the
    // nearest of the box to the line, on the side the push came from.
    let reach = spec.body_size.length() * 0.5;
    (from - along * reach).clamp(low, high)
}

/// What each wheel pushes the car up with over a step, in newtons: the car's mass times how
/// hard its wheels push it up between them (`lifted`, in g, as its instruments have it),
/// shared among the wheels as each is loaded at the step's end.
pub fn wheel_forces(spec: &VehicleSpec, loads: [f32; 4], lifted: f32) -> [f32; 4] {
    let all: f32 = loads.iter().map(|load| load.max(0.0)).sum();
    if all <= 0.0 {
        return [0.0; 4];
    }
    let pushed = spec.mass * solstice_physics::GRAVITY * lifted.max(0.0);
    loads.map(|load| pushed * load.max(0.0) / all)
}

impl Blows {
    /// A step of the car's body: what hitting things did to it, how it is turned, its
    /// numbers, and where its body is touching things (asked for only in a step that is
    /// part of a blow). Gives a blow, once one is over.
    pub fn body(
        &mut self,
        knock: Knock,
        rotation: Quat,
        spec: &VehicleSpec,
        touches: impl FnOnce() -> Vec<Vec3>,
    ) -> Option<Impact> {
        // What holds a car up that lies on its belly, its side or its roof is no blow: it
        // undoes a step's gravity, and no more than that is taken off.
        let held = (solstice_physics::GRAVITY * STEP_SECONDS as f32).min(knock.velocity.y.max(0.0));
        let knocked = knock.velocity - Vec3::Y * held;
        let quiet = knocked.length() < LEAST_KNOCK;
        if !quiet {
            let to_car = rotation.inverse();
            let gathering = self.body.get_or_insert_with(Gathering::default);
            gathering.push += to_car * knocked * spec.mass;
            gathering.turn += to_car * knock.spin * spec.inertia;
            gathering.steps += 1;
            for touch in touches() {
                let (low, high) = gathering.patch.get_or_insert((touch, touch));
                (*low, *high) = (low.min(touch), high.max(touch));
            }
        }
        let over = self
            .body
            .is_some_and(|gathering| quiet || gathering.steps >= MOST_STEPS);
        let gathered = self.body.take_if(|_| over)?;
        let impulse = gathered.push.length();
        if impulse <= 0.0 {
            return None;
        }
        let at = landed(spec, gathered.push, gathered.turn);
        let (low, high) = gathered.patch.unwrap_or((at, at));
        Some(Impact {
            at,
            push: gathered.push / impulse,
            low,
            high,
            impulse,
            force: impulse / CRUSH_SECONDS,
            wheel: None,
        })
    }

    /// A step of one of the car's wheels: what it pushes the car up with, in newtons
    /// ([`wheel_forces`]). Gives a blow, once the wheel has been struck and the load has
    /// come back down.
    pub fn wheel(&mut self, index: usize, load: f32, spec: &VehicleSpec) -> Option<Impact> {
        let (front, rear) = spec.static_loads(solstice_physics::GRAVITY);
        let rest = if index < 2 { front } else { rear };
        let struck = &mut self.wheels[index];
        match struck {
            None if load >= rest * WHEEL_STRUCK => {
                *struck = Some(Struck {
                    impulse: (load - rest) * STEP_SECONDS as f32,
                });
                None
            }
            None => None,
            Some(blow) if load > rest * WHEEL_OVER => {
                blow.impulse += (load - rest) * STEP_SECONDS as f32;
                None
            }
            Some(blow) => {
                let blow = *blow;
                *struck = None;
                // Where its tyre meets the ground.
                let at = spec.hubs()[index] - Vec3::Y * spec.axle(index).radius;
                Some(Impact {
                    at,
                    push: Vec3::Y,
                    low: at,
                    high: at,
                    impulse: blow.impulse,
                    force: blow.impulse / CRUSH_SECONDS,
                    wheel: Some(index),
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TRUCK: &str = include_str!("../../solstice-vehicle/tests/wagon.vehicle");

    fn spec() -> VehicleSpec {
        VehicleSpec::parse(TRUCK).expect("the truck's file reads")
    }

    /// The physics names no place the body touched.
    fn none() -> Vec<Vec3> {
        Vec::new()
    }

    /// The knock a push at a place on the car gives it: the other way round from what is
    /// measured.
    fn knock_of(spec: &VehicleSpec, at: Vec3, push: Vec3) -> Knock {
        Knock {
            velocity: push / spec.mass,
            spin: (at - spec.centre).cross(push) / spec.inertia,
        }
    }

    #[test]
    fn a_blow_is_found_where_it_landed_and_is_as_hard_as_its_impulse() {
        let spec = spec();
        // Its nose's left corner, half a metre up, pushed straight back: two tonnes stopped
        // from three metres a second.
        let corner = Vec3::new(-0.7, 0.6, spec.body_at.z - spec.body_size.z * 0.5);
        let push = Vec3::Z * spec.mass * 3.0;
        let mut blows = Blows::default();
        // It is told when it is over: the step after.
        let turned = Quat::IDENTITY;
        assert!(blows
            .body(knock_of(&spec, corner, push), turned, &spec, none)
            .is_none());
        let blow = blows.body(Knock::default(), turned, &spec, none).unwrap();
        assert!(blow.at.distance(corner) < 0.02, "{}", blow.at);
        assert!(blow.push.distance(Vec3::Z) < 1.0e-4);
        assert!((blow.impulse - spec.mass * 3.0).abs() < 1.0);
        assert!((blow.force - spec.mass * 3.0 / CRUSH_SECONDS).abs() < 10.0);
        assert_eq!(blow.wheel, None);
        // With no place named, its patch is the point it landed at.
        assert_eq!((blow.low, blow.high), (blow.at, blow.at));
        // And nothing more is told of it.
        assert!(blows.body(Knock::default(), turned, &spec, none).is_none());
    }

    #[test]
    fn a_blow_over_several_steps_is_one_blow_however_the_car_is_turned() {
        let spec = spec();
        // The car faces +X in the world, and is hit in its right side, at the door: in the
        // world the push is along -Z... and it comes in three steps.
        let turned = Quat::from_rotation_y(-std::f32::consts::FRAC_PI_2);
        let door = Vec3::new(spec.body_size.x * 0.5, 1.0, 0.3);
        let push = Vec3::NEG_X * spec.mass * 2.0;
        let own = knock_of(&spec, door, push / 3.0);
        let world = Knock {
            velocity: turned * own.velocity,
            spin: turned * own.spin,
        };
        let mut blows = Blows::default();
        // Each step its body touches the thing in two places, a little further along.
        for step in 0..3 {
            let touches = move || {
                let along = 0.1 * step as f32;
                vec![door + Vec3::Z * along, door - Vec3::new(0.0, 0.4, along)]
            };
            assert!(blows.body(world, turned, &spec, touches).is_none());
        }
        let blow = blows.body(Knock::default(), turned, &spec, none).unwrap();
        // Its patch is the box round all of them.
        assert!(blow.low.distance(door - Vec3::new(0.0, 0.4, 0.2)) < 1.0e-5);
        assert!(blow.high.distance(door + Vec3::Z * 0.2) < 1.0e-5);
        assert!(blow.at.distance(door) < 0.02, "{}", blow.at);
        assert!(blow.push.distance(Vec3::NEG_X) < 1.0e-4);
        assert!((blow.impulse - spec.mass * 2.0).abs() < 1.0);
    }

    #[test]
    fn what_holds_a_car_up_is_not_a_blow_and_grinding_is_many() {
        let spec = spec();
        let mut blows = Blows::default();
        // Lying on its roof: the ground undoes a step's gravity, every step.
        let held = Knock {
            velocity: Vec3::Y * solstice_physics::GRAVITY * STEP_SECONDS as f32,
            spin: Vec3::ZERO,
        };
        for _ in 0..200 {
            assert!(blows.body(held, Quat::IDENTITY, &spec, none).is_none());
        }
        // Landing on it is: what is over and above the holding up.
        let landing = Knock {
            velocity: Vec3::Y * 4.0,
            spin: Vec3::ZERO,
        };
        assert!(blows.body(landing, Quat::IDENTITY, &spec, none).is_none());
        let blow = blows.body(held, Quat::IDENTITY, &spec, none).unwrap();
        let above = 4.0 - solstice_physics::GRAVITY * STEP_SECONDS as f32;
        assert!((blow.impulse - spec.mass * above).abs() < 1.0);
        // A push that goes on and on is told every so many steps.
        let grind = knock_of(&spec, Vec3::new(0.9, 1.0, 0.0), Vec3::NEG_X * 900.0);
        let mut told = 0;
        for _ in 0..MOST_STEPS * 3 {
            told += u32::from(blows.body(grind, Quat::IDENTITY, &spec, none).is_some());
        }
        assert_eq!(told, 3);
    }

    #[test]
    fn pushes_that_do_not_sum_to_one_through_the_car_land_on_its_near_side() {
        let spec = spec();
        // A push back with a turn no one push through the box could give it.
        let at = landed(&spec, Vec3::Z * 4000.0, Vec3::Y * 40_000.0);
        let (low, high) = (
            spec.body_at - spec.body_size * 0.5,
            spec.body_at + spec.body_size * 0.5,
        );
        assert!(at.cmpge(low).all() && at.cmple(high).all(), "{at}");
        assert!((at.z - low.z).abs() < 1.0e-4, "on its nose: {at}");
    }

    #[test]
    fn a_wheel_is_struck_by_several_times_its_load_and_as_hard_as_its_impulse() {
        let spec = spec();
        let (front, _) = spec.static_loads(solstice_physics::GRAVITY);
        let mut blows = Blows::default();
        // Driven over rough ground: twice its load is nothing.
        for _ in 0..30 {
            assert!(blows.wheel(1, front * 2.0, &spec).is_none());
        }
        // A landing: up to six times, and back down.
        for times in [3.5, 6.0, 4.0, 2.0] {
            assert!(blows.wheel(1, front * times, &spec).is_none());
        }
        let blow = blows.wheel(1, front, &spec).unwrap();
        assert_eq!(blow.wheel, Some(1));
        assert_eq!(blow.push, Vec3::Y);
        // Under the front right wheel.
        assert!(blow.at.x > 0.5 && blow.at.z < -1.0 && blow.at.y.abs() < 1.0e-4);
        let summed = front * (2.5 + 5.0 + 3.0 + 1.0) * STEP_SECONDS as f32;
        assert!((blow.impulse - summed).abs() < 1.0);
        assert!((blow.force - summed / CRUSH_SECONDS).abs() < 10.0);
        assert!(blows.wheel(1, front, &spec).is_none());
    }
}
