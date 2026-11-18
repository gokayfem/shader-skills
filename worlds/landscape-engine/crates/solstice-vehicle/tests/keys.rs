//! A rear-engined GT car driven as a player with keys drives it: the wheel hard over or straight, a
//! foot flat down or off. That is the roughest driving there is, and the car has to stay one
//! that can be caught.

use glam::Vec3;
use solstice_vehicle::{Asked, Body, Contact, Driver, Flat, Ground, Surface, Vehicle, VehicleSpec};

const G: f32 = 9.81;
const DT: f32 = 1.0 / 60.0;
const ROAD: Flat = Flat {
    height: 0.0,
    surface: Surface::Asphalt,
};

fn spec() -> VehicleSpec {
    VehicleSpec::parse(include_str!("gt.vehicle")).unwrap()
}

/// The gear the automatic gearbox would be in at a speed.
fn gear_for(spec: &VehicleSpec, kmh: f32) -> i32 {
    let speed = kmh / 3.6;
    let mut gear = 1;
    for (index, ratio) in spec.gearbox.gears.iter().enumerate() {
        let rpm = speed / spec.rear.radius * ratio * spec.gearbox.final_drive * 60.0
            / std::f32::consts::TAU;
        if rpm > 4500.0 {
            gear = index as i32 + 1;
        }
    }
    gear
}

fn car_at(kmh: f32) -> (Vehicle, Body, Driver) {
    let spec = spec();
    let gear = gear_for(&spec, kmh);
    let mut vehicle = Vehicle::new(spec, G);
    let mut body = Body::at_rest(Vec3::ZERO, 0.0);
    body.velocity = Vec3::NEG_Z * (kmh / 3.6);
    vehicle.set_gear(gear);
    vehicle.roll_with(&body);
    (vehicle, body, Driver::new(true))
}

/// How far the car's nose points from the way it is going, in degrees.
fn drift(body: &Body) -> f32 {
    let going = body.velocity.normalize_or_zero();
    if body.velocity.length() < 3.0 {
        return 0.0;
    }
    going
        .dot(body.right())
        .atan2(going.dot(body.forward()))
        .to_degrees()
}

/// What came of a piece of driving.
#[derive(Debug, Default)]
struct Outcome {
    /// The most the car's nose pointed away from where it was going, in degrees.
    slid: f32,
    /// The hardest it cornered, in g.
    cornered: f32,
    /// How far it turned in all, in degrees.
    turned: f32,
    /// Its speed at the end, in km/h.
    left: f32,
}

/// Drives for a while with what `asked` gives for each moment, in seconds from the start.
fn drive(kmh: f32, seconds: f32, asked: impl Fn(f32) -> Asked) -> Outcome {
    drive_on(&ROAD, 0.0, kmh, seconds, asked)
}

/// A road running north, with something else beside it: asphalt east of `edge`, and `beside`
/// west of it.
struct Edge {
    edge: f32,
    beside: Surface,
}

impl Ground for Edge {
    fn cast(&self, from: Vec3, direction: Vec3, reach: f32) -> Option<Contact> {
        let mut contact = ROAD.cast(from, direction, reach)?;
        if (from + direction * contact.distance).x < self.edge {
            contact.surface = self.beside;
        }
        Some(contact)
    }
}

/// The same on any ground, starting `east` metres to the east.
fn drive_on(
    ground: &dyn Ground,
    east: f32,
    kmh: f32,
    seconds: f32,
    asked: impl Fn(f32) -> Asked,
) -> Outcome {
    let (mut vehicle, mut body, mut driver) = car_at(kmh);
    body.position.x = east;
    let mut outcome = Outcome::default();
    // Settled at speed first, on a steady throttle.
    for _ in 0..30 {
        let hold = Asked {
            throttle: 0.3,
            ..Asked::default()
        };
        let controls = driver.drive(&hold, &vehicle, &body, G, DT);
        vehicle.step(&mut body, &controls, ground, DT);
    }
    let mut time = 0.0;
    while time < seconds {
        let controls = driver.drive(&asked(time), &vehicle, &body, G, DT);
        vehicle.step(&mut body, &controls, ground, DT);
        time += DT;
        // Once it has all but stopped there is no more to learn: held on the brake, the
        // automatic gearbox would find reverse and back away.
        if body.velocity.dot(body.forward()) < 4.0 {
            break;
        }
        outcome.slid = outcome.slid.max(drift(&body).abs());
        outcome.cornered = outcome.cornered.max(vehicle.telemetry().g.y.abs());
        outcome.turned += body.spin.y.abs().to_degrees() * DT;
    }
    outcome.left = body.velocity.length() * 3.6;
    outcome
}

const SPEEDS: [f32; 6] = [50.0, 80.0, 120.0, 160.0, 200.0, 250.0];

fn show(what: &str, kmh: f32, outcome: &Outcome) {
    println!(
        "{what:34} {kmh:3.0} km/h: slid {:5.1} deg, {:.2} g, turned {:4.0} deg, left at {:3.0} km/h",
        outcome.slid, outcome.cornered, outcome.turned, outcome.left
    );
}

#[test]
fn the_wheel_hard_over_with_a_foot_flat_down() {
    let mut worst = 0.0_f32;
    for kmh in SPEEDS {
        let outcome = drive(kmh, 4.0, |_| Asked {
            throttle: 1.0,
            steer: 1.0,
            ..Asked::default()
        });
        show("hard over, flat out", kmh, &outcome);
        worst = worst.max(outcome.slid);
    }
    assert!(worst < 12.0, "it slid to {worst} degrees");
}

#[test]
fn the_wheel_hard_over_with_both_feet_off() {
    let mut worst = 0.0_f32;
    for kmh in SPEEDS {
        let outcome = drive(kmh, 4.0, |_| Asked {
            steer: 1.0,
            ..Asked::default()
        });
        show("hard over, coasting", kmh, &outcome);
        worst = worst.max(outcome.slid);
    }
    assert!(worst < 12.0, "it slid to {worst} degrees");
}

#[test]
fn the_wheel_hard_over_on_the_brakes() {
    let mut worst = 0.0_f32;
    for kmh in SPEEDS {
        let outcome = drive(kmh, 4.0, |_| Asked {
            brake: 1.0,
            steer: 1.0,
            ..Asked::default()
        });
        show("hard over, braking", kmh, &outcome);
        worst = worst.max(outcome.slid);
    }
    assert!(worst < 12.0, "it slid to {worst} degrees");
}

#[test]
fn lifting_off_in_the_middle_of_a_corner_taken_flat_out() {
    let mut worst = 0.0_f32;
    for kmh in SPEEDS {
        let outcome = drive(kmh, 5.0, |time| Asked {
            throttle: if time < 2.0 { 1.0 } else { 0.0 },
            steer: 1.0,
            ..Asked::default()
        });
        show("hard over, then lifting", kmh, &outcome);
        worst = worst.max(outcome.slid);
    }
    assert!(worst < 12.0, "it slid to {worst} degrees");
}

#[test]
fn flicked_one_way_and_then_the_other() {
    let mut worst = 0.0_f32;
    for kmh in SPEEDS {
        let outcome = drive(kmh, 5.0, |time| Asked {
            throttle: 1.0,
            steer: if time < 1.0 {
                1.0
            } else if time < 2.2 {
                -1.0
            } else if time < 3.2 {
                1.0
            } else {
                0.0
            },
            ..Asked::default()
        });
        show("left, right, left, flat out", kmh, &outcome);
        worst = worst.max(outcome.slid);
    }
    assert!(worst < 14.0, "it slid to {worst} degrees");
}

#[test]
fn flat_out_from_a_slow_corner() {
    let mut worst = 0.0_f32;
    for kmh in [30.0, 45.0, 60.0, 80.0] {
        let outcome = drive(kmh, 5.0, |time| Asked {
            throttle: if time < 1.5 { 0.2 } else { 1.0 },
            steer: if time < 3.0 { 1.0 } else { 0.0 },
            ..Asked::default()
        });
        show("power on out of a slow corner", kmh, &outcome);
        worst = worst.max(outcome.slid);
    }
    assert!(worst < 12.0, "it slid to {worst} degrees");
}

#[test]
fn braking_hard_and_then_turning_in() {
    let mut worst = 0.0_f32;
    for kmh in [120.0, 160.0, 200.0, 250.0] {
        let outcome = drive(kmh, 5.0, |time| Asked {
            brake: if time < 1.6 { 1.0 } else { 0.0 },
            throttle: if time > 2.6 { 1.0 } else { 0.0 },
            steer: if time > 0.9 { 1.0 } else { 0.0 },
            ..Asked::default()
        });
        show("brake, turn in, power", kmh, &outcome);
        worst = worst.max(outcome.slid);
    }
    assert!(worst < 12.0, "it slid to {worst} degrees");
}

/// With its left wheels off the road's edge: a road whose edge is half a metre inside them.
fn two_wheels_off(beside: Surface) -> (Edge, f32) {
    let edge = Edge { edge: 0.0, beside };
    (edge, spec().front.track * 0.5 - 0.5)
}

#[test]
fn two_wheels_on_the_grass_with_a_foot_flat_down_and_then_steering_back() {
    let mut worst = 0.0_f32;
    for kmh in [80.0, 120.0, 160.0, 200.0] {
        let (ground, east) = two_wheels_off(Surface::Grass);
        let outcome = drive_on(&ground, east, kmh, 5.0, |time| Asked {
            throttle: 1.0,
            steer: if (2.0..3.0).contains(&time) { 0.5 } else { 0.0 },
            ..Asked::default()
        });
        show("two wheels on grass, flat out", kmh, &outcome);
        worst = worst.max(outcome.slid);
    }
    assert!(worst < 14.0, "it slid to {worst} degrees");
}

#[test]
fn braking_hard_with_two_wheels_on_the_grass() {
    let mut worst = 0.0_f32;
    for kmh in [120.0, 160.0, 200.0, 250.0] {
        let (ground, east) = two_wheels_off(Surface::Grass);
        let outcome = drive_on(&ground, east, kmh, 6.0, |_| Asked {
            brake: 1.0,
            ..Asked::default()
        });
        show("two wheels on grass, braking", kmh, &outcome);
        worst = worst.max(outcome.slid);
    }
    assert!(worst < 14.0, "it slid to {worst} degrees");
}

#[test]
fn cornering_hard_over_a_kerb() {
    let mut worst = 0.0_f32;
    for kmh in [60.0, 100.0, 140.0, 180.0] {
        // Turning left, towards the kerb, with the power on: the inside wheels ride it.
        let ground = Edge {
            edge: -1.2,
            beside: Surface::Kerb,
        };
        let outcome = drive_on(&ground, 0.0, kmh, 3.0, |time| Asked {
            throttle: 1.0,
            steer: if time < 1.2 { -1.0 } else { 0.0 },
            ..Asked::default()
        });
        show("over a kerb, flat out", kmh, &outcome);
        worst = worst.max(outcome.slid);
    }
    assert!(worst < 14.0, "it slid to {worst} degrees");
}

#[test]
fn running_wide_onto_the_grass_in_a_corner_and_coming_back() {
    let mut worst = 0.0_f32;
    for kmh in [80.0, 120.0, 160.0] {
        // Turning right, off the left of a road that ends two metres to the car's left,
        // then back on to it.
        let ground = Edge {
            edge: -2.0,
            beside: Surface::Grass,
        };
        let outcome = drive_on(&ground, 0.0, kmh, 6.0, |time| Asked {
            throttle: if (1.0..=3.0).contains(&time) {
                0.0
            } else {
                1.0
            },
            steer: if time < 1.0 {
                -0.6
            } else if time < 4.0 {
                1.0
            } else {
                0.0
            },
            ..Asked::default()
        });
        show("onto the grass and back", kmh, &outcome);
        worst = worst.max(outcome.slid);
    }
    assert!(worst < 20.0, "it slid to {worst} degrees");
}
