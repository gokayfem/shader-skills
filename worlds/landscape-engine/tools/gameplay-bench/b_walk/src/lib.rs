include!("../../common.rs");
// Physics' cost a step, in a world like the cove: a large static scene, the player walking on it,
// and a few doors. The scene is made of triangle meshes and a heightfield, as a baked one is.
use glam::{Mat4, Vec3};
use solstice_asset::{Collision, CollisionShape, Heightfield};
use solstice_physics::{ActorCollider, BodyKind, Character, Physics, Shape};
use solstice_world::{Game, Input, Schedule, Transform, Walk, World};
use std::cell::RefCell;
use std::rc::Rc;

/// A rippled sheet `size` across of `cells` squares a side: 2 * cells^2 triangles.
fn sheet(size: f32, cells: u32) -> CollisionShape {
    let row = cells + 1;
    let mut vertices = Vec::new();
    for z in 0..row {
        for x in 0..row {
            let (u, v) = (x as f32 / cells as f32, z as f32 / cells as f32);
            let height = (u * 90.0).sin() * (v * 70.0).cos() * 0.15;
            vertices.push([(u - 0.5) * size, height, (v - 0.5) * size]);
        }
    }
    let mut triangles = Vec::new();
    for z in 0..cells {
        for x in 0..cells {
            let a = z * row + x;
            triangles.push([a, a + row, a + 1]);
            triangles.push([a + 1, a + row, a + row + 1]);
        }
    }
    CollisionShape { hull: false, vertices, triangles }
}

fn ground(samples: u32, spacing: f32) -> Heightfield {
    let mut heights = Vec::new();
    for z in 0..samples {
        for x in 0..samples {
            heights.push((x as f32 * 0.09).sin() * (z as f32 * 0.07).cos() * 0.3);
        }
    }
    let origin = (samples - 1) as f32 * spacing * -0.5;
    Heightfield { columns: samples, rows: samples, origin: [origin, origin], spacing: [spacing, spacing], heights }
}

/// A world of `meshes` placements of a mesh of `triangles`, on a metre-grid heightfield.
fn world(meshes: u32, cells: u32, doors: u32, walkers: u32, crates: u32) -> (Game, Vec<solstice_world::Entity>) {
    let shape = sheet(60.0, cells);
    let collision = Collision {
        shapes: vec![shape],
        instance_shapes: vec![0; meshes as usize],
        heightfield: Some(ground(901, 1.0)),
    };
    let physics = Rc::new(RefCell::new(Physics::new()));
    let grid = (meshes as f32).sqrt().ceil() as u32;
    physics.borrow_mut().add_scene(&collision, |i| {
        Some(Mat4::from_translation(Vec3::new((i % grid) as f32 * 65.0 - 200.0, 0.5, (i / grid) as f32 * 65.0 - 200.0)))
    });
    let mut schedule = Schedule::new();
    Physics::install(&physics, &mut schedule);
    let mut game = Game::new(World::new(), schedule);
    for i in 0..doors {
        game.world.spawn((
            Transform::from_translation(Vec3::new(i as f32 * 3.0 - 30.0, 1.5, 40.0)),
            ActorCollider { shape: Shape::Box(Vec3::new(1.0, 3.0, 0.2)), kind: BodyKind::Kinematic, offset: Vec3::ZERO },
        ));
    }
    // Crates dropped in a pile, so that the end state depends on real contact solving.
    for i in 0..crates {
        game.world.spawn((
            Transform::from_translation(Vec3::new((i % 6) as f32 * 0.9, 3.0 + (i / 6) as f32 * 1.2, 20.0 + (i % 5) as f32 * 0.13)),
            ActorCollider { shape: Shape::Box(Vec3::splat(0.8)), kind: BodyKind::Dynamic(20.0), offset: Vec3::ZERO },
        ));
    }
    let mut heroes = Vec::new();
    for i in 0..walkers {
        heroes.push(game.world.spawn((
            Transform::from_translation(Vec3::new(i as f32 * 2.0 - 10.0, 2.0, 0.0)),
            Character { radius: 0.3, height: 1.8 },
            Walk(Vec3::new(0.0, 0.0, 1.5)),
        )));
    }
    (game, heroes)
}

pub fn bench() -> String {
    let mut out = String::new();
    let input = Input::new();
    for (meshes, cells, doors, walkers, crates, what) in [
        (40, 64, 8, 1, 0, "cove-sized scene (330k triangles), the player, 8 doors"),
        (40, 64, 8, 8, 0, "the same, 8 walkers"),
        (40, 64, 8, 1, 30, "the same, the player, 30 crates falling in a pile"),
        (100, 100, 8, 1, 0, "a large scene (2M triangles), the player, 8 doors"),
    ] {
        let t = now_ms();
        let (mut game, _) = world(meshes, cells, doors, walkers, crates);
        let build = now_ms() - t;
        for _ in 0..30 { game.step(&input); }
        let steps = 120;
        let ms = best(3, || { for _ in 0..steps { game.step(&input); } }) / steps as f64;
        // Where everyone ended up, to the bit: the same natively and on the web if physics is
        // deterministic across platforms.
        let mut print = 0u64;
        for (_, t) in game.world.entities.query::<(solstice_world::Entity, &Transform)>().iter() {
            for bits in t.translation.to_array().map(f32::to_bits) {
                print = print.rotate_left(7) ^ u64::from(bits);
            }
        }
        out += &format!("{what}: built in {build:.0} ms, {ms:.3} ms a step, ended at {print:016x}\n");
    }
    out
}
