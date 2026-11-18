include!("../../common.rs");
use glam::{Quat, Vec3};
use solstice_world::{Game, Input, Parent, Schedule, TickGroup, Transform, World};

#[derive(Clone)]
struct Tick;

/// `roots` entities that turn every step, each with `children` attached, each of those with one
/// more: `roots * (1 + 2 * children)` entities with transforms. A system turns the roots, and
/// every step sends and reads an event per root.
fn game(roots: usize, children: usize) -> Game {
    let mut world = World::new();
    for r in 0..roots {
        let root = world.spawn((Transform::from_translation(Vec3::new(r as f32, 0.0, 0.0)),));
        for c in 0..children {
            let child = world.spawn((
                Transform::from_translation(Vec3::new(0.0, c as f32, 0.0)),
                Parent(root),
            ));
            world.spawn((Transform::from_translation(Vec3::Z), Parent(child)));
        }
    }
    let mut schedule = Schedule::new();
    schedule.add(TickGroup::PrePhysics, "turn", |world: &mut World| {
        let mut count = 0;
        for (transform,) in world
            .entities
            .query_mut::<(&mut Transform,)>()
            .without::<&Parent>()
        {
            transform.rotation *= Quat::from_rotation_y(0.01);
            count += 1;
        }
        for _ in 0..count {
            world.send(Tick);
        }
    });
    schedule.add(TickGroup::Physics, "read", |world: &mut World| {
        std::hint::black_box(world.read::<Tick>().count());
    });
    Game::new(world, schedule)
}

pub fn bench() -> String {
    let mut out = String::new();
    for (roots, children) in [(100, 4), (1000, 4), (1000, 9)] {
        let mut game = game(roots, children);
        let input = Input::new();
        game.step(&input);
        let steps = 60;
        let ms = best(3, || {
            for _ in 0..steps {
                game.step(&input);
            }
        }) / steps as f64;
        let entities = roots * (1 + 2 * children);
        out += &format!(
            "world: {entities} entities with transforms, {roots} moving with their children: {ms:.3} ms a step ({:.1} ns an entity)\n",
            ms * 1.0e6 / entities as f64
        );
    }
    out
}
