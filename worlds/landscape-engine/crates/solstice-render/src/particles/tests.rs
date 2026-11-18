//! The simulation on a real GPU, against the CPU reference and the pool's invariants. They pass
//! trivially, with a note, if no GPU is available.

use super::*;
use crate::frame::FrameUniform;
use reference::{is_dead, spawn, spawn_elapsed, step};
use std::collections::{HashMap, HashSet};

use crate::gpu::test_gpu as request_gpu;

/// A pool on the GPU, failing the test on any GPU error.
struct Rig {
    gpu: Gpu,
    particles: Particles,
    errors: Arc<Mutex<Vec<String>>>,
}

impl Rig {
    fn new(capacity: u32) -> Option<Self> {
        let gpu = request_gpu()?;
        let errors: Arc<Mutex<Vec<String>>> = Default::default();
        let sink = errors.clone();
        gpu.device
            .on_uncaptured_error(Arc::new(move |error: wgpu::Error| {
                sink.lock().unwrap().push(error.to_string());
            }));
        let frame = gpu.uniform_buffer("frame", std::mem::size_of::<FrameUniform>() as u64);
        // The light pass is not run here, but its inputs are bound with the pool.
        let shadow_maps = gpu
            .device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("no shadows"),
                size: wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 2,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Depth32Float,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&wgpu::TextureViewDescriptor {
                dimension: Some(wgpu::TextureViewDimension::D2Array),
                ..Default::default()
            });
        let inputs = LightInputs {
            lights: gpu.storage_buffer("no lights", 32, wgpu::BufferUsages::empty()),
            shadow_uniform: gpu.uniform_buffer("no shadows", 1024),
            shadow_maps,
            shadow_sampler: gpu.device.create_sampler(&wgpu::SamplerDescriptor {
                compare: Some(wgpu::CompareFunction::GreaterEqual),
                ..Default::default()
            }),
        };
        let environment = crate::lighting::environment_layout(&gpu);
        let waterline = crate::ocean::waterline::Waterline::new(&gpu);
        let particles = Particles::new(
            &gpu,
            &frame,
            capacity,
            &environment,
            waterline.resolve_layout(),
            inputs,
        );
        Some(Self {
            gpu,
            particles,
            errors,
        })
    }

    /// Simulates one frame of `seconds`.
    fn frame(&mut self, seconds: f32) {
        self.frame_at(seconds, Vec3::ZERO);
    }

    /// Simulates one frame of `seconds`, seen from the origin looking along -Z, and sorts what
    /// asks to be sorted.
    fn frame_sorted(&mut self, seconds: f32) {
        self.frame_sorted_from(
            seconds,
            &Camera {
                position: Vec3::ZERO,
                ..Default::default()
            },
        );
    }

    /// The same, seen from `camera`.
    fn frame_sorted_from(&mut self, seconds: f32, camera: &Camera) {
        self.particles.prepare(&self.gpu, camera, seconds, 1.0);
        let mut encoder = self.gpu.device.create_command_encoder(&Default::default());
        self.particles.record_simulation(&mut encoder);
        self.particles.record_sort(&mut encoder);
        self.gpu.queue.submit([encoder.finish()]);
        self.gpu
            .device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
        let errors = self.errors.lock().unwrap();
        assert!(errors.is_empty(), "GPU errors: {}", errors.join("\n"));
    }

    /// Simulates one frame of `seconds` with the camera at `camera`.
    fn frame_at(&mut self, seconds: f32, camera: Vec3) {
        let camera = Camera {
            position: camera,
            ..Default::default()
        };
        self.particles.prepare(&self.gpu, &camera, seconds, 1.0);
        let mut encoder = self.gpu.device.create_command_encoder(&Default::default());
        self.particles.record_simulation(&mut encoder);
        self.gpu.queue.submit([encoder.finish()]);
        self.gpu
            .device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
        let errors = self.errors.lock().unwrap();
        assert!(errors.is_empty(), "GPU errors: {}", errors.join("\n"));
    }

    fn read<T: bytemuck::Pod>(&self, buffer: &wgpu::Buffer) -> Vec<T> {
        let size = buffer.size();
        let staging = self.gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("particle test readback"),
            size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self.gpu.device.create_command_encoder(&Default::default());
        encoder.copy_buffer_to_buffer(buffer, 0, &staging, 0, size);
        self.gpu.queue.submit([encoder.finish()]);
        staging
            .slice(..)
            .map_async(wgpu::MapMode::Read, |r| r.unwrap());
        self.gpu
            .device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
        let data = staging.slice(..).get_mapped_range().unwrap();
        bytemuck::cast_slice(&data).to_vec()
    }

    /// Each live emitter's slot, slice, state, free list and alive particles, as the next frame
    /// will read them.
    fn emitters(&self) -> Vec<Snapshot> {
        if self.particles.pool.is_none() {
            assert!(!self.particles.slots.iter().any(|&live| live));
            return Vec::new();
        }
        let states: Vec<EmitterState> = self.read(&self.particles.pool().state);
        let records: Vec<EmitterRecord> = self.read(&self.particles.pool().emitters);
        let free: Vec<u32> = self.read(&self.particles.pool().free);
        let alive: Vec<u32> = self.read(&self.particles.pool().alive);
        let pool: Vec<Particle> = self.read(&self.particles.pool().particles);
        let capacity = self.particles.pool().capacity;
        let parity = self.particles.parity;
        (0..MAX_EMITTERS)
            .filter(|&slot| self.particles.slots[slot])
            .map(|slot| {
                let state = states[slot];
                let record = records[slot];
                let (base, budget) = (record.base, record.budget);
                let free = free[base as usize..(base + state.free_count) as usize].to_vec();
                let start = (parity * capacity + base) as usize;
                let alive: Vec<u32> =
                    alive[start..start + state.alive[parity as usize] as usize].to_vec();
                let particles = alive.iter().map(|&s| pool[s as usize]).collect();
                Snapshot {
                    slot,
                    base,
                    budget,
                    state,
                    record,
                    free,
                    alive,
                    particles,
                }
            })
            .collect()
    }
}

struct Snapshot {
    #[allow(dead_code)]
    slot: usize,
    base: u32,
    budget: u32,
    state: EmitterState,
    record: EmitterRecord,
    free: Vec<u32>,
    alive: Vec<u32>,
    particles: Vec<Particle>,
}

#[test]
fn unused_pool_is_lazy_and_late_effects_keep_their_full_budget() {
    let Some(mut rig) = Rig::new(262_144) else {
        return;
    };
    assert_eq!(rig.particles.capacity(), 262_144);
    assert_eq!(rig.particles.buffer_bytes(), 0);
    for _ in 0..8 {
        rig.frame(1.0 / 60.0);
    }
    rig.particles.resize(&rig.gpu, 8192);
    assert_eq!(rig.particles.stats().capacity, 8192);
    assert_eq!(rig.particles.buffer_bytes(), 0);
    let effect =
        Effect::parse("emitter late\n budget 8192\n spawn burst=8192\n lifetime 10").unwrap();
    // An effect cancelled before prepare never needs GPU storage.
    let cancelled = rig.particles.spawn(&effect, Mat4::IDENTITY).unwrap();
    rig.particles.kill(cancelled);
    rig.frame(1.0 / 60.0);
    assert_eq!(rig.particles.buffer_bytes(), 0);
    let live = rig.particles.spawn(&effect, Mat4::IDENTITY).unwrap();
    rig.frame(1.0 / 60.0);
    let allocated = rig.particles.buffer_bytes();
    assert!(allocated > 8192 * std::mem::size_of::<Particle>() as u64);
    assert_eq!(rig.emitters()[0].alive.len(), 8192);
    rig.particles.kill(live);
    rig.frame(1.0 / 60.0);
    // Keep used storage to avoid a hitch when another effect starts.
    assert_eq!(rig.particles.buffer_bytes(), allocated);
    rig.particles.spawn(&effect, Mat4::IDENTITY).unwrap();
    rig.frame(1.0 / 60.0);
    assert_eq!(rig.emitters()[0].alive.len(), 8192);
}

const BALLISTIC: &str = "
    emitter shot
      budget 2000
      spawn burst=1500
      shape sphere 0.5
      offset 1 2 3
      lifetime 30
      velocity 2 8 1 cone=25 spread=0.2
      size 0.1 0.2
      gravity
      drag 0.35
      temperature 900 1400
      cooling 0.5
";

/// Frame times that vary as a real frame rate does.
const UNEVEN: [f32; 12] = [
    1.0 / 60.0,
    1.0 / 30.0,
    1.0 / 144.0,
    0.05,
    1.0 / 75.0,
    0.0021,
    1.0 / 20.0,
    1.0 / 60.0,
    0.004,
    1.0 / 90.0,
    0.031,
    1.0 / 60.0,
];

/// Every particle the GPU simulated is where the CPU reference puts it.
#[test]
fn gpu_trajectories_match_the_reference() {
    let Some(mut rig) = Rig::new(4096) else {
        return;
    };
    let effect = Effect::parse(BALLISTIC).unwrap();
    let transform = Mat4::from_rotation_y(0.7) * Mat4::from_scale(Vec3::splat(1.5));
    rig.particles.spawn(&effect, transform).unwrap();
    for seconds in UNEVEN {
        rig.frame(seconds);
    }
    let [emitter] = rig.emitters().try_into().ok().unwrap();
    assert_eq!(emitter.particles.len(), 1500);
    let mut worst: f32 = 0.0;
    for p in &emitter.particles {
        let mut expected = spawn(&emitter.record, p.index);
        step(
            &mut expected,
            &emitter.record,
            spawn_elapsed(p.index, 1500, UNEVEN[0]),
        );
        for &seconds in &UNEVEN[1..] {
            step(&mut expected, &emitter.record, seconds);
        }
        let error = (Vec3::from(p.position) - Vec3::from(expected.position)).length();
        let speed_error = (Vec3::from(p.velocity) - Vec3::from(expected.velocity)).length();
        worst = worst.max(error);
        assert!(error < 1.0e-3, "particle {} is {error} m off", p.index);
        assert!(
            speed_error < 1.0e-3,
            "particle {} is {speed_error} m/s off",
            p.index
        );
        assert!((p.age - expected.age).abs() < 1.0e-5);
        assert!((p.size - expected.size).abs() < 1.0e-6);
        assert!((p.temperature - expected.temperature).abs() < 1.0e-3);
        assert!((900.0..=1400.0).contains(&p.temperature));
        assert!((p.lifetime - expected.lifetime).abs() < 1.0e-5);
    }
    // They really moved: a few metres up and out over a third of a second.
    let mean_height = emitter.particles.iter().map(|p| p.position[1]).sum::<f32>() / 1500.0;
    assert!(mean_height > 4.0, "{mean_height}");
    eprintln!("worst position error {worst} m");
}

/// Under every force at once, wind, curl noise, a vortex, attraction and buoyancy, each particle
/// the GPU moved is where the CPU reference puts it.
#[test]
fn forced_trajectories_match_the_reference() {
    let Some(mut rig) = Rig::new(4096) else {
        return;
    };
    let effect = Effect::parse(
        "
        emitter smoke
          budget 600
          spawn burst=500
          shape box 4 1 4
          offset 0 2 0
          lifetime 30
          velocity 0 1 0 cone=40 spread=0.3
          gravity
          drag 1.5
          wind 1
          curl 2 scale=3 change=0.5
          vortex 0.6
          attract 1
          buoyancy 0.5
          temperature 600 900
          cooling 0.3
        ",
    )
    .unwrap();
    rig.particles.set_air([5.0, -2.0], -1.0);
    rig.particles
        .spawn(&effect, Mat4::from_translation(Vec3::new(3.0, 0.0, -2.0)))
        .unwrap();
    let mut time = 0.0;
    let mut ends = Vec::new();
    for seconds in UNEVEN {
        rig.frame(seconds);
        time += seconds;
        ends.push((seconds, time));
    }
    let [emitter] = rig.emitters().try_into().ok().unwrap();
    assert_eq!(emitter.particles.len(), 500);
    let air = |time: f32| reference::Air {
        time,
        wind: [5.0, -2.0],
        floor: -1.0,
    };
    let mut worst: f32 = 0.0;
    for p in &emitter.particles {
        let mut expected = spawn(&emitter.record, p.index);
        let (first, first_end) = ends[0];
        reference::step_in(
            &mut expected,
            &emitter.record,
            spawn_elapsed(p.index, 500, first),
            &air(first_end),
        );
        for &(seconds, end) in &ends[1..] {
            reference::step_in(&mut expected, &emitter.record, seconds, &air(end));
        }
        let error = (Vec3::from(p.position) - Vec3::from(expected.position)).length();
        worst = worst.max(error);
        assert!(error < 1.0e-4, "particle {} is {error} m off", p.index);
    }
    // They really were pushed about: the wind is taking them east, at over a metre a second
    // already.
    let mean_east = emitter.particles.iter().map(|p| p.velocity[0]).sum::<f32>() / 500.0;
    assert!(mean_east > 1.0, "{mean_east}");
    eprintln!("worst position error under forces {worst} m");
}

/// Splitting the same span of time differently gives the same particles.
#[test]
fn uneven_frame_times_give_the_same_result_as_even_ones() {
    let total: f32 = UNEVEN[1..].iter().sum();
    let run = |times: &[f32]| {
        let mut rig = Rig::new(4096)?;
        let effect = Effect::parse(BALLISTIC).unwrap();
        rig.particles.spawn(&effect, Mat4::IDENTITY).unwrap();
        rig.frame(UNEVEN[0]);
        for &seconds in times {
            rig.frame(seconds);
        }
        let [emitter] = rig.emitters().try_into().ok().unwrap();
        Some(
            emitter
                .particles
                .iter()
                .map(|p| (p.index, Vec3::from(p.position)))
                .collect::<HashMap<u32, Vec3>>(),
        )
    };
    let Some(uneven) = run(&UNEVEN[1..]) else {
        return;
    };
    let even = run(&[total / 11.0; 11]).unwrap();
    assert_eq!(uneven.len(), even.len());
    for (index, position) in &uneven {
        let other = even[index];
        assert!(
            (*position - other).length() < 1.0e-3,
            "particle {index}: {position} against {other}"
        );
    }
}

/// Over thousands of frames of particles being born and dying, every slot of every slice is
/// always either free or holding exactly one live particle, never both, never neither.
#[test]
fn particles_are_conserved_as_they_live_and_die() {
    let Some(mut rig) = Rig::new(8192) else {
        return;
    };
    let effect = Effect::parse(
        "
        emitter quick
          budget 700
          spawn rate=2400 burst=300 every=0.7
          lifetime 0.05 0.6
          velocity 0 1 0 cone=90
        emitter slow
          budget 1500
          spawn rate=500
          lifetime 1 4
        ",
    )
    .unwrap();
    rig.particles.spawn(&effect, Mat4::IDENTITY).unwrap();
    rig.particles
        .spawn(&effect, Mat4::from_translation(Vec3::X))
        .unwrap();
    let mut checked = 0;
    for frame in 0..2400 {
        rig.frame(UNEVEN[frame % UNEVEN.len()]);
        if frame % 97 != 0 {
            continue;
        }
        let emitters = rig.emitters();
        assert_eq!(emitters.len(), 4);
        for e in &emitters {
            let total = e.state.free_count + e.state.alive[rig.particles.parity as usize];
            assert_eq!(
                total, e.budget,
                "frame {frame}: free and alive must fill the budget"
            );
            let mut seen = HashSet::new();
            for &slot in e.free.iter().chain(&e.alive) {
                assert!(
                    (e.base..e.base + e.budget).contains(&slot),
                    "slot {slot} is outside its slice"
                );
                assert!(
                    seen.insert(slot),
                    "frame {frame}: slot {slot} is listed twice"
                );
            }
            for p in &e.particles {
                assert!(!is_dead(p), "a dead particle is still alive");
            }
            checked += 1;
        }
    }
    assert!(checked > 40);
}

/// Spawns beyond an emitter's budget are dropped and counted, and its particles never spill out
/// of its slice into its neighbour's.
#[test]
fn spawns_beyond_the_budget_are_dropped_and_counted() {
    let Some(mut rig) = Rig::new(1024) else {
        return;
    };
    let greedy = Effect::parse("emitter e\n budget 100\n spawn burst=1000\n lifetime 10").unwrap();
    let quiet = Effect::parse("emitter e\n budget 200\n lifetime 10").unwrap();
    rig.particles.spawn(&greedy, Mat4::IDENTITY).unwrap();
    rig.particles.spawn(&quiet, Mat4::IDENTITY).unwrap();
    for _ in 0..3 {
        rig.frame(1.0 / 60.0);
    }
    let emitters = rig.emitters();
    let (greedy, quiet) = (&emitters[0], &emitters[1]);
    // The burst is clamped to the free list on the GPU, and the rest counted.
    assert_eq!(greedy.alive.len(), 100);
    assert_eq!(greedy.state.free_count, 0);
    assert_eq!(greedy.state.spawned, 100);
    assert_eq!(greedy.state.dropped, 900);
    assert!(greedy
        .alive
        .iter()
        .all(|s| (greedy.base..greedy.base + 100).contains(s)));
    assert_eq!(quiet.alive.len(), 0);
    assert_eq!(quiet.state.free_count, 200);
    let mut free = quiet.free.clone();
    free.sort_unstable();
    assert_eq!(free, (quiet.base..quiet.base + 200).collect::<Vec<_>>());

    // A rate the budget cannot hold: what does not fit is dropped and counted on the GPU.
    let mut rig = Rig::new(1024).unwrap();
    let flood = Effect::parse("emitter e\n budget 64\n spawn rate=6000\n lifetime 10").unwrap();
    rig.particles.spawn(&flood, Mat4::IDENTITY).unwrap();
    for _ in 0..10 {
        rig.frame(1.0 / 60.0);
    }
    let [flood] = rig.emitters().try_into().ok().unwrap();
    assert_eq!(flood.alive.len(), 64);
    assert_eq!(flood.state.spawned, 64);
    assert_eq!(flood.state.spawned + flood.state.dropped, 1000);
}

/// A rate spawns its count over time however the time is split into frames.
#[test]
fn rates_spawn_their_count_whatever_the_frame_rate() {
    let Some(mut rig) = Rig::new(4096) else {
        return;
    };
    let effect = Effect::parse("emitter e\n budget 2000\n spawn rate=240\n lifetime 100").unwrap();
    rig.particles.spawn(&effect, Mat4::IDENTITY).unwrap();
    let mut elapsed = 0.0;
    let mut frame = 0;
    while elapsed < 4.0 - 1.0e-4 {
        let seconds = UNEVEN[frame % UNEVEN.len()].min(4.0 - elapsed);
        rig.frame(seconds);
        elapsed += seconds;
        frame += 1;
    }
    let [e] = rig.emitters().try_into().ok().unwrap();
    assert!(
        (959..=960).contains(&e.state.spawned),
        "{}",
        e.state.spawned
    );
    assert_eq!(e.alive.len() as u32, e.state.spawned);
}

/// A stopped effect lives out its particles and then gives its slice back; a killed one gives
/// it back at once.
#[test]
fn stopped_effects_end_and_their_slices_are_reused() {
    let Some(mut rig) = Rig::new(1000) else {
        return;
    };
    let effect = Effect::parse("emitter e\n budget 1000\n spawn rate=600\n lifetime 0.5").unwrap();
    let first = rig.particles.spawn(&effect, Mat4::IDENTITY).unwrap();
    // The pool is full.
    assert!(rig.particles.spawn(&effect, Mat4::IDENTITY).is_none());
    for _ in 0..30 {
        rig.frame(1.0 / 30.0);
    }
    rig.particles.stop(first);
    for _ in 0..6 {
        rig.frame(1.0 / 30.0);
    }
    assert!(
        rig.particles.is_running(first),
        "it still has particles alive"
    );
    for _ in 0..30 {
        rig.frame(1.0 / 30.0);
    }
    assert!(!rig.particles.is_running(first));
    assert!(!rig.particles.set(first, "anything", 1.0));

    let second = rig.particles.spawn(&effect, Mat4::IDENTITY).unwrap();
    assert_ne!(first, second);
    for _ in 0..10 {
        rig.frame(1.0 / 30.0);
    }
    let [e] = rig.emitters().try_into().ok().unwrap();
    // A fresh slice: it counts from zero, and holds only the new effect's particles.
    assert_eq!(e.state.spawned, 200);
    rig.particles.kill(second);
    assert!(!rig.particles.is_running(second));
    rig.frame(1.0 / 30.0);
    assert!(!rig.particles.is_active());
    assert!(rig.particles.spawn(&effect, Mat4::IDENTITY).is_some());
}

/// Changing the pool's size, as changing quality does, starts running effects again in the new
/// pool, with budgets that fit it, and nothing is lost or listed twice.
#[test]
fn a_running_effect_moves_to_a_resized_pool() {
    let Some(mut rig) = Rig::new(8192) else {
        return;
    };
    let effect = Effect::parse(
        "
        emitter a
          budget 3000
          spawn rate=3000
          lifetime 0.2 1.5
        emitter b
          budget 3000
          spawn rate=900
          lifetime 2
        ",
    )
    .unwrap();
    let id = rig.particles.spawn(&effect, Mat4::IDENTITY).unwrap();
    for frame in 0..40 {
        rig.frame(UNEVEN[frame % UNEVEN.len()]);
    }
    rig.particles.resize(&rig.gpu, 4000);
    assert_eq!(rig.particles.capacity(), 4000);
    assert!(rig.particles.is_running(id));
    for frame in 0..200 {
        rig.frame(UNEVEN[frame % UNEVEN.len()]);
    }
    let emitters = rig.emitters();
    assert_eq!(emitters.len(), 2);
    // The first takes all it asked for; the second only what was left.
    assert_eq!((emitters[0].budget, emitters[1].budget), (3000, 1000));
    for e in &emitters {
        assert_eq!(
            e.state.free_count + e.state.alive[rig.particles.parity as usize],
            e.budget
        );
        let mut seen = HashSet::new();
        for &slot in e.free.iter().chain(&e.alive) {
            assert!((e.base..e.base + e.budget).contains(&slot));
            assert!(seen.insert(slot));
        }
        assert!(!e.alive.is_empty());
    }
}

/// An emitter that follows the camera spawns around it, wherever it goes.
#[test]
fn camera_following_emitters_spawn_around_the_camera() {
    let Some(mut rig) = Rig::new(4096) else {
        return;
    };
    let effect = Effect::parse(
        "emitter rain\n budget 2000\n follow camera\n spawn rate=6000\n shape box 20 4 20\n offset 0 10 0\n lifetime 5",
    )
    .unwrap();
    rig.particles.spawn(&effect, Mat4::IDENTITY).unwrap();
    let camera = Vec3::new(140.0, 3.0, -60.0);
    rig.frame_at(1.0 / 60.0, camera);
    let [e] = rig.emitters().try_into().ok().unwrap();
    assert_eq!(e.particles.len(), 100);
    for p in &e.particles {
        let local = Vec3::from(p.position) - camera - Vec3::new(0.0, 10.0, 0.0);
        assert!(
            local.abs().cmple(Vec3::new(10.0, 2.0, 10.0)).all(),
            "{local}"
        );
    }
}

/// A paused frame, of no time, moves nothing and spawns nothing.
#[test]
fn a_paused_frame_moves_nothing() {
    let Some(mut rig) = Rig::new(4096) else {
        return;
    };
    let effect = Effect::parse(
        BALLISTIC
            .replace("burst=1500", "burst=500 rate=300")
            .as_str(),
    )
    .unwrap();
    rig.particles.spawn(&effect, Mat4::IDENTITY).unwrap();
    for seconds in &UNEVEN[..4] {
        rig.frame(*seconds);
    }
    let before = rig.emitters().remove(0);
    for _ in 0..5 {
        rig.frame(0.0);
    }
    let after = rig.emitters().remove(0);
    assert_eq!(after.state.spawned, before.state.spawned);
    let positions = |e: &Snapshot| -> HashMap<u32, [f32; 3]> {
        e.particles.iter().map(|p| (p.index, p.position)).collect()
    };
    assert_eq!(positions(&after), positions(&before));
}

/// A burst at low detail still gives at least one particle.
#[test]
fn bursts_survive_low_detail() {
    let effect = Effect::parse("emitter e\n spawn burst=1 every=0.5").unwrap();
    let mut emitter = LiveEmitter {
        slot: 0,
        base: 0,
        budget: 10,
        carry: 0.0,
        burst_started: false,
        until_burst: 0.0,
        longest_life: 0.0,
    };
    let desc = &effect.emitters[0];
    assert_eq!(spawns_due(&mut emitter, desc, &[], 0.1, 0.35), 1);
    assert_eq!(spawns_due(&mut emitter, desc, &[], 0.3, 0.35), 0);
    assert_eq!(spawns_due(&mut emitter, desc, &[], 0.2, 0.35), 1);
    // And a burst of nothing stays nothing.
    let none = Effect::parse("emitter e\n spawn rate=0").unwrap();
    emitter.burst_started = false;
    assert_eq!(
        spawns_due(&mut emitter, &none.emitters[0], &[], 0.1, 0.35),
        0
    );
}

/// Every record the shaders share with the CPU has the same size and field offsets in both, as
/// naga lays out the WGSL.
#[test]
fn shader_records_match_their_rust_twins() {
    let source = crate::shaders::expand("particles/simulate.wgsl");
    let module = naga::front::wgsl::parse_str(&source).unwrap();
    let layout = |name: &str| -> (u32, HashMap<String, u32>) {
        let ty = module
            .types
            .iter()
            .find(|(_, t)| t.name.as_deref() == Some(name))
            .unwrap_or_else(|| panic!("no struct {name}"))
            .1;
        let naga::TypeInner::Struct { members, span } = &ty.inner else {
            panic!("{name} is not a struct");
        };
        let offsets = members
            .iter()
            .map(|m| (m.name.clone().unwrap(), m.offset))
            .collect();
        (*span, offsets)
    };
    macro_rules! twin {
        ($wgsl:literal, $rust:ty, [$($field:ident),* $(,)?]) => {{
            let (span, offsets) = layout($wgsl);
            assert_eq!(span as usize, std::mem::size_of::<$rust>(), "size of {}", $wgsl);
            $(
                assert_eq!(
                    offsets[stringify!($field)] as usize,
                    std::mem::offset_of!($rust, $field),
                    "{}.{}", $wgsl, stringify!($field)
                );
            )*
        }};
    }
    twin!(
        "Emitter",
        EmitterRecord,
        [
            transform,
            shape_size,
            offset_cone,
            velocity_spread,
            acceleration_drag,
            lifetime_size,
            color,
            base,
            budget,
            spawn_request,
            seed,
            shape_kind,
            flags,
            soft,
            scale,
            temperature_glow,
            phase,
            wind_curl,
            buoyancy_attract_vortex,
            restitution_friction,
            collide_against,
            response,
            event_target,
            event_spawn,
        ]
    );
    twin!(
        "Particle",
        Particle,
        [
            position,
            age,
            velocity,
            lifetime,
            previous,
            size,
            index,
            rotation,
            temperature,
            flags
        ]
    );
    twin!(
        "EmitterState",
        EmitterState,
        [
            free_count,
            alive,
            spawned,
            dropped,
            spawn_count,
            spawn_first,
            spawn_index,
            update_count,
            spawn_start,
            update_start,
            light_start,
            event_count,
            events_in,
        ]
    );
    twin!(
        "SimParams",
        SimUniform,
        [seconds, parity, capacity, light_interval, ray_steps]
    );
}

#[test]
fn ranges_split_and_join() {
    let mut ranges = vec![(0, 100)];
    let a = allocate(&mut ranges, 30).unwrap();
    let b = allocate(&mut ranges, 30).unwrap();
    let c = allocate(&mut ranges, 60).unwrap();
    assert_eq!((a, b, c), ((0, 30), (30, 30), (60, 40)));
    assert!(allocate(&mut ranges, 1).is_none());
    release(&mut ranges, b.0, b.1);
    release(&mut ranges, a.0, a.1);
    assert_eq!(ranges, vec![(0, 60)]);
    release(&mut ranges, c.0, c.1);
    assert_eq!(ranges, vec![(0, 100)]);
}

/// Runs the light record's packing and the black body from `particles/light_common.wgsl` on the
/// GPU: each input's colour (xyz) through RGB9E5 and back, and the luminance of a black body at
/// its w kelvin.
fn run_light_common(gpu: &Gpu, inputs: &[[f32; 4]]) -> Vec<[f32; 4]> {
    let source = crate::shaders::expand("particles/light_common.wgsl")
        + "
@group(0) @binding(0) var<storage, read> inputs: array<vec4f>;
@group(0) @binding(1) var<storage, read_write> outputs: array<vec4f>;

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3u) {
    if id.x >= arrayLength(&inputs) {
        return;
    }
    let v = inputs[id.x];
    outputs[id.x] = vec4f(unpack_rgb9e5(pack_rgb9e5(v.xyz)), black_body_xyz(v.w).y);
}
";
    let module = gpu
        .device
        .create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("light common test"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
    let layout = LayoutBuilder::new()
        .storage_read()
        .storage_write()
        .build(gpu, "light common test");
    let pipeline = gpu.compute_pipeline("light common test", &module, "main", &[&layout]);
    let input = gpu.storage_buffer_init("inputs", inputs);
    let bytes = std::mem::size_of_val(inputs) as u64;
    let output = gpu.storage_buffer("outputs", bytes, wgpu::BufferUsages::COPY_SRC);
    let group = gpu.bind_group(
        "light common test",
        &layout,
        &[input.as_entire_binding(), output.as_entire_binding()],
    );
    let staging = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("light common readback"),
        size: bytes,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_compute_pass(&Default::default());
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.dispatch_workgroups((inputs.len() as u32).div_ceil(64), 1, 1);
    }
    encoder.copy_buffer_to_buffer(&output, 0, &staging, 0, bytes);
    gpu.queue.submit([encoder.finish()]);
    staging
        .slice(..)
        .map_async(wgpu::MapMode::Read, |r| r.unwrap());
    gpu.device
        .poll(wgpu::PollType::wait_indefinitely())
        .unwrap();
    let data = staging.slice(..).get_mapped_range().unwrap();
    bytemuck::pod_collect_to_vec(&data)
}

/// RGB9E5 keeps every channel to within a step of its largest, holds zero and tiny values, and
/// clips at its maximum; and a black body at the freezing point of platinum, 2,042 K, has the
/// luminance that once defined the candela, 60 cd/cm².
#[test]
fn light_packing_and_black_bodies_hold_their_values() {
    let Some(gpu) = request_gpu() else {
        return;
    };
    let mut inputs = vec![
        [0.0, 0.0, 0.0, 2042.0],
        [1.0, 1.0, 1.0, 0.0],
        [0.5, 0.25, 0.125, 0.0],
        [1.0e-6, 1.0e-3, 1.0, 0.0],
        [3.0e-7, 0.0, 0.0, 0.0],
        [65408.0, 2.0, 0.0, 0.0],
        [90000.0, 1.0, 1.0, 0.0],
        [0.333, 7.77, 1234.5, 0.0],
    ];
    for i in 0..200u32 {
        let r = |k| reference::hash_to_unit(i * 4 + k);
        let scale = 2.0f32.powf(r(3) * 40.0 - 24.0);
        inputs.push([r(0) * scale, r(1) * scale, r(2) * scale, 0.0]);
    }
    let outputs = run_light_common(&gpu, &inputs);
    for (input, output) in inputs.iter().zip(&outputs) {
        let wanted = [input[0], input[1], input[2]].map(|c| c.clamp(0.0, 65408.0));
        let largest = wanted.iter().copied().fold(0.0, f32::max);
        // Half a step of the shared exponent, at most 1/512 of the largest channel.
        let tolerance = largest / 512.0 + 1.0e-7;
        for c in 0..3 {
            assert!(
                (output[c] - wanted[c]).abs() <= tolerance,
                "{input:?} came back as {output:?}"
            );
        }
    }
    assert_eq!(&outputs[0][..3], &[0.0; 3]);
    assert_eq!(outputs[6][0], 65408.0, "beyond the maximum, it clips to it");
    let platinum = outputs[0][3];
    assert!(
        (platinum / 600_000.0 - 1.0).abs() < 0.03,
        "a black body at 2,042 K: {platinum} cd/m²"
    );
}

/// A sorted emitter's list, as drawn, runs far to near along the view, holding exactly the
/// particles it did, at every size up to its limit; an emitter that does not sort is left as it
/// was.
#[test]
fn sorted_emitters_list_their_particles_far_to_near() {
    let Some(mut rig) = Rig::new(16384) else {
        return;
    };
    // Spread along the view, -Z, and across it.
    let sorted = |count: u32| {
        Effect::parse(&format!(
            "emitter smoke\n budget {count}\n spawn burst={count}\n lifetime 100\n shape box 6 6 60\n offset 0 0 -40\n render sprite sort"
        ))
        .unwrap()
    };
    for count in [3, 100, 1000, 4096] {
        rig.particles.spawn(&sorted(count), Mat4::IDENTITY).unwrap();
    }
    let plain = Effect::parse(
        "emitter plain\n budget 500\n spawn burst=500\n lifetime 100\n shape box 6 6 60\n offset 0 0 -40",
    )
    .unwrap();
    rig.particles.spawn(&plain, Mat4::IDENTITY).unwrap();
    rig.frame_sorted(1.0 / 60.0);
    rig.frame_sorted(1.0 / 60.0);
    let emitters = rig.emitters();
    assert_eq!(emitters.len(), 5);
    for e in &emitters[..4] {
        let depths: Vec<f32> = e.particles.iter().map(|p| -p.position[2]).collect();
        // Far first: each no nearer than the next, but for the keys' quantisation, about a
        // millionth of the log of the depth.
        for pair in depths.windows(2) {
            assert!(
                pair[0] >= pair[1] * (1.0 - 1.0e-4),
                "{} then {}",
                pair[0],
                pair[1]
            );
        }
        let mut slots = e.alive.clone();
        slots.sort_unstable();
        slots.dedup();
        assert_eq!(slots.len(), e.alive.len(), "each particle listed once");
        assert!(slots
            .iter()
            .all(|s| (e.base..e.base + e.budget).contains(s)));
    }
    // The plain emitter's list is in whatever order the update left it, not sorted.
    let plain: Vec<f32> = emitters[4]
        .particles
        .iter()
        .map(|p| -p.position[2])
        .collect();
    assert!(plain.windows(2).any(|pair| pair[0] < pair[1]));
}

/// A solid particle moving across the view writes its own depth, and motion that takes it back
/// to where it was last frame, into the scene's targets that the temporal filter reads: the
/// G-buffer's, and the water's when there is a sea.
#[test]
fn solid_particles_write_their_depth_and_motion() {
    solid_spark_reads_back(None);
    solid_spark_reads_back(Some(-3.0));
}

fn solid_spark_reads_back(sea_level: Option<f32>) {
    use solstice_builder::{build_virtual_mesh, paginate, BuildOptions, Mesh, Vertex};
    let Some(gpu) = request_gpu() else {
        return;
    };
    let errors: Arc<Mutex<Vec<String>>> = Default::default();
    let sink = errors.clone();
    gpu.device
        .on_uncaptured_error(Arc::new(move |error: wgpu::Error| {
            sink.lock().unwrap().push(error.to_string());
        }));
    // A wall 20 m off, along -Z.
    let mut wall = Mesh::default();
    for (x, y) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
        wall.vertices.push(Vertex {
            position: Vec3::new(x * 60.0, y * 60.0, -20.0),
            normal: Vec3::Z,
            uv: glam::Vec2::new(x, y),
        });
    }
    wall.indices = vec![0, 1, 2, 0, 2, 3];
    let scene = solstice_asset::SceneAsset {
        meshes: vec![build_virtual_mesh(&wall, &BuildOptions::default())],
        materials: vec![solstice_asset::Material {
            base_color: [0.5, 0.5, 0.5, 1.0],
            flags: solstice_asset::Material::TWO_SIDED,
            ..Default::default()
        }],
        instances: vec![solstice_asset::Instance {
            mesh: 0,
            material: 0,
            transform: Mat4::IDENTITY,
        }],
        ..Default::default()
    };
    let settings = crate::RendererSettings {
        temporal_accumulation: false,
        ..Default::default()
    };
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let mut renderer = crate::Renderer::new(&gpu, &paginate(&scene), format, settings);
    if let Some(level) = sea_level {
        renderer.set_ocean(Some(crate::Ocean::by_name("coastal").unwrap()), 0.0);
        renderer.set_sea_level(level);
    }
    // Moving right at 6 m/s, 5 m in front of the camera.
    let spark = Effect::parse(
        "emitter spark\n budget 1\n spawn burst=1\n lifetime 100\n size 0.4\n velocity 6 0 0\n color 1 1 1 1\n render sprite solid shading=unlit",
    )
    .unwrap();
    renderer
        .particles()
        .spawn(&spark, Mat4::from_translation(Vec3::new(-1.0, 0.0, -5.0)))
        .unwrap();
    let size = 128u32;
    let target = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("test target"),
        size: wgpu::Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = target.create_view(&Default::default());
    let camera = Camera {
        position: Vec3::ZERO,
        ..Default::default()
    };
    let seconds = 1.0 / 60.0;
    let frames = 10;
    for _ in 0..frames {
        renderer.render(
            &gpu,
            &camera,
            &crate::Environment::default(),
            &view,
            (size, size),
            seconds,
        );
        gpu.device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
    }
    // Where the spark is now, and was a frame before.
    let now = Vec3::new(-1.0 + 6.0 * seconds * frames as f32, 0.0, -5.0);
    let before = now - Vec3::new(6.0 * seconds, 0.0, 0.0);
    let view_proj = camera.projection(1.0) * camera.view();
    let uv = |p: Vec3| {
        let ndc = view_proj.project_point3(p);
        glam::Vec2::new(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5)
    };
    let expected = uv(before) - uv(now);
    let pixel = (uv(now) * size as f32).as_uvec2();

    let (depth, motion) = renderer.scene_depth_and_motion().unwrap();
    let read = |texture: &wgpu::Texture, texel_bytes: u32| -> Vec<u8> {
        let row = (size * texel_bytes).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let staging = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("scene readback"),
            size: u64::from(row * size),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &staging,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: Some(size),
                },
            },
            wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: 1,
            },
        );
        gpu.queue.submit([encoder.finish()]);
        staging
            .slice(..)
            .map_async(wgpu::MapMode::Read, |r| r.unwrap());
        gpu.device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
        let data = staging.slice(..).get_mapped_range().unwrap();
        let mut texels = Vec::new();
        for line in data.chunks_exact(row as usize) {
            texels.extend_from_slice(&line[..(size * texel_bytes) as usize]);
        }
        texels
    };
    let depths: Vec<f32> = bytemuck::pod_collect_to_vec(&read(depth, 4));
    let motions: Vec<[f32; 2]> = bytemuck::pod_collect_to_vec(&read(motion, 8));
    let at = (pixel.y * size + pixel.x) as usize;
    let distance = camera.near / depths[at];
    assert!(
        (distance - 5.0).abs() < 0.1,
        "the spark's depth: {distance} m"
    );
    let got = glam::Vec2::from(motions[at]);
    assert!(
        (got - expected).length() < expected.length() * 0.1,
        "motion {got}, expected {expected}"
    );
    // The wall beside it has not moved.
    let wall_at = (pixel.y * size + 5) as usize;
    assert!(glam::Vec2::from(motions[wall_at]).length() < 1.0e-5);
    assert!((camera.near / depths[wall_at] - 20.0).abs() < 0.5);
    let errors = errors.lock().unwrap();
    assert!(errors.is_empty(), "GPU errors: {}", errors.join("\n"));
}

/// An emitter told to sort more than the sort can hold, by changing its budget in code after
/// it was read, is drawn unsorted rather than sorted wrongly: its particles stay whole.
#[test]
fn an_emitter_too_large_to_sort_is_left_unsorted() {
    let Some(mut rig) = Rig::new(16384) else {
        return;
    };
    let mut effect = Effect::parse(
        "emitter smoke\n budget 4096\n spawn burst=6000 rate=600\n lifetime 0.2 3\n shape box 6 6 60\n offset 0 0 -40\n render sprite sort",
    )
    .unwrap();
    effect.emitters[0].budget = 8000;
    rig.particles.spawn(&effect, Mat4::IDENTITY).unwrap();
    for frame in 0..60 {
        rig.frame_sorted(UNEVEN[frame % UNEVEN.len()]);
    }
    let [e] = rig.emitters().try_into().ok().unwrap();
    assert!(e.alive.len() > 4096, "{}", e.alive.len());
    assert_eq!(e.state.free_count + e.alive.len() as u32, e.budget);
    let mut seen = HashSet::new();
    for &slot in e.free.iter().chain(&e.alive) {
        assert!((e.base..e.base + e.budget).contains(&slot));
        assert!(seen.insert(slot), "slot {slot} is listed twice");
    }
}

/// Sorting follows the view however the camera is turned.
#[test]
fn sorting_follows_a_turned_camera() {
    let Some(mut rig) = Rig::new(4096) else {
        return;
    };
    let effect = Effect::parse(
        "emitter smoke\n budget 2000\n spawn burst=2000\n lifetime 100\n shape box 60 6 60\n render sprite sort",
    )
    .unwrap();
    rig.particles.spawn(&effect, Mat4::IDENTITY).unwrap();
    let camera = Camera {
        position: Vec3::new(-40.0, 2.0, 15.0),
        yaw: 2.2,
        pitch: -0.2,
        ..Default::default()
    };
    rig.frame_sorted_from(1.0 / 60.0, &camera);
    rig.frame_sorted_from(1.0 / 60.0, &camera);
    let [e] = rig.emitters().try_into().ok().unwrap();
    let depths: Vec<f32> = e
        .particles
        .iter()
        .map(|p| {
            (Vec3::from(p.position) - camera.position)
                .dot(camera.forward())
                .max(0.0)
        })
        .collect();
    for pair in depths.windows(2) {
        assert!(
            pair[0] >= pair[1] * (1.0 - 1.0e-4) - 1.0e-3,
            "{} then {}",
            pair[0],
            pair[1]
        );
    }
}

/// Once particles stop, the reactive mask is cleared, so the temporal filter is left as it was
/// before any particle was drawn.
#[test]
fn the_reactive_mask_clears_when_particles_stop() {
    use solstice_builder::{build_virtual_mesh, paginate, BuildOptions, Mesh, Vertex};
    let Some(gpu) = request_gpu() else {
        return;
    };
    let mut floor = Mesh::default();
    for (x, z) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
        floor.vertices.push(Vertex {
            position: Vec3::new(x * 50.0, -2.0, z * 50.0),
            normal: Vec3::Y,
            uv: glam::Vec2::new(x, z),
        });
    }
    floor.indices = vec![0, 2, 1, 0, 3, 2];
    let scene = solstice_asset::SceneAsset {
        meshes: vec![build_virtual_mesh(&floor, &BuildOptions::default())],
        materials: vec![solstice_asset::Material {
            base_color: [0.5; 4],
            flags: solstice_asset::Material::TWO_SIDED,
            ..Default::default()
        }],
        instances: vec![solstice_asset::Instance {
            mesh: 0,
            material: 0,
            transform: Mat4::IDENTITY,
        }],
        ..Default::default()
    };
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let mut renderer = crate::Renderer::new(
        &gpu,
        &paginate(&scene),
        format,
        crate::RendererSettings::default(),
    );
    let size = 64u32;
    let target = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("test target"),
        size: wgpu::Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = target.create_view(&Default::default());
    let camera = Camera {
        position: Vec3::ZERO,
        ..Default::default()
    };
    let frames = |renderer: &mut crate::Renderer, count: u32| {
        for _ in 0..count {
            renderer.render(
                &gpu,
                &camera,
                &crate::Environment::default(),
                &view,
                (size, size),
                1.0 / 60.0,
            );
            gpu.device
                .poll(wgpu::PollType::wait_indefinitely())
                .unwrap();
        }
    };
    let marked = |renderer: &crate::Renderer| -> u32 {
        let texture = renderer.reactive_mask().unwrap();
        let row = size.next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let staging = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("reactive readback"),
            size: u64::from(row * size),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &staging,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: Some(size),
                },
            },
            wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: 1,
            },
        );
        gpu.queue.submit([encoder.finish()]);
        staging
            .slice(..)
            .map_async(wgpu::MapMode::Read, |r| r.unwrap());
        gpu.device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
        let data = staging.slice(..).get_mapped_range().unwrap();
        data.chunks_exact(row as usize)
            .map(|line| line[..size as usize].iter().filter(|&&v| v > 0).count() as u32)
            .sum()
    };
    let puff = Effect::parse(
        "emitter e\n budget 1\n spawn burst=1\n lifetime 1000\n size 1\n color 1 1 1 0.5\n render sprite soft=0",
    )
    .unwrap();
    let id = renderer
        .particles()
        .spawn(&puff, Mat4::from_translation(Vec3::new(0.0, 0.0, -5.0)))
        .unwrap();
    frames(&mut renderer, 3);
    assert!(marked(&renderer) > 50, "the particle marks the mask");
    renderer.particles().kill(id);
    frames(&mut renderer, 2);
    assert_eq!(
        marked(&renderer),
        0,
        "the mask is clear once nothing is drawn"
    );
}

/// A renderer over a floor of `size` metres at height `floor`, optionally with a sea at
/// `sea_level`, seen from above and in front, for collision tests.
struct Ground {
    gpu: Gpu,
    renderer: crate::Renderer,
    view: wgpu::TextureView,
    camera: Camera,
    errors: Arc<Mutex<Vec<String>>>,
}

impl Ground {
    fn new(floor: f32, sea_level: Option<f32>) -> Option<Self> {
        Self::build(floor, sea_level, None)
    }

    /// The floor at height 0 with a wall across it at `x`, 6 m high.
    fn with_wall(x: f32) -> Option<Self> {
        Self::with_wall_of_height(x, 6.0)
    }

    fn with_wall_of_height(x: f32, height: f32) -> Option<Self> {
        Self::build(0.0, None, Some((x, height)))
    }

    fn build(floor: f32, sea_level: Option<f32>, wall: Option<(f32, f32)>) -> Option<Self> {
        use solstice_builder::{build_virtual_mesh, paginate, BuildOptions, Mesh, Vertex};
        let gpu = request_gpu()?;
        let errors: Arc<Mutex<Vec<String>>> = Default::default();
        let sink = errors.clone();
        gpu.device
            .on_uncaptured_error(Arc::new(move |error: wgpu::Error| {
                sink.lock().unwrap().push(error.to_string());
            }));
        let mut ground = Mesh::default();
        for (x, z) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
            ground.vertices.push(Vertex {
                position: Vec3::new(x * 40.0, floor, z * 40.0),
                normal: Vec3::Y,
                uv: glam::Vec2::new(x, z),
            });
        }
        ground.indices = vec![0, 2, 1, 0, 3, 2];
        if let Some((x, height)) = wall {
            for (y, z) in [(0.0, -40.0), (0.0, 40.0), (height, 40.0), (height, -40.0)] {
                ground.vertices.push(Vertex {
                    position: Vec3::new(x, y, z),
                    normal: -Vec3::X,
                    uv: glam::Vec2::new(y, z),
                });
            }
            ground.indices.extend([4, 5, 6, 4, 6, 7]);
        }
        let scene = solstice_asset::SceneAsset {
            meshes: vec![build_virtual_mesh(&ground, &BuildOptions::default())],
            materials: vec![solstice_asset::Material {
                base_color: [0.5, 0.5, 0.5, 1.0],
                flags: solstice_asset::Material::TWO_SIDED,
                ..Default::default()
            }],
            instances: vec![solstice_asset::Instance {
                mesh: 0,
                material: 0,
                transform: Mat4::IDENTITY,
            }],
            ..Default::default()
        };
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let settings = crate::RendererSettings {
            temporal_accumulation: false,
            ..Default::default()
        };
        let mut renderer = crate::Renderer::new(&gpu, &paginate(&scene), format, settings);
        if let Some(level) = sea_level {
            renderer.set_ocean(Some(crate::Ocean::by_name("coastal").unwrap()), 0.0);
            renderer.set_sea_level(level);
        }
        let target = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("test target"),
            size: wgpu::Extent3d {
                width: 128,
                height: 128,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let view = target.create_view(&Default::default());
        // Above and in front of the origin, looking down at it.
        let camera = Camera {
            position: Vec3::new(0.0, floor + 9.0, 9.0),
            pitch: -std::f32::consts::FRAC_PI_4,
            ..Default::default()
        };
        Some(Self {
            gpu,
            renderer,
            view,
            camera,
            errors,
        })
    }

    fn frame(&mut self) {
        self.renderer.render(
            &self.gpu,
            &self.camera,
            &crate::Environment::default(),
            &self.view,
            (128, 128),
            1.0 / 60.0,
        );
        self.gpu
            .device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
        let errors = self.errors.lock().unwrap();
        assert!(errors.is_empty(), "GPU errors: {}", errors.join("\n"));
    }

    fn read<T: bytemuck::Pod>(&self, buffer: &wgpu::Buffer) -> Vec<T> {
        let size = buffer.size();
        let staging = self.gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("collision readback"),
            size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self.gpu.device.create_command_encoder(&Default::default());
        encoder.copy_buffer_to_buffer(buffer, 0, &staging, 0, size);
        self.gpu.queue.submit([encoder.finish()]);
        staging
            .slice(..)
            .map_async(wgpu::MapMode::Read, |r| r.unwrap());
        self.gpu
            .device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
        let data = staging.slice(..).get_mapped_range().unwrap();
        bytemuck::pod_collect_to_vec(&data)
    }

    /// Each live emitter's state and live particles, in slot order.
    fn emitters(&mut self) -> Vec<(EmitterState, Vec<Particle>)> {
        let particles = self.renderer.particles();
        let (state, alive, pool, emitters) = (
            particles.pool().state.clone(),
            particles.pool().alive.clone(),
            particles.pool().particles.clone(),
            particles.pool().emitters.clone(),
        );
        let (capacity, parity, slots) =
            (particles.pool().capacity, particles.parity, particles.slots);
        let records: Vec<EmitterRecord> = self.read(&emitters);
        let states: Vec<EmitterState> = self.read(&state);
        let lists: Vec<u32> = self.read(&alive);
        let pool: Vec<Particle> = self.read(&pool);
        (0..MAX_EMITTERS)
            .filter(|&s| slots[s])
            .map(|s| {
                let start = (parity * capacity + records[s].base) as usize;
                let count = states[s].alive[parity as usize] as usize;
                let live = lists[start..start + count]
                    .iter()
                    .map(|&slot| pool[slot as usize])
                    .filter(|p| !reference::is_dead(p))
                    .collect();
                (states[s], live)
            })
            .collect()
    }
}

/// Particles thrown at a floor in view never fall through it, whether they find it by the depth
/// drawn this frame or by the GI volume's distance field. (Depth alone is blind off screen.)
#[test]
fn particles_never_fall_through_the_floor() {
    for against in ["depth", "field"] {
        let Some(mut ground) = Ground::new(0.0, None) else {
            return;
        };
        let effect = Effect::parse(&format!(
            "emitter e\n budget 400\n spawn burst=300\n lifetime 100\n shape box 3 1 3\n offset 0 3 -2\n velocity 0 -2 0 cone=20\n size 0.05\n gravity\n collide {against} bounce restitution=0.3"
        ))
        .unwrap();
        ground
            .renderer
            .particles()
            .spawn(&effect, Mat4::IDENTITY)
            .unwrap();
        let mut lowest = f32::MAX;
        for frame in 0..150 {
            ground.frame();
            if frame % 10 == 9 {
                let [(_, live)] = ground.emitters().try_into().ok().unwrap();
                assert_eq!(live.len(), 300);
                lowest = live.iter().map(|p| p.position[1]).fold(lowest, f32::min);
            }
        }
        // The field finds the floor to within its voxels, 0.2 m near the camera.
        let allowed = if against == "field" { -0.2 } else { -0.05 };
        assert!(lowest > allowed, "{against}: a particle reached {lowest} m");
    }
}

/// A particle dropped on the floor rebounds to its restitution squared of the height it fell.
#[test]
fn bounces_keep_the_energy_their_restitution_allows() {
    let Some(mut ground) = Ground::new(0.0, None) else {
        return;
    };
    let effect = Effect::parse(
        "emitter e\n budget 1\n spawn burst=1\n lifetime 100\n size 0.04\n gravity\n collide depth bounce restitution=0.5 friction=0",
    )
    .unwrap();
    ground
        .renderer
        .particles()
        .spawn(&effect, Mat4::from_translation(Vec3::new(0.0, 2.0, 0.0)))
        .unwrap();
    let mut heights = Vec::new();
    for _ in 0..90 {
        ground.frame();
        let [(_, live)] = ground.emitters().try_into().ok().unwrap();
        heights.push(live[0].position[1]);
    }
    // After it first comes down, the highest it climbs again.
    let landed = heights.iter().position(|&y| y < 0.1).expect("it lands");
    let rebound = heights[landed..].iter().copied().fold(f32::MIN, f32::max);
    let fall = 2.0 - 0.02;
    assert!(
        (rebound - 0.02 - fall * 0.25).abs() < 0.08,
        "rebounded to {rebound} m from {fall} m"
    );
}

/// A particle landing with sideways speed slides on, whether the floor is found by depth or by
/// the field, and friction brings it to rest about where Coulomb's law says.
#[test]
fn sliding_particles_keep_moving_until_friction_stops_them() {
    for against in ["depth", "field"] {
        let mut travel = [0.0; 2];
        for (result, friction) in travel.iter_mut().zip([0.0, 0.5]) {
            let Some(mut ground) = Ground::new(0.0, None) else {
                return;
            };
            let effect = Effect::parse(&format!(
                "emitter e\n budget 20\n spawn burst=10\n lifetime 100\n shape box 0.2 0.02 0.2\n offset -1 0.3 0\n velocity 2 0 0\n size 0.04\n gravity\n collide {against} bounce restitution=0 friction={friction}"
            ))
            .unwrap();
            ground
                .renderer
                .particles()
                .spawn(&effect, Mat4::IDENTITY)
                .unwrap();
            for _ in 0..90 {
                ground.frame();
            }
            let [(_, live)] = ground.emitters().try_into().ok().unwrap();
            assert_eq!(live.len(), 10);
            *result = live.iter().map(|p| p.position[0] + 1.0).sum::<f32>() / 10.0;
            if friction > 0.0 {
                let fastest = live
                    .iter()
                    .map(|p| Vec3::from(p.velocity).length())
                    .fold(0.0, f32::max);
                assert!(fastest < 0.05, "{against}: still moving at {fastest} m/s");
            }
        }
        // Two metres a second for a second and a half.
        assert!(travel[0] > 2.5, "{against}: slid only {} m", travel[0]);
        // 0.41 m of sliding, plus what it travelled before it landed, less what landing took.
        assert!(
            travel[1] > 0.05 && travel[1] < 0.8,
            "{against}: with friction it went {} m",
            travel[1]
        );
    }
}

/// A particle resting on the ground lands once: its landing sends an event, and lying there
/// sends no more. That holds seen from a shallow angle, and pressed there by a force other than
/// gravity.
#[test]
fn resting_particles_send_no_more_events() {
    // What brings it down, how large it is, how far below the origin the effect stands, and
    // whether the camera looks along the ground from far away.
    let cases = [
        ("gravity", 0.04, 0.0, false),
        ("gravity", 0.2, 0.0, true),
        ("attract 15", 0.04, 50.0, false),
    ];
    for (force, size, below, grazing) in cases {
        let Some(mut ground) = Ground::new(0.0, None) else {
            return;
        };
        if grazing {
            ground.camera = Camera {
                position: Vec3::new(0.0, 1.5, 30.0),
                ..Default::default()
            };
        }
        let height = 1.0 + below;
        let effect = Effect::parse(&format!(
            "
            emitter e
              budget 50
              spawn burst=50
              lifetime 100
              shape box 2 0.1 2
              offset 0 {height} -1
              size {size}
              {force}
              collide depth bounce restitution=0 friction=1 event=hit
            emitter child
              budget 1000
              spawn event=hit count=1
              lifetime 100
              size 0.01
            ",
        ))
        .unwrap();
        ground
            .renderer
            .particles()
            .spawn(&effect, Mat4::from_translation(Vec3::new(0.0, -below, 0.0)))
            .unwrap();
        for _ in 0..70 {
            ground.frame();
        }
        let settled = ground.emitters()[1].0.spawned;
        assert!(
            (50..=100).contains(&settled),
            "{force}: {settled} events to land"
        );
        for _ in 0..30 {
            ground.frame();
        }
        assert_eq!(ground.emitters()[1].0.spawned, settled, "{force}, {size}");
    }
}

/// What is born on a surface can leave it, though under the field's ridge the field points the
/// wrong way, and what is spawned by an event knows which side of the surface is open.
#[test]
fn particles_born_on_a_surface_know_which_side_is_open() {
    let Some(mut ground) = Ground::new(0.0, None) else {
        return;
    };
    let effect = Effect::parse(
        "emitter e\n budget 40\n spawn burst=20\n lifetime 100\n shape box 1 0 1\n offset 0 0.03 -1\n velocity 0 0.5 0\n size 0.02\n collide field kill",
    )
    .unwrap();
    ground
        .renderer
        .particles()
        .spawn(&effect, Mat4::IDENTITY)
        .unwrap();
    for _ in 0..20 {
        ground.frame();
    }
    let [(_, live)] = ground.emitters().try_into().ok().unwrap();
    assert_eq!(live.len(), 20, "leaving the floor isn't a hit");

    let Some(mut ground) = Ground::new(0.0, None) else {
        return;
    };
    let effect = Effect::parse(
        "
        emitter drops
          budget 20
          spawn burst=20
          lifetime 100
          shape box 2 0 2
          offset 0 1 -1
          size 0.01
          gravity
          collide depth kill event=hit
        emitter child
          budget 40
          spawn event=hit count=1
          lifetime 100
          velocity 0 -0.5 0
          size 0.02
          collide field bounce restitution=0
        ",
    )
    .unwrap();
    ground
        .renderer
        .particles()
        .spawn(&effect, Mat4::IDENTITY)
        .unwrap();
    for _ in 0..90 {
        ground.frame();
    }
    let emitters = ground.emitters();
    assert_eq!(emitters[1].1.len(), 20);
    let lowest = emitters[1]
        .1
        .iter()
        .map(|p| p.position[1])
        .fold(f32::MAX, f32::min);
    assert!(lowest > -0.25, "a child went down to {lowest} m");
}

/// A particle born within reach of one surface still meets the next one in its path.
#[test]
fn particles_born_near_a_surface_still_hit_the_next() {
    let Some(mut ground) = Ground::with_wall(1.5) else {
        return;
    };
    let effect = Effect::parse(
        "emitter e\n budget 40\n spawn burst=20\n lifetime 100\n shape box 0.02 0.02 0.5\n offset 1.45 0.45 0\n velocity -20 -40 0\n size 0.02\n collide field kill",
    )
    .unwrap();
    ground
        .renderer
        .particles()
        .spawn(&effect, Mat4::IDENTITY)
        .unwrap();
    for _ in 0..30 {
        ground.frame();
    }
    let [(_, live)] = ground.emitters().try_into().ok().unwrap();
    assert!(live.is_empty(), "{} went through the floor", live.len());
}

/// A particle thrown up beside a wall comes back down rather than staying up there. (It may then
/// go through the floor: within a voxel of the wall's foot, the normal it lands on is the corner's
/// blur. See docs/particles.md.)
#[test]
fn a_neighbouring_surface_holds_nothing_up() {
    let Some(mut ground) = Ground::with_wall_of_height(1.5, 0.9) else {
        return;
    };
    let effect = Effect::parse(
        "
        emitter drops
          budget 20
          spawn burst=20
          lifetime 100
          shape box 0.02 0 2
          offset 1.42 1 0
          size 0.01
          gravity
          collide depth kill event=hit
        emitter child
          budget 40
          spawn event=hit count=1
          lifetime 100
          velocity 0 3 0
          size 0.02
          gravity
          collide field bounce restitution=0
        ",
    )
    .unwrap();
    ground
        .renderer
        .particles()
        .spawn(&effect, Mat4::IDENTITY)
        .unwrap();
    for _ in 0..150 {
        ground.frame();
    }
    let emitters = ground.emitters();
    assert_eq!(emitters[1].1.len(), 20);
    let highest = emitters[1]
        .1
        .iter()
        .map(|p| p.position[1])
        .fold(f32::MIN, f32::max);
    assert!(highest < 0.35, "a particle hung at {highest} m");
}

/// One that hops off the floor, still within reach and under the field's ridge, remembers which
/// side it was on when it comes back down.
#[test]
fn hopping_particles_remember_their_side() {
    let Some(mut ground) = Ground::new(0.0, None) else {
        return;
    };
    let effect = Effect::parse(
        "
        emitter drops
          budget 20
          spawn burst=20
          lifetime 100
          shape box 2 0 2
          offset 0 1 -1
          size 0.01
          gravity
          collide depth kill event=hit
        emitter child
          budget 40
          spawn event=hit count=1
          lifetime 100
          velocity 0 0.3 0
          size 0.02
          gravity
          collide field bounce restitution=0
        ",
    )
    .unwrap();
    ground
        .renderer
        .particles()
        .spawn(&effect, Mat4::IDENTITY)
        .unwrap();
    for _ in 0..90 {
        ground.frame();
    }
    let emitters = ground.emitters();
    assert_eq!(emitters[1].1.len(), 20);
    let lowest = emitters[1]
        .1
        .iter()
        .map(|p| p.position[1])
        .fold(f32::MAX, f32::min);
    assert!(lowest > -0.25, "a particle fell to {lowest} m");
}

/// A particle born just above a floor and falling, where the field's gradient is up, lands.
#[test]
fn falling_from_just_above_a_floor_lands() {
    let Some(mut ground) = Ground::new(0.0, None) else {
        return;
    };
    // Above the field's ridge, 0.1 to 0.2 m up here, and within reach of it.
    let effect = Effect::parse(
        "emitter e\n budget 40\n spawn burst=20\n lifetime 100\n shape box 1 0 1\n offset 0 0.25 -1\n velocity 0 -1 0\n size 0.02\n gravity\n collide field bounce restitution=0",
    )
    .unwrap();
    ground
        .renderer
        .particles()
        .spawn(&effect, Mat4::IDENTITY)
        .unwrap();
    for _ in 0..60 {
        ground.frame();
    }
    let [(_, live)] = ground.emitters().try_into().ok().unwrap();
    assert_eq!(live.len(), 20);
    let lowest = live.iter().map(|p| p.position[1]).fold(f32::MAX, f32::min);
    assert!(lowest > -0.2, "a particle fell to {lowest} m");
}

/// A slow particle sliding towards a wall stays where its speed takes it, rather than being
/// taken to a wall that the search ahead of it found.
#[test]
fn slow_sliding_particles_do_not_jump_to_a_wall() {
    let Some(mut ground) = Ground::with_wall(1.5) else {
        return;
    };
    let effect = Effect::parse(
        "
        emitter e
          budget 20
          spawn burst=10
          lifetime 100
          shape box 0.02 0.02 0.2
          offset 1.05 0.3 0
          velocity 0.1 0 0
          size 0.04
          gravity
          collide field bounce restitution=0 friction=0 event=hit
        emitter child
          budget 400
          spawn event=hit count=1
          lifetime 100
          size 0.01
        ",
    )
    .unwrap();
    ground
        .renderer
        .particles()
        .spawn(&effect, Mat4::IDENTITY)
        .unwrap();
    for _ in 0..50 {
        ground.frame();
    }
    let settled = ground.emitters()[1].0.spawned;
    assert!((10..=20).contains(&settled), "{settled} hits to land");
    for _ in 0..30 {
        ground.frame();
    }
    let emitters = ground.emitters();
    assert_eq!(emitters[0].1.len(), 10);
    assert_eq!(emitters[1].0.spawned, settled, "hits while sliding");
    // A tenth of a metre a second for 80 frames, from 1.05.
    let furthest = emitters[0]
        .1
        .iter()
        .map(|p| p.position[0])
        .fold(f32::MIN, f32::max);
    assert!(furthest < 1.3, "a particle jumped to x = {furthest}");
}

/// A particle sliding along the floor still finds a wall in its way.
#[test]
fn sliding_particles_stop_at_a_wall() {
    let Some(mut ground) = Ground::with_wall(1.5) else {
        return;
    };
    let effect = Effect::parse(
        "emitter e\n budget 20\n spawn burst=10\n lifetime 100\n shape box 0.2 0.02 0.2\n offset -2 0.3 0\n velocity 12 0 0\n size 0.04\n gravity\n collide field bounce restitution=0 friction=0",
    )
    .unwrap();
    ground
        .renderer
        .particles()
        .spawn(&effect, Mat4::IDENTITY)
        .unwrap();
    for _ in 0..60 {
        ground.frame();
    }
    let [(_, live)] = ground.emitters().try_into().ok().unwrap();
    assert_eq!(live.len(), 10);
    let furthest = live.iter().map(|p| p.position[0]).fold(f32::MIN, f32::max);
    // Sliding on at 12 m/s would have taken them to x = 9. The field finds the wall to a voxel.
    assert!(furthest < 1.9, "a particle reached x = {furthest}");
    assert!(furthest > 1.0, "they stopped short: {furthest}");
}

/// Hits past what an emitter takes in a frame are counted as dropped, not lost unseen.
#[test]
fn events_past_the_limit_count_as_dropped() {
    let Some(mut ground) = Ground::new(0.0, None) else {
        return;
    };
    let effect = Effect::parse(
        "
        emitter e
          budget 400
          spawn burst=400
          lifetime 100
          shape box 1 0 1
          offset 0 1 -1
          size 0.01
          gravity
          collide depth kill event=hit
        emitter child
          budget 1000
          spawn event=hit count=1
          lifetime 100
          size 0.01
        ",
    )
    .unwrap();
    ground
        .renderer
        .particles()
        .spawn(&effect, Mat4::IDENTITY)
        .unwrap();
    for _ in 0..50 {
        ground.frame();
    }
    let emitters = ground.emitters();
    assert!(emitters[0].1.is_empty(), "every drop landed");
    let child = emitters[1].0;
    assert!(child.dropped > 0, "they all landed in the same few frames");
    assert_eq!(child.spawned + child.dropped, 400);
}

/// What is already behind a surface when the frame begins stays there, rather than being moved
/// onto it: a particle under the floor, or a bubble under the sea.
#[test]
fn particles_behind_a_surface_are_left_alone() {
    let cases = [
        (0.0, None, "depth bounce", -0.2),
        (-12.0, Some(0.0), "water kill", -1.0),
    ];
    for (floor, sea, collide, start) in cases {
        let Some(mut ground) = Ground::new(floor, sea) else {
            return;
        };
        let effect = Effect::parse(&format!(
            "emitter e\n budget 40\n spawn burst=20\n lifetime 100\n shape box 2 0 2\n offset 0 {start} -1\n velocity 0 -0.2 0\n size 0.02\n collide {collide}"
        ))
        .unwrap();
        ground
            .renderer
            .particles()
            .spawn(&effect, Mat4::IDENTITY)
            .unwrap();
        for _ in 0..30 {
            ground.frame();
        }
        let [(_, live)] = ground.emitters().try_into().ok().unwrap();
        assert_eq!(live.len(), 20, "{collide}");
        assert!(
            live.iter().all(|p| p.position[1] < start),
            "{collide}: a particle was moved up"
        );
    }
}

/// An effect whose emitters spawn on events lasts until the last of those could have died: they
/// can be born as late as their sender's last particle dies.
#[test]
fn effects_with_events_outlast_their_senders_children() {
    let Some(mut rig) = Rig::new(1000) else {
        return;
    };
    let effect = Effect::parse(
        "emitter e\n budget 100\n spawn rate=60\n lifetime 0.5\n collide depth kill event=hit\nemitter child\n budget 100\n spawn event=hit count=1\n lifetime 0.5",
    )
    .unwrap();
    let id = rig.particles.spawn(&effect, Mat4::IDENTITY).unwrap();
    rig.frame(1.0 / 30.0);
    rig.particles.stop(id);
    // Past either life and the margin, but not their sum and the margin.
    for _ in 0..36 {
        rig.frame(1.0 / 30.0);
    }
    assert!(rig.particles.is_running(id));
    for _ in 0..12 {
        rig.frame(1.0 / 30.0);
    }
    assert!(!rig.particles.is_running(id));
}

/// Drops falling on the sea die where they meet it, each sending its splash: the splash
/// emitter spawns its count for every drop that hit.
#[test]
fn drops_landing_on_the_sea_splash() {
    let Some(mut ground) = Ground::new(-12.0, Some(0.0)) else {
        return;
    };
    let effect = Effect::parse(
        "
        emitter drops
          budget 200
          spawn burst=200
          lifetime 100
          shape box 4 0.2 4
          offset 0 3 0
          size 0.02
          gravity
          collide water kill event=splash
        emitter splash
          budget 1200
          spawn event=splash count=3
          lifetime 100
          velocity 0 1 0 cone=30
          size 0.01
        ",
    )
    .unwrap();
    ground
        .renderer
        .particles()
        .spawn(&effect, Mat4::IDENTITY)
        .unwrap();
    let mut deepest = f32::MAX;
    for _ in 0..90 {
        ground.frame();
        let emitters = ground.emitters();
        deepest = emitters[0]
            .1
            .iter()
            .map(|p| p.position[1])
            .fold(deepest, f32::min);
    }
    let emitters = ground.emitters();
    let (drops, splash) = (&emitters[0], &emitters[1]);
    assert!(drops.1.is_empty(), "{} drops still falling", drops.1.len());
    // No living drop ever went more than a step's fall below the waves.
    assert!(deepest > -1.0, "a drop reached {deepest} m");
    assert_eq!(
        splash.0.spawned,
        3 * 200,
        "three splash particles for each drop"
    );
    assert_eq!(splash.0.dropped, 0);
}

/// Sticking particles stay where they landed.
#[test]
fn sticking_particles_stay_put() {
    let Some(mut ground) = Ground::new(0.0, None) else {
        return;
    };
    let effect = Effect::parse(
        "emitter flakes\n budget 200\n spawn burst=150\n lifetime 100\n shape box 5 1 5\n offset 0 2 0\n size 0.03\n gravity 3\n collide depth field stick",
    )
    .unwrap();
    ground
        .renderer
        .particles()
        .spawn(&effect, Mat4::IDENTITY)
        .unwrap();
    for _ in 0..90 {
        ground.frame();
    }
    let [(_, before)] = ground.emitters().try_into().ok().unwrap();
    assert!(before
        .iter()
        .all(|p| p.flags & reference::PARTICLE_STUCK != 0));
    for _ in 0..20 {
        ground.frame();
    }
    let [(_, after)] = ground.emitters().try_into().ok().unwrap();
    let place = |ps: &[Particle]| -> HashMap<u32, [f32; 3]> {
        ps.iter().map(|p| (p.index, p.position)).collect()
    };
    assert_eq!(place(&before), place(&after));
    assert!(after
        .iter()
        .all(|p| p.position[1] > -0.25 && p.position[1] < 0.3));
}
