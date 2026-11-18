//! A car in a scene's game: driven by the keys on the scene's collision, drawn with its wheels,
//! followed by the camera, timed round a lap, and stopped by a wall.

use glam::{Mat4, Vec3};
use solstice_asset::{Collision, CollisionShape, GameFile};
use solstice_game::{Devices, Play, GAME_FILE};
use solstice_physics::vehicle::{Surface, VehicleSpec};
use solstice_physics::Car;
use solstice_world::{Transform, STEP_SECONDS};
use std::collections::HashMap;

fn files(list: &[(&str, &str)]) -> Vec<GameFile> {
    list.iter()
        .map(|(name, text)| GameFile {
            name: (*name).to_owned(),
            text: (*text).to_owned(),
        })
        .collect()
}

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

const CAR: &str = "name car\nmesh gt.glb 1\nvehicle gt.vehicle\ncamera 0 1.1 0\n";
const KEYS: &str = "\
throttle key KeyW
brake key KeyS
steer key KeyD
steer key KeyA -1
steer gamepad left_x
shift_up key KeyE
shift_down key KeyQ
reset key KeyR
camera key KeyC
look_back key KeyB
look_round gamepad right_x
look_over gamepad right_y
autopilot key KeyP
";

/// Where the bake placed the car's instances, for a car placed at the origin facing -Z:
/// instance 0 is the scene's floor, 1 the body, 2 to 5 the wheels, each with its origin at its
/// hub, and 6 the front left wheel's brake caliper, with its origin 30 mm inboard of the hub.
fn baked(id: u32) -> Option<Mat4> {
    let hubs = VehicleSpec::gt().hubs();
    match id {
        0 | 1 => Some(Mat4::IDENTITY),
        2..=5 => Some(Mat4::from_translation(hubs[id as usize - 2])),
        6 => Some(Mat4::from_translation(hubs[0] + Vec3::X * 0.03)),
        _ => None,
    }
}

fn game(scene: &str, extra: &[(&str, &str)]) -> Play {
    let mut list = vec![
        (GAME_FILE, scene),
        ("car.actor", CAR),
        ("gt.vehicle", "# the built-in GT\n"),
        ("keys.input", KEYS),
    ];
    list.extend_from_slice(extra);
    Play::load(&files(&list), baked, Some(&floor())).unwrap_or_else(|e| panic!("{e:?}"))
}

const SCENE: &str = "input keys.input\nactor car.actor 0 0 0 0 instances=1+6\n";

fn car_at(play: &Play) -> (Vec3, f32) {
    let world = &play.world().entities;
    let mut query = world.query::<(&Transform, &Car)>();
    let (transform, car) = query.iter().next().expect("a car");
    (transform.translation, car.vehicle.telemetry().speed)
}

fn run(play: &mut Play, devices: &mut Devices, seconds: f64) -> HashMap<u32, Mat4> {
    let mut placed = HashMap::new();
    for _ in 0..(seconds / STEP_SECONDS).round() as u32 {
        play.frame(STEP_SECONDS, devices, |id, at| {
            placed.insert(id, at);
        });
    }
    placed
}

#[test]
fn the_throttle_drives_it_away_and_its_wheels_roll_with_it() {
    let mut play = game(SCENE, &[]);
    assert!(play.has_player());
    let mut devices = Devices::new();
    // It stands where it was put, on its wheels.
    run(&mut play, &mut devices, 1.0);
    let (at, speed) = car_at(&play);
    assert!(at.length() < 0.01 && speed.abs() < 0.01, "{at} {speed}");

    devices.key("KeyW", true);
    let placed = run(&mut play, &mut devices, 4.0);
    let (at, speed) = car_at(&play);
    assert!(
        at.z < -40.0 && at.x.abs() < 0.05,
        "it drove off straight: {at}"
    );
    assert!(speed > 25.0, "at {speed} m/s");
    let hud = play
        .hud()
        .expect("the player's car has instruments")
        .clone();
    assert!(hud.telemetry.gear >= 2 && hud.telemetry.rpm > 2000.0);
    assert!((hud.throttle - 1.0).abs() < 1.0e-6);
    assert!(hud.line().contains("km/h"));

    // The body is drawn where the car is, and each wheel at its hub, turned about its axle.
    let body = placed[&1];
    assert!(body.w_axis.truncate().distance(at) < 1.0, "{body}");
    let hubs = VehicleSpec::gt().hubs();
    for (index, hub) in hubs.iter().enumerate() {
        let wheel = placed[&(index as u32 + 2)];
        let local = body.inverse() * wheel;
        let offset = local.w_axis.truncate() - *hub;
        assert!(
            offset.x.abs() < 1.0e-3 && offset.z.abs() < 1.0e-3,
            "wheel {index}: {offset}"
        );
        assert!(offset.y.abs() < 0.03, "on its suspension: {}", offset.y);
        // Rolling: its own up and forward are turned about its axle, which still points across.
        let axle = local.x_axis.truncate();
        assert!((axle - Vec3::X).length() < 0.02, "{axle}");
    }
    // Two frames a moment apart draw the wheels turned differently.
    let later = run(&mut play, &mut devices, STEP_SECONDS);
    assert!(!later[&2].abs_diff_eq(placed[&2], 1.0e-3));
    // The caliper rides with its wheel, 30 mm inboard of the hub, and stays the right way up
    // while the wheel turns beside it.
    let (wheel, caliper) = (body.inverse() * placed[&2], body.inverse() * placed[&6]);
    let apart = caliper.w_axis.truncate() - wheel.w_axis.truncate();
    assert!((apart - Vec3::X * 0.03).length() < 1.0e-3, "{apart}");
    assert!(
        (caliper.y_axis.truncate() - Vec3::Y).length() < 0.01,
        "upright"
    );
}

#[test]
fn the_camera_follows_from_behind_and_the_views_change() {
    let mut play = game(SCENE, &[]);
    let mut devices = Devices::new();
    devices.key("KeyW", true);
    let mut view = None;
    for _ in 0..240 {
        view = play.frame(STEP_SECONDS, &mut devices, |_, _| {});
    }
    let view = view.expect("a player");
    let (at, _) = car_at(&play);
    // Behind the car, which heads -Z, above it, looking along the way it goes.
    assert!(view.position.z > at.z + 4.0 && view.position.z < at.z + 8.0);
    assert!(view.position.y > at.y + 1.0 && view.yaw.abs() < 0.02 && view.pitch < 0.0);
    use solstice_game::drive::CameraView;
    assert_eq!(play.hud().unwrap().view, CameraView::Chase);

    devices.key("KeyC", true);
    play.frame(STEP_SECONDS, &mut devices, |_, _| {});
    devices.key("KeyC", false);
    play.frame(STEP_SECONDS, &mut devices, |_, _| {});
    play.frame(STEP_SECONDS, &mut devices, |_, _| {});
    // Each press steps a view further out, to one from high above and well behind.
    for expected in [CameraView::Wide, CameraView::High] {
        devices.key("KeyC", true);
        play.frame(STEP_SECONDS, &mut devices, |_, _| {});
        devices.key("KeyC", false);
        play.frame(STEP_SECONDS, &mut devices, |_, _| {});
        assert_eq!(play.hud().unwrap().view, expected);
    }
    let mut far = None;
    for _ in 0..60 {
        far = play.frame(STEP_SECONDS, &mut devices, |_, _| {});
    }
    let (at, _) = car_at(&play);
    assert!(
        far.unwrap().position.distance(at) > 20.0,
        "well back from the car"
    );
    // The right stick swings the camera round the car: pushed to the right, the camera is
    // on the car's left, which heading -Z is -X.
    devices.gamepad_axis("right_x", 0.6);
    let mut swung = None;
    for _ in 0..60 {
        swung = play.frame(STEP_SECONDS, &mut devices, |_, _| {});
    }
    let (at, _) = car_at(&play);
    assert!(
        swung.unwrap().position.x < at.x - 5.0,
        "round to the left of the car"
    );
    devices.gamepad_axis("right_x", 0.0);
    // And then the views from on the car.
    devices.key("KeyC", true);
    let view = play.frame(STEP_SECONDS, &mut devices, |_, _| {}).unwrap();
    assert_eq!(play.hud().unwrap().view, CameraView::Bonnet);
    let (at, _) = car_at(&play);
    assert!(view.position.distance(at) < 2.0, "on the car");
}

#[test]
fn steering_turns_it_and_the_brake_stops_it() {
    let mut play = game(SCENE, &[]);
    let mut devices = Devices::new();
    devices.key("KeyW", true);
    run(&mut play, &mut devices, 3.0);
    devices.key("KeyD", true);
    run(&mut play, &mut devices, 1.5);
    let (at, _) = car_at(&play);
    assert!(at.x > 5.0, "it turned right: {at}");
    devices.key("KeyD", false);
    devices.key("KeyW", false);
    devices.key("KeyS", true);
    run(&mut play, &mut devices, 2.5);
    devices.key("KeyS", false);
    let (_, speed) = car_at(&play);
    assert!(speed.abs() < 4.0, "braked to {speed}");
}

#[test]
fn a_wall_stops_it() {
    // A wall across the road 60 metres ahead.
    let scene = format!("{SCENE}blocker 0 1 -60 30 2 1 0\n");
    let mut play = game(&scene, &[]);
    let mut devices = Devices::new();
    devices.key("KeyW", true);
    let mut fastest = 0.0_f32;
    for _ in 0..(6.0 / STEP_SECONDS) as u32 {
        play.frame(STEP_SECONDS, &mut devices, |_, _| {});
        fastest = fastest.max(car_at(&play).1);
    }
    let (at, speed) = car_at(&play);
    assert!(fastest > 20.0, "it reached {fastest} m/s");
    assert!(at.z > -60.0, "it is this side of the wall: {at}");
    assert!(
        at.z < -50.0 && speed.abs() < 3.0,
        "against it, stopped: {at} at {speed}"
    );
    assert!(at.is_finite() && at.y.abs() < 0.5);
}

#[test]
fn a_barrier_stops_it_however_fast_it_arrives() {
    // A crash barrier's wall, a third of a metre thick, across the road far enough ahead for
    // the car to be flat out when it gets there: it goes further in a step than the wall is
    // thick.
    for (ahead, least) in [(250.0, 40.0), (700.0, 68.0)] {
        // Wide enough that a car turned along it, with its power still on, finds no end.
        let scene = format!(
            "{SCENE}blocker 0 0.7 -{ahead} 1900 2 0.3 0
"
        );
        let mut play = game(&scene, &[]);
        let mut devices = Devices::new();
        devices.key("KeyW", true);
        let (mut fastest, mut furthest) = (0.0_f32, 0.0_f32);
        for _ in 0..(22.0 / STEP_SECONDS) as u32 {
            play.frame(STEP_SECONDS, &mut devices, |_, _| {});
            let (at, speed) = car_at(&play);
            fastest = fastest.max(speed);
            furthest = furthest.max(-at.z);
        }
        let (at, _) = car_at(&play);
        assert!(fastest > least, "it reached {fastest} m/s");
        assert!(
            furthest < ahead && at.z > -ahead,
            "the wall {ahead} m ahead held it: it got to {furthest} m, and is at {at}"
        );
        assert!(at.is_finite() && at.y.abs() < 1.0, "on the ground: {at}");
    }
}

/// A square circuit, 400 m a side, round the origin, driven clockwise from the middle of the
/// west side heading north (-Z), sampled every 10 m. Sixteen metres wide, with gravel to the
/// left of the first side.
fn square_track() -> String {
    square_track_at(0.0)
}

/// The same circuit, at a height.
fn square_track_at(height: f32) -> String {
    let mut text = String::from("name Square\nsector 80\n");
    let corners = [
        (Vec3::new(-200.0, 0.0, 0.0), Vec3::NEG_Z, 20),
        (Vec3::new(-200.0, 0.0, -200.0), Vec3::X, 40),
        (Vec3::new(200.0, 0.0, -200.0), Vec3::Z, 40),
        (Vec3::new(200.0, 0.0, 200.0), Vec3::NEG_X, 40),
        (Vec3::new(-200.0, 0.0, 200.0), Vec3::NEG_Z, 20),
    ];
    for (from, forward, count) in corners {
        let left = Vec3::Y.cross(forward);
        for step in 0..count {
            let p = from + forward * (step as f32 * 10.0) + Vec3::Y * height;
            text.push_str(&format!(
                "p {} {} {} {} {} {} 8 8 l=gravel:20\n",
                p.x, p.y, p.z, left.x, left.y, left.z
            ));
        }
    }
    text
}

#[test]
fn a_tyre_knows_what_it_is_on_and_reset_puts_the_car_back() {
    let track = square_track();
    // Placed on the gravel to the left of the first side, pointing along it.
    let scene =
        "input keys.input\ntrack square.track\nactor car.actor -215 0 -50 0 instances=1+6\n";
    let mut play = Play::load(
        &files(&[
            (GAME_FILE, scene),
            ("car.actor", CAR),
            ("gt.vehicle", ""),
            ("keys.input", KEYS),
            ("square.track", &track),
        ]),
        |id| baked(id).map(|at| Mat4::from_translation(Vec3::new(-215.0, 0.0, -50.0)) * at),
        Some(&floor()),
    )
    .unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(play.track().unwrap().name, "Square");
    let mut devices = Devices::new();
    run(&mut play, &mut devices, 0.5);
    {
        let world = &play.world().entities;
        let mut query = world.query::<&Car>();
        let car = query.iter().next().unwrap();
        assert!(car
            .vehicle
            .wheels
            .iter()
            .all(|wheel| wheel.on_ground && wheel.surface == Surface::Gravel));
    }
    // Gravel holds it back: flat out for three seconds, it has hardly got going.
    devices.key("KeyW", true);
    run(&mut play, &mut devices, 3.0);
    let (_, speed) = car_at(&play);
    assert!(speed < 14.0, "{speed} m/s on gravel");
    devices.key("KeyW", false);

    devices.key("KeyR", true);
    run(&mut play, &mut devices, STEP_SECONDS);
    devices.key("KeyR", false);
    run(&mut play, &mut devices, 1.0);
    let (at, speed) = car_at(&play);
    assert!((at.x + 200.0).abs() < 0.05, "on the centre line: {at}");
    assert!(speed.abs() < 0.05 && at.y.abs() < 0.02);
    let world = &play.world().entities;
    let mut query = world.query::<(&Car, &Transform)>();
    let (car, transform) = query.iter().next().unwrap();
    assert!(car
        .vehicle
        .wheels
        .iter()
        .all(|w| w.surface == Surface::Asphalt));
    let forward = transform.rotation * Vec3::NEG_Z;
    assert!(
        (forward - Vec3::NEG_Z).length() < 0.01,
        "facing round the lap"
    );
}

#[test]
fn a_lap_of_the_circuit_is_timed() {
    // Driven by its own instruments: flat out on the straights, braking for each corner.
    let track_text = square_track();
    let scene = "input keys.input\ntrack square.track\nactor car.actor -200 0 30 0 instances=1+6\n";
    let mut play = Play::load(
        &files(&[
            (GAME_FILE, scene),
            ("car.actor", CAR),
            ("gt.vehicle", ""),
            ("keys.input", KEYS),
            ("square.track", &track_text),
        ]),
        |id| baked(id).map(|at| Mat4::from_translation(Vec3::new(-200.0, 0.0, 30.0)) * at),
        Some(&floor()),
    )
    .unwrap_or_else(|e| panic!("{e:?}"));
    let track = play.track().unwrap().clone();
    let mut devices = Devices::new();
    let mut messages = Vec::new();
    let mut seconds = 0.0;
    while seconds < 120.0
        && messages
            .iter()
            .filter(|m: &&String| m.contains(':'))
            .count()
            < 1
    {
        // A simple driver: aim at the centre line a way ahead, and keep to a speed that the
        // corner coming allows.
        let (at, speed) = car_at(&play);
        let place = track.locate(at, None);
        let (target, _, _) = track.place(place.along + 12.0 + speed * 0.5);
        let (_, heading, _) = track.place(place.along);
        let forward = {
            let world = &play.world().entities;
            let mut query = world.query::<(&Transform, &Car)>();
            let (transform, _) = query.iter().next().unwrap();
            transform.rotation * Vec3::NEG_Z
        };
        let to = (target - at).normalize_or_zero();
        let turn = forward.cross(to).y;
        devices.gamepad_axis("left_x", (-turn * 4.0).clamp(-1.0, 1.0));
        // The corners are square: slow for the turn ahead when the line there bends.
        let (_, ahead, _) = track.place(place.along + 20.0 + speed * speed / 24.0);
        let bends = heading.dot(ahead) < 0.9;
        let limit = if bends { 14.0 } else { 70.0 };
        devices.key("KeyW", speed < limit);
        devices.key("KeyS", speed > limit + 2.0);
        play.frame(STEP_SECONDS, &mut devices, |_, _| {});
        seconds += STEP_SECONDS;
        messages.extend(play.take_messages().into_iter().map(|m| m.text));
    }
    assert!(
        messages.iter().any(|m| m == "lap 1 started"),
        "{messages:?}"
    );
    let lap = play.hud().unwrap().lap.clone().unwrap();
    assert_eq!(lap.count, 1, "after {seconds} s: {messages:?}");
    let time = lap.last.unwrap();
    // 1600 metres with four slow corners: between forty seconds and seventy.
    assert!((40.0..70.0).contains(&time), "a lap of {time} s");
    assert!(lap.last_valid && lap.best == lap.last);
    assert!(
        messages.iter().any(|m| m.starts_with("lap 1: 0:")),
        "{messages:?}"
    );
}

#[test]
fn the_car_drives_itself_round_a_lap_and_stays_on_the_road() {
    let track_text = square_track();
    let scene = "input keys.input
track square.track
actor car.actor -200 0 30 0 instances=1+6
";
    let mut play = Play::load(
        &files(&[
            (GAME_FILE, scene),
            ("car.actor", CAR),
            ("gt.vehicle", ""),
            ("keys.input", KEYS),
            ("square.track", &track_text),
        ]),
        |id| baked(id).map(|at| Mat4::from_translation(Vec3::new(-200.0, 0.0, 30.0)) * at),
        Some(&floor()),
    )
    .unwrap_or_else(|e| panic!("{e:?}"));
    let track = play.track().unwrap().clone();
    let mut devices = Devices::new();
    // It does nothing until it is asked to; then one press of the key and it is away.
    run(&mut play, &mut devices, 0.5);
    assert!(car_at(&play).1.abs() < 0.01);
    devices.key("KeyP", true);
    run(&mut play, &mut devices, 0.1);
    devices.key("KeyP", false);
    let mut messages = Vec::new();
    let (mut seconds, mut widest, mut fastest) = (0.0, 0.0_f32, 0.0_f32);
    while seconds < 120.0 && !messages.iter().any(|m: &String| m.starts_with("lap 1:")) {
        play.frame(STEP_SECONDS, &mut devices, |_, _| {});
        seconds += STEP_SECONDS;
        messages.extend(play.take_messages().into_iter().map(|m| m.text));
        let (at, speed) = car_at(&play);
        widest = widest.max(track.locate(at, None).offset.abs());
        fastest = fastest.max(speed);
    }
    let lap = play.hud().unwrap().lap.clone().unwrap();
    assert_eq!(lap.count, 1, "after {seconds} s: {messages:?}");
    assert!(lap.last_valid, "it kept to the road");
    assert!(
        widest < 7.0,
        "it strayed {widest} m from the middle of a road 8 m to each edge"
    );
    assert!(fastest > 45.0, "it only reached {fastest} m/s");
    assert!(
        (35.0..60.0).contains(&lap.last.unwrap()),
        "a lap of {:?} s",
        lap.last
    );
    // Pressed again, the player has the car back: with nothing held, it coasts.
    devices.key("KeyP", true);
    run(&mut play, &mut devices, 0.1);
    devices.key("KeyP", false);
    let before = car_at(&play).1;
    run(&mut play, &mut devices, 2.0);
    assert!(car_at(&play).1 < before, "coasting");
}

#[test]
fn a_manual_car_driving_itself_changes_gear_and_the_player_gets_their_gearbox_back() {
    let track_text = square_track();
    let scene = "input keys.input
track square.track
actor car.actor -200 0 30 0 instances=1+6
";
    let manual = CAR.replace("vehicle gt.vehicle", "vehicle gt.vehicle manual");
    let mut play = Play::load(
        &files(&[
            (GAME_FILE, scene),
            ("car.actor", &manual),
            ("gt.vehicle", ""),
            ("keys.input", KEYS),
            ("square.track", &track_text),
        ]),
        |id| baked(id).map(|at| Mat4::from_translation(Vec3::new(-200.0, 0.0, 30.0)) * at),
        Some(&floor()),
    )
    .unwrap_or_else(|e| panic!("{e:?}"));
    let gearbox = |play: &Play| {
        let world = &play.world().entities;
        let mut query = world.query::<&Car>();
        let car = query.iter().next().expect("a car");
        (car.driver.automatic, car.vehicle.gear())
    };
    let mut devices = Devices::new();
    run(&mut play, &mut devices, 0.5);
    assert!(!gearbox(&play).0, "the player's car is manual");
    devices.key("KeyP", true);
    run(&mut play, &mut devices, 0.1);
    devices.key("KeyP", false);
    let mut highest = 0;
    for _ in 0..(12.0 / STEP_SECONDS) as u32 {
        play.frame(STEP_SECONDS, &mut devices, |_, _| {});
        highest = highest.max(gearbox(&play).1);
    }
    assert!(gearbox(&play).0, "its own driver changes gear");
    assert!(highest >= 3, "it got no higher than gear {highest} in 12 s");
    // Handed back, the gearbox is the player's again, and stays in the gear it was in.
    devices.key("KeyP", true);
    run(&mut play, &mut devices, 0.1);
    devices.key("KeyP", false);
    let (automatic, now) = gearbox(&play);
    assert!(!automatic);
    run(&mut play, &mut devices, 2.0);
    assert_eq!(gearbox(&play).1, now, "nothing changes gear for the player");
}

#[test]
fn a_car_that_falls_far_below_its_track_is_put_back_on_it() {
    // The square circuit forty metres up, over a floor at nought: the car is put down beside
    // the track with nothing under it but the floor far below.
    let track_text = square_track_at(40.0);
    let scene =
        "input keys.input\ntrack square.track\nactor car.actor -200 41 30 0 instances=1+6\n";
    let mut play = Play::load(
        &files(&[
            (GAME_FILE, scene),
            ("car.actor", CAR),
            ("gt.vehicle", ""),
            ("keys.input", KEYS),
            ("square.track", &track_text),
        ]),
        // The floor stays where it is, at nought; the car is up at the track.
        |id| {
            let up = Vec3::new(-200.0, if id == 0 { 0.0 } else { 41.0 }, 30.0);
            baked(id).map(|at| Mat4::from_translation(up) * at)
        },
        Some(&floor()),
    )
    .unwrap_or_else(|e| panic!("{e:?}"));
    let mut devices = Devices::new();
    // It falls; and before it has reached the floor it is back at the track's height, on its
    // centre line, at rest.
    let mut lowest = f32::INFINITY;
    for _ in 0..(4.0 / STEP_SECONDS) as u32 {
        play.frame(STEP_SECONDS, &mut devices, |_, _| {});
        lowest = lowest.min(car_at(&play).0.y);
    }
    assert!(lowest < 40.0 - 12.0 && lowest > 10.0, "it fell to {lowest}");
    // Put back, it falls again: there is still nothing under the track. But each time it is
    // put back on the centre line.
    let mut highest = f32::NEG_INFINITY;
    for _ in 0..(3.0 / STEP_SECONDS) as u32 {
        play.frame(STEP_SECONDS, &mut devices, |_, _| {});
        let (at, _) = car_at(&play);
        highest = highest.max(at.y);
        assert!((at.x + 200.0).abs() < 0.5, "over the centre line: {at}");
    }
    assert!(highest > 39.0, "put back at the track's height: {highest}");
}
