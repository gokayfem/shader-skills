//! The world run as a game: tick order, events, timers, input, deferred changes, and exact
//! replay.

use glam::{Quat, Vec3};
use solstice_world::{
    Entity, Game, GlobalTransform, Input, Parent, Schedule, Snap, TickGroup, Transform, World,
    STEP_SECONDS,
};
use std::cell::RefCell;
use std::rc::Rc;

type Log = Rc<RefCell<Vec<String>>>;

/// A frame long enough for `steps` steps, with a little over.
fn frame_of(steps: u32) -> f64 {
    STEP_SECONDS * f64::from(steps) + 1.0e-4
}

/// A game whose systems write their group's name, and the step, to a log.
fn logging_game() -> (Game, Log) {
    let log: Log = Rc::default();
    let mut schedule = Schedule::new();
    for group in TickGroup::ALL {
        for n in 1..=2 {
            let log = log.clone();
            schedule.add(group, "log", move |world: &mut World| {
                log.borrow_mut()
                    .push(format!("{}:{group:?}{n}", world.step()));
            });
        }
    }
    (Game::new(World::new(), schedule), log)
}

#[test]
fn groups_run_in_order_each_step_and_each_frame() {
    let (mut game, log) = logging_game();
    assert_eq!(game.frame(frame_of(2), &Input::new()).steps, 2);
    let expected: Vec<String> = [
        "1:PrePhysics1",
        "1:PrePhysics2",
        "1:Physics1",
        "1:Physics2",
        "1:PostPhysics1",
        "1:PostPhysics2",
        "2:PrePhysics1",
        "2:PrePhysics2",
        "2:Physics1",
        "2:Physics2",
        "2:PostPhysics1",
        "2:PostPhysics2",
        "2:Animation1",
        "2:Animation2",
        "2:Late1",
        "2:Late2",
    ]
    .map(String::from)
    .to_vec();
    assert_eq!(*log.borrow(), expected);

    // A frame too short for a step still runs the per-frame groups.
    log.borrow_mut().clear();
    assert_eq!(game.frame(STEP_SECONDS * 0.1, &Input::new()).steps, 0);
    assert_eq!(
        *log.borrow(),
        ["2:Animation1", "2:Animation2", "2:Late1", "2:Late2"]
            .map(String::from)
            .to_vec()
    );
}

#[derive(Clone, Debug, PartialEq)]
struct Hit(u32);

/// A game that logs, for every group, the `Hit`s it reads; `send` says which groups send which.
fn hit_game(send: Vec<(TickGroup, u64, u32)>) -> (Game, Log) {
    let seen: Log = Rc::default();
    let mut schedule = Schedule::new();
    for group in TickGroup::ALL {
        if group.is_fixed() {
            let sends: Vec<(u64, u32)> = send
                .iter()
                .filter(|s| s.0 == group)
                .map(|s| (s.1, s.2))
                .collect();
            schedule.add(group, "send", move |world: &mut World| {
                for &(step, hit) in &sends {
                    if world.step() == step {
                        world.send(Hit(hit));
                    }
                }
            });
        }
        let seen = seen.clone();
        schedule.add(group, "read", move |world: &mut World| {
            for hit in world.read::<Hit>() {
                seen.borrow_mut()
                    .push(format!("{}:{group:?} {}", world.step(), hit.0));
            }
        });
    }
    (Game::new(World::new(), schedule), seen)
}

#[test]
fn every_group_sees_every_event_once_whichever_sent_it() {
    use TickGroup::*;
    let (mut game, seen) = hit_game(vec![
        (PrePhysics, 1, 1),
        (Physics, 1, 2),
        (PostPhysics, 1, 3),
    ]);
    for _ in 0..3 {
        game.step(&Input::new());
    }
    assert_eq!(
        *seen.borrow(),
        [
            // Each group sees, within a step, what was sent before it in the step, and the rest
            // the step after.
            "1:Physics 1",
            "1:PostPhysics 1",
            "1:PostPhysics 2",
            "2:PrePhysics 1",
            "2:PrePhysics 2",
            "2:PrePhysics 3",
            "2:Physics 2",
            "2:Physics 3",
            "2:PostPhysics 3",
        ]
        .map(String::from)
        .to_vec()
    );
}

#[test]
fn the_per_frame_groups_see_each_event_once_however_many_steps_a_frame_holds() {
    use TickGroup::*;
    let (mut game, seen) = hit_game(vec![
        (PrePhysics, 1, 1),
        (PostPhysics, 2, 2),
        (Physics, 3, 3),
    ]);
    // Three steps in one frame, then frames with none.
    game.frame(frame_of(3), &Input::new());
    game.frame(STEP_SECONDS * 0.1, &Input::new());
    game.frame(STEP_SECONDS * 0.1, &Input::new());
    let frame: Vec<String> = seen
        .borrow()
        .iter()
        .filter(|line| line.contains("Animation") || line.contains("Late"))
        .cloned()
        .collect();
    assert_eq!(
        frame,
        [
            "3:Animation 1",
            "3:Animation 2",
            "3:Animation 3",
            "3:Late 1",
            "3:Late 2",
            "3:Late 3"
        ]
        .map(String::from)
        .to_vec()
    );
}

#[derive(Clone, Debug, PartialEq)]
struct Bell(u64);

#[test]
fn setup_events_and_timers_reach_every_group_of_their_step() {
    let seen: Log = Rc::default();
    let mut schedule = Schedule::new();
    for group in TickGroup::ALL {
        let seen = seen.clone();
        schedule.add(group, "read", move |world: &mut World| {
            for bell in world.read::<Bell>() {
                seen.borrow_mut()
                    .push(format!("{}:{group:?} {}", world.step(), bell.0));
            }
        });
    }
    schedule.add(TickGroup::PostPhysics, "ring", |world: &mut World| {
        if world.step() == 1 {
            // Two steps on: step 3. Zero steps: still the next step, 2.
            world.after(STEP_SECONDS * 2.0, Bell(3));
            world.after(0.0, Bell(2));
        }
    });
    let mut world = World::new();
    world.send(Bell(0));
    let mut game = Game::new(world, schedule);
    for _ in 0..3 {
        game.step(&Input::new());
    }
    let lines = |step: u64, bell: u64| {
        ["PrePhysics", "Physics", "PostPhysics"].map(|g| format!("{step}:{g} {bell}"))
    };
    let mut expected: Vec<String> = lines(1, 0).to_vec();
    expected.extend(lines(2, 2));
    expected.extend(lines(3, 3));
    assert_eq!(*seen.borrow(), expected);
}

#[test]
#[should_panic(expected = "per-frame tick groups don't send events")]
fn the_per_frame_groups_cannot_send_events() {
    let mut schedule = Schedule::new();
    schedule.add(TickGroup::Animation, "send", |world: &mut World| {
        world.send(Hit(1))
    });
    Game::new(World::new(), schedule).frame(0.0, &Input::new());
}

#[test]
#[should_panic(expected = "per-frame tick groups don't change the world's shape")]
fn the_per_frame_groups_cannot_spawn_or_despawn() {
    let mut schedule = Schedule::new();
    schedule.add(TickGroup::Late, "spawn", |world: &mut World| {
        world.commands().spawn((Hit(1),));
    });
    Game::new(World::new(), schedule).frame(0.0, &Input::new());
}

#[test]
fn spawning_during_a_group_happens_when_it_ends() {
    #[derive(Debug)]
    struct Spark;
    type Counts = Rc<RefCell<Vec<(&'static str, usize)>>>;
    let counts: Counts = Rc::default();
    let named: Rc<RefCell<Option<Entity>>> = Rc::default();
    let mut schedule = Schedule::new();
    let count = |name: &'static str, counts: &Counts| {
        let counts = counts.clone();
        move |world: &mut World| {
            let sparks = world.entities.query::<&Spark>().iter().count();
            counts.borrow_mut().push((name, sparks));
        }
    };
    let name = named.clone();
    schedule.add(TickGroup::PrePhysics, "spawn", move |world: &mut World| {
        world.commands().spawn((Spark,));
        *name.borrow_mut() = Some(world.spawn_later((Spark,)));
    });
    schedule.add(TickGroup::PrePhysics, "count", count("same group", &counts));
    schedule.add(TickGroup::Physics, "count", count("next group", &counts));
    let mut game = Game::new(World::new(), schedule);
    game.step(&Input::new());
    assert_eq!(*counts.borrow(), [("same group", 0), ("next group", 2)]);
    // The entity named early is the one that was spawned.
    let named = named.borrow().unwrap();
    assert!(game.world.entities.get::<&Spark>(named).is_ok());
}

#[test]
fn a_slow_frame_runs_three_steps_and_the_game_falls_behind() {
    let (mut game, _) = logging_game();
    let steps = game.frame(0.5, &Input::new());
    assert_eq!(steps.steps, 3);
    assert!(steps.slowed);
    assert_eq!(game.world.step(), 3);
}

#[test]
fn transforms_are_placed_after_every_step_with_the_step_before_kept() {
    let mut world = World::new();
    let cart = world.spawn((Transform::default(),));
    let rider = world.spawn((Transform::from_translation(Vec3::Y), Parent(cart)));
    let mut schedule = Schedule::new();
    schedule.add(TickGroup::PrePhysics, "roll", move |world: &mut World| {
        world
            .entities
            .get::<&mut Transform>(cart)
            .unwrap()
            .translation
            .x += 1.0;
    });
    let mut game = Game::new(world, schedule);
    let frame = game.frame(STEP_SECONDS * 2.5, &Input::new());
    assert_eq!(frame.steps, 2);
    let global = *game.world.entities.get::<&GlobalTransform>(rider).unwrap();
    assert_eq!(Vec3::from(global.now.translation), Vec3::new(2.0, 1.0, 0.0));
    assert_eq!(
        Vec3::from(global.before.translation),
        Vec3::new(1.0, 1.0, 0.0)
    );
    let drawn = global.interpolated(frame.alpha).w_axis.truncate();
    assert!(
        drawn.abs_diff_eq(Vec3::new(1.5, 1.0, 0.0), 1.0e-3),
        "{drawn}"
    );
}

#[test]
fn each_step_sees_the_input_of_the_frame_it_runs_in() {
    let seen: Rc<RefCell<Vec<f32>>> = Rc::default();
    let mut schedule = Schedule::new();
    let log = seen.clone();
    schedule.add(TickGroup::Physics, "read", move |world: &mut World| {
        log.borrow_mut().push(world.input().value("forward"));
    });
    let mut game = Game::new(World::new(), schedule);
    let mut input = Input::new();
    for value in [0.25, 0.5, 1.0] {
        input.set("forward", value);
        game.frame(frame_of(2), &input);
    }
    assert_eq!(*seen.borrow(), [0.25, 0.25, 0.5, 0.5, 1.0, 1.0]);
}

#[test]
fn a_press_between_steps_reaches_the_next_step_once() {
    let seen: Rc<RefCell<Vec<(u64, bool)>>> = Rc::default();
    let mut schedule = Schedule::new();
    let log = seen.clone();
    schedule.add(TickGroup::PrePhysics, "read", move |world: &mut World| {
        log.borrow_mut()
            .push((world.step(), world.input().pressed("jump")));
    });
    let mut game = Game::new(World::new(), schedule);
    let (mut down, up) = (Input::new(), Input::new());
    down.set("jump", 1.0);
    // Pressed and released again in frames too short for a step: the next step sees the press.
    game.frame(STEP_SECONDS * 0.3, &down);
    game.frame(STEP_SECONDS * 0.3, &up);
    game.frame(STEP_SECONDS * 0.5, &up);
    // Held across two steps: pressed only in the first.
    game.frame(frame_of(1), &down);
    game.frame(frame_of(1), &down);
    // A tap reported straight from the platform, in a frame of two steps: the first sees it.
    let mut tap = Input::new();
    tap.press("jump");
    game.frame(frame_of(2), &tap);
    assert_eq!(
        *seen.borrow(),
        [(1, true), (2, true), (3, false), (4, true), (5, false)]
    );
}

#[test]
fn the_per_frame_groups_see_the_frames_own_input() {
    let seen: Rc<RefCell<Vec<f32>>> = Rc::default();
    let mut schedule = Schedule::new();
    let log = seen.clone();
    schedule.add(TickGroup::Late, "look", move |world: &mut World| {
        log.borrow_mut().push(world.frame_input().value("look"));
    });
    let mut game = Game::new(World::new(), schedule);
    // Frames with no step between them still turn the camera.
    for look in [0.1, 0.2, 0.3] {
        let mut input = Input::new();
        input.set("look", look);
        game.frame(STEP_SECONDS * 0.2, &input);
    }
    assert_eq!(*seen.borrow(), [0.1, 0.2, 0.3]);
}

// --- Replay -------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq)]
struct Velocity(Vec3);
#[derive(Clone, Copy, Debug, PartialEq)]
struct Shot;
#[derive(Clone, Debug, PartialEq)]
struct Fire;
#[derive(Clone, Debug, PartialEq)]
struct Gone;
#[derive(Default)]
struct Score(u32);
/// Drawn only: the per-frame groups write it, and nothing in the steps reads it.
#[derive(Default)]
struct Pose(f32);

/// A small game with input, timers, spawning, despawning, a parent and events: a player steers
/// with "turn" and "forward", and carries a gun; a timer fires a shot from the gun every half
/// second while "fire" is held; shots fly and disappear beyond 20 m, and each that does adds to
/// a score. A per-frame system poses the player for drawing, from what it reads.
fn shooting_game() -> Game {
    let mut world = World::new();
    let player = world.spawn((Transform::default(), Velocity(Vec3::ZERO), Pose::default()));
    let gun = world.spawn((
        Transform::from_translation(Vec3::new(0.3, 1.2, -0.5)),
        Parent(player),
    ));
    world.spawn((Score::default(),));
    world.every(0.5, Fire);
    let mut schedule = Schedule::new();
    schedule.add(TickGroup::PrePhysics, "steer", move |world: &mut World| {
        let (turn, forward) = (world.input().value("turn"), world.input().value("forward"));
        let (transform, velocity) = world
            .entities
            .query_one_mut::<(&mut Transform, &mut Velocity)>(player)
            .unwrap();
        transform.rotation *= Quat::from_rotation_y(turn * 0.05);
        velocity.0 = transform.rotation * Vec3::NEG_Z * forward * 4.0;
    });
    schedule.add(TickGroup::PrePhysics, "fire", move |world: &mut World| {
        if world.read::<Fire>().next().is_none() || !world.input().held("fire") {
            return;
        }
        let Ok(muzzle) = world.entities.get::<&GlobalTransform>(gun).map(|g| g.now) else {
            return;
        };
        let (_, rotation, translation) = muzzle.to_scale_rotation_translation();
        let from = Transform::from_translation(translation).with_rotation(rotation);
        world.spawn_later((Shot, from, Velocity(rotation * Vec3::NEG_Z * 15.0)));
    });
    schedule.add(TickGroup::Physics, "move", |world: &mut World| {
        for (transform, velocity) in world.entities.query_mut::<(&mut Transform, &Velocity)>() {
            transform.translation += velocity.0 * STEP_SECONDS as f32;
        }
    });
    schedule.add(
        TickGroup::PostPhysics,
        "expire",
        move |world: &mut World| {
            let origin = world
                .entities
                .get::<&Transform>(player)
                .unwrap()
                .translation;
            let far: Vec<Entity> = world
                .entities
                .query::<(Entity, &Transform, &Shot)>()
                .iter()
                .filter(|(_, transform, _)| transform.translation.distance(origin) > 20.0)
                .map(|(entity, _, _)| entity)
                .collect();
            for entity in far {
                world.commands().despawn(entity);
                world.send(Gone);
            }
        },
    );
    schedule.add(TickGroup::PrePhysics, "score", |world: &mut World| {
        let gone = world.read::<Gone>().count() as u32;
        for score in world.entities.query_mut::<&mut Score>() {
            score.0 += gone;
        }
    });
    schedule.add(TickGroup::Animation, "pose", move |world: &mut World| {
        let speed = world.entities.get::<&Velocity>(player).unwrap().0.length();
        let shots = world.read::<Gone>().count() as f32;
        world.entities.get::<&mut Pose>(player).unwrap().0 = speed + shots;
    });
    Game::new(world, schedule)
}

/// Everything in the shooting game, exactly, in the world's own order: each entity, its local
/// and world transforms as bits, the score and the step.
fn snapshot(game: &Game) -> (Vec<(u64, Vec<u32>)>, u32, u64) {
    let mut entities = Vec::new();
    for (entity, transform, global) in game
        .world
        .entities
        .query::<(Entity, &Transform, Option<&GlobalTransform>)>()
        .iter()
    {
        let mut bits: Vec<u32> = transform
            .translation
            .to_array()
            .into_iter()
            .chain(transform.rotation.to_array())
            .map(f32::to_bits)
            .collect();
        if let Some(global) = global {
            bits.extend(global.now.to_cols_array().map(f32::to_bits));
            bits.extend(global.before.to_cols_array().map(f32::to_bits));
        }
        entities.push((entity.to_bits().get(), bits));
    }
    let score = game
        .world
        .entities
        .query::<&Score>()
        .iter()
        .next()
        .unwrap()
        .0;
    (entities, score, game.world.step())
}

/// Input that changes as a run goes on.
fn input_at(frame: u32) -> Input {
    let mut input = Input::new();
    input.set("forward", if frame % 200 < 120 { 1.0 } else { 0.3 });
    input.set("turn", ((frame as f32) * 0.05).sin());
    if frame % 90 < 60 {
        input.set("fire", 1.0);
    }
    input
}

#[test]
fn a_recorded_run_replays_exactly_whatever_its_frames_were() {
    // Two runs with frames of different lengths, each replayed with no frames at all. (The two
    // runs differ from each other: a frame of several steps gives each the same input.)
    let mut snapshots = Vec::new();
    for frames in [
        [0.004, 0.016, 0.031, 0.009, 0.07],
        [0.0166, 0.05, 0.001, 0.02, 0.012],
    ] {
        let mut game = shooting_game();
        game.record();
        let mut frame = 0;
        while game.world.step() < 900 {
            game.frame(
                frames[frame as usize % 5],
                &input_at(game.world.step() as u32),
            );
            frame += 1;
        }
        let recording = game.take_recording().unwrap();
        assert_eq!(recording.len(), game.world.step());
        let mut replayed = shooting_game();
        replayed.replay(&recording);
        assert_eq!(snapshot(&replayed), snapshot(&game), "a replay differs");
        snapshots.push(snapshot(&game));
    }
    assert!(
        snapshots[0].1 > 5,
        "the run should have scored: {}",
        snapshots[0].1
    );
}

#[test]
#[should_panic(expected = "a recording replays from the step it started at")]
fn a_recording_must_replay_from_where_it_started() {
    let mut game = shooting_game();
    game.step(&Input::new());
    game.record();
    game.step(&Input::new());
    let recording = game.take_recording().unwrap();
    shooting_game().replay(&recording);
}

#[test]
fn a_snapped_entity_jumps_with_no_motion() {
    let mut world = World::new();
    let ball = world.spawn((Transform::default(),));
    let mut schedule = Schedule::new();
    schedule.add(
        TickGroup::PrePhysics,
        "respawn",
        move |world: &mut World| {
            if world.step() == 2 {
                world
                    .entities
                    .get::<&mut Transform>(ball)
                    .unwrap()
                    .translation
                    .x = 40.0;
                world.commands().insert_one(ball, Snap);
            }
        },
    );
    let mut game = Game::new(world, schedule);
    game.step(&Input::new());
    game.step(&Input::new());
    let global = *game.world.entities.get::<&GlobalTransform>(ball).unwrap();
    assert_eq!(global.now, global.before);
    assert_eq!(Vec3::from(global.now.translation).x, 40.0);
}
