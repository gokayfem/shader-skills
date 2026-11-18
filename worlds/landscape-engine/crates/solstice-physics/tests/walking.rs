//! A character walking the baked world, and actors colliding with it.

use glam::{Mat4, Quat, Vec3};
use solstice_asset::{Collision, CollisionShape, Heightfield, NO_SHAPE};
use solstice_physics::{ActorCollider, BodyKind, Character, Overlap, Physics, Shape};
use solstice_world::{
    Entity, Game, Input, Schedule, TickGroup, Transform, Walk, World, STEP_SECONDS,
};
use std::cell::RefCell;
use std::rc::Rc;

/// A flat square floor at height zero, 40 m across, as a triangle mesh.
fn floor() -> CollisionShape {
    let s = 20.0;
    CollisionShape {
        hull: false,
        vertices: vec![[-s, 0.0, -s], [s, 0.0, -s], [s, 0.0, s], [-s, 0.0, s]],
        triangles: vec![[0, 2, 1], [0, 3, 2]],
    }
}

/// A closed box from its low corner to its high corner, as a triangle mesh.
fn solid(low: Vec3, high: Vec3) -> CollisionShape {
    let vertices = (0..8)
        .map(|i| {
            [
                if i & 1 == 0 { low.x } else { high.x },
                if i & 2 == 0 { low.y } else { high.y },
                if i & 4 == 0 { low.z } else { high.z },
            ]
        })
        .collect();
    let triangles = vec![
        [0, 2, 1],
        [1, 2, 3],
        [4, 5, 6],
        [5, 7, 6],
        [0, 1, 4],
        [1, 5, 4],
        [2, 6, 3],
        [3, 6, 7],
        [0, 4, 2],
        [2, 4, 6],
        [1, 3, 5],
        [3, 7, 5],
    ];
    CollisionShape {
        hull: false,
        vertices,
        triangles,
    }
}

/// A world of these shapes, each once at the origin, and its physics.
fn world_of(shapes: Vec<CollisionShape>) -> (Game, Rc<RefCell<Physics>>) {
    let count = shapes.len() as u32;
    let collision = Collision {
        instance_shapes: (0..count).collect(),
        shapes,
        heightfield: None,
    };
    with_collision(&collision, |_| Some(Mat4::IDENTITY))
}

fn with_collision(
    collision: &Collision,
    instance: impl Fn(u32) -> Option<Mat4>,
) -> (Game, Rc<RefCell<Physics>>) {
    let physics = Rc::new(RefCell::new(Physics::new()));
    physics.borrow_mut().add_scene(collision, instance);
    let mut schedule = Schedule::new();
    Physics::install(&physics, &mut schedule);
    (Game::new(World::new(), schedule), physics)
}

const WALKER: Character = Character {
    radius: 0.3,
    height: 1.8,
};

fn walker(game: &mut Game, at: Vec3) -> Entity {
    game.world
        .spawn((Transform::from_translation(at), WALKER, Walk::default()))
}

fn steps(game: &mut Game, count: u32) {
    for _ in 0..count {
        game.step(&Input::new());
    }
}

fn place(game: &Game, entity: Entity) -> Vec3 {
    game.world
        .entities
        .get::<&Transform>(entity)
        .unwrap()
        .translation
}

fn walk(game: &mut Game, entity: Entity, velocity: Vec3) {
    game.world.entities.get::<&mut Walk>(entity).unwrap().0 = velocity;
}

#[test]
fn a_character_falls_to_the_floor_and_stands_on_it() {
    let (mut game, _) = world_of(vec![floor()]);
    let hero = walker(&mut game, Vec3::new(0.0, 3.0, 0.0));
    steps(&mut game, 20);
    assert!(
        place(&game, hero).y > 1.0,
        "still falling: {}",
        place(&game, hero)
    );
    steps(&mut game, 100);
    let y = place(&game, hero).y;
    assert!(y.abs() < 0.05, "stands on the floor: {y}");
    steps(&mut game, 60);
    assert!((place(&game, hero).y - y).abs() < 1.0e-3, "and stays there");
}

#[test]
fn a_character_walks_as_far_as_it_is_told() {
    let (mut game, _) = world_of(vec![floor()]);
    let hero = walker(&mut game, Vec3::new(0.0, 0.05, 0.0));
    steps(&mut game, 10);
    walk(&mut game, hero, Vec3::new(2.0, 0.0, 0.0));
    steps(&mut game, 60);
    let at = place(&game, hero);
    assert!(
        (at.x - 2.0).abs() < 0.05,
        "two metres in a second at 2 m/s: {at}"
    );
    assert!(at.y.abs() < 0.05 && at.z.abs() < 1.0e-3, "{at}");
    // What a walk says of going up is a jump: off the ground, and back down onto it.
    walk(&mut game, hero, Vec3::new(0.0, 5.0, 0.0));
    steps(&mut game, 12);
    let up = place(&game, hero).y;
    assert!(up > 0.3, "it has left the ground: {up}");
    walk(&mut game, hero, Vec3::ZERO);
    steps(&mut game, 120);
    assert!(place(&game, hero).y.abs() < 0.05, "{}", place(&game, hero));
}

#[test]
fn a_wall_stops_a_character_and_it_slides_along_it() {
    // A wall along z, its face at x = 2.
    let wall = solid(Vec3::new(2.0, 0.0, -10.0), Vec3::new(3.0, 3.0, 10.0));
    let (mut game, _) = world_of(vec![floor(), wall]);
    let hero = walker(&mut game, Vec3::new(0.0, 0.05, 0.0));
    steps(&mut game, 10);
    walk(&mut game, hero, Vec3::new(3.0, 0.0, 0.0));
    steps(&mut game, 120);
    let at = place(&game, hero);
    assert!(
        at.x < 2.0 - 0.3 + 0.05 && at.x > 1.5,
        "stopped by the face, a radius off: {at}"
    );
    walk(&mut game, hero, Vec3::new(3.0, 0.0, 3.0));
    let before = place(&game, hero);
    steps(&mut game, 60);
    let at = place(&game, hero);
    assert!(at.z > before.z + 2.0, "slides along it: {at}");
    assert!(at.x < 2.0, "without going through: {at}");
}

#[test]
fn a_blocker_is_an_invisible_wall() {
    let (mut game, physics) = world_of(vec![floor()]);
    physics
        .borrow_mut()
        .add_blocker(Vec3::new(2.5, 2.0, 0.0), Vec3::new(1.0, 4.0, 40.0), 0.0);
    let hero = walker(&mut game, Vec3::new(0.0, 0.05, 0.0));
    steps(&mut game, 10);
    walk(&mut game, hero, Vec3::new(3.0, 0.0, 0.0));
    steps(&mut game, 120);
    assert!(place(&game, hero).x < 2.0, "{}", place(&game, hero));
}

#[test]
fn a_blocker_turns_by_its_yaw() {
    // Long along x once turned a quarter: it now spans z = -20..20 at x = 2.5.
    let (mut game, physics) = world_of(vec![floor()]);
    physics.borrow_mut().add_blocker(
        Vec3::new(2.5, 2.0, 0.0),
        Vec3::new(40.0, 4.0, 1.0),
        std::f32::consts::FRAC_PI_2,
    );
    let hero = walker(&mut game, Vec3::new(0.0, 0.05, 5.0));
    steps(&mut game, 10);
    walk(&mut game, hero, Vec3::new(3.0, 0.0, 0.0));
    steps(&mut game, 120);
    assert!(place(&game, hero).x < 2.0, "{}", place(&game, hero));
}

#[test]
fn a_character_climbs_a_step_but_not_a_wall() {
    let low = solid(Vec3::new(2.0, 0.0, -5.0), Vec3::new(8.0, 0.25, 5.0));
    let (mut game, _) = world_of(vec![floor(), low]);
    let hero = walker(&mut game, Vec3::new(0.0, 0.05, 0.0));
    steps(&mut game, 10);
    walk(&mut game, hero, Vec3::new(2.0, 0.0, 0.0));
    steps(&mut game, 150);
    let at = place(&game, hero);
    assert!(at.x > 3.0, "over the edge: {at}");
    assert!((at.y - 0.25).abs() < 0.06, "standing on top: {at}");

    let high = solid(Vec3::new(2.0, 0.0, -5.0), Vec3::new(8.0, 0.6, 5.0));
    let (mut game, _) = world_of(vec![floor(), high]);
    let hero = walker(&mut game, Vec3::new(0.0, 0.05, 0.0));
    steps(&mut game, 10);
    walk(&mut game, hero, Vec3::new(2.0, 0.0, 0.0));
    steps(&mut game, 150);
    let at = place(&game, hero);
    assert!(at.x < 2.0 && at.y < 0.1, "too high to step up: {at}");
}

/// Solid steps going up along -x from x = 0, each `rise` high and `tread` deep.
fn stairs(count: u32, rise: f32, tread: f32) -> Vec<CollisionShape> {
    (0..count)
        .map(|i| {
            let i = i as f32;
            solid(
                Vec3::new(-(i + 1.0) * tread, 0.0, -2.0),
                Vec3::new(-i * tread, (i + 1.0) * rise, 2.0),
            )
        })
        .collect()
}

#[test]
fn a_character_climbs_a_staircase() {
    // A fort's stair, its treads narrower than the character is wide.
    for (rise, tread) in [
        (0.25, 0.3),
        (0.2, 0.3),
        (0.17, 0.28),
        (0.3, 0.33),
        (0.17, 0.5),
    ] {
        let mut shapes = vec![floor()];
        shapes.extend(stairs(12, rise, tread));
        let (mut game, _) = world_of(shapes);
        let hero = walker(&mut game, Vec3::new(1.5, 0.05, 0.0));
        steps(&mut game, 10);
        walk(&mut game, hero, Vec3::new(-1.6, 0.0, 0.0));
        let mut highest = 0.0_f32;
        for _ in 0..900 {
            steps(&mut game, 1);
            highest = highest.max(place(&game, hero).y);
        }
        let top = 12.0 * rise;
        assert!(
            (highest - top).abs() < 0.1,
            "rise {rise}, tread {tread}: got up to {highest}, top is {top}"
        );
    }
}

/// A ramp rising along +x from the origin, by `degrees`.
fn ramp(degrees: f32) -> CollisionShape {
    let rise = 20.0 * degrees.to_radians().tan();
    CollisionShape {
        hull: false,
        vertices: vec![
            [0.0, 0.0, -5.0],
            [20.0, rise, -5.0],
            [20.0, rise, 5.0],
            [0.0, 0.0, 5.0],
        ],
        triangles: vec![[0, 2, 1], [0, 3, 2]],
    }
}

#[test]
fn a_character_walks_up_a_gentle_slope_and_is_refused_a_steep_one() {
    let (mut game, _) = world_of(vec![floor(), ramp(30.0)]);
    let hero = walker(&mut game, Vec3::new(-1.0, 0.05, 0.0));
    steps(&mut game, 10);
    walk(&mut game, hero, Vec3::new(2.0, 0.0, 0.0));
    steps(&mut game, 180);
    let at = place(&game, hero);
    assert!(at.x > 3.0, "{at}");
    let expected = at.x * 30_f32.to_radians().tan();
    assert!((at.y - expected).abs() < 0.2, "follows the slope: {at}");

    let (mut game, _) = world_of(vec![floor(), ramp(60.0)]);
    let hero = walker(&mut game, Vec3::new(-1.0, 0.05, 0.0));
    steps(&mut game, 10);
    walk(&mut game, hero, Vec3::new(2.0, 0.0, 0.0));
    steps(&mut game, 180);
    let at = place(&game, hero);
    assert!(at.y < 0.8, "can't climb it: {at}");
}

#[test]
fn the_heightfield_lies_where_the_ground_is() {
    // Height is x + 2z, over x from 10 to 14 and z from -2 to 2.
    let (columns, rows) = (5_u32, 3_u32);
    let mut heights = Vec::new();
    for row in 0..rows {
        for column in 0..columns {
            heights.push((10.0 + column as f32) + 2.0 * (-2.0 + row as f32 * 2.0));
        }
    }
    let collision = Collision {
        heightfield: Some(Heightfield {
            columns,
            rows,
            origin: [10.0, -2.0],
            spacing: [1.0, 2.0],
            heights,
        }),
        ..Default::default()
    };
    let (mut game, physics) = with_collision(&collision, |_| None);
    steps(&mut game, 1);
    let physics = physics.borrow();
    for (x, z) in [(11.0, 0.0), (13.5, 1.0), (10.25, -1.5)] {
        let y = physics.ground_below(Vec3::new(x, 100.0, z)).unwrap();
        assert!((y - (x + 2.0 * z)).abs() < 1.0e-3, "at {x}, {z}: {y}");
    }
    assert!(
        physics.ground_below(Vec3::new(9.0, 100.0, 0.0)).is_none(),
        "ends at its edge"
    );
    assert!(
        physics.ground_below(Vec3::new(11.0, 100.0, 3.0)).is_none(),
        "ends at its edge"
    );
}

#[test]
fn instances_are_placed_turned_and_scaled_and_skipped() {
    // A unit square at height 0 around the origin: instance 0 doubled and raised to 3,
    // instance 1 at x = 10, and instance 2 has no collision.
    let square = CollisionShape {
        hull: false,
        vertices: vec![
            [-1.0, 0.0, -1.0],
            [1.0, 0.0, -1.0],
            [1.0, 0.0, 1.0],
            [-1.0, 0.0, 1.0],
        ],
        triangles: vec![[0, 2, 1], [0, 3, 2]],
    };
    let collision = Collision {
        shapes: vec![square],
        instance_shapes: vec![0, 0, NO_SHAPE],
        heightfield: None,
    };
    let transforms = [
        Mat4::from_scale_rotation_translation(Vec3::splat(2.0), Quat::IDENTITY, Vec3::Y * 3.0),
        Mat4::from_translation(Vec3::X * 10.0),
        Mat4::from_translation(Vec3::X * 20.0),
    ];
    let (mut game, physics) = with_collision(&collision, |i| transforms.get(i as usize).copied());
    steps(&mut game, 1);
    let physics = physics.borrow();
    assert_eq!(
        physics.ground_below(Vec3::new(1.5, 50.0, 0.0)),
        Some(3.0),
        "scaled to 2"
    );
    assert_eq!(physics.ground_below(Vec3::new(10.5, 50.0, 0.0)), Some(0.0));
    assert_eq!(
        physics.ground_below(Vec3::new(11.5, 50.0, 0.0)),
        None,
        "not scaled"
    );
    assert_eq!(
        physics.ground_below(Vec3::new(20.0, 50.0, 0.0)),
        None,
        "no shape"
    );
    assert_eq!(physics.collider_count(), 2);
}

#[test]
fn a_hull_is_solid_inside_its_points() {
    let hull = CollisionShape {
        hull: true,
        vertices: vec![
            [-1.0, 0.0, -1.0],
            [1.0, 0.0, -1.0],
            [1.0, 0.0, 1.0],
            [-1.0, 0.0, 1.0],
            [0.0, 2.0, 0.0],
        ],
        triangles: vec![],
    };
    let (mut game, physics) = world_of(vec![hull]);
    steps(&mut game, 1);
    let y = physics
        .borrow()
        .ground_below(Vec3::new(0.0, 50.0, 0.0))
        .unwrap();
    assert!((y - 2.0).abs() < 1.0e-3, "{y}");
}

#[test]
fn a_door_blocks_until_it_moves_away() {
    let (mut game, _) = world_of(vec![floor()]);
    let door = game.world.spawn((
        Transform::from_translation(Vec3::new(3.0, 1.5, 0.0)),
        ActorCollider {
            shape: Shape::Box(Vec3::new(0.5, 3.0, 8.0)),
            kind: BodyKind::Kinematic,
            offset: Vec3::ZERO,
        },
    ));
    let hero = walker(&mut game, Vec3::new(0.0, 0.05, 0.0));
    steps(&mut game, 10);
    walk(&mut game, hero, Vec3::new(3.0, 0.0, 0.0));
    steps(&mut game, 120);
    assert!(place(&game, hero).x < 3.0, "{}", place(&game, hero));
    // Lifted out of the way, by its script writing its transform.
    game.world
        .entities
        .get::<&mut Transform>(door)
        .unwrap()
        .translation
        .y = 10.0;
    steps(&mut game, 60);
    assert!(place(&game, hero).x > 3.5, "{}", place(&game, hero));
}

#[test]
fn a_collider_can_sit_off_the_actors_origin() {
    // A door hinged at its origin, x = 3, its box 4 m out along +z and turned with it.
    let (mut game, _) = world_of(vec![floor()]);
    let door = game.world.spawn((
        Transform::from_translation(Vec3::new(3.0, 0.0, 0.0)),
        ActorCollider {
            shape: Shape::Box(Vec3::new(0.2, 3.0, 8.0)),
            kind: BodyKind::Kinematic,
            offset: Vec3::new(0.0, 1.5, 4.0),
        },
    ));
    // In front of the box, at z = 2: blocked. At z = -2, past the hinge: not.
    let blocked = walker(&mut game, Vec3::new(1.0, 0.05, 2.0));
    let free = walker(&mut game, Vec3::new(1.0, 0.05, -2.0));
    steps(&mut game, 10);
    walk(&mut game, blocked, Vec3::new(3.0, 0.0, 0.0));
    walk(&mut game, free, Vec3::new(3.0, 0.0, 0.0));
    steps(&mut game, 90);
    assert!(place(&game, blocked).x < 3.0, "{}", place(&game, blocked));
    assert!(place(&game, free).x > 4.0, "{}", place(&game, free));
    // Swung a quarter turn on its hinge, the box now lies along +x, out of the way of z = 2.
    game.world
        .entities
        .get::<&mut Transform>(door)
        .unwrap()
        .rotation = Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);
    steps(&mut game, 60);
    assert!(place(&game, blocked).x > 3.3, "{}", place(&game, blocked));
}

#[derive(Clone, Default)]
struct Seen(Rc<RefCell<Vec<(Entity, Entity, bool)>>>);

#[test]
fn a_sensor_reports_who_enters_and_leaves() {
    let physics = Rc::new(RefCell::new(Physics::new()));
    physics.borrow_mut().add_scene(
        &Collision {
            shapes: vec![floor()],
            instance_shapes: vec![0],
            heightfield: None,
        },
        |_| Some(Mat4::IDENTITY),
    );
    let mut schedule = Schedule::new();
    Physics::install(&physics, &mut schedule);
    let seen = Seen::default();
    let log = seen.clone();
    schedule.add(TickGroup::PostPhysics, "seen", move |world| {
        for o in world.read_cloned::<Overlap>() {
            log.0.borrow_mut().push((o.sensor, o.other, o.entered));
        }
    });
    let mut game = Game::new(World::new(), schedule);
    let plate = game.world.spawn((
        Transform::from_translation(Vec3::new(3.0, 1.0, 0.0)),
        ActorCollider {
            shape: Shape::Box(Vec3::new(1.0, 2.0, 4.0)),
            kind: BodyKind::Sensor,
            offset: Vec3::ZERO,
        },
    ));
    let hero = walker(&mut game, Vec3::new(0.0, 0.05, 0.0));
    steps(&mut game, 10);
    assert!(seen.0.borrow().is_empty());
    walk(&mut game, hero, Vec3::new(3.0, 0.0, 0.0));
    steps(&mut game, 60);
    assert!(
        place(&game, hero).x > 2.9,
        "walks straight through: {}",
        place(&game, hero)
    );
    assert_eq!(*seen.0.borrow(), vec![(plate, hero, true)]);
    steps(&mut game, 60);
    assert_eq!(
        *seen.0.borrow(),
        vec![(plate, hero, true), (plate, hero, false)]
    );
}

#[test]
fn a_sensor_notices_what_falls_through_it_too() {
    // A falling crate reports no events of its own: the sensor has to.
    let physics = Rc::new(RefCell::new(Physics::new()));
    physics.borrow_mut().add_scene(
        &Collision {
            shapes: vec![floor()],
            instance_shapes: vec![0],
            heightfield: None,
        },
        |_| Some(Mat4::IDENTITY),
    );
    let mut schedule = Schedule::new();
    Physics::install(&physics, &mut schedule);
    let seen = Seen::default();
    let log = seen.clone();
    schedule.add(TickGroup::PostPhysics, "seen", move |world| {
        for o in world.read_cloned::<Overlap>() {
            log.0.borrow_mut().push((o.sensor, o.other, o.entered));
        }
    });
    let mut game = Game::new(World::new(), schedule);
    let zone = game.world.spawn((
        Transform::from_translation(Vec3::new(0.0, 2.0, 0.0)),
        ActorCollider {
            shape: Shape::Box(Vec3::new(4.0, 1.0, 4.0)),
            kind: BodyKind::Sensor,
            offset: Vec3::ZERO,
        },
    ));
    let falling = game.world.spawn((
        Transform::from_translation(Vec3::new(0.0, 5.0, 0.0)),
        ActorCollider {
            shape: Shape::Box(Vec3::splat(0.5)),
            kind: BodyKind::Dynamic(5.0),
            offset: Vec3::ZERO,
        },
    ));
    steps(&mut game, 120);
    assert_eq!(
        *seen.0.borrow(),
        vec![(zone, falling, true), (zone, falling, false)]
    );
}

#[test]
fn a_dynamic_actor_falls_and_comes_to_rest() {
    let (mut game, _) = world_of(vec![floor()]);
    let crate_ = game.world.spawn((
        Transform::from_translation(Vec3::new(0.0, 4.0, 0.0)),
        ActorCollider {
            shape: Shape::Box(Vec3::splat(1.0)),
            kind: BodyKind::Dynamic(20.0),
            offset: Vec3::ZERO,
        },
    ));
    steps(&mut game, 300);
    let y = place(&game, crate_).y;
    assert!((y - 0.5).abs() < 0.05, "resting on the floor: {y}");
}

#[test]
fn a_character_moved_by_a_script_goes_there_at_once() {
    let (mut game, _) = world_of(vec![floor()]);
    let hero = walker(&mut game, Vec3::new(0.0, 0.05, 0.0));
    steps(&mut game, 10);
    game.world
        .entities
        .get::<&mut Transform>(hero)
        .unwrap()
        .translation = Vec3::new(10.0, 0.05, -4.0);
    steps(&mut game, 2);
    assert!(
        place(&game, hero).abs_diff_eq(Vec3::new(10.0, 0.0, -4.0), 0.1),
        "{}",
        place(&game, hero)
    );
}

#[test]
fn removed_actors_leave_physics() {
    let (mut game, physics) = world_of(vec![floor()]);
    let hero = walker(&mut game, Vec3::new(0.0, 0.05, 0.0));
    steps(&mut game, 2);
    assert_eq!(physics.borrow().collider_count(), 2);
    game.world.entities.despawn(hero).unwrap();
    steps(&mut game, 2);
    assert_eq!(physics.borrow().collider_count(), 1);
    assert_eq!(
        physics.borrow().tracked_colliders(),
        0,
        "nothing is kept of it"
    );
}

#[test]
fn a_run_comes_out_the_same_every_time() {
    let run = || {
        let wall = solid(Vec3::new(3.0, 0.0, -1.0), Vec3::new(4.0, 3.0, 1.0));
        let (mut game, _) = world_of(vec![floor(), wall]);
        let hero = walker(&mut game, Vec3::new(0.0, 2.0, 0.1));
        let crate_ = game.world.spawn((
            Transform::from_translation(Vec3::new(1.0, 3.0, 2.0)),
            ActorCollider {
                shape: Shape::Box(Vec3::splat(1.0)),
                kind: BodyKind::Dynamic(10.0),
                offset: Vec3::ZERO,
            },
        ));
        for i in 0..300 {
            let t = i as f32 * STEP_SECONDS as f32;
            walk(&mut game, hero, Vec3::new(2.0, 0.0, (t * 3.0).sin()));
            steps(&mut game, 1);
        }
        (
            place(&game, hero).to_array().map(f32::to_bits),
            place(&game, crate_).to_array().map(f32::to_bits),
        )
    };
    assert_eq!(run(), run());
}

/// A crate dropped from 3 m above the origin, where it ended up after a few seconds.
fn dropped_on(collision: &Collision, instance: impl Fn(u32) -> Option<Mat4>) -> f32 {
    let (mut game, physics) = with_collision(collision, instance);
    assert!(
        physics.borrow().warnings.is_empty(),
        "{:?}",
        physics.borrow().warnings
    );
    let crate_ = game.world.spawn((
        Transform::from_translation(Vec3::new(0.5, 3.0, 0.5)),
        ActorCollider {
            shape: Shape::Box(Vec3::splat(1.0)),
            kind: BodyKind::Dynamic(10.0),
            offset: Vec3::ZERO,
        },
    ));
    steps(&mut game, 240);
    place(&game, crate_).y
}

#[test]
fn a_dynamic_body_lands_on_either_side_of_a_mesh() {
    // The floor's triangles face up, and then face down: scanned meshes are often open behind.
    let up = floor();
    let mut down = floor();
    for t in &mut down.triangles {
        t.swap(1, 2);
    }
    for shape in [up, down] {
        let collision = Collision {
            shapes: vec![shape],
            instance_shapes: vec![0],
            heightfield: None,
        };
        let y = dropped_on(&collision, |_| Some(Mat4::IDENTITY));
        assert!((y - 0.5).abs() < 0.05, "{y}");
    }
}

#[test]
fn a_mirrored_instance_still_holds_things_up() {
    let collision = Collision {
        shapes: vec![floor()],
        instance_shapes: vec![0],
        heightfield: None,
    };
    let mirrored = Mat4::from_scale(Vec3::new(-1.0, 1.0, 1.0));
    let y = dropped_on(&collision, |_| Some(mirrored));
    assert!((y - 0.5).abs() < 0.05, "{y}");
}

#[test]
fn degenerate_instances_and_far_flung_shapes_are_left_out_with_a_word() {
    let far = CollisionShape {
        hull: false,
        vertices: vec![[1.0e7, 0.0, 0.0], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0]],
        triangles: vec![[0, 1, 2]],
    };
    let collision = Collision {
        shapes: vec![floor(), far],
        instance_shapes: vec![0, 0, 1],
        heightfield: None,
    };
    let flat = Mat4::from_scale(Vec3::new(1.0, 0.0, 1.0));
    let transforms = [flat, Mat4::from_cols_array(&[f32::NAN; 16]), Mat4::IDENTITY];
    let (_, physics) = with_collision(&collision, |i| transforms.get(i as usize).copied());
    let physics = physics.borrow();
    assert_eq!(physics.collider_count(), 0);
    assert_eq!(physics.warnings.len(), 3, "{:?}", physics.warnings);
}

#[test]
fn instances_at_other_turns_share_their_shape() {
    // The same 2.4 scale at two yaws comes out of the matrices differing in the last bits.
    let scale = Vec3::splat(2.4);
    let at = |yaw: f32| {
        Mat4::from_scale_rotation_translation(scale, Quat::from_rotation_y(yaw), Vec3::ZERO)
    };
    let transforms: Vec<Mat4> = (0..40).map(|i| at(i as f32 * 0.37)).collect();
    let collision = Collision {
        shapes: vec![floor()],
        instance_shapes: vec![0; 40],
        heightfield: None,
    };
    let (_, physics) = with_collision(&collision, |i| transforms.get(i as usize).copied());
    assert_eq!(physics.borrow().distinct_shapes(), 1);
}

#[test]
fn a_dynamic_actor_moved_by_a_script_stops_where_it_is_put() {
    let (mut game, _) = world_of(vec![floor()]);
    let crate_ = game.world.spawn((
        Transform::from_translation(Vec3::new(0.0, 8.0, 0.0)),
        ActorCollider {
            shape: Shape::Box(Vec3::splat(1.0)),
            kind: BodyKind::Dynamic(10.0),
            offset: Vec3::ZERO,
        },
    ));
    steps(&mut game, 40);
    let turned = Quat::from_rotation_y(1.0);
    {
        let mut t = game.world.entities.get::<&mut Transform>(crate_).unwrap();
        t.translation = Vec3::new(6.0, 5.0, 0.0);
        t.rotation = turned;
    }
    steps(&mut game, 1);
    let at = place(&game, crate_);
    // Its falling speed was forgotten: it has fallen one step's worth, not many.
    assert!(at.y > 4.95, "{at}");
    assert!(at.abs_diff_eq(Vec3::new(6.0, 5.0, 0.0), 0.1), "{at}");
    let rotation = game
        .world
        .entities
        .get::<&Transform>(crate_)
        .unwrap()
        .rotation;
    assert!(rotation.abs_diff_eq(turned, 1.0e-3), "{rotation}");
}

fn watching_sensors() -> (Game, Seen) {
    let physics = Rc::new(RefCell::new(Physics::new()));
    let mut schedule = Schedule::new();
    Physics::install(&physics, &mut schedule);
    let seen = Seen::default();
    let log = seen.clone();
    schedule.add(TickGroup::PostPhysics, "seen", move |world| {
        for o in world.read_cloned::<Overlap>() {
            log.0.borrow_mut().push((o.sensor, o.other, o.entered));
        }
    });
    (Game::new(World::new(), schedule), seen)
}

fn sensor_at(game: &mut Game, x: f32) -> Entity {
    game.world.spawn((
        Transform::from_translation(Vec3::new(x, 0.0, 0.0)),
        ActorCollider {
            shape: Shape::Box(Vec3::splat(4.0)),
            kind: BodyKind::Sensor,
            offset: Vec3::ZERO,
        },
    ))
}

#[test]
fn a_sensor_hears_of_an_actor_removed_inside_it() {
    let (mut game, seen) = watching_sensors();
    let plate = sensor_at(&mut game, 0.0);
    let guest = sensor_guest(&mut game);
    steps(&mut game, 3);
    assert_eq!(*seen.0.borrow(), vec![(plate, guest, true)]);
    game.world.entities.despawn(guest).unwrap();
    steps(&mut game, 3);
    assert_eq!(
        *seen.0.borrow(),
        vec![(plate, guest, true), (plate, guest, false)]
    );
}

/// A kinematic box, standing still, that nothing but sensors will notice.
fn sensor_guest(game: &mut Game) -> Entity {
    game.world.spawn((
        Transform::default(),
        ActorCollider {
            shape: Shape::Box(Vec3::splat(1.0)),
            kind: BodyKind::Kinematic,
            offset: Vec3::ZERO,
        },
    ))
}

#[test]
fn two_sensors_that_overlap_each_hear_of_the_other() {
    let (mut game, seen) = watching_sensors();
    let a = sensor_at(&mut game, 0.0);
    let b = sensor_at(&mut game, 1.0);
    steps(&mut game, 3);
    let mut heard = seen.0.borrow().clone();
    heard.sort();
    let mut expected = vec![(a, b, true), (b, a, true)];
    expected.sort();
    assert_eq!(heard, expected);
}

#[test]
fn removing_and_adding_actors_comes_out_the_same_every_time() {
    let run = || {
        let (mut game, _) = world_of(vec![floor()]);
        let drop_at = |game: &mut Game, x: f32| {
            game.world.spawn((
                Transform::from_translation(Vec3::new(x, 2.0, 0.3)),
                ActorCollider {
                    shape: Shape::Box(Vec3::splat(0.5)),
                    kind: BodyKind::Dynamic(1.0),
                    offset: Vec3::ZERO,
                },
            ))
        };
        let first: Vec<Entity> = (0..20)
            .map(|i| drop_at(&mut game, i as f32 * 0.7))
            .collect();
        steps(&mut game, 30);
        for entity in first {
            game.world.entities.despawn(entity).unwrap();
        }
        let second: Vec<Entity> = (0..20)
            .map(|i| drop_at(&mut game, i as f32 * 0.31))
            .collect();
        steps(&mut game, 120);
        second
            .iter()
            .map(|&e| place(&game, e).to_array().map(f32::to_bits))
            .collect::<Vec<_>>()
    };
    let first = run();
    for _ in 0..5 {
        assert_eq!(run(), first);
    }
}

#[test]
fn a_blocker_out_of_bounds_is_left_out_with_a_warning() {
    let (_, physics) = world_of(vec![floor()]);
    let mut physics = physics.borrow_mut();
    let before = physics.collider_count();
    let big = Vec3::splat(1.0e30);
    physics.add_blocker(Vec3::ZERO, big, 0.0);
    physics.add_blocker(big, Vec3::ONE, 0.0);
    physics.add_blocker(Vec3::ZERO, Vec3::new(1.0, -1.0, 1.0), 0.0);
    physics.add_blocker(Vec3::ZERO, Vec3::ONE, f32::NAN);
    assert_eq!(physics.collider_count(), before);
    assert_eq!(physics.warnings.len(), 4, "{:?}", physics.warnings);
    // As big and as far out as is allowed.
    physics.add_blocker(Vec3::splat(1.0e6), Vec3::splat(1.0e6), 0.0);
    assert_eq!(physics.collider_count(), before + 1);
}

#[test]
fn an_actor_out_of_bounds_collides_with_nothing_and_says_so_once() {
    let at = Transform::from_translation(Vec3::ZERO);
    let huge = 1.0e30;
    let far = Transform::from_translation(Vec3::new(huge, 0.0, 0.0));
    let boxed = |shape, kind, offset| ActorCollider {
        shape,
        kind,
        offset,
    };
    let unit = Shape::Box(Vec3::ONE);
    let (mut game, physics) = world_of(vec![floor()]);
    let turned = Transform {
        rotation: Quat::NAN,
        ..at
    };
    let entities = [
        game.world
            .spawn((turned, boxed(unit, BodyKind::Kinematic, Vec3::ZERO))),
        game.world.spawn((
            at,
            boxed(
                Shape::Box(Vec3::splat(huge)),
                BodyKind::Kinematic,
                Vec3::ZERO,
            ),
        )),
        game.world.spawn((
            at,
            boxed(
                Shape::Capsule {
                    radius: huge,
                    height: huge * 2.0,
                },
                BodyKind::Sensor,
                Vec3::ZERO,
            ),
        )),
        game.world
            .spawn((at, boxed(unit, BodyKind::Kinematic, Vec3::splat(huge)))),
        game.world
            .spawn((at, boxed(unit, BodyKind::Dynamic(huge), Vec3::ZERO))),
        game.world.spawn((
            at,
            Character {
                radius: huge,
                height: huge * 2.0,
            },
        )),
        game.world
            .spawn((far, boxed(unit, BodyKind::Kinematic, Vec3::ZERO))),
    ];
    steps(&mut game, 5);
    let physics = physics.borrow();
    assert_eq!(physics.tracked_colliders(), 0);
    assert_eq!(
        physics.warnings.len(),
        entities.len(),
        "{:?}",
        physics.warnings
    );
}

#[test]
fn an_actor_at_the_very_edge_of_the_bounds_is_built() {
    let most = 1.0e6;
    let (mut game, physics) = world_of(vec![floor()]);
    game.world.spawn((
        Transform::from_translation(Vec3::splat(most)),
        ActorCollider {
            shape: Shape::Box(Vec3::splat(most)),
            kind: BodyKind::Dynamic(most),
            offset: Vec3::splat(most),
        },
    ));
    game.world.spawn((
        Transform::from_translation(Vec3::new(-most, 0.0, 0.0)),
        Character {
            radius: most,
            height: most,
        },
    ));
    steps(&mut game, 5);
    let physics = physics.borrow();
    assert_eq!(physics.tracked_colliders(), 2);
    assert_eq!(physics.warnings, Vec::<String>::new());
}

#[test]
fn close_scales_share_a_shape_and_tiny_ones_do_not() {
    let scaled = |s: f32| Mat4::from_scale(Vec3::splat(s));
    let distinct = |scales: Vec<f32>| {
        let collision = Collision {
            shapes: vec![floor()],
            instance_shapes: vec![0; scales.len()],
            heightfield: None,
        };
        let (_, physics) =
            with_collision(&collision, |i| scales.get(i as usize).map(|s| scaled(*s)));
        let shapes = physics.borrow().distinct_shapes();
        shapes
    };
    assert_eq!(
        distinct(vec![1.0, 1.000_000_1, 0.999_999_9]),
        1,
        "the last bits"
    );
    assert_eq!(
        distinct(vec![0.000_1, 0.000_4]),
        2,
        "small, and four times the size"
    );
    assert_eq!(distinct(vec![1.0, 1.1]), 2);
}
