//! The water-line map: where the sea stands around the camera, and the highest it has stood there
//! lately, for how wet each surface is (docs/wetness.md).
//!
//! The sea's surface is only drawn where the water is now. A rock the last wave washed over is
//! still wet after the wave draws back, and dries over the next minute or so. This map remembers
//! that. Each texel holds two heights in the world: the water's now, from the waves shrunk over
//! the seabed exactly as the drawn surface is, and the highest it has stood lately, which falls
//! back towards the water's present height as the spot dries. They are heights in the world,
//! not above the calm sea, so that when the sea falls, as a tide going out, the line it reached
//! stays behind and dries.
//!
//! The map is fixed in the world: each spot keeps its texel as the camera moves, and texels
//! wrap around the map's edges, so what is remembered stays put. Only texels newly come into
//! view start afresh. Two maps take turns, one read while the other is written.
//!
//! The resolve pass reads it through [`Waterline::resolve_group`]; without a sea it is never
//! updated, and the frame's sea level, far below everything, keeps it from being read.

use crate::gpu::{groups_for, Gpu, LayoutBuilder};
use crate::shaders;
use bytemuck::{Pod, Zeroable};
use glam::{Vec2, Vec3};

/// Texels across the map.
pub const TEXELS: u32 = 512;
/// Metres per texel: fine enough for a wet line on a rock, over 128 m around the camera.
pub const TEXEL_SIZE: f32 = 0.25;
/// Seconds over which a wet line's height above the water halves as it dries.
pub const DRYING_HALF_LIFE: f32 = 30.0;
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rg32Float;

/// Twin of `WaterlineParams` in `ocean/waterline_params.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct WaterlineUniform {
    window: [f32; 4],
    grid: [f32; 4],
}

/// The first texel of the window around a camera at `camera`, in texels of the world grid.
pub fn window_for(camera: Vec3) -> Vec2 {
    (Vec2::new(camera.x, camera.z) / TEXEL_SIZE).floor() - Vec2::splat((TEXELS / 2) as f32)
}

/// How much of a wet line's height above the water is left after `seconds` of drying.
pub fn drying(seconds: f32) -> f32 {
    0.5f32.powf(seconds.max(0.0) / DRYING_HALF_LIFE)
}

/// What the update reads of the sea.
pub struct SeaViews<'a> {
    /// Identifies the rest, whose bind groups are made again when it changes.
    pub key: u64,
    /// The sea surface's parameters, as `ocean/surface_params.wgsl` reads them.
    pub surface: &'a wgpu::Buffer,
    /// The waves' displacement, and the sampler it is read through.
    pub displacement: &'a wgpu::TextureView,
    pub sampler: &'a wgpu::Sampler,
    /// The seabed map.
    pub seabed: &'a wgpu::TextureView,
}

/// The water-line maps, the pass that updates them, and what the resolve pass reads.
pub struct Waterline {
    maps: [wgpu::TextureView; 2],
    uniform: wgpu::Buffer,
    resolve_layout: wgpu::BindGroupLayout,
    /// For resolve: the map written on even frames, then on odd ones.
    resolve_groups: [wgpu::BindGroup; 2],
    update_layout: wgpu::BindGroupLayout,
    update: wgpu::ComputePipeline,
    /// Update bind groups, writing each map in turn, and what they were made for.
    update_groups: Option<(u64, [wgpu::BindGroup; 2])>,
    /// The map written this frame, and last frame's window if the map holds it.
    parity: usize,
    previous_window: Option<Vec2>,
}

impl Waterline {
    pub fn new(gpu: &Gpu) -> Self {
        let map = |label| {
            gpu.device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width: TEXELS,
                        height: TEXELS,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: FORMAT,
                    usage: wgpu::TextureUsages::STORAGE_BINDING
                        | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
                .create_view(&Default::default())
        };
        let maps = [map("water line a"), map("water line b")];
        let uniform =
            gpu.uniform_buffer("water line", std::mem::size_of::<WaterlineUniform>() as u64);
        let resolve_layout = LayoutBuilder::new()
            .uniform()
            .texture_unfilterable()
            .build(gpu, "water line for resolve");
        let resolve_groups = std::array::from_fn(|i| {
            gpu.bind_group(
                "water line for resolve",
                &resolve_layout,
                &[
                    uniform.as_entire_binding(),
                    wgpu::BindingResource::TextureView(&maps[i]),
                ],
            )
        });
        let update_layout = LayoutBuilder::new()
            .uniform()
            .uniform()
            .texture_array()
            .sampler()
            .depth_texture()
            .texture_unfilterable()
            .storage_texture(FORMAT)
            .build(gpu, "water line update");
        let update = gpu.compute_pipeline(
            "water line update",
            &shaders::module(&gpu.device, "ocean/waterline.wgsl"),
            "main",
            &[&update_layout],
        );
        Self {
            maps,
            uniform,
            resolve_layout,
            resolve_groups,
            update_layout,
            update,
            update_groups: None,
            parity: 0,
            previous_window: None,
        }
    }

    /// The layout of [`Self::resolve_group`].
    pub fn resolve_layout(&self) -> &wgpu::BindGroupLayout {
        &self.resolve_layout
    }

    /// What the resolve pass reads this frame: the map just updated.
    pub fn resolve_group(&self) -> &wgpu::BindGroup {
        &self.resolve_groups[self.parity]
    }

    /// Places this frame's window around a camera at `camera`, `seconds` after the last frame.
    /// `active` is whether the map is updated this frame, as there is a sea; while it is not,
    /// what it remembered is forgotten.
    pub fn prepare(&mut self, gpu: &Gpu, camera: Vec3, seconds: f32, active: bool) {
        if !active {
            self.previous_window = None;
            return;
        }
        self.parity ^= 1;
        let window = window_for(camera);
        let previous = self.previous_window.unwrap_or(window);
        let uniform = WaterlineUniform {
            window: [window.x, window.y, previous.x, previous.y],
            grid: [
                TEXEL_SIZE,
                TEXELS as f32,
                drying(seconds),
                if self.previous_window.is_some() {
                    1.0
                } else {
                    0.0
                },
            ],
        };
        gpu.queue
            .write_buffer(&self.uniform, 0, bytemuck::bytes_of(&uniform));
        self.previous_window = Some(window);
    }

    /// Updates the map from the sea's waves over its seabed.
    pub fn record(&mut self, gpu: &Gpu, encoder: &mut wgpu::CommandEncoder, sea: SeaViews) {
        if self
            .update_groups
            .as_ref()
            .is_none_or(|(k, _)| *k != sea.key)
        {
            let groups = std::array::from_fn(|written| {
                gpu.bind_group(
                    "water line update",
                    &self.update_layout,
                    &[
                        sea.surface.as_entire_binding(),
                        self.uniform.as_entire_binding(),
                        wgpu::BindingResource::TextureView(sea.displacement),
                        wgpu::BindingResource::Sampler(sea.sampler),
                        wgpu::BindingResource::TextureView(sea.seabed),
                        wgpu::BindingResource::TextureView(&self.maps[1 - written]),
                        wgpu::BindingResource::TextureView(&self.maps[written]),
                    ],
                )
            });
            self.update_groups = Some((sea.key, groups));
        }
        let (_, groups) = self.update_groups.as_ref().expect("made above");
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("water line"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.update);
        pass.set_bind_group(0, &groups[self.parity], &[]);
        pass.dispatch_workgroups(groups_for(TEXELS, 8), groups_for(TEXELS, 8), 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_window_is_centred_on_the_camera() {
        let camera = Vec3::new(37.3, 5.0, -12.9);
        let first = window_for(camera);
        let centre = (first + Vec2::splat(TEXELS as f32 / 2.0)) * TEXEL_SIZE;
        assert!((centre - Vec2::new(camera.x, camera.z)).abs().max_element() <= TEXEL_SIZE);
    }

    #[test]
    fn a_wet_line_halves_over_its_half_life_and_never_rises() {
        assert!((drying(DRYING_HALF_LIFE) - 0.5).abs() < 1.0e-6);
        assert!(
            (drying(DRYING_HALF_LIFE * 0.5) * drying(DRYING_HALF_LIFE * 0.5) - 0.5).abs() < 1.0e-6
        );
        assert_eq!(drying(0.0), 1.0);
        assert_eq!(drying(-1.0), 1.0);
    }
}
