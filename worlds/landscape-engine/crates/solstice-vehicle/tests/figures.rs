//! The GT car's figures, measured by driving it: printed, for tuning it against real cars.
//!
//!     cargo test -p solstice-vehicle --release --test figures -- --nocapture

use glam::Vec3;
use solstice_vehicle::{Asked, Body, Controls, Driver, Flat, Surface, Vehicle, VehicleSpec};

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

fn kmh(body: &Body) -> f32 {
    body.velocity.length() * 3.6
}

#[test]
fn print_the_figures() {
    // Standing start, flat out, with the automatic gearbox.
    let (mut vehicle, mut body) = car();
    let mut driver = Driver::new(true);
    let asked = Asked {
        throttle: 1.0,
        ..Asked::default()
    };
    let mut seconds = 0.0;
    let mut marks = [100.0, 160.0, 200.0, 250.0].into_iter().peekable();
    let mut quarter = None;
    while seconds < 60.0 {
        let controls = driver.drive(&asked, &vehicle, &body, G, DT);
        vehicle.step(&mut body, &controls, &ROAD, DT);
        seconds += DT;
        if marks.peek().is_some_and(|mark| kmh(&body) >= *mark) {
            println!(
                "0-{} km/h: {seconds:.2} s, gear {}",
                marks.next().unwrap(),
                vehicle.gear()
            );
        }
        if quarter.is_none() && -body.position.z >= 402.3 {
            quarter = Some(seconds);
            println!("quarter mile: {seconds:.2} s at {:.0} km/h", kmh(&body));
        }
    }
    println!(
        "top speed: {:.1} km/h in gear {} at {:.0} rpm, {:.2} m off line",
        kmh(&body),
        vehicle.gear(),
        vehicle.rpm(),
        body.position.x
    );

    // Braking from that, pedal hard down.
    for from in [300.0_f32, 200.0, 100.0] {
        let (mut vehicle, mut body) = car();
        vehicle.set_gear(4);
        body.velocity = Vec3::NEG_Z * (from / 3.6);
        vehicle.roll_with(&body);
        let brake = Controls {
            brake: 1.0,
            ..Controls::default()
        };
        let start = body.position.z;
        let (mut seconds, mut peak) = (0.0, 0.0_f32);
        while body.velocity.length() > 0.3 && seconds < 20.0 {
            vehicle.step(&mut body, &brake, &ROAD, DT);
            seconds += DT;
            peak = peak.max(-vehicle.telemetry().g.x);
        }
        println!(
            "{from}-0 km/h: {:.1} m in {seconds:.2} s, peak {peak:.2} g, {:.2} m off line",
            start - body.position.z,
            body.position.x
        );
    }

    // Cornering: hold a speed, wind on lock slowly, and see how hard it turns at most.
    for speed in [80.0_f32, 150.0, 220.0] {
        let (mut vehicle, mut body) = car();
        let gear = if speed < 100.0 {
            3
        } else if speed < 180.0 {
            4
        } else {
            5
        };
        vehicle.set_gear(gear);
        body.velocity = Vec3::NEG_Z * (speed / 3.6);
        vehicle.roll_with(&body);
        let (mut steer, mut throttle) = (0.0_f32, 0.3_f32);
        let mut best = (0.0_f32, 0.0_f32);
        let mut seconds = 0.0;
        while seconds < 30.0 && steer < 1.0 {
            steer += DT * 0.02;
            let going = body.velocity.length() * 3.6;
            throttle = (throttle + (speed - going) * 0.02).clamp(0.0, 1.0);
            let controls = Controls {
                throttle,
                steer,
                ..Controls::default()
            };
            vehicle.step(&mut body, &controls, &ROAD, DT);
            seconds += DT;
            let sideways = vehicle.telemetry().g.y;
            if sideways > best.0 {
                best = (sideways, steer);
            }
        }
        let lock = vehicle.spec().steering_lock.to_degrees();
        println!(
            "cornering at {speed} km/h: {:.2} g at {:.1} degrees of lock",
            best.0,
            best.1 * lock
        );
    }
}
