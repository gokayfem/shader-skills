//! Picking for the editor (`docs/editor.md`): which instance is under a pixel, and how far
//! away, read from the visibility buffer once the frame resolves.
//!
//! Its pipeline is made on the first pick, and a frame without a pick records nothing, so a
//! scene that is only looked at pays nothing for it.

use crate::gpu::{Gpu, LayoutBuilder};
use crate::shaders;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// What a pick found.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Picked {
    /// The instance under the pixel, or `None` for the sky.
    pub instance: Option<u32>,
    /// Its reversed depth, as the visibility buffer holds it: `near / distance along the view`.
    pub depth: f32,
}

/// Bytes of the result: found, instance, depth, padding.
const RESULT_BYTES: u64 = 16;

struct Pipeline {
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::ComputePipeline,
    params: wgpu::Buffer,
    result: wgpu::Buffer,
    staging: wgpu::Buffer,
}

/// Picks waiting to be recorded, recorded and waiting to be read, and read.
#[derive(Default)]
pub struct Picker {
    pipeline: Option<Pipeline>,
    /// A point to pick on the next frame, as fractions of the screen from its top left.
    wanted: Option<(f32, f32)>,
    /// Whether a pick was recorded and copied out, and its staging buffer not yet mapped.
    recorded: bool,
    /// Set by the map callback when the staging buffer can be read.
    mapped: Arc<AtomicBool>,
    /// Whether a map is in flight, so another pick waits for it.
    busy: Arc<AtomicBool>,
    result: Arc<Mutex<Option<Picked>>>,
}

impl Picker {
    /// Asks for what is under `point`, as fractions of the screen's width and height from its
    /// top left, on the next frame. A later request before then replaces it.
    pub fn request(&mut self, point: (f32, f32)) {
        self.wanted = Some(point);
    }

    /// The result of the last pick, once, when it has been read back.
    pub fn take(&mut self) -> Option<Picked> {
        self.read();
        self.result.lock().ok()?.take()
    }

    /// Records the pick asked for, if any, after the frame's visibility buffer resolves.
    pub fn record(
        &mut self,
        gpu: &Gpu,
        encoder: &mut wgpu::CommandEncoder,
        vis_depth: &wgpu::Buffer,
        vis_ids: &wgpu::Buffer,
        visible: &wgpu::Buffer,
        size: (u32, u32),
    ) {
        if self.busy.load(Ordering::Acquire) || self.recorded {
            return;
        }
        let Some((x, y)) = self.wanted.take() else {
            return;
        };
        let pipeline = self.pipeline.get_or_insert_with(|| Pipeline::new(gpu));
        let pixel = |t: f32, size: u32| ((t.clamp(0.0, 1.0) * size as f32) as u32).min(size - 1);
        let pixel = [pixel(x, size.0), pixel(y, size.1), size.0, 0];
        gpu.queue
            .write_buffer(&pipeline.params, 0, bytemuck::cast_slice(&pixel));
        let group = gpu.bind_group(
            "pick",
            &pipeline.layout,
            &[
                pipeline.params.as_entire_binding(),
                vis_depth.as_entire_binding(),
                vis_ids.as_entire_binding(),
                visible.as_entire_binding(),
                pipeline.result.as_entire_binding(),
            ],
        );
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("pick"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&pipeline.pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.dispatch_workgroups(1, 1, 1);
        }
        encoder.copy_buffer_to_buffer(&pipeline.result, 0, &pipeline.staging, 0, RESULT_BYTES);
        self.recorded = true;
    }

    /// Starts reading back a recorded pick, once its frame is submitted.
    pub fn request_map(&mut self) {
        let Some(pipeline) = &self.pipeline else {
            return;
        };
        if !std::mem::take(&mut self.recorded) {
            return;
        }
        self.busy.store(true, Ordering::Release);
        let (mapped, busy) = (self.mapped.clone(), self.busy.clone());
        pipeline
            .staging
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| match result {
                Ok(()) => mapped.store(true, Ordering::Release),
                Err(_) => busy.store(false, Ordering::Release),
            });
    }

    fn read(&mut self) {
        let Some(pipeline) = &self.pipeline else {
            return;
        };
        if !self.mapped.swap(false, Ordering::AcqRel) {
            return;
        }
        let words: [u32; 4] = {
            let view = pipeline
                .staging
                .slice(..)
                .get_mapped_range()
                .expect("a mapped buffer is readable");
            bytemuck::pod_read_unaligned(&view[..RESULT_BYTES as usize])
        };
        pipeline.staging.unmap();
        self.busy.store(false, Ordering::Release);
        if let Ok(mut result) = self.result.lock() {
            *result = Some(Picked {
                instance: (words[0] != 0).then_some(words[1]),
                depth: f32::from_bits(words[2]),
            });
        }
    }
}

impl Pipeline {
    fn new(gpu: &Gpu) -> Self {
        let layout = LayoutBuilder::new()
            .uniform()
            .storage_read()
            .storage_read()
            .storage_read()
            .storage_write()
            .build(gpu, "pick");
        let module = shaders::module(&gpu.device, "editor/pick.wgsl");
        Self {
            pipeline: gpu.compute_pipeline("pick", &module, "main", &[&layout]),
            layout,
            params: gpu.uniform_buffer("pick params", 16),
            result: gpu.storage_buffer("pick result", RESULT_BYTES, wgpu::BufferUsages::COPY_SRC),
            staging: gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("pick staging"),
                size: RESULT_BYTES,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
        }
    }
}
