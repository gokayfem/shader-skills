include!("../../common.rs");
use rapier3d::prelude::*;

struct World {
    bodies: RigidBodySet, colliders: ColliderSet, impulse_joints: ImpulseJointSet,
    multibody_joints: MultibodyJointSet, soft_bodies: SoftBodySet, pipeline: PhysicsPipeline,
    islands: IslandManager, broad_phase: DefaultBroadPhase, narrow_phase: NarrowPhase, ccd: CCDSolver,
    params: IntegrationParameters,
}
impl World {
    fn new() -> Self {
        Self { bodies: RigidBodySet::new(), colliders: ColliderSet::new(), impulse_joints: ImpulseJointSet::new(),
            multibody_joints: MultibodyJointSet::new(), soft_bodies: SoftBodySet::new(), pipeline: PhysicsPipeline::new(),
            islands: IslandManager::new(), broad_phase: DefaultBroadPhase::new(), narrow_phase: NarrowPhase::new(),
            ccd: CCDSolver::new(), params: IntegrationParameters::default() }
    }
    fn step(&mut self) {
        self.pipeline.step(Vector::new(0.0, -9.81, 0.0), &self.params, &mut self.islands, &mut self.broad_phase,
            &mut self.narrow_phase, &mut self.bodies, &mut self.colliders, &mut self.impulse_joints,
            &mut self.multibody_joints, &mut self.soft_bodies, &mut self.ccd, &(), &());
    }
}

/// `n` boxes dropped as a loose column grid onto ground, stepped `frames` times at 60 Hz.
fn pile(n: usize, frames: usize) -> (f64, f64, f64, usize) {
    let mut w = World::new();
    w.colliders.insert(ColliderBuilder::cuboid(50.0, 0.5, 50.0).translation(Vector::new(0.0, -0.5, 0.0)));
    let side = (n as f64).cbrt().ceil() as usize;
    let mut k = 0;
    'o: for y in 0..side * 4 { for x in 0..side { for z in 0..side {
        if k == n { break 'o; }
        let jit = if y % 2 == 1 { 0.45 } else { 0.0 }; let h = w.bodies.insert(RigidBodyBuilder::dynamic().rotation(Vector::new(0.3 * (k % 7) as f32, 0.2, 0.1)).translation(Vector::new(jit + x as f32 * 1.1 - side as f32 * 0.55, 0.6 + y as f32 * 1.1, z as f32 * 1.1)));
        w.colliders.insert_with_parent(ColliderBuilder::cuboid(0.5, 0.5, 0.5), h, &mut w.bodies);
        k += 1;
    }}}
    let mut times = Vec::with_capacity(frames);
    for _ in 0..frames { let t = now_ms(); w.step(); times.push(now_ms() - t); }
    let mean = times[..120].iter().sum::<f64>() / 120.0;
    let worst = times.iter().cloned().fold(0.0, f64::max);
    // 1000 raycasts down through the pile.
    let q = w.broad_phase.as_query_pipeline(w.narrow_phase.query_dispatcher(), &w.bodies, &w.colliders, QueryFilter::new());
    let t = now_ms();
    let mut hits = 0;
    for i in 0..1000 {
        let ray = Ray::new(Vector::new((i % 32) as f32 * 0.3 - 5.0, 40.0, (i / 32) as f32 * 0.3), Vector::new(0.0, -1.0, 0.0));
        if q.cast_ray(&ray, 100.0, true).is_some() { hits += 1; }
    }
    let rays = now_ms() - t;
    (mean, worst, rays, hits)
}

pub fn bench() -> String {
    let mut out = String::new();
    for n in [100usize, 500, 1000] {
        let (mean, worst, rays, hits) = pile(n, 300);
        out += &format!("rapier {n} boxes: mean step over the first 2 s {mean:.3} ms, worst {worst:.3} ms, 1000 rays {rays:.3} ms ({hits} hits)\n");
    }
    out
}
