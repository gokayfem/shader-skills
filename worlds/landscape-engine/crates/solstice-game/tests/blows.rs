//! Blows: a truck driven into things, and what the game measures of it (`impact.rs`). The
//! truck is driven by the game as it is played, on the physics as it is played; nothing
//! here is worked out by hand but what the answer should be.

use glam::{Mat4, Vec3};
use solstice_asset::{Collision, CollisionShape, GameFile};
use solstice_game::drive::Handicap;
use solstice_game::{Devices, Impact, Play, GAME_FILE};
use solstice_physics::vehicle::VehicleSpec;
use solstice_world::STEP_SECONDS;

const TRUCK: &str = include_str!("../../solstice-vehicle/tests/wagon.vehicle");
const ACTOR: &str = "name truck\nmesh wagon.glb 1\nvehicle wagon.vehicle\ncamera 0 1.6 0\n";
const KEYS: &str = "throttle key KeyW\nbrake key KeyS\nsteer key KeyD\nsteer key KeyA -1\n";

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

fn spec() -> VehicleSpec {
    VehicleSpec::parse(TRUCK).expect("the truck's file reads")
}

/// The truck on the floor at a height, facing -Z, and the scene's blockers.
fn truck(height: f32, blockers: &str) -> Play {
    let hubs = spec().hubs();
    let scene =
        format!("input keys.input\nactor truck.actor 0 {height} 0 0 instances=1+5\n{blockers}");
    let files: Vec<GameFile> = [
        (GAME_FILE, scene.as_str()),
        ("truck.actor", ACTOR),
        ("wagon.vehicle", TRUCK),
        ("keys.input", KEYS),
    ]
    .iter()
    .map(|(name, text)| GameFile {
        name: (*name).to_owned(),
        text: (*text).to_owned(),
    })
    .collect();
    let placed = Mat4::from_translation(Vec3::Y * height);
    // The floor is the scene's first instance, and the truck's body its second.
    let baked = move |id: u32| match id {
        0 => Some(Mat4::IDENTITY),
        1 => Some(placed),
        2..=5 => Some(placed * Mat4::from_translation(hubs[id as usize - 2])),
        _ => None,
    };
    Play::load(&files, baked, Some(&floor())).unwrap_or_else(|e| panic!("{e:?}"))
}

/// Runs the game, and gives every blow the truck took.
fn run(play: &mut Play, devices: &mut Devices, seconds: f64) -> Vec<Impact> {
    let mut blows = Vec::new();
    for _ in 0..(seconds / STEP_SECONDS).round() as u32 {
        play.frame(STEP_SECONDS, devices, |_, _| {});
        blows.extend(play.take_impacts());
    }
    blows
}

/// The hardest blow to the body among some.
fn hardest(blows: &[Impact]) -> Impact {
    let body = blows.iter().filter(|blow| blow.wheel.is_none());
    *body
        .max_by(|a, b| a.force.total_cmp(&b.force))
        .expect("the body was hit")
}

/// How fast the truck is going over the ground, in metres a second.
fn speed(play: &Play) -> f32 {
    play.hud().expect("it is driven").telemetry.speed
}

/// Drives at a wall across the road so far ahead, and gives how fast it was going when it
/// got there and every blow from then on.
fn into_a_wall(play: &mut Play, ahead: f32) -> (f32, Vec<Impact>) {
    let nose = spec().body_at.z - spec().body_size.z * 0.5;
    let mut devices = Devices::new();
    // Settled on its springs first: it is put down a little above the ground.
    let settling = run(play, &mut devices, 1.5);
    assert!(
        settling.iter().all(|blow| blow.wheel.is_some()),
        "standing on its wheels, its body meets nothing: {settling:?}"
    );
    devices.key("KeyW", true);
    let mut going = 0.0;
    for _ in 0..3000 {
        let (at, _) = play.car_pose().expect("a truck");
        // A step short of the wall: how fast it is going as it gets there.
        if at.z + nose - speed(play) * STEP_SECONDS as f32 * 2.0 < -ahead {
            going = speed(play);
            break;
        }
        let before = run(play, &mut devices, STEP_SECONDS);
        assert!(before.iter().all(|blow| blow.wheel.is_some()), "{before:?}");
    }
    assert!(going > 0.5, "it got to the wall");
    devices.key("KeyW", false);
    (going, run(play, &mut devices, 1.0))
}

#[test]
fn a_wall_hit_square_on_is_a_blow_to_the_nose_as_hard_as_the_truck_was_going() {
    let spec = spec();
    // A wall across the road: slowly, and then three times as fast.
    let mut slow = truck(0.1, "blocker 0 1.5 -12 20 3 1 0\n");
    let (crawl, blows) = into_a_wall(&mut slow, 11.5);
    let tap = hardest(&blows);
    let mut fast = truck(0.1, "blocker 0 1.5 -62 20 3 1 0\n");
    let (going, blows) = into_a_wall(&mut fast, 61.5);
    let blow = hardest(&blows);
    assert!(going > crawl * 1.5, "{crawl} m/s and then {going} m/s");

    // It landed on the nose, about the middle of it, and pushed the truck back.
    let nose = spec.body_at.z - spec.body_size.z * 0.5;
    assert!((blow.at.z - nose).abs() < 0.1, "on its nose: {}", blow.at);
    assert!(blow.at.x.abs() < 0.35, "about its middle: {}", blow.at);
    assert!(blow.push.z > 0.95, "pushed back: {}", blow.push);
    // The wall took all its speed off it, and a little more if it bounced: its impulse is
    // its mass times that.
    let stopped = spec.mass * going;
    assert!(
        blow.impulse > stopped * 0.85 && blow.impulse < stopped * 1.35,
        "{} N s for {stopped} N s of going",
        blow.impulse
    );
    // Faster is harder, by as much.
    let share = (blow.impulse / tap.impulse) / (going / crawl);
    assert!(share > 0.7 && share < 1.4, "{share}");
    // The whole of its nose met the wall: the patch is as wide as the truck, all but, and
    // most of the height of its body (it meets a wall a little nose up or nose down).
    let across = blow.high.x - blow.low.x;
    assert!(
        across > spec.body_size.x * 0.8,
        "{} to {}",
        blow.low,
        blow.high
    );
    assert!(blow.high.y - blow.low.y > spec.body_size.y * 0.5);
    assert!(blow.high.z < nose + 0.3, "and no deeper than its nose");
    // And it has stopped.
    assert!(speed(&fast).abs() < 1.0);
}

#[test]
fn a_post_met_with_one_corner_is_a_blow_to_that_corner() {
    let spec = spec();
    // A post a metre square, in front of the truck's right headlamp only.
    let mut play = truck(0.1, "blocker 1.1 1.5 -40 1 3 1 0\n");
    let (_, blows) = into_a_wall(&mut play, 39.5);
    let blow = hardest(&blows);
    let nose = spec.body_at.z - spec.body_size.z * 0.5;
    assert!((blow.at.z - nose).abs() < 0.25, "on its nose: {}", blow.at);
    assert!(blow.at.x > 0.45, "on its right: {}", blow.at);
    // The post is in front of its right side only, and so is the patch.
    assert!(blow.low.x > 0.4, "{} to {}", blow.low, blow.high);
    assert!(blow.push.z > 0.7, "pushed back: {}", blow.push);
}

#[test]
fn a_drop_is_a_blow_up_through_the_wheels_and_driving_on_the_flat_is_none() {
    // Driven off down the flat for ten seconds: nothing strikes it.
    let mut play = truck(0.1, "");
    let mut devices = Devices::new();
    run(&mut play, &mut devices, 1.5);
    devices.key("KeyW", true);
    let driven = run(&mut play, &mut devices, 10.0);
    assert!(driven.is_empty(), "{driven:?}");

    // Dropped from a height, every wheel is struck, harder than it stands: and from three
    // times as high, harder again.
    let spec = spec();
    let (front, rear) = spec.static_loads(9.81);
    let landing = |height: f32| -> [f32; 4] {
        let mut dropped = truck(height, "");
        let blows = run(&mut dropped, &mut Devices::new(), 3.0);
        std::array::from_fn(|wheel| {
            let struck = blows.iter().filter(|blow| blow.wheel == Some(wheel));
            struck.map(|blow| blow.force).fold(0.0, f32::max)
        })
    };
    let (low, high) = (landing(0.8), landing(2.4));
    for wheel in 0..4 {
        let rest = if wheel < 2 { front } else { rear };
        assert!(low[wheel] > rest * 2.0, "wheel {wheel}: {} N", low[wheel]);
        assert!(high[wheel] > low[wheel] * 1.4, "wheel {wheel}: {high:?}");
    }
}

#[test]
fn a_handicap_is_felt_at_the_wheel_and_under_the_foot() {
    // Two trucks driven off flat out with the wheel straight: one sound, one that pulls to
    // its right and has lost half its power.
    let drive = |handicap: Option<Handicap>| {
        let mut play = truck(0.1, "");
        let mut devices = Devices::new();
        run(&mut play, &mut devices, 1.5);
        if let Some(handicap) = handicap {
            play.set_handicap(handicap);
        }
        devices.key("KeyW", true);
        run(&mut play, &mut devices, 5.0);
        let at = play.car_pose().expect("a truck").0;
        (at, speed(&play))
    };
    let (sound, pace) = drive(None);
    let (wrecked, slower) = drive(Some(Handicap {
        pull: 0.25,
        power: 0.5,
    }));
    assert!(sound.x.abs() < 0.5, "a sound truck runs straight: {sound}");
    assert!(wrecked.x > 2.0, "this one has gone to its right: {wrecked}");
    assert!(slower < pace * 0.85, "{slower} m/s against {pace} m/s");
}
