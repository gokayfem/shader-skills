//! Lightning and thunder (`docs/weather-effects.md`, step 5).
//!
//! **When**: strikes come at random, as a Poisson process at the weather's `thunder` rate, in
//! strikes a minute. Most are in the cloud and only light it from within; about a third reach
//! the ground with a bolt.
//!
//! **How a flash goes** (Rakov and Uman, *Lightning: Physics and Effects*): a flash to the ground
//! is a few return strokes, typically three to five, tens of milliseconds apart. Each is a
//! brilliant spike decaying over tens of milliseconds; the last often carries a continuing
//! current that glows for a few tenths of a second. A flash within the cloud is one or more
//! slower pulses.
//!
//! **What it lights**: the cloud deck glows from within around the flash, so the sky brightens
//! towards it, and the scene is lit from that way, through the same sky light that reflections,
//! water and rain see (`clouds/params.wgsl`, `flash`). Surfaces facing it are lit directly,
//! shadowed by a trace through the GI volume's distance field; the rest by the lit cloud deck.
//! The bolt is drawn as a glowing, branching channel (`lightning/bolt.wgsl`). Exposure holds
//! still while a flash lasts, so it dazzles, as it does the eye.
//!
//! **Thunder** reaches the camera at the speed of sound, a crack from a near bolt, a long low
//! rumble from a far one: the renderer reports each strike, and the web page makes the sound.

use crate::clouds::Weather;
use crate::gpu::{Gpu, LayoutBuilder};
use crate::shaders;
use bytemuck::{Pod, Zeroable};
use glam::{Vec2, Vec3};

/// The speed of sound in air, m/s.
pub const SPEED_OF_SOUND: f32 = 343.0;
/// How near and how far strikes fall from the camera, in metres.
pub const NEAREST: f32 = 600.0;
pub const FARTHEST: f32 = 9000.0;
/// The share of flashes that reach the ground.
const TO_GROUND: f32 = 0.35;
/// How long a return stroke's light takes to fall to 1/e, and a continuing current's, s.
const STROKE_DECAY: f32 = 0.045;
const CONTINUING_DECAY: f32 = 0.25;
const CONTINUING_SHARE: f32 = 0.18;
/// How long a stroke takes to brighten, s.
const STROKE_RISE: f32 = 0.004;
/// The illuminance a flash to the ground gives the scene at 2 km at its brightest, in the
/// scene's units of light (as the sun's, in lux).
const GROUND_FLASH: f32 = 600.0;
/// The same for a flash within the cloud: dimmer, spread through the cloud.
const CLOUD_FLASH: f32 = 250.0;
/// The brightest a flash ever lights the scene, however near.
const BRIGHTEST: f32 = 8000.0;
/// Seconds a flash's light, and its hold on exposure, lasts at most.
const LONGEST: f32 = 1.5;
/// Segments of a bolt at most, its channel and its branches together. Twin of `MAX_SEGMENTS`
/// in `lightning/bolt.wgsl`.
pub const MAX_SEGMENTS: usize = 384;

/// What kind of flash.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// From the cloud to the ground, with a bolt.
    ToGround,
    /// Within the cloud: it lights the cloud and the scene, with no bolt to see.
    InCloud,
}

/// One part of a bolt's channel: from `a` to `b`, `width` metres across, and how bright, 0 to 1:
/// the main channel 1, branches dimmer as they fork.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Segment {
    pub a: Vec3,
    pub b: Vec3,
    pub width: f32,
    pub brightness: f32,
}

/// One flash.
#[derive(Clone, Debug, PartialEq)]
pub struct Strike {
    pub kind: Kind,
    /// Where it lights the cloud from, in the world.
    pub at: Vec3,
    /// How far from the camera it was when it struck, in metres along the ground.
    pub distance: f32,
    /// Its strokes: each when it begins, in seconds after the flash began, and how bright, 0 to 1.
    pub strokes: Vec<(f32, f32)>,
    /// Its bolt, if it reaches the ground.
    pub bolt: Vec<Segment>,
    /// Seconds since it began.
    pub age: f32,
}

impl Strike {
    /// How bright it is `age` seconds after it began, against its brightest stroke at its peak.
    pub fn brightness_at(&self, age: f32) -> f32 {
        let mut light = 0.0_f32;
        let last = self.strokes.len().saturating_sub(1);
        for (n, &(start, peak)) in self.strokes.iter().enumerate() {
            let t = age - start;
            if t < 0.0 {
                continue;
            }
            let rise = (t / STROKE_RISE).min(1.0);
            let mut stroke = peak * rise * (-t / STROKE_DECAY).exp();
            if n == last {
                stroke += peak * CONTINUING_SHARE * rise * (-t / CONTINUING_DECAY).exp();
            }
            light = light.max(stroke);
        }
        light
    }

    /// How bright it is now.
    pub fn brightness(&self) -> f32 {
        self.brightness_at(self.age)
    }

    /// The illuminance it gives the scene at its brightest.
    pub fn peak_illuminance(&self) -> f32 {
        let at_two_km = match self.kind {
            Kind::ToGround => GROUND_FLASH,
            Kind::InCloud => CLOUD_FLASH,
        };
        (at_two_km * (2000.0 / self.distance.max(NEAREST)).powi(2)).min(BRIGHTEST)
    }

    /// The illuminance it gives the scene now, in the scene's units of light.
    pub fn illuminance(&self) -> f32 {
        let at_two_km = match self.kind {
            Kind::ToGround => GROUND_FLASH,
            Kind::InCloud => CLOUD_FLASH,
        };
        (at_two_km * (2000.0 / self.distance.max(NEAREST)).powi(2) * self.brightness())
            .min(BRIGHTEST)
    }

    /// Whether anything of it is still to be seen.
    pub fn alive(&self) -> bool {
        self.age < LONGEST
    }
}

/// Thunder on its way to the camera, for the page to sound.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Thunder {
    /// Seconds from now until it is heard.
    pub delay: f32,
    /// How far off the strike was, in metres.
    pub distance: f32,
    /// How loud, 0 to 1, for a strike at the nearest.
    pub loudness: f32,
    /// How long it rolls, in seconds: a far strike's channel spans a wide spread of distances.
    pub rumble: f32,
    /// Whether the bolt reached the ground, which cracks; a flash in the cloud only rumbles.
    pub to_ground: bool,
}

/// The flash as the shaders see it: from where, and how bright.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Flash {
    /// Towards the flash from the camera, a unit direction.
    pub direction: Vec3,
    /// The illuminance it gives the scene, in the scene's units of light.
    pub illuminance: f32,
}

/// The storm's lightning as it goes on.
#[derive(Clone, Debug)]
pub struct Lightning {
    state: u64,
    strike: Option<Strike>,
    thunder: Vec<Thunder>,
    /// Seconds a requested strike waits, if one was asked for.
    requested: bool,
}

impl Default for Lightning {
    fn default() -> Self {
        Self::new(0x2545_f491_4f6c_dd1d)
    }
}

impl Lightning {
    pub fn new(seed: u64) -> Self {
        Self {
            state: seed | 1,
            strike: None,
            thunder: Vec::new(),
            requested: false,
        }
    }

    /// A random number in `0..1`.
    fn random(&mut self) -> f32 {
        // xorshift64*
        self.state ^= self.state >> 12;
        self.state ^= self.state << 25;
        self.state ^= self.state >> 27;
        let bits = self.state.wrapping_mul(0x2545_f491_4f6c_dd1d) >> 40;
        bits as f32 / (1u64 << 24) as f32
    }

    fn between(&mut self, low: f32, high: f32) -> f32 {
        low + (high - low) * self.random()
    }

    /// Asks for a strike to the ground on the next frame, whatever the weather: for a key in the
    /// viewer, and for tests.
    pub fn strike_soon(&mut self) {
        self.requested = true;
    }

    /// Moves the storm on by `seconds` in `weather`, around a camera at `camera`. A new strike
    /// may begin; its thunder is queued for [`Lightning::take_thunder`]. One asked for falls
    /// ahead, along `facing`, where it can be seen.
    pub fn advance(&mut self, weather: &Weather, camera: Vec3, seconds: f32) {
        self.advance_facing(weather, camera, Vec3::NEG_Z, seconds);
    }

    /// As [`Lightning::advance`], for a camera looking along `facing`.
    pub fn advance_facing(&mut self, weather: &Weather, camera: Vec3, facing: Vec3, seconds: f32) {
        let seconds = seconds.max(0.0);
        if let Some(strike) = &mut self.strike {
            strike.age += seconds;
            if !strike.alive() {
                self.strike = None;
            }
        }
        for thunder in &mut self.thunder {
            thunder.delay -= seconds;
        }
        let rate = weather.thunder / 60.0;
        let due = self.requested || (rate > 0.0 && self.random() < 1.0 - (-rate * seconds).exp());
        if due && self.strike.is_none() {
            let asked = self.requested;
            self.requested = false;
            let strike = self.make_strike(weather, camera, facing, asked);
            // A host that never sounds thunder keeps only the latest.
            if self.thunder.len() >= 32 {
                self.thunder.remove(0);
            }
            self.thunder.push(Thunder {
                delay: strike.distance / SPEED_OF_SOUND,
                distance: strike.distance,
                loudness: (NEAREST / strike.distance).min(1.0),
                rumble: 2.0 + strike.distance / 1200.0,
                to_ground: strike.kind == Kind::ToGround,
            });
            self.strike = Some(strike);
        }
    }

    /// Thunder from strikes since the last call, each with its delay from now: the page plays
    /// them when they arrive.
    pub fn take_thunder(&mut self) -> Vec<Thunder> {
        std::mem::take(&mut self.thunder)
    }

    /// The flash now, if any, seen from `camera`.
    pub fn flash(&self, camera: Vec3) -> Option<Flash> {
        let strike = self.strike.as_ref()?;
        let illuminance = strike.illuminance();
        if illuminance <= 0.0 {
            return None;
        }
        Some(Flash {
            direction: (strike.at - camera).normalize_or(Vec3::Y),
            illuminance,
        })
    }

    /// The strike lighting the sky now, if any.
    pub fn strike(&self) -> Option<&Strike> {
        self.strike.as_ref()
    }

    /// Whether exposure should hold still: while a flash bright enough to see lights the scene,
    /// as an eye stays dazzled, through its strokes and a moment after.
    pub fn holds_exposure(&self) -> bool {
        self.strike.as_ref().is_some_and(|s| {
            let last = s.strokes.last().map_or(0.0, |l| l.0);
            s.age < last + 0.3 && s.peak_illuminance() > 1.0
        })
    }

    fn make_strike(
        &mut self,
        weather: &Weather,
        camera: Vec3,
        facing: Vec3,
        asked: bool,
    ) -> Strike {
        // One asked for is the kind worth asking for: a bolt to the ground.
        let kind = if asked || self.random() < TO_GROUND {
            Kind::ToGround
        } else {
            Kind::InCloud
        };
        // Distances spread as the area of each ring of ground grows, but storms are seldom
        // right overhead: from 600 m to 9 km, most of them a few kilometres off.
        let mut distance = NEAREST + (FARTHEST - NEAREST) * self.random().powf(1.4);
        let mut bearing = self.between(0.0, std::f32::consts::TAU);
        if asked {
            // Ahead, within a third of a right angle of where the camera looks, a few km off.
            distance = self.between(1500.0, 4000.0);
            bearing = facing.z.atan2(facing.x) + self.between(-0.5, 0.5);
        }
        let ground = Vec2::new(camera.x, camera.z) + Vec2::from_angle(bearing) * distance;
        let base = weather.base.max(300.0);
        let at = Vec3::new(ground.x, base + self.between(100.0, 900.0), ground.y);
        let strokes = match kind {
            Kind::ToGround => {
                let count = 1 + (self.random() * 4.5) as usize;
                let mut start = 0.0;
                (0..count)
                    .map(|n| {
                        let stroke = (start, if n == 0 { 1.0 } else { self.between(0.4, 0.9) });
                        start += self.between(0.03, 0.09);
                        stroke
                    })
                    .collect()
            }
            Kind::InCloud => {
                let count = 1 + (self.random() * 2.5) as usize;
                let mut start = 0.0;
                (0..count)
                    .map(|_| {
                        let stroke = (start, self.between(0.5, 1.0));
                        start += self.between(0.08, 0.25);
                        stroke
                    })
                    .collect()
            }
        };
        // Into the ground: the land there is unknown, so the bolt runs below the camera's feet,
        // and whatever stands in front hides where it ends.
        let bolt = match kind {
            Kind::ToGround => self.grow_bolt(Vec3::new(ground.x, base, ground.y), camera.y - 30.0),
            Kind::InCloud => Vec::new(),
        };
        Strike {
            kind,
            at,
            distance,
            strokes,
            bolt,
            age: 0.0,
        }
    }

    /// A channel from the cloud base at `top` down to `ground`, stepping as a leader does:
    /// jagged, wandering, with branches forking off and dying out before they reach the ground.
    fn grow_bolt(&mut self, top: Vec3, ground: f32) -> Vec<Segment> {
        let mut segments = Vec::new();
        let height = (top.y - ground).max(100.0);
        let steps = 48;
        let step = height / steps as f32;
        let mut at = top;
        let mut drift = Vec2::ZERO;
        for n in 0..steps {
            // The channel wanders sideways, more in short kinks than in long sweeps.
            drift = drift * 0.6
                + Vec2::new(self.between(-1.0, 1.0), self.between(-1.0, 1.0)) * step * 0.9;
            let next = Vec3::new(
                at.x + drift.x,
                at.y - step * self.between(0.7, 1.3),
                at.z + drift.y,
            );
            let next = if n + 1 == steps {
                Vec3::new(next.x, ground, next.z)
            } else {
                next
            };
            segments.push(Segment {
                a: at,
                b: next,
                width: 3.0,
                brightness: 1.0,
            });
            // Now and then a branch forks off, downwards and out, and fades as it goes.
            if n < steps - 6 && self.random() < 0.18 && segments.len() < MAX_SEGMENTS - 16 {
                let mut branch = next;
                let out = Vec2::from_angle(self.between(0.0, std::f32::consts::TAU));
                let length = 4 + (self.random() * 12.0) as usize;
                for k in 0..length {
                    let wander =
                        Vec2::new(self.between(-1.0, 1.0), self.between(-1.0, 1.0)) * step * 0.7;
                    let side = out * step * 0.8 + wander;
                    let following = Vec3::new(
                        branch.x + side.x,
                        branch.y - step * self.between(0.4, 1.0),
                        branch.z + side.y,
                    );
                    if following.y < ground + step {
                        break;
                    }
                    segments.push(Segment {
                        a: branch,
                        b: following,
                        width: 1.5,
                        brightness: 0.45 * (1.0 - k as f32 / length as f32),
                    });
                    branch = following;
                    if segments.len() >= MAX_SEGMENTS {
                        break;
                    }
                }
            }
            at = next;
        }
        segments.truncate(MAX_SEGMENTS);
        segments
    }
}

/// The bolt channel's radiance at full brightness, in the scene's units: far brighter than
/// anything else in the sky, so it blooms.
const BOLT_RADIANCE: [f32; 3] = [2.0e5, 2.2e5, 2.6e5];
/// How much the air between dims a bolt, per metre: rain-laden, but not so thick that a bolt a
/// few kilometres off vanishes.
const BOLT_AIR: f32 = 1.0 / 4500.0;

/// Twin of `BoltParams` in `lightning/bolt.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
struct BoltUniform {
    color: [f32; 4],
    air: [f32; 4],
}

/// Twin of `Segment` in `lightning/bolt.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
struct GpuSegment {
    a: [f32; 4],
    b: [f32; 4],
}

/// Draws the bolt of a flash to the ground over the scene.
pub struct BoltDraw {
    uniform: wgpu::Buffer,
    segments: wgpu::Buffer,
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
    /// The bind group, and the scene depth it was made for.
    group: Option<(usize, wgpu::BindGroup)>,
    /// The bolt now uploaded, by its first segment, and how many segments it has.
    uploaded: Option<(Vec3, u32)>,
}

impl BoltDraw {
    pub fn new(gpu: &Gpu, color_format: wgpu::TextureFormat) -> Self {
        let uniform = gpu.uniform_buffer("bolt", std::mem::size_of::<BoltUniform>() as u64);
        let segments = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("bolt segments"),
            size: (MAX_SEGMENTS * std::mem::size_of::<GpuSegment>()) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let layout = LayoutBuilder::new()
            .visibility(wgpu::ShaderStages::VERTEX_FRAGMENT)
            .uniform()
            .uniform()
            .storage_read()
            .texture_unfilterable()
            .build(gpu, "bolt");
        let module = shaders::module(&gpu.device, "lightning/bolt.wgsl");
        let pipeline_layout = gpu
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("bolt"),
                bind_group_layouts: &[Some(&layout)],
                immediate_size: 0,
            });
        let add = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::One,
            operation: wgpu::BlendOperation::Add,
        };
        let pipeline = gpu
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("bolt"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some("fs_main"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: color_format,
                        blend: Some(wgpu::BlendState {
                            color: add,
                            alpha: add,
                        }),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            });
        Self {
            uniform,
            segments,
            layout,
            pipeline,
            group: None,
            uploaded: None,
        }
    }

    /// Draws `strike`'s bolt over `color`, if it has one and is bright, tested against
    /// `scene_depth`, which `depth_key` names.
    #[allow(clippy::too_many_arguments)]
    pub fn record(
        &mut self,
        gpu: &Gpu,
        encoder: &mut wgpu::CommandEncoder,
        frame: &wgpu::Buffer,
        strike: &Strike,
        color: &wgpu::TextureView,
        scene_depth: &wgpu::TextureView,
        depth_key: usize,
    ) {
        let brightness = strike.brightness();
        if strike.bolt.is_empty() || brightness < 1.0e-3 {
            return;
        }
        let count = strike.bolt.len().min(MAX_SEGMENTS) as u32;
        let key = (strike.bolt[0].a, count);
        if self.uploaded != Some(key) {
            let data: Vec<GpuSegment> = strike
                .bolt
                .iter()
                .take(MAX_SEGMENTS)
                .map(|s| GpuSegment {
                    a: s.a.extend(s.width).to_array(),
                    b: s.b.extend(s.brightness).to_array(),
                })
                .collect();
            gpu.queue
                .write_buffer(&self.segments, 0, bytemuck::cast_slice(&data));
            self.uploaded = Some(key);
        }
        gpu.queue.write_buffer(
            &self.uniform,
            0,
            bytemuck::bytes_of(&BoltUniform {
                color: [
                    BOLT_RADIANCE[0],
                    BOLT_RADIANCE[1],
                    BOLT_RADIANCE[2],
                    brightness,
                ],
                air: [BOLT_AIR, count as f32, 0.0, 0.0],
            }),
        );
        if self.group.as_ref().is_none_or(|(k, _)| *k != depth_key) {
            let group = gpu.bind_group(
                "bolt",
                &self.layout,
                &[
                    frame.as_entire_binding(),
                    self.uniform.as_entire_binding(),
                    self.segments.as_entire_binding(),
                    wgpu::BindingResource::TextureView(scene_depth),
                ],
            );
            self.group = Some((depth_key, group));
        }
        let (_, group) = self.group.as_ref().expect("just made");
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("bolt"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: color,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, group, &[]);
        pass.draw(0..6, 0..count);
    }
}

#[cfg(test)]
mod tests;
