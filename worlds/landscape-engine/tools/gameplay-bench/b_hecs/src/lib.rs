include!("../../common.rs");
struct Pos([f32; 3]);
struct Vel([f32; 3]);
pub fn bench() -> String {
    const N: usize = 10_000;
    let mut world = hecs::World::new();
    world.spawn_batch((0..N).map(|i| (Pos([i as f32, 0.0, 0.0]), Vel([1.0, 2.0, 3.0]))));
    let tq = best(5, || for _ in 0..100 { for (p, v) in world.query_mut::<(&mut Pos, &Vel)>() { for k in 0..3 { p.0[k] += v.0[k] * 0.016; } } });
    format!("hecs: query {:.2} ns/entity ({N} entities)", tq * 1e6 / (100 * N) as f64)
}
