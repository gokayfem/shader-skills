include!("../../common.rs");
use bevy_ecs::prelude::*;
#[derive(Component)] struct Pos([f32; 3]);
#[derive(Component)] struct Vel([f32; 3]);
#[derive(Resource)] struct Dt(f32);
fn integrate(dt: Res<Dt>, mut q: Query<(&mut Pos, &Vel)>) {
    for (mut p, v) in &mut q { for k in 0..3 { p.0[k] += v.0[k] * dt.0; } }
}
pub fn bench() -> String {
    const N: usize = 10_000;
    let mut world = World::new();
    world.insert_resource(Dt(0.016));
    world.spawn_batch((0..N).map(|i| (Pos([i as f32, 0.0, 0.0]), Vel([1.0, 2.0, 3.0]))));
    let mut schedule = Schedule::default();
    schedule.add_systems(integrate);
    schedule.run(&mut world);
    let t = best(5, || for _ in 0..100 { schedule.run(&mut world); });
    let mut q = world.query::<(&mut Pos, &Vel)>();
    let tq = best(5, || for _ in 0..100 { for (mut p, v) in q.iter_mut(&mut world) { for k in 0..3 { p.0[k] += v.0[k] * 0.016; } } });
    format!("bevy_ecs: schedule+system {:.2} ns/entity, raw query {:.2} ns/entity ({N} entities)", t * 1e6 / (100 * N) as f64, tq * 1e6 / (100 * N) as f64)
}
