//! Reads back what culling asked for, without ever blocking the render loop.
//!
//! A buffer can only map while no command uses it, so a read takes several frames: copy, map,
//! wait for the callback, read, unmap. That lag only delays when a page starts loading.

use crate::Gpu;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

#[derive(Default)]
struct Shared {
    /// Set when a copy was recorded and still needs its map request.
    copied: AtomicBool,
    /// Set from the map request until the data has been consumed.
    busy: AtomicBool,
    /// Set by the map callback once the buffer is readable.
    mapped: AtomicBool,
}

/// The array culling writes requests to, and its route back to the CPU.
pub struct RequestFeedback {
    buffer: wgpu::Buffer,
    staging: wgpu::Buffer,
    size: u64,
    shared: Arc<Shared>,
    latest: Option<Vec<u32>>,
}

impl RequestFeedback {
    pub fn new(gpu: &Gpu, entries: u32) -> Self {
        let size = u64::from(entries.max(1)) * 4;
        let buffer = gpu.storage_buffer(
            "page requests",
            size,
            wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC,
        );
        let staging = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("page requests readback"),
            size: buffer.size(),
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            size: buffer.size(),
            buffer,
            staging,
            shared: Default::default(),
            latest: None,
        }
    }

    pub fn buffer(&self) -> &wgpu::Buffer {
        &self.buffer
    }

    /// Queues a copy of the requests so far, unless an earlier one is still on its way. Returns
    /// whether it did.
    pub fn capture(&mut self, encoder: &mut wgpu::CommandEncoder) -> bool {
        if self.shared.mapped.swap(false, Ordering::AcqRel) {
            self.consume_mapped_data();
            self.shared.busy.store(false, Ordering::Release);
        }
        let free = !self.shared.busy.load(Ordering::Acquire);
        if free {
            encoder.copy_buffer_to_buffer(&self.buffer, 0, &self.staging, 0, self.size);
            self.shared.copied.store(true, Ordering::Release);
        }
        free
    }

    fn consume_mapped_data(&mut self) {
        {
            let data = self
                .staging
                .slice(..)
                .get_mapped_range()
                .expect("the map has completed");
            self.latest = Some(bytemuck::cast_slice(&data).to_vec());
        }
        self.staging.unmap();
    }

    /// Starts reading back a copy captured this frame. Call after submitting.
    pub fn request_map(&self) {
        if !self.shared.copied.swap(false, Ordering::AcqRel) {
            return;
        }
        self.shared.busy.store(true, Ordering::Release);
        let shared = self.shared.clone();
        self.staging
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| match result {
                Ok(()) => shared.mapped.store(true, Ordering::Release),
                Err(_) => shared.busy.store(false, Ordering::Release),
            });
    }

    /// The most recent requests read back, if any have come since the last call.
    pub fn take_latest(&mut self) -> Option<Vec<u32>> {
        self.latest.take()
    }
}
