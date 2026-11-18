//! Ground that keeps the ruts a truck presses into it: that a tyre stands in the rut that
//! is kept, that the ruts are left behind it and stay, that spinning digs, that the wheels
//! behind ride in the ruts of those ahead, and that a deep rut holds a truck back.

use glam::{Vec2, Vec3};
use solstice_vehicle::{
    Asked, Body, Contact, DriveMode, Driver, Flat, Ground, Press, Ruts, Surface, Vehicle,
    VehicleSpec,
};
use std::cell::RefCell;

const G: f32 = 9.81;
const DT: f32 = 1.0 / 60.0;

/// Level ground of one surface that keeps what is pressed into it.
struct Soft {
    surface: Surface,
    ruts: RefCell<Ruts>,
}

impl Soft {
    fn new(surface: Surface) -> Self {
        Self {
            surface,
            ruts: RefCell::default(),
        }
    }

    fn depth(&self, x: f32, z: f32) -> f32 {
        self.ruts.borrow().depth(x, z)
    }
}

impl Ground for Soft {
    fn cast(&self, from: Vec3, direction: Vec3, reach: f32) -> Option<Contact> {
        Flat {
            height: 0.0,
            surface: self.surface,
        }
        .cast(from, direction, reach)
    }

    fn rut(&self, at: Vec3) -> f32 {
        self.ruts.borrow().depth(at.x, at.z)
    }

    fn press(&self, press: &Press) {
        self.ruts.borrow_mut().press(press);
    }
}

fn truck(mode: DriveMode) -> (Vehicle, Body) {
    let spec = VehicleSpec::parse(include_str!("truck.vehicle")).unwrap();
    let mut vehicle = Vehicle::new(spec, G);
    vehicle.set_drive_mode(mode);
    (vehicle, Body::at_rest(Vec3::ZERO, 0.0))
}

/// Where each tyre stands on the ground, in the world: x and z.
fn feet(vehicle: &Vehicle, body: &Body) -> [Vec2; 4] {
    let hubs = vehicle.spec().hubs();
    std::array::from_fn(|index| {
        let at = body.position + body.rotation * hubs[index];
        Vec2::new(at.x, at.z)
    })
}

/// Drives for `seconds` at this much throttle, and gives back how far the truck went.
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

#[test]
fn a_standing_truck_sits_in_the_ruts_it_pressed() {
    for (surface, low, high) in [
        (Surface::Dirt, 0.002, 0.01),
        (Surface::Mud, 0.05, 0.1),
        (Surface::Bog, 0.1, 0.2),
    ] {
        let ground = Soft::new(surface);
        let (mut vehicle, mut body) = truck(DriveMode::FourHigh);
        for _ in 0..300 {
            vehicle.step(&mut body, &Default::default(), &ground, DT);
        }
        for (wheel, foot) in vehicle.wheels.iter().zip(feet(&vehicle, &body)) {
            // As deep as on ground that keeps nothing, and the map says the same depth
            // under the tyre as the tyre has sunk.
            assert!(
                (low..=high).contains(&wheel.sink),
                "{surface:?}: sunk {}",
                wheel.sink
            );
            let kept = ground.depth(foot.x, foot.y);
            assert!(
                (kept - wheel.sink).abs() < 0.004,
                "{surface:?}: the tyre is {} down in a rut {kept} deep",
                wheel.sink
            );
        }
        // And the body sits lower by it.
        assert!((body.position.y + vehicle.wheels[0].sink).abs() < 0.03);
    }
    // Rock keeps nothing.
    let ground = Soft::new(Surface::Rock);
    let (mut vehicle, mut body) = truck(DriveMode::FourHigh);
    for _ in 0..120 {
        vehicle.step(&mut body, &Default::default(), &ground, DT);
    }
    assert!(ground.ruts.borrow().is_empty());
}

#[test]
fn a_truck_leaves_its_ruts_behind_it_with_berms_beside_them() {
    let ground = Soft::new(Surface::Mud);
    let (mut vehicle, mut body) = truck(DriveMode::FourHigh);
    let went = drive(&mut vehicle, &mut body, &ground, 0.5, 5.0);
    assert!(went > 12.0, "it went {went} m");
    let half_track = vehicle.spec().front.track * 0.5;
    // Behind it, under where each side's wheels ran, a rut all the way back to the start;
    // between them and outside them, ground nobody pressed.
    for along in [2.0, 5.0, 9.0] {
        for side in [-1.0, 1.0] {
            let rut = ground.depth(side * half_track, -along);
            assert!((0.04..0.2).contains(&rut), "{along} m on: {rut}");
            // Beside the rut, a berm.
            let berm = (0..30)
                .map(|k| -ground.depth(side * (half_track + 0.1 + k as f32 * 0.01), -along))
                .fold(f32::MIN, f32::max);
            assert!(berm > 0.008, "{along} m on: a berm of {berm}");
        }
        assert_eq!(ground.depth(0.0, -along), 0.0);
        assert_eq!(ground.depth(2.0 * half_track, -along), 0.0);
    }
    // They are still there when it has gone and time has passed.
    let before = ground.depth(half_track, -5.0);
    for _ in 0..600 {
        ground.ruts.borrow_mut().advance(1.0);
    }
    assert_eq!(ground.depth(half_track, -5.0), before);
}

#[test]
fn the_wheels_behind_ride_in_the_ruts_of_the_wheels_ahead() {
    let ground = Soft::new(Surface::Mud);
    let (mut vehicle, mut body) = truck(DriveMode::FourHigh);
    drive(&mut vehicle, &mut body, &ground, 0.35, 6.0);
    // Rolling steadily, a rear tyre is no deeper than the rut it is in: it stands on the
    // floor the front one made, or has pressed it a little deeper and stands on that.
    for (wheel, foot) in vehicle.wheels.iter().zip(feet(&vehicle, &body)).skip(2) {
        let kept = ground.depth(foot.x, foot.y);
        assert!(wheel.sink > 0.03, "{}", wheel.sink);
        assert!(
            (wheel.sink - kept).abs() < 0.012,
            "a rear tyre {} down in a rut {kept} deep",
            wheel.sink
        );
    }
    // A second truck down the same ruts makes them no deeper than a first does a fresh
    // line beside them.
    let first = ground.depth(vehicle.spec().front.track * 0.5, -6.0);
    let (mut second, mut body) = truck(DriveMode::FourHigh);
    drive(&mut second, &mut body, &ground, 0.35, 6.0);
    let both = ground.depth(vehicle.spec().front.track * 0.5, -6.0);
    assert!(both >= first && both < first + 0.03, "{first} then {both}");
}

#[test]
fn a_spinning_wheel_digs_itself_in() {
    // Two-wheel drive, flat out in a bog, traction control or not: the rear wheels spin
    // and dig, and the fronts, which only roll, do not.
    let ground = Soft::new(Surface::Bog);
    let (mut vehicle, mut body) = truck(DriveMode::TwoHigh);
    drive(&mut vehicle, &mut body, &ground, 1.0, 8.0);
    let (front, rear) = (vehicle.wheels[0].sink, vehicle.wheels[2].sink);
    assert!(rear > front + 0.03, "front {front}, rear {rear}");
    // No deeper than a wheel can dig: it is on its axle by then.
    let radius = vehicle.spec().rear.radius;
    assert!(rear < 0.75 * radius, "{rear}");
    // Deeper than the same truck sinks on ground that keeps nothing.
    let (mut other, mut body) = truck(DriveMode::TwoHigh);
    drive(
        &mut other,
        &mut body,
        &Flat {
            height: 0.0,
            surface: Surface::Bog,
        },
        1.0,
        8.0,
    );
    assert!(rear > other.wheels[2].sink + 0.02);
}

#[test]
fn four_wheel_drive_still_gets_through_a_bog_that_keeps_its_ruts() {
    let across = |mode: DriveMode| {
        let ground = Soft::new(Surface::Bog);
        let (mut vehicle, mut body) = truck(mode);
        drive(&mut vehicle, &mut body, &ground, 0.7, 12.0)
    };
    let (two, four, low) = (
        across(DriveMode::TwoHigh),
        across(DriveMode::FourHigh),
        across(DriveMode::FourLow),
    );
    assert!(two < 6.0, "two-wheel drive went {two} m through the bog");
    assert!(four > 15.0, "four-wheel drive went only {four} m");
    assert!(low > 10.0, "low range went only {low} m");
}

#[test]
fn a_deep_rut_holds_a_truck_back() {
    // Rolling at 8 m/s out of gear along a clay track: once on ground nobody has driven,
    // once along ruts a hand deep.
    let coast = |rutted: bool| {
        let ground = Soft::new(Surface::Dirt);
        let (mut vehicle, mut body) = truck(DriveMode::FourHigh);
        if rutted {
            let half_track = vehicle.spec().front.track * 0.5;
            for (side, metre) in [-1.0, 1.0]
                .into_iter()
                .flat_map(|side| (-5..80).map(move |metre| (side, metre as f32)))
            {
                ground.ruts.borrow_mut().press(&Press {
                    from: Vec2::new(side * half_track, -metre),
                    to: Vec2::new(side * half_track, -metre - 1.0),
                    along: Vec2::NEG_Y,
                    half_width: 0.2,
                    half_length: 0.15,
                    depth: 0.12,
                    berm: 0.4,
                    phase: (0.0, 0.0),
                    print: 0.0,
                    wet: 0.5,
                    stays: 0.3,
                    churn: 1.0,
                });
            }
        }
        for _ in 0..120 {
            vehicle.step(&mut body, &Default::default(), &ground, DT);
        }
        body.velocity = body.forward() * 8.0;
        vehicle.roll_with(&body);
        for _ in 0..180 {
            vehicle.step(&mut body, &Default::default(), &ground, DT);
        }
        (body.velocity.length(), vehicle.wheels[0].sink)
    };
    let ((open, sink), (rutted, deep)) = (coast(false), coast(true));
    assert!(
        sink < 0.02 && (deep - 0.12).abs() < 0.02,
        "{sink} and {deep}"
    );
    assert!(
        rutted < open - 0.5,
        "{open} m/s on open ground, {rutted} in ruts"
    );
}

#[test]
fn a_tyre_out_of_a_creek_leaves_wet_tracks_on_dry_rock() {
    // Half the ground is a ford, the rest bare rock, which takes no rut.
    struct Bank(RefCell<Ruts>);
    impl Ground for Bank {
        fn cast(&self, from: Vec3, direction: Vec3, reach: f32) -> Option<Contact> {
            let mut contact = Flat {
                height: 0.0,
                surface: Surface::Ford,
            }
            .cast(from, direction, reach)?;
            if (from + direction * contact.distance).z < -6.0 {
                contact.surface = Surface::Rock;
            }
            Some(contact)
        }
        fn rut(&self, at: Vec3) -> f32 {
            self.0.borrow().depth(at.x, at.z)
        }
        fn press(&self, press: &Press) {
            self.0.borrow_mut().press(press);
        }
    }
    let ground = Bank(RefCell::default());
    let (mut vehicle, mut body) = truck(DriveMode::FourHigh);
    let went = drive(&mut vehicle, &mut body, &ground, 0.5, 6.0);
    assert!(went > 20.0, "{went}");
    let half_track = vehicle.spec().front.track * 0.5;
    let ruts = ground.0.borrow();
    // Just out of the water the rock is wet where the tyres ran, and not pressed; further
    // on, less wet; between the tyres, dry.
    let near = ruts.wet(half_track, -8.0);
    let far = ruts.wet(half_track, -(went - 1.0).min(40.0));
    assert!(near > 0.5, "{near}");
    assert!(far < near, "{near} then {far}");
    assert_eq!(ruts.height(half_track, -8.0), 0.0);
    assert_eq!(ruts.wet(0.0, -8.0), 0.0);
    assert!(vehicle.carried()[0].0 < 1.0);
}
