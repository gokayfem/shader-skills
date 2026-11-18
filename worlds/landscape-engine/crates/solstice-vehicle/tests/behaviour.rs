//! How the GT car behaves, driven headless on flat ground: what a driver would notice.

use glam::{Quat, Vec3};
use solstice_vehicle::{
    Asked, Body, Contact, Controls, Driver, Flat, Ground, Surface, Vehicle, VehicleSpec,
};

const G: f32 = 9.81;
const DT: f32 = 1.0 / 60.0;
const ROAD: Flat = Flat {
    height: 0.0,
    surface: Surface::Asphalt,
};

fn car() -> (Vehicle, Body) {
    (
        Vehicle::new(VehicleSpec::gt(), G),
        Body::at_rest(Vec3::ZERO, 0.0),
    )
}

/// A car already going, in a gear that suits the speed.
fn car_at(kmh: f32) -> (Vehicle, Body) {
    let (mut vehicle, mut body) = car();
    body.velocity = Vec3::NEG_Z * (kmh / 3.6);
    let gear = match kmh {
        k if k < 70.0 => 2,
        k if k < 110.0 => 3,
        k if k < 150.0 => 4,
        k if k < 200.0 => 5,
        _ => 6,
    };
    vehicle.set_gear(gear);
    vehicle.roll_with(&body);
    (vehicle, body)
}

fn run(
    vehicle: &mut Vehicle,
    body: &mut Body,
    controls: &Controls,
    ground: &dyn Ground,
    seconds: f32,
) {
    for _ in 0..(seconds / DT).round() as u32 {
        vehicle.step(body, controls, ground, DT);
    }
}

fn yaw_of(body: &Body) -> f32 {
    let forward = body.forward();
    (-forward.x).atan2(-forward.z)
}

/// The angle between where the car points and where it is going, in degrees.
fn drift(body: &Body) -> f32 {
    let going = body.velocity.normalize_or_zero();
    going
        .dot(body.right())
        .atan2(going.dot(body.forward()))
        .to_degrees()
}

#[test]
fn it_rests_where_it_was_drawn() {
    // The springs carry the car's weight with the wheels where the model has them, so the
    // body neither sinks nor rises, and nothing creeps.
    let (mut vehicle, mut body) = car();
    run(&mut vehicle, &mut body, &Controls::default(), &ROAD, 5.0);
    assert!(body.position.length() < 0.002, "moved to {}", body.position);
    assert!(body.velocity.length() < 0.002 && body.spin.length() < 0.002);
    let (front, rear) = vehicle.spec().static_loads(G);
    for (index, wheel) in vehicle.wheels.iter().enumerate() {
        let rest = if index < 2 { front } else { rear };
        assert!((wheel.load - rest).abs() < rest * 0.005, "wheel {index}");
        assert!(wheel.travel.abs() < 0.001 && wheel.on_ground);
        assert!(wheel.speed.abs() < 0.01);
    }
    let weight: f32 = vehicle.wheels.iter().map(|w| w.load).sum();
    assert!((weight - 1320.0 * G).abs() < 20.0);
    assert!(
        (vehicle.rpm() - 1000.0).abs() < 80.0,
        "idling at {}",
        vehicle.rpm()
    );
}

#[test]
fn dropped_it_lands_and_settles() {
    let (mut vehicle, mut body) = car();
    body.position.y = 0.6;
    body.rotation = Quat::from_rotation_z(0.08) * Quat::from_rotation_x(0.05);
    let mut highest_load = 0.0_f32;
    for _ in 0..(4.0 / DT) as u32 {
        vehicle.step(&mut body, &Controls::default(), &ROAD, DT);
        highest_load = highest_load.max(vehicle.wheels.iter().map(|w| w.load).fold(0.0, f32::max));
        assert!(body.position.is_finite() && body.position.y > -0.2);
    }
    assert!(highest_load > 6000.0, "it landed: {highest_load}");
    assert!(body.position.y.abs() < 0.003, "{}", body.position.y);
    assert!(body.up().y > 0.9999, "level again");
    assert!(body.velocity.length() < 0.02 && body.spin.length() < 0.02);
}

#[test]
fn its_figures_are_a_gt_cars() {
    // A GT3 car reaches 100 km/h in a little over 3 seconds and 200 in about 9, tops out
    // near 280 to 290, stops from 100 in under 30 metres, and corners at about 1.6 g where
    // downforce is slight, and 2 g where it is not.
    let (mut vehicle, mut body) = car();
    let mut driver = Driver::new(true);
    let asked = Asked {
        throttle: 1.0,
        ..Asked::default()
    };
    let (mut seconds, mut to_100, mut to_200) = (0.0, None, None);
    let mut launch_g = 0.0_f32;
    while seconds < 50.0 {
        let controls = driver.drive(&asked, &vehicle, &body, G, DT);
        vehicle.step(&mut body, &controls, &ROAD, DT);
        seconds += DT;
        let speed = body.velocity.length() * 3.6;
        if seconds < 3.0 {
            launch_g = launch_g.max(vehicle.telemetry().g.x);
        }
        if to_100.is_none() && speed >= 100.0 {
            to_100 = Some(seconds);
        }
        if to_200.is_none() && speed >= 200.0 {
            to_200 = Some(seconds);
        }
        assert!(
            vehicle.rpm() < vehicle.spec().engine.limit + 250.0,
            "over-revved"
        );
    }
    let (to_100, to_200) = (to_100.unwrap(), to_200.unwrap());
    assert!((2.9..3.8).contains(&to_100), "0-100 in {to_100}");
    assert!((8.0..10.5).contains(&to_200), "0-200 in {to_200}");
    // Traction, not power, is what limits a rear-driven car off the line.
    assert!((1.0..1.5).contains(&launch_g), "{launch_g} g off the line");
    let top = body.velocity.length() * 3.6;
    assert!((275.0..295.0).contains(&top), "top speed {top}");
    assert_eq!(vehicle.gear(), 6);
    assert!(body.position.x.abs() < 0.05, "it ran straight");

    let (mut vehicle, mut body) = car_at(100.0);
    let brake = Controls {
        brake: 1.0,
        ..Controls::default()
    };
    let start = body.position.z;
    while body.velocity.length() > 0.3 {
        vehicle.step(&mut body, &brake, &ROAD, DT);
    }
    let distance = start - body.position.z;
    assert!((21.0..30.0).contains(&distance), "100-0 in {distance} m");
    assert!(body.position.x.abs() < 0.05 && yaw_of(&body).abs() < 0.01);
}

/// The hardest the car corners at a speed, in g, winding lock on slowly: and how far its nose
/// points into the turn at that moment, in degrees.
fn cornering(kmh: f32) -> (f32, f32) {
    let (mut vehicle, mut body) = car_at(kmh);
    let (mut steer, mut throttle) = (0.0_f32, 0.3_f32);
    let mut best = (0.0_f32, 0.0_f32);
    while steer < 1.0 {
        steer += DT * 0.02;
        let going = body.velocity.length() * 3.6;
        throttle = (throttle + (kmh - going) * 0.02).clamp(0.0, 1.0);
        let controls = Controls {
            throttle,
            steer,
            ..Controls::default()
        };
        vehicle.step(&mut body, &controls, &ROAD, DT);
        if vehicle.telemetry().g.y > best.0 {
            best = (vehicle.telemetry().g.y, drift(&body));
        }
        assert!(drift(&body).abs() < 25.0, "it spun at {steer} of lock");
    }
    best
}

#[test]
fn it_corners_as_hard_as_slicks_and_wings_allow() {
    let (slow, _) = cornering(80.0);
    let (fast, _) = cornering(220.0);
    assert!((1.5..1.75).contains(&slow), "{slow} g at 80 km/h");
    assert!((1.85..2.25).contains(&fast), "{fast} g at 220 km/h");
}

#[test]
fn at_the_limit_the_front_lets_go_first() {
    // Wound past the limit the car runs wide rather than spinning: the fronts slip more than
    // the rears. A car that did the opposite would be undrivable.
    let (mut vehicle, mut body) = car_at(120.0);
    let mut throttle = 0.3_f32;
    for step in 0..(8.0 / DT) as u32 {
        let steer = (step as f32 * DT * 0.25).min(0.8);
        let going = body.velocity.length() * 3.6;
        throttle = (throttle + (120.0 - going) * 0.02).clamp(0.0, 1.0);
        let controls = Controls {
            throttle,
            steer,
            ..Controls::default()
        };
        vehicle.step(&mut body, &controls, &ROAD, DT);
    }
    let front = vehicle.wheels[0]
        .slide
        .abs()
        .max(vehicle.wheels[1].slide.abs());
    let rear = vehicle.wheels[2]
        .slide
        .abs()
        .max(vehicle.wheels[3].slide.abs());
    assert!(front > rear * 1.3, "front {front}, rear {rear}");
    assert!(
        drift(&body).abs() < 8.0,
        "pointing {} off its path",
        drift(&body)
    );
    assert!(vehicle.telemetry().g.y > 1.3, "still turning hard");
}

#[test]
fn it_runs_straight_and_settles_after_a_flick() {
    // Flat out in a straight line it stays on it; knocked off line by a jab at the wheel, it
    // comes back to running straight without weaving.
    let (mut vehicle, mut body) = car_at(250.0);
    let flat = Controls {
        throttle: 1.0,
        ..Controls::default()
    };
    run(&mut vehicle, &mut body, &flat, &ROAD, 5.0);
    assert!(body.position.x.abs() < 0.01 && yaw_of(&body).abs() < 1.0e-3);

    let jab = Controls {
        throttle: 0.6,
        steer: 0.1,
        ..Controls::default()
    };
    run(&mut vehicle, &mut body, &jab, &ROAD, 0.3);
    let steady = Controls {
        throttle: 0.6,
        ..Controls::default()
    };
    let mut swings = 0;
    let mut last = body.spin.y;
    for step in 0..(3.0 / DT) as u32 {
        vehicle.step(&mut body, &steady, &ROAD, DT);
        if body.spin.y * last < 0.0 && body.spin.y.abs() > 0.004 {
            swings += 1;
        }
        last = body.spin.y;
        if step as f32 * DT > 1.5 {
            assert!(
                body.spin.y.abs() < 0.01,
                "still turning at {}",
                step as f32 * DT
            );
        }
    }
    assert!(swings <= 2, "it weaved {swings} times");
    assert!(drift(&body).abs() < 0.2);
}

#[test]
fn lifting_off_in_a_corner_tucks_the_nose_in_but_doesnt_spin_it() {
    let (mut vehicle, mut body) = car_at(140.0);
    let mut throttle = 0.4_f32;
    for _ in 0..(4.0 / DT) as u32 {
        let going = body.velocity.length() * 3.6;
        throttle = (throttle + (140.0 - going) * 0.02).clamp(0.0, 1.0);
        let controls = Controls {
            throttle,
            steer: 0.11,
            ..Controls::default()
        };
        vehicle.step(&mut body, &controls, &ROAD, DT);
    }
    let cornering = vehicle.telemetry().g.y;
    assert!(cornering > 1.0, "cornering at {cornering} g");
    let before = body.spin.y.abs();
    let lifted = Controls {
        steer: 0.11,
        ..Controls::default()
    };
    let mut most = 0.0_f32;
    for _ in 0..(2.0 / DT) as u32 {
        vehicle.step(&mut body, &lifted, &ROAD, DT);
        most = most.max(drift(&body).abs());
    }
    assert!(most < 9.0, "it slid to {most} degrees");
    assert!(body.spin.y.abs() < before * 1.6, "no snap");
}

#[test]
fn flooring_it_out_of_a_slow_corner_is_caught_by_traction_control_and_not_without() {
    let exit = |traction: bool, stability: bool| {
        let mut spec = VehicleSpec::gt();
        spec.traction_control = traction;
        spec.stability = stability;
        let mut vehicle = Vehicle::new(spec, G);
        let mut body = Body::at_rest(Vec3::ZERO, 0.0);
        body.velocity = Vec3::NEG_Z * (60.0 / 3.6);
        vehicle.set_gear(2);
        vehicle.roll_with(&body);
        let turning = Controls {
            throttle: 0.25,
            steer: 0.45,
            ..Controls::default()
        };
        run(&mut vehicle, &mut body, &turning, &ROAD, 1.5);
        let floored = Controls {
            throttle: 1.0,
            steer: 0.45,
            ..Controls::default()
        };
        let mut most = 0.0_f32;
        for _ in 0..(2.0 / DT) as u32 {
            vehicle.step(&mut body, &floored, &ROAD, DT);
            most = most.max(drift(&body).abs());
        }
        most
    };
    // Traction control by itself, with nothing else to catch the tail.
    let (with, without) = (exit(true, false), exit(false, false));
    assert!(
        with < 12.0,
        "with traction control it slid to {with} degrees"
    );
    assert!(without > with * 1.5, "without, {without}: power oversteer");
    // Stability control catches the tail even with traction control off, and with both the
    // car hardly steps out at all.
    let (caught, both) = (exit(false, true), exit(true, true));
    assert!(
        caught < without * 0.6,
        "stability control alone: {caught} against {without}"
    );
    assert!(both < 8.0, "with both it slid to {both} degrees");
}

#[test]
fn the_brakes_lock_without_anti_lock_and_stop_shorter_with_it() {
    let stop = |abs: bool| {
        let mut spec = VehicleSpec::gt();
        spec.abs = abs;
        let mut vehicle = Vehicle::new(spec, G);
        let mut body = Body::at_rest(Vec3::ZERO, 0.0);
        body.velocity = Vec3::NEG_Z * (150.0 / 3.6);
        vehicle.set_gear(4);
        vehicle.roll_with(&body);
        let brake = Controls {
            brake: 1.0,
            ..Controls::default()
        };
        let mut locked = false;
        while body.velocity.length() > 0.5 {
            vehicle.step(&mut body, &brake, &ROAD, DT);
            locked |= body.velocity.length() > 10.0 && vehicle.wheels[0].speed.abs() < 1.0;
        }
        (-body.position.z, locked)
    };
    let (with, with_locked) = stop(true);
    let (without, without_locked) = stop(false);
    assert!(!with_locked && without_locked);
    assert!(with < without * 0.9, "{with} m with, {without} m without");
}

#[test]
fn grass_and_gravel_are_slippery_and_slow() {
    let stop_on = |surface: Surface| {
        let ground = Flat {
            height: 0.0,
            surface,
        };
        let (mut vehicle, mut body) = car_at(100.0);
        let brake = Controls {
            brake: 1.0,
            ..Controls::default()
        };
        while body.velocity.length() > 0.5 {
            vehicle.step(&mut body, &brake, &ground, DT);
        }
        -body.position.z
    };
    let (road, grass) = (stop_on(Surface::Asphalt), stop_on(Surface::Grass));
    assert!(
        grass > road * 1.8,
        "{grass} m on grass, {road} m on the road"
    );

    // Coasting, gravel stops a car in a few lengths; the road hardly slows it.
    let coast_on = |surface: Surface| {
        let ground = Flat {
            height: 0.0,
            surface,
        };
        let (mut vehicle, mut body) = car_at(100.0);
        vehicle.set_gear(0);
        run(&mut vehicle, &mut body, &Controls::default(), &ground, 2.0);
        body.velocity.length() * 3.6
    };
    assert!(coast_on(Surface::Asphalt) > 94.0);
    assert!(coast_on(Surface::Gravel) < 70.0);

    // But a car stopped in the gravel can drive out of it, slowly.
    let gravel = Flat {
        height: 0.0,
        surface: Surface::Gravel,
    };
    let (mut vehicle, mut body) = car();
    let mut driver = Driver::new(true);
    let asked = Asked {
        throttle: 1.0,
        ..Asked::default()
    };
    for _ in 0..(5.0 / DT) as u32 {
        let controls = driver.drive(&asked, &vehicle, &body, G, DT);
        vehicle.step(&mut body, &controls, &gravel, DT);
    }
    let speed = body.velocity.length();
    assert!(
        (3.0..14.0).contains(&speed),
        "{speed} m/s after five seconds in gravel"
    );
}

#[test]
fn a_run_repeats_exactly() {
    let drive = || {
        let (mut vehicle, mut body) = car();
        let mut driver = Driver::new(true);
        for step in 0..1500 {
            let asked = Asked {
                throttle: if step < 900 { 1.0 } else { 0.0 },
                brake: if step > 1100 { 0.8 } else { 0.0 },
                steer: ((step as f32) * 0.01).sin() * 0.6,
                ..Asked::default()
            };
            let controls = driver.drive(&asked, &vehicle, &body, G, DT);
            vehicle.step(&mut body, &controls, &ROAD, DT);
        }
        (body, vehicle.wheels, vehicle.rpm())
    };
    assert_eq!(drive(), drive());
}

#[test]
fn held_on_the_brake_the_automatic_finds_reverse_and_then_first_again() {
    let (mut vehicle, mut body) = car();
    let mut driver = Driver::new(true);
    let mut go = |asked: Asked, seconds: f32, vehicle: &mut Vehicle, body: &mut Body| {
        for _ in 0..(seconds / DT) as u32 {
            let controls = driver.drive(&asked, vehicle, body, G, DT);
            vehicle.step(body, &controls, &ROAD, DT);
        }
    };
    let backwards = Asked {
        brake: 1.0,
        ..Asked::default()
    };
    go(backwards, 4.0, &mut vehicle, &mut body);
    assert_eq!(vehicle.gear(), -1);
    let speed = body.velocity.dot(body.forward());
    assert!(speed < -3.0, "reversing at {speed}");
    assert!(body.position.z > 2.0);
    let forwards = Asked {
        throttle: 1.0,
        ..Asked::default()
    };
    go(forwards, 4.0, &mut vehicle, &mut body);
    assert!(vehicle.gear() >= 1);
    assert!(body.velocity.dot(body.forward()) > 10.0);
}

#[test]
fn a_change_down_that_would_over_rev_is_refused() {
    let (mut vehicle, mut body) = car_at(240.0);
    assert_eq!(vehicle.gear(), 6);
    let down = Controls {
        shift_down: true,
        ..Controls::default()
    };
    // Fifth is fine at this speed; fourth is not.
    vehicle.step(&mut body, &down, &ROAD, DT);
    assert_eq!(vehicle.gear(), 5);
    run(&mut vehicle, &mut body, &Controls::default(), &ROAD, 0.3);
    vehicle.step(&mut body, &down, &ROAD, DT);
    assert_eq!(
        vehicle.gear(),
        5,
        "fourth would over-rev from {} rpm",
        vehicle.rpm()
    );
    assert!(vehicle.rpm() < vehicle.spec().engine.limit + 300.0);
    // Slowed, it is.
    let brake = Controls {
        brake: 0.6,
        ..Controls::default()
    };
    run(&mut vehicle, &mut body, &brake, &ROAD, 1.0);
    vehicle.step(&mut body, &down, &ROAD, DT);
    assert_eq!(vehicle.gear(), 4);
}

/// A road with a kerb along one side: a ramp up to a flat top, as kerbs are.
struct Kerbed;

impl Ground for Kerbed {
    fn cast(&self, from: Vec3, direction: Vec3, reach: f32) -> Option<Contact> {
        // Kerb between x = 0.6 and x = 1.6, rising 5 cm over its first 40 cm.
        let height = |x: f32| ((x - 0.6) / 0.4).clamp(0.0, 1.0) * 0.05;
        // March to the surface: the ray is nearly straight down.
        let mut distance = (0.0 - from.y) / direction.y;
        for _ in 0..4 {
            let at = from + direction * distance;
            distance = (height(at.x) - from.y) / direction.y;
        }
        let at = from + direction * distance;
        let on_kerb = at.x > 0.6 && at.x < 1.6;
        let slope = if at.x > 0.6 && at.x < 1.0 { 0.125 } else { 0.0 };
        (0.0..=reach).contains(&distance).then_some(Contact {
            distance,
            normal: Vec3::new(-slope, 1.0, 0.0).normalize(),
            surface: if on_kerb {
                Surface::Kerb
            } else {
                Surface::Asphalt
            },
        })
    }
}

#[test]
fn it_rides_a_kerb_without_being_thrown() {
    // Drifting across onto a kerb at speed with the right-hand wheels.
    let (mut vehicle, mut body) = car_at(160.0);
    body.velocity += Vec3::X * 1.2;
    let steady = Controls {
        throttle: 0.4,
        ..Controls::default()
    };
    let mut kerbed = false;
    let mut most_roll = 0.0_f32;
    for _ in 0..(1.2 / DT) as u32 {
        vehicle.step(&mut body, &steady, &Kerbed, DT);
        kerbed |= vehicle.wheels[1].surface == Surface::Kerb;
        most_roll = most_roll.max(body.right().y.abs());
        assert!(body.position.y < 0.12, "thrown to {}", body.position.y);
    }
    assert!(kerbed);
    assert!(most_roll > 0.003 && most_roll < 0.06, "rolled {most_roll}");
    assert!(body.velocity.length() * 3.6 > 150.0);
}
