//! The wind near the ground: one field that what sways, the water, rain and particles all read,
//! so that a gust crosses all of them in turn (`docs/weather-effects.md`).
//!
//! The weather gives the wind at cloud height. Near the ground it is about half that, and it
//! never blows quite steadily:
//! * its mean turns back and forth by some degrees and swells and slackens over tens of seconds;
//! * gusts and lulls tens of metres across, with rolls a few metres across within them, are
//!   carried downwind at the mean wind's speed, changing shape slowly as they go (Taylor's frozen
//!   turbulence). A gust is seen running across a meadow, then a pond;
//! * its direction wanders from place to place, more in turbulent air.
//!
//! [`WindSnapshot::velocity_at`] is the field itself, on the CPU. `common/wind.wgsl` is its twin
//! on the GPU, and `wind/field.wgsl` evaluates it each frame over a square around the camera, this
//! frame's and last frame's, into a texture that what sways reads (`common/sway_vertex.wgsl`).

use crate::clouds::Weather;
use crate::gpu::{groups_for, Gpu, LayoutBuilder, Target};
use crate::particles::reference::gradient_noise;
use crate::shaders;
use bytemuck::{Pod, Zeroable};
use glam::{Vec2, Vec3};

/// Texels along each side of the wind field. Twin of `FIELD_TEXELS` in `wind/field.wgsl` and
/// `WIND_FIELD_TEXELS` in `common/sway_vertex.wgsl`.
pub const FIELD_TEXELS: u32 = 128;
/// Metres between the field's texels: 256 m across in all, enough for everything that sways in
/// view to sway, and fine enough for gusts' rolls.
pub const FIELD_TEXEL_METRES: f32 = 2.0;
/// The format of the field. 32-bit floats are written by a compute pass in core WebGPU and read
/// without a sampler.
pub const FIELD_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba32Float;

/// The farthest the wind carries a point of grass, as a share of its height. Twin of
/// `GRASS_REACH` in `common/sway.wgsl`, which culling grows bounds by.
pub const GRASS_REACH: f32 = 0.65;
/// The farthest the wind carries a point of a tree, as a share of its height plus a reach, in
/// metres. Twins of `TREE_REACH` and `TREE_REACH_BASE` in `common/sway.wgsl`.
pub const TREE_REACH: f32 = 0.1;
pub const TREE_REACH_BASE: f32 = 0.6;

/// The wind 10 m up as a share of the weather's wind at cloud height.
pub const GROUND_SHARE: f32 = 0.5;
/// How far the mean wind turns either way, in degrees, and over about how long.
const VEER_DEGREES: f32 = 12.0;
const VEER_SECONDS: f32 = 45.0;
/// How far the mean wind swells and slackens, as a share of it, and over about how long.
const SWELL: f32 = 0.15;
const SWELL_SECONDS: f32 = 23.0;

/// Twins of the constants of `wind_velocity` in `common/wind.wgsl`.
const GUST_SIZE: f32 = 42.0;
const ROLL_SIZE: f32 = 11.0;
const VEER_SIZE: f32 = 50.0;
const GUST_CHANGE: f32 = 0.045;
const ROLL_CHANGE: f32 = 0.2;
const VEER_CHANGE: f32 = 0.07;
const GUST_STRENGTH: f32 = 1.4;
const ROLL_STRENGTH: f32 = 0.6;
const VEER_ANGLE: f32 = 0.7;
const LULL: f32 = 0.1;
const SEED_GUST: u32 = 0x5bd1_e995;
const SEED_ROLL: u32 = 0x27d4_eb2f;
const SEED_VEER: u32 = 0x1656_67b1;
const SEED_MEAN_VEER: u32 = 0x9e37_79b9;
const SEED_MEAN_SWELL: u32 = 0x85eb_ca6b;

/// The wind at one moment. Twin of `WindSnapshot` in `common/wind.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct WindSnapshot {
    /// The mean wind 10 m up, in m/s as (east, south): the scene's +X and +Z.
    pub mean: [f32; 2],
    /// How far the air has carried everything since the wind began, in metres.
    pub offset: [f32; 2],
    /// Seconds since the wind began.
    pub time: f32,
    /// `0..1`, from the weather.
    pub gusts: f32,
    pub turbulence: f32,
    pub _pad: f32,
}

impl WindSnapshot {
    /// Packed into two vectors, as the water's uniforms carry it: the mean wind, gusts and
    /// turbulence; then the air's offset and the time. Twin of `unpack_wind`.
    pub fn packed(&self) -> [[f32; 4]; 2] {
        [
            [self.mean[0], self.mean[1], self.gusts, self.turbulence],
            [self.offset[0], self.offset[1], self.time, 0.0],
        ]
    }

    /// The wind 10 m up at `p`, the world's (x, z), in m/s as (east, south). Twin of
    /// `wind_velocity` in `common/wind.wgsl`.
    pub fn velocity_at(&self, p: Vec2) -> Vec2 {
        let mean = Vec2::from_array(self.mean);
        let speed = mean.length();
        if speed < 1.0e-4 {
            return Vec2::ZERO;
        }
        let q = p - Vec2::from_array(self.offset);
        let noise = |size: f32, change: f32, seed: u32| {
            gradient_noise((q / size).extend(self.time * change), seed).0
        };
        let gust = noise(GUST_SIZE, GUST_CHANGE, SEED_GUST);
        let roll = noise(ROLL_SIZE, ROLL_CHANGE, SEED_ROLL);
        let veer = noise(VEER_SIZE, VEER_CHANGE, SEED_VEER);
        let factor = (1.0 + self.gusts * (GUST_STRENGTH * gust + ROLL_STRENGTH * roll)).max(LULL);
        let angle = self.turbulence * VEER_ANGLE * veer;
        let (s, c) = angle.sin_cos();
        let along = mean / speed;
        Vec2::new(along.x * c - along.y * s, along.x * s + along.y * c) * speed * factor
    }
}

/// The wind as it goes on: where the air has carried everything, and its mean now.
#[derive(Clone, Debug, Default)]
pub struct Wind {
    time: f64,
    offset: [f64; 2],
    now: WindSnapshot,
    previous: WindSnapshot,
    started: bool,
}

/// A slow wander in `-1..1`, `seconds` long or so, from a seed.
fn wander(time: f64, seconds: f32, seed: u32) -> f32 {
    // Noise along a line through the lattice, which wanders about ±0.5; scaled to about ±1.
    let t = (time / f64::from(seconds)).rem_euclid(65_536.0) as f32;
    (gradient_noise(Vec3::new(t, 0.37, 0.71), seed).0 * 2.0).clamp(-1.0, 1.0)
}

impl Wind {
    pub fn new() -> Self {
        Self::default()
    }

    /// Moves the wind on by `seconds` in `weather`. With no time passing, nothing moves, and the
    /// wind a frame ago is the wind now.
    pub fn advance(&mut self, weather: &Weather, seconds: f32) {
        self.previous = self.now;
        self.time += f64::from(seconds.max(0.0));
        let [east, south] = weather.wind_velocity();
        let base = Vec2::new(east, south) * GROUND_SHARE;
        let turn = (VEER_DEGREES * wander(self.time, VEER_SECONDS, SEED_MEAN_VEER)).to_radians();
        let swell = 1.0 + SWELL * wander(self.time, SWELL_SECONDS, SEED_MEAN_SWELL);
        let mean = Vec2::from_angle(turn).rotate(base) * swell;
        self.offset[0] += f64::from(mean.x) * f64::from(seconds.max(0.0));
        self.offset[1] += f64::from(mean.y) * f64::from(seconds.max(0.0));
        self.now = WindSnapshot {
            mean: mean.to_array(),
            offset: [self.offset[0] as f32, self.offset[1] as f32],
            time: self.time as f32,
            gusts: weather.gusts,
            turbulence: weather.turbulence,
            _pad: 0.0,
        };
        if !self.started {
            // There was no wind before the first step to come from.
            self.previous = self.now;
            self.started = true;
        }
    }

    /// The wind now.
    pub fn now(&self) -> &WindSnapshot {
        &self.now
    }

    /// The wind a frame ago.
    pub fn previous(&self) -> &WindSnapshot {
        &self.previous
    }
}

/// What the field pass needs. Twin of `FieldParams` in `wind/field.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct FieldParams {
    pub now: WindSnapshot,
    pub previous: WindSnapshot,
    /// The world's (x, z) at the centre of texel (0, 0).
    pub origin: [f32; 2],
    /// Metres between texel centres.
    pub texel: f32,
    /// 1 if things sway this frame, 0 if they stand still.
    pub sway: f32,
    /// Metres from the camera over which swaying fades out.
    pub fade: [f32; 2],
    pub _pad: [f32; 2],
}

impl FieldParams {
    /// The field around `camera`, the world's (x, z), for `wind`, with swaying fading out over
    /// `fade` metres from the camera.
    pub fn around(camera: Vec2, wind: &Wind, sway: bool, fade: [f32; 2]) -> Self {
        let half = (FIELD_TEXELS / 2) as f32;
        let centre = (camera / FIELD_TEXEL_METRES).floor();
        let origin = (centre - Vec2::splat(half) + Vec2::splat(0.5)) * FIELD_TEXEL_METRES;
        Self {
            now: *wind.now(),
            previous: *wind.previous(),
            origin: origin.to_array(),
            texel: FIELD_TEXEL_METRES,
            sway: if sway { 1.0 } else { 0.0 },
            fade,
            _pad: [0.0; 2],
        }
    }
}

/// The texture the field pass fills: the field, then a row of what readers need besides. A scene
/// with nothing that sways gets a texel, never written, only to bind.
pub fn field_texture(gpu: &Gpu, sways: bool) -> Target {
    let size = if sways {
        (FIELD_TEXELS, FIELD_TEXELS + 1)
    } else {
        (1, 1)
    };
    Target::new(
        gpu,
        "wind field",
        size,
        FIELD_FORMAT,
        wgpu::TextureUsages::COPY_SRC,
    )
}

/// The pass that fills the wind field. Made only for a scene with something that sways.
pub struct WindField {
    pipeline: wgpu::ComputePipeline,
    params: wgpu::Buffer,
    group: wgpu::BindGroup,
}

impl WindField {
    pub fn new(gpu: &Gpu, field: &Target) -> Self {
        let layout = LayoutBuilder::new()
            .uniform()
            .storage_texture(FIELD_FORMAT)
            .build(gpu, "wind field");
        let module = shaders::module(&gpu.device, "wind/field.wgsl");
        let pipeline = gpu.compute_pipeline("wind field", &module, "main", &[&layout]);
        let params = gpu.uniform_buffer("wind field", std::mem::size_of::<FieldParams>() as u64);
        let group = gpu.bind_group(
            "wind field",
            &layout,
            &[
                params.as_entire_binding(),
                wgpu::BindingResource::TextureView(&field.view),
            ],
        );
        Self {
            pipeline,
            params,
            group,
        }
    }

    /// Fills the field from `params`.
    pub fn record(&self, gpu: &Gpu, encoder: &mut wgpu::CommandEncoder, params: &FieldParams) {
        gpu.queue
            .write_buffer(&self.params, 0, bytemuck::bytes_of(params));
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("wind field"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.group, &[]);
        // The extra row holds the parameters.
        pass.dispatch_workgroups(
            groups_for(FIELD_TEXELS, 8),
            groups_for(FIELD_TEXELS + 1, 8),
            1,
        );
    }
}

#[cfg(test)]
mod tests;
