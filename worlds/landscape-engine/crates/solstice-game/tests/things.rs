//! Loose things: what is put in a truck's tray lies in it and rides with it, weighs on the
//! truck, and what is put on the ground lies there.

use glam::{Mat4, Quat, Vec3};
use solstice_asset::{Collision, CollisionShape, GameFile};
use solstice_game::{Devices, Play, GAME_FILE};
use solstice_physics::vehicle::VehicleSpec;
use solstice_world::STEP_SECONDS;

const TRUCK: &str = include_str!("../../solstice-vehicle/tests/truck.vehicle");
const ACTOR: &str = "name truck\nmesh truck.glb 1\nvehicle truck.vehicle\ncamera 0 1.6 0\n";
const KEYS: &str = "throttle key KeyW\nbrake key KeyS\nsteer key KeyD\nsteer key KeyA -1\n";
const SCENE: &str = "input keys.input\nactor truck.actor 0 0 0 0 instances=1+5\n";

/// A flat floor at height zero, 2 km across.
fn floor() -> Collision {
    let far = 1000.0;
    Collision {
        shapes: vec![CollisionShape {
            hull: false,
            vertices: vec![
                [-far, 0.0, -far],
                [far, 0.0, -far],
                [far, 0.0, far],
                [-far, 0.0, far],
            ],
            triangles: vec![[0, 2, 1], [0, 3, 2]],
        }],
        instance_shapes: vec![0],
        heightfield: None,
    }
}

fn truck() -> Play {
    truck_with("")
}

/// The truck on the floor, and whatever else the scene's game is given: its blockers.
fn truck_with(more: &str) -> Play {
    let spec = VehicleSpec::parse(TRUCK).expect("the truck's file reads");
    let hubs = spec.hubs();
    let scene = format!("{SCENE}{more}");
    let files: Vec<GameFile> = [
        (GAME_FILE, scene.as_str()),
        ("truck.actor", ACTOR),
        ("truck.vehicle", TRUCK),
        ("keys.input", KEYS),
    ]
    .iter()
    .map(|(name, text)| GameFile {
        name: (*name).to_owned(),
        text: (*text).to_owned(),
    })
    .collect();
    let baked = move |id: u32| match id {
        0 | 1 => Some(Mat4::IDENTITY),
        2..=5 => Some(Mat4::from_translation(hubs[id as usize - 2])),
        _ => None,
    };
    Play::load(&files, baked, Some(&floor())).unwrap_or_else(|e| panic!("{e:?}"))
}

fn run(play: &mut Play, devices: &mut Devices, seconds: f64) {
    for _ in 0..(seconds / STEP_SECONDS).round() as u32 {
        play.frame(STEP_SECONDS, devices, |_, _| {});
    }
}

/// Where a thing is in the truck's own space.
fn aboard(play: &Play, thing: solstice_world::Entity) -> Vec3 {
    let (truck, turned) = play.car_pose().expect("a truck");
    let (at, _) = play.thing(thing).expect("the thing");
    turned.inverse() * (at - truck)
}

#[test]
fn what_is_put_in_the_tray_rides_in_it_and_weighs_on_the_truck() {
    let (size, floor) = VehicleSpec::parse(TRUCK)
        .expect("reads")
        .tray
        .expect("it has a tray");
    let inside = |at: Vec3| {
        (at.x - floor.x).abs() < size.x * 0.5
            && (at.z - floor.z).abs() < size.z * 0.5
            && at.y > floor.y - 0.05
            && at.y < floor.y + size.y + 0.4
    };

    // An empty truck settles on its springs: how high it rides.
    let mut empty = truck();
    let mut devices = Devices::new();
    run(&mut empty, &mut devices, 3.0);
    let unladen = empty.car_pose().expect("a truck").0.y;

    // The same truck with a third of a tonne of gold put in its tray, a nugget at a time.
    let mut play = truck();
    run(&mut play, &mut devices, 1.0);
    let mut nuggets = Vec::new();
    for index in 0..24 {
        let (across, along, up) = (index % 4, index / 4 % 3, index / 12);
        let local = floor
            + Vec3::new(
                (across as f32 - 1.5) * 0.32,
                0.2 + up as f32 * 0.3,
                (along as f32 - 1.0) * 0.45,
            );
        let (truck, turned) = play.car_pose().expect("a truck");
        nuggets.push(play.add_thing(
            truck + turned * local,
            Quat::IDENTITY,
            Vec3::new(0.2, 0.14, 0.2),
            14.0,
        ));
    }
    run(&mut play, &mut devices, 3.0);
    for &nugget in &nuggets {
        let at = aboard(&play, nugget);
        assert!(inside(at), "at rest, one lies at {at}");
    }
    let laden = play.car_pose().expect("a truck").0.y;
    assert!(
        unladen - laden > 0.004,
        "336 kg in the tray sets it lower on its springs: {unladen} empty, {laden} laden"
    );

    // Driven off as hard as it goes, braked as hard as it stops, and swung through a turn.
    devices.key("KeyW", true);
    run(&mut play, &mut devices, 5.0);
    devices.key("KeyW", false);
    devices.key("KeyS", true);
    run(&mut play, &mut devices, 4.0);
    devices.key("KeyS", false);
    devices.key("KeyW", true);
    run(&mut play, &mut devices, 3.0);
    devices.key("KeyD", true);
    run(&mut play, &mut devices, 2.5);
    devices.key("KeyD", false);
    devices.key("KeyW", false);
    devices.key("KeyS", true);
    run(&mut play, &mut devices, 5.0);
    let (truck, _) = play.car_pose().expect("a truck");
    assert!(truck.length() > 30.0, "it went somewhere: {truck}");
    let kept = nuggets
        .iter()
        .filter(|&&nugget| inside(aboard(&play, nugget)))
        .count();
    assert_eq!(kept, nuggets.len(), "every nugget is still in the tray");
}

#[test]
fn what_is_put_down_lies_where_it_is_put_and_can_be_taken_away() {
    let mut play = truck();
    let mut devices = Devices::new();
    let size = Vec3::new(0.2, 0.14, 0.2);
    // Let go a little above the ground, well clear of the truck.
    let thing = play.add_thing(Vec3::new(8.0, 0.6, 3.0), Quat::IDENTITY, size, 5.0);
    run(&mut play, &mut devices, 2.0);
    let (at, _) = play.thing(thing).expect("it is there");
    assert!(
        (at.x - 8.0).abs() < 0.05 && (at.z - 3.0).abs() < 0.05,
        "it fell straight: {at}"
    );
    assert!(
        (at.y - size.y * 0.5).abs() < 0.03,
        "and lies on the ground: {at}"
    );
    // What is under a place is found from above: the thing, and beside it the ground.
    let top = play
        .surface_below(Vec3::new(8.0, 3.0, 3.0))
        .expect("something is there");
    assert!((top - size.y).abs() < 0.03, "{top}");
    assert!(
        play.surface_below(Vec3::new(9.0, 3.0, 3.0))
            .expect("the ground")
            .abs()
            < 0.01
    );
    // Moved, it is where it was moved to, at rest; taken away, it is gone.
    play.move_thing(thing, Vec3::new(-6.0, 0.08, 2.0), Quat::IDENTITY);
    run(&mut play, &mut devices, 1.0);
    let (at, _) = play.thing(thing).expect("it is there");
    assert!((at.x + 6.0).abs() < 0.05 && at.y < 0.12, "{at}");
    play.remove_thing(thing);
    run(&mut play, &mut devices, 0.1);
    assert!(play.thing(thing).is_none());
}

#[test]
fn what_is_let_go_over_an_open_chest_lies_in_it() {
    // A chest as the shed's is to things: a floor, and four sides round what is in it.
    // 0.56 across and 1.15 long, its floor 0.128 up and its rim 0.548, well clear of the truck.
    let (x, z, floor, rim, side) = (9.0_f32, 4.0_f32, 0.128_f32, 0.548_f32, 0.05_f32);
    let mut chest = format!(
        "blocker {x} {} {z} 0.56 {floor} 1.15 0
",
        floor * 0.5
    );
    let (middle, high) = ((floor + rim) * 0.5, rim - floor);
    for end in [-1.0_f32, 1.0] {
        chest += &format!(
            "blocker {} {middle} {z} {side} {high} 1.15 0
",
            x + end * 0.255
        );
        chest += &format!(
            "blocker {x} {middle} {} 0.46 {high} {side} 0
",
            z + end * 0.55
        );
    }
    let mut play = truck_with(&chest);
    let mut devices = Devices::new();
    run(&mut play, &mut devices, 0.5);
    // Nuggets are let go over it one after another, each put down on what is under it:
    // the floor, and then the nuggets already there.
    let size = Vec3::new(0.18, 0.12, 0.18);
    let mut nuggets = Vec::new();
    for index in 0..10 {
        let over = Vec3::new(x, rim + 0.45, z + (index % 5) as f32 * 0.16 - 0.32);
        let top = play.surface_below(over).expect("the chest is under it");
        assert!(
            top >= floor - 0.01 && top < rim + 0.4,
            "it is put on the floor or on a nugget: {top}"
        );
        nuggets.push(play.add_thing(
            Vec3::new(over.x, top + size.y * 0.5 + 0.03, over.z),
            Quat::IDENTITY,
            size,
            3.0,
        ));
        run(&mut play, &mut devices, 0.4);
    }
    run(&mut play, &mut devices, 3.0);
    for nugget in nuggets {
        let (at, _) = play.thing(nugget).expect("it is there");
        assert!(
            (at.x - x).abs() < 0.23 && (at.z - z).abs() < 0.53 && at.y > floor && at.y < rim + 0.3,
            "one lies outside the chest, at {at}"
        );
    }
}
