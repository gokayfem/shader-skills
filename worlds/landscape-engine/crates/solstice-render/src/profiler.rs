//! GPU time per section of the frame, where the device can measure it.
//!
//! Timestamps between passes need a native-only feature, so browsers simply report nothing.
//! Readback follows the same never-block cycle as culling statistics, so results lag
//! slightly behind the frame they describe.

use crate::gpu::Gpu;
use std::cell::Cell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// The sections of a frame, in the order they run.
pub const SECTIONS: [&str; 26] = [
    "sky",
    "clouds",
    "ocean",
    "wind",
    "shadows",
    "wet line",
    "rain",
    "gi cull",
    "gi voxelize",
    "gi flood",
    "gi write",
    "light shadows",
    "gi light",
    "cull 1",
    "raster 1",
    "cull 2",
    "raster 2",
    "resolve",
    "clouds on screen",
    "screen gi",
    "lighting",
    "water",
    "particles",
    "particle draw",
    "rain draw",
    "post",
];
/// Indices into [`SECTIONS`], for the marks that begin each one.
pub mod section {
    pub const SKY: usize = 0;
    pub const CLOUDS: usize = 1;
    /// The sea's waves.
    pub const OCEAN: usize = 2;
    /// The wind near the ground, for what sways in it.
    pub const WIND: usize = 3;
    pub const SHADOWS: usize = 4;
    /// Where the sea stands and has stood, for wetness.
    pub const WATERLINE: usize = 5;
    /// The rain's cover map, its light and its ripples.
    pub const RAIN: usize = 6;
    pub const GI_CULL: usize = 7;
    pub const GI_VOXELIZE: usize = 8;
    pub const GI_FLOOD: usize = 9;
    pub const GI_WRITE: usize = 10;
    /// The local lights' shadow maps, traced through the volume.
    pub const LIGHT_SHADOWS: usize = 11;
    pub const GI_LIGHT: usize = 12;
    /// Culling and rasterizing what was visible last frame, then the occlusion pyramid.
    pub const CULL_FIRST: usize = 13;
    pub const RASTER_FIRST: usize = 14;
    /// Culling and rasterizing what that revealed.
    pub const CULL_SECOND: usize = 15;
    pub const RASTER_SECOND: usize = 16;
    pub const RESOLVE: usize = 17;
    pub const CLOUDS_ON_SCREEN: usize = 18;
    pub const SCREEN_GI: usize = 19;
    pub const LIGHTING: usize = 20;
    /// The sea's surface, drawn and shaded.
    pub const WATER: usize = 21;
    /// Spawning and moving particles.
    pub const PARTICLES: usize = 22;
    /// Drawing them over the scene.
    pub const PARTICLE_DRAW: usize = 23;
    /// Drawing the rain.
    pub const RAIN_DRAW: usize = 24;
    pub const POST: usize = 25;
    pub const END: usize = 26;
}
const QUERY_COUNT: u32 = SECTIONS.len() as u32 + 1;
const BYTES: u64 = QUERY_COUNT as u64 * 8;

/// Records timestamps and reads them back.
pub struct Profiler {
    queries: wgpu::QuerySet,
    resolve: wgpu::Buffer,
    staging: wgpu::Buffer,
    period_ns: f32,
    copied: bool,
    busy: bool,
    mapped: Arc<AtomicBool>,
    latest_ms: Option<[f32; SECTIONS.len()]>,
    /// The last mark written this frame.
    last_mark: Cell<Option<usize>>,
}

impl Profiler {
    /// Returns `None` if the device cannot take timestamps between passes.
    pub fn new(gpu: &Gpu) -> Option<Self> {
        let needed =
            wgpu::Features::TIMESTAMP_QUERY | wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS;
        if !gpu.device.features().contains(needed) {
            return None;
        }
        Some(Self {
            queries: gpu.device.create_query_set(&wgpu::QuerySetDescriptor {
                label: Some("frame timestamps"),
                ty: wgpu::QueryType::Timestamp,
                count: QUERY_COUNT,
            }),
            resolve: gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("timestamp resolve"),
                size: BYTES,
                usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            }),
            staging: gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("timestamp staging"),
                size: BYTES,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            period_ns: gpu.queue.get_timestamp_period(),
            copied: false,
            busy: false,
            mapped: Default::default(),
            latest_ms: None,
            last_mark: Cell::new(None),
        })
    }

    /// Marks the boundary before section `index`. Pass `SECTIONS.len()` to end the frame.
    ///
    /// Sections that did not run this frame, such as GI on a frame it is not due, are marked
    /// here too, so they read as zero and the time since the last mark goes to the section that
    /// really spent it, rather than being lost against a timestamp left over from another frame.
    pub fn mark(&self, encoder: &mut wgpu::CommandEncoder, index: usize) {
        let first = match self.last_mark.get() {
            Some(last) if last < index && index != 0 => last + 1,
            _ => index,
        };
        for skipped in first..=index {
            encoder.write_timestamp(&self.queries, skipped as u32);
        }
        self.last_mark.set(if index == section::END {
            None
        } else {
            Some(index)
        });
    }

    /// Consumes any finished readback and queues the next. Call at the end of encoding.
    pub fn resolve(&mut self, encoder: &mut wgpu::CommandEncoder) {
        if self.mapped.swap(false, Ordering::AcqRel) {
            {
                let data = self
                    .staging
                    .slice(..)
                    .get_mapped_range()
                    .expect("the map has completed");
                // Copied out rather than cast in place: a mapped range is only promised
                // 4-byte alignment, and the AMD integrated GPU gives no more.
                let ticks: Vec<u64> = bytemuck::pod_collect_to_vec(&data);
                let mut result = [0.0; SECTIONS.len()];
                for (section, slot) in result.iter_mut().enumerate() {
                    let elapsed = ticks[section + 1].saturating_sub(ticks[section]);
                    *slot = elapsed as f32 * self.period_ns / 1.0e6;
                }
                self.latest_ms = Some(result);
            }
            self.staging.unmap();
            self.busy = false;
        }
        encoder.resolve_query_set(&self.queries, 0..QUERY_COUNT, &self.resolve, 0);
        if !self.busy {
            encoder.copy_buffer_to_buffer(&self.resolve, 0, &self.staging, 0, BYTES);
            self.copied = true;
        }
    }

    /// Starts reading back timestamps copied this frame. Call after submitting.
    pub fn request_map(&mut self) {
        if std::mem::take(&mut self.copied) {
            self.busy = true;
            let mapped = self.mapped.clone();
            self.staging
                .slice(..)
                .map_async(wgpu::MapMode::Read, move |result| {
                    if result.is_ok() {
                        mapped.store(true, Ordering::Release);
                    }
                });
        }
    }

    /// Milliseconds per entry of [`SECTIONS`], from a recent frame.
    pub fn latest_ms(&self) -> Option<[f32; SECTIONS.len()]> {
        self.latest_ms
    }
}
