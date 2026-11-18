//! A four-wheel-drive desert truck, driven off the road: that its figures are those of the truck
//! it says it is, that its long suspension carries it, and that mud, bog and a wet clay hill do
//! to it what they do to a real one, each as its transfer case and lockers are set.

use glam::Vec3;
use solstice_vehicle::{
    Asked, Body, Contact, DriveMode, Driver, Flat, Ground, Surface, Vehicle, VehicleSpec,
};

const G: f32 = 9.81;
const DT: f32 = 1.0 / 60.0;

fn spec() -> VehicleSpec {
    VehicleSpec::parse(include_str!("truck.vehicle")).unwrap()
}

fn flat(surface: Surface) -> Flat {
    Flat {
        height: 0.0,
        surface,
    }
}

/// A slope of one surface, rising along -Z, the way a car at rest faces: `grade` is its rise
/// over its run.
struct Hill {
    grade: f32,
    surface: Surface,
}

impl Ground for Hill {
    fn cast(&self, from: Vec3, direction: Vec3, reach: f32) -> Option<Contact> {
        // The plane y = -grade * z.
        let normal = Vec3::new(0.0, 1.0, self.grade).normalize();
        let facing = direction.dot(normal);
        if facing >= -1.0e-6 {
            return None;
        }
        let distance = -from.dot(normal) / facing;
        (0.0..=reach).contains(&distance).then_some(Contact {
            distance,
            normal,
            surface: self.surface,
        })
    }
}

/// Ground whose left and right halves are different surfaces.
struct Split {
    left: Surface,
    right: Surface,
}

impl Ground for Split {
    fn cast(&self, from: Vec3, direction: Vec3, reach: f32) -> Option<Contact> {
        let mut contact = flat(self.left).cast(from, direction, reach)?;
        if (from + direction * contact.distance).x > 0.0 {
            contact.surface = self.right;
        }
        Some(contact)
    }
}

/// Drives flat out for `seconds` and gives back how far the car went, in metres.
fn drive(
    vehicle: &mut Vehicle,
    body: &mut Body,
    ground: &dyn Ground,
    throttle: f32,
    seconds: f32,
) -> f32 {
    let mut driver = Driver::new(true);
    let asked = Asked {
        throttle,
        ..Asked::default()
    };
    let start = body.position;
    let mut time = 0.0;
    while time < seconds {
        let controls = driver.drive(&asked, vehicle, body, G, DT);
        vehicle.step(body, &controls, ground, DT);
        time += DT;
        assert!(body.position.is_finite() && body.velocity.is_finite());
    }
    (start - body.position).z
}

fn truck(mode: DriveMode) -> (Vehicle, Body) {
    let mut vehicle = Vehicle::new(spec(), G);
    vehicle.set_drive_mode(mode);
    (vehicle, Body::at_rest(Vec3::ZERO, 0.0))
}

/// A truck standing on a hill of this grade, nose up it.
fn truck_on(grade: f32, mode: DriveMode) -> (Vehicle, Body) {
    let (vehicle, mut body) = truck(mode);
    body.rotation = glam::Quat::from_rotation_x(grade.atan());
    (vehicle, body)
}

#[test]
fn the_file_is_the_truck_it_says() {
    let spec = spec();
    assert!((spec.wheelbase() - 3.686).abs() < 1.0e-3);
    let (power, rpm) = spec.engine.peak_power();
    assert!((525.0e3..545.0e3).contains(&power), "{} kW", power / 1000.0);
    assert!((6300.0..6900.0).contains(&rpm));
    assert_eq!(spec.gearbox.gears.len(), 10);
    let front = spec.front_weight();
    assert!((0.54..0.58).contains(&front), "{front} on the front");
    let transfer = spec.transfer.expect("a transfer case");
    assert!((transfer.low - 2.64).abs() < 1.0e-6);
    // A foot of wheel travel at each end.
    assert!(spec.front.travel_up + spec.front.travel_down > 0.3);
    assert!(spec.rear.travel_up + spec.rear.travel_down > 0.34);
    // A soft ride: each end's own bounce near 1.3 Hz.
    for (axle, share) in [(&spec.front, front), (&spec.rear, 1.0 - front)] {
        let corner = spec.mass * share * 0.5;
        let hertz = (axle.spring / corner).sqrt() / std::f32::consts::TAU;
        assert!((1.1..1.6).contains(&hertz), "{hertz} Hz");
    }
}

#[test]
fn it_stands_still_on_its_springs_and_settles_after_a_drop() {
    let (mut vehicle, mut body) = truck(DriveMode::FourHigh);
    body.position.y = 0.35;
    let road = flat(Surface::Dirt);
    let mut deepest = 0.0_f32;
    for _ in 0..360 {
        vehicle.step(&mut body, &Default::default(), &road, DT);
        deepest = deepest.max(vehicle.wheels[0].travel);
    }
    // It used a good part of its travel, and did not hit the stops hard enough to bounce off.
    assert!(deepest > 0.08 && deepest < 0.2, "{deepest}");
    assert!(body.position.y.abs() < 0.02, "at {}", body.position.y);
    assert!(body.velocity.length() < 0.05);
    let loads: f32 = vehicle.wheels.iter().map(|wheel| wheel.load).sum();
    assert!((loads - 2800.0 * G).abs() < 300.0, "{loads}");
}

#[test]
fn on_the_road_it_is_as_quick_as_the_truck_is() {
    // Ford's truck reaches 100 km/h in about four seconds, in four-wheel drive.
    let (mut vehicle, mut body) = truck(DriveMode::FourHigh);
    let road = flat(Surface::Asphalt);
    let mut driver = Driver::new(true);
    let asked = Asked {
        throttle: 1.0,
        ..Asked::default()
    };
    let (mut seconds, mut to_100) = (0.0, None);
    while seconds < 12.0 {
        let controls = driver.drive(&asked, &vehicle, &body, G, DT);
        vehicle.step(&mut body, &controls, &road, DT);
        seconds += DT;
        if to_100.is_none() && body.velocity.length() * 3.6 >= 100.0 {
            to_100 = Some(seconds);
        }
        assert!(vehicle.rpm() < 7600.0, "over-revved");
    }
    let to_100 = to_100.expect("it reaches 100 km/h");
    assert!((3.4..5.6).contains(&to_100), "{to_100} s to 100 km/h");
}

#[test]
fn a_light_foot_changes_up_early_and_does_not_hunt() {
    let (mut vehicle, mut body) = truck(DriveMode::FourHigh);
    let road = flat(Surface::Dirt);
    let mut driver = Driver::new(true);
    let asked = Asked {
        throttle: 0.25,
        ..Asked::default()
    };
    let (mut changes, mut last, mut highest) = (0, 0, 0.0_f32);
    for step in 0..1500 {
        let controls = driver.drive(&asked, &vehicle, &body, G, DT);
        vehicle.step(&mut body, &controls, &road, DT);
        if vehicle.gear() != last {
            assert!(vehicle.gear() > last, "it changed down at step {step}");
            changes += 1;
            last = vehicle.gear();
        }
        if step > 120 {
            highest = highest.max(vehicle.rpm());
        }
    }
    assert!(changes >= 3, "only {changes} changes");
    assert!(highest < 4600.0, "it revved to {highest}");
}

#[test]
fn low_range_crawls() {
    let top = |mode: DriveMode| {
        let (mut vehicle, mut body) = truck(mode);
        // Held in first.
        vehicle.set_gear(1);
        let road = flat(Surface::Dirt);
        let controls = solstice_vehicle::Controls {
            throttle: 1.0,
            ..Default::default()
        };
        for _ in 0..600 {
            vehicle.step(&mut body, &controls, &road, DT);
        }
        body.velocity.length()
    };
    let (high, low) = (top(DriveMode::FourHigh), top(DriveMode::FourLow));
    assert!((high / low - 2.64).abs() < 0.2, "{high} and {low}");
    assert!(low < 7.0, "first low tops out at {} km/h", low * 3.6);
}

#[test]
fn a_tyre_sinks_into_mud_and_stands_on_rock() {
    for (surface, low, high) in [
        (Surface::Rock, 0.0, 0.001),
        (Surface::Dirt, 0.002, 0.01),
        (Surface::Mud, 0.05, 0.1),
        (Surface::Bog, 0.1, 0.2),
    ] {
        let (mut vehicle, mut body) = truck(DriveMode::FourHigh);
        let ground = flat(surface);
        for _ in 0..300 {
            vehicle.step(&mut body, &Default::default(), &ground, DT);
        }
        for wheel in &vehicle.wheels {
            assert!(
                (low..=high).contains(&wheel.sink),
                "{surface:?}: sunk {}",
                wheel.sink
            );
        }
        // The body sits lower by what its tyres sank.
        assert!(
            (body.position.y + vehicle.wheels[0].sink).abs() < 0.03,
            "{surface:?}: body at {}",
            body.position.y
        );
    }
}

#[test]
fn a_bog_stops_two_wheel_drive_and_four_wheel_drive_gets_through() {
    let across = |mode: DriveMode| {
        let (mut vehicle, mut body) = truck(mode);
        drive(&mut vehicle, &mut body, &flat(Surface::Bog), 0.7, 12.0)
    };
    let (two, four) = (across(DriveMode::TwoHigh), across(DriveMode::FourHigh));
    assert!(two < 6.0, "two-wheel drive went {two} m through the bog");
    assert!(four > 15.0, "four-wheel drive went only {four} m");
    // Low range pulls through as well, slowly.
    let low = across(DriveMode::FourLow);
    assert!(low > 10.0, "low range went only {low} m");
}

#[test]
fn mud_holds_a_truck_back_and_a_dirt_track_does_not() {
    let after = |surface: Surface| {
        let (mut vehicle, mut body) = truck(DriveMode::FourHigh);
        drive(&mut vehicle, &mut body, &flat(surface), 1.0, 6.0);
        body.velocity.length() * 3.6
    };
    let (dirt, mud, bog) = (
        after(Surface::Dirt),
        after(Surface::Mud),
        after(Surface::Bog),
    );
    assert!(dirt > 95.0, "{dirt} km/h on dirt");
    assert!(mud < dirt * 0.75 && mud > 25.0, "{mud} km/h in mud");
    assert!(bog < mud * 0.8, "{bog} km/h in the bog");
}

#[test]
fn rain_turns_a_clay_hill_from_a_climb_into_a_slide() {
    // A 30% clay track: a dry climb in four-wheel drive. Soaked, two-wheel drive spins.
    let climb = |mode: DriveMode, wetness: f32| {
        let (mut vehicle, mut body) = truck_on(0.3, mode);
        vehicle.set_wetness(wetness);
        let hill = Hill {
            grade: 0.3,
            surface: Surface::Dirt,
        };
        drive(&mut vehicle, &mut body, &hill, 0.8, 8.0)
    };
    let dry = climb(DriveMode::FourHigh, 0.0);
    let wet_two = climb(DriveMode::TwoHigh, 1.0);
    let wet_four = climb(DriveMode::FourHigh, 1.0);
    assert!(dry > 40.0, "dry, it climbed {dry} m");
    assert!(wet_four > 15.0, "wet, in four-wheel drive, {wet_four} m");
    assert!(
        wet_two < wet_four * 0.5,
        "wet, in two-wheel drive, {wet_two} m against {wet_four}"
    );
}

#[test]
fn lockers_pull_it_off_a_split_of_rock_and_bog() {
    // The left wheels on rock, the right ones in a bog, on a hill: open differentials send the
    // drive to the side that spins.
    struct SplitHill;
    impl Ground for SplitHill {
        fn cast(&self, from: Vec3, direction: Vec3, reach: f32) -> Option<Contact> {
            let hill = Hill {
                grade: 0.25,
                surface: Surface::Rock,
            };
            let mut contact = hill.cast(from, direction, reach)?;
            if (from + direction * contact.distance).x > 0.0 {
                contact.surface = Surface::Bog;
            }
            Some(contact)
        }
    }
    let climb = |locked: bool| {
        let (mut vehicle, mut body) = truck_on(0.25, DriveMode::FourLow);
        vehicle.set_lockers(locked, locked);
        vehicle.set_wetness(1.0);
        drive(&mut vehicle, &mut body, &SplitHill, 0.6, 8.0)
    };
    let (open, locked) = (climb(false), climb(true));
    assert!(
        locked > open * 1.25 && locked > 5.0,
        "locked {locked} m, open {open} m"
    );
    // On the level the same split is no trouble either way.
    let (mut vehicle, mut body) = truck(DriveMode::FourHigh);
    let level = Split {
        left: Surface::Rock,
        right: Surface::Mud,
    };
    assert!(drive(&mut vehicle, &mut body, &level, 0.5, 5.0) > 20.0);
}

#[test]
fn it_brakes_and_stays_straight_on_dirt() {
    let (mut vehicle, mut body) = truck(DriveMode::FourHigh);
    let road = flat(Surface::Dirt);
    body.velocity = Vec3::NEG_Z * (80.0 / 3.6);
    vehicle.set_gear(4);
    vehicle.roll_with(&body);
    let mut driver = Driver::new(true);
    let asked = Asked {
        brake: 1.0,
        ..Asked::default()
    };
    let start = body.position;
    let mut seconds = 0.0;
    while body.velocity.length() > 0.5 && seconds < 15.0 {
        let controls = driver.drive(&asked, &vehicle, &body, G, DT);
        vehicle.step(&mut body, &controls, &road, DT);
        seconds += DT;
    }
    let distance = (start - body.position).z;
    // About 0.65 g on hard dirt: 80 km/h in something under 45 m.
    assert!((28.0..50.0).contains(&distance), "stopped in {distance} m");
    assert!(
        body.position.x.abs() < 1.0,
        "it pulled to {}",
        body.position.x
    );
}
