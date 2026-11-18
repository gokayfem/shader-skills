//! A rear-engined GT car, driven: that its figures are those of its class, and that a car with
//! its engine behind the rear axle is still one a driver can trust.

use glam::Vec3;
use solstice_vehicle::{Asked, Body, Controls, Driver, Flat, Surface, Vehicle, VehicleSpec};

const G: f32 = 9.81;
const DT: f32 = 1.0 / 60.0;
const ROAD: Flat = Flat {
    height: 0.0,
    surface: Surface::Asphalt,
};

fn spec() -> VehicleSpec {
    VehicleSpec::parse(include_str!("gt.vehicle")).unwrap()
}

fn car_at(kmh: f32, gear: i32) -> (Vehicle, Body) {
    let mut vehicle = Vehicle::new(spec(), G);
    let mut body = Body::at_rest(Vec3::ZERO, 0.0);
    body.velocity = Vec3::NEG_Z * (kmh / 3.6);
    vehicle.set_gear(gear);
    vehicle.roll_with(&body);
    (vehicle, body)
}

fn drift(body: &Body) -> f32 {
    let going = body.velocity.normalize_or_zero();
    going
        .dot(body.right())
        .atan2(going.dot(body.forward()))
        .to_degrees()
}

#[test]
fn the_file_is_the_car_it_says() {
    let spec = spec();
    assert!((spec.wheelbase() - 2.5).abs() < 1.0e-4);
    let (power, rpm) = spec.engine.peak_power();
    assert!((390.0e3..410.0e3).contains(&power), "{} kW", power / 1000.0);
    assert!((8000.0..8800.0).contains(&rpm));
    assert_eq!(spec.engine.limit, 9000.0);
    assert_eq!(spec.gearbox.gears.len(), 6);
    let rear = 1.0 - spec.front_weight();
    assert!(
        (0.57..0.61).contains(&rear),
        "{rear} of its weight on the rear"
    );
    // As wide over its tyres as its class is at each axle.
    assert!((spec.front.track + 0.30 - 2.0).abs() < 0.05);
    assert!((spec.rear.track + 0.31 - 2.03).abs() < 0.05);
}

#[test]
fn its_figures_are_its_classs() {
    let mut vehicle = Vehicle::new(spec(), G);
    let mut body = Body::at_rest(Vec3::ZERO, 0.0);
    let mut driver = Driver::new(true);
    let asked = Asked {
        throttle: 1.0,
        ..Asked::default()
    };
    let (mut seconds, mut to_100, mut to_200) = (0.0, None, None);
    while seconds < 50.0 {
        let controls = driver.drive(&asked, &vehicle, &body, G, DT);
        vehicle.step(&mut body, &controls, &ROAD, DT);
        seconds += DT;
        let speed = body.velocity.length() * 3.6;
        if to_100.is_none() && speed >= 100.0 {
            to_100 = Some(seconds);
        }
        if to_200.is_none() && speed >= 200.0 {
            to_200 = Some(seconds);
        }
        assert!(vehicle.rpm() < 9500.0, "over-revved");
    }
    let (to_100, to_200, top) = (
        to_100.unwrap(),
        to_200.unwrap(),
        body.velocity.length() * 3.6,
    );
    println!("0-100 {to_100:.2} s, 0-200 {to_200:.2} s, top {top:.0} km/h");
    // With its weight over the driven wheels a rear-engined car leaves the line harder than most.
    assert!((2.6..3.4).contains(&to_100), "0-100 in {to_100}");
    assert!((7.5..10.0).contains(&to_200), "0-200 in {to_200}");
    assert!((272.0..296.0).contains(&top), "top speed {top}");
    assert!(body.position.x.abs() < 0.05);

    let (mut vehicle, mut body) = car_at(200.0, 5);
    let brake = Controls {
        brake: 1.0,
        ..Controls::default()
    };
    let start = body.position.z;
    while body.velocity.length() > 0.3 {
        vehicle.step(&mut body, &brake, &ROAD, DT);
    }
    let distance = start - body.position.z;
    println!("200-0 in {distance:.1} m");
    assert!((70.0..95.0).contains(&distance), "200-0 in {distance} m");
    assert!(body.position.x.abs() < 0.1, "it stopped straight");
}

#[test]
fn it_corners_hard_and_runs_wide_at_the_limit_rather_than_spinning() {
    for (kmh, gear, least) in [(90.0, 3, 1.5), (200.0, 5, 1.85)] {
        let (mut vehicle, mut body) = car_at(kmh, gear);
        let (mut steer, mut throttle, mut best) = (0.0_f32, 0.3_f32, 0.0_f32);
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
            best = best.max(vehicle.telemetry().g.y);
            assert!(
                drift(&body).abs() < 20.0,
                "it spun at {kmh} km/h, {steer} of lock"
            );
        }
        println!("{kmh} km/h: {best:.2} g");
        assert!(best > least && best < least + 0.5, "{best} g at {kmh} km/h");
        let front = vehicle.wheels[0]
            .slide
            .abs()
            .max(vehicle.wheels[1].slide.abs());
        let rear = vehicle.wheels[2]
            .slide
            .abs()
            .max(vehicle.wheels[3].slide.abs());
        assert!(front > rear * 1.2, "front {front}, rear {rear}");
    }
}

#[test]
fn lifting_off_in_a_corner_turns_it_in_without_swapping_ends() {
    // What a rear-engined car is known for: shut the throttle in a corner and the tail comes round. It
    // should tighten its line, and no more than a driver can hold.
    let (mut vehicle, mut body) = car_at(140.0, 4);
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
    let (cornering, before) = (vehicle.telemetry().g.y, body.spin.y.abs());
    assert!(cornering > 1.0, "cornering at {cornering} g");
    let lifted = Controls {
        steer: 0.11,
        ..Controls::default()
    };
    let (mut most, mut fastest) = (0.0_f32, 0.0_f32);
    for _ in 0..(2.5 / DT) as u32 {
        vehicle.step(&mut body, &lifted, &ROAD, DT);
        most = most.max(drift(&body).abs());
        fastest = fastest.max(body.spin.y.abs());
    }
    println!("lift-off: slid to {most:.1} degrees, yaw {before:.2} to {fastest:.2}");
    assert!(most < 10.0, "it slid to {most} degrees");
    assert!(fastest < before * 1.7, "it snapped round");
}

#[test]
fn flat_out_it_runs_straight_and_settles_after_a_jab_at_the_wheel() {
    let (mut vehicle, mut body) = car_at(260.0, 6);
    let jab = Controls {
        throttle: 0.7,
        steer: 0.1,
        ..Controls::default()
    };
    for _ in 0..18 {
        vehicle.step(&mut body, &jab, &ROAD, DT);
    }
    let steady = Controls {
        throttle: 0.7,
        ..Controls::default()
    };
    let (mut swings, mut last) = (0, body.spin.y);
    for step in 0..(3.0 / DT) as u32 {
        vehicle.step(&mut body, &steady, &ROAD, DT);
        if body.spin.y * last < 0.0 && body.spin.y.abs() > 0.004 {
            swings += 1;
        }
        last = body.spin.y;
        if step as f32 * DT > 1.8 {
            assert!(body.spin.y.abs() < 0.01, "still turning");
        }
    }
    assert!(swings <= 2, "it weaved {swings} times");
}
