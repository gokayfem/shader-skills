//! The seabed under the sea: a map of the scene seen from straight above, which tells the water
//! how deep it is everywhere around the camera.
//!
//! Waves feel the bottom. As the water shoals they stand up and break, and no crest stands taller
//! than the water beneath it can hold. Over dry land there are no waves at all. The water layer
//! reads this map to shrink its waves in the shallows (`ocean/seabed.wgsl`), so the sea meets
//! the shore along its calm waterline rather than cutting through the sand wherever a crest
//! happens to rise, and it never surfaces in hollows inland, however low the land.
//!
//! The map is drawn by the shadow maps' depth-only pass through the same cluster culling, from a
//! little above the sea: decks, cliff tops and anything else high above the water are cut away,
//! so it is what lies under the water that counts. The scene is static, so the map is redrawn
//! only when the camera has moved a long way, the sea's level has changed, or finer geometry has
//! streamed in.

use crate::gpu::Gpu;
use crate::scene::GpuScene;
use crate::vg::{
    CullPhase, CullPipelines, CullWork, PassView, RasterBindings, Rasterizer, ViewSlots,
};
use glam::{Mat4, Vec2, Vec3};

/// Texels across the map.
const RESOLUTION: u32 = 1024;
/// Metres across the map: a metre a texel, about as fine as the shore's slope needs.
const EXTENT: f32 = 1024.0;
/// The map moves in steps of this many metres, so it rarely has to be redrawn.
const SNAP: f32 = 128.0;
/// How far above the calm sea the map's top lies. Waves and their swash reach no higher.
pub const ABOVE: f32 = 2.0;
/// How far the map reaches down from its top. Deeper water than this is as good as open sea.
pub const REACH: f32 = 64.0;
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

/// Where the map lies and how to read its heights.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SeabedView {
    /// Projects a point in the world onto the map, with depth 0 at its top and 1 at its bottom.
    pub view_proj: Mat4,
    /// Height of the map's top, and how far down it reaches.
    pub top: f32,
    pub reach: f32,
}

impl SeabedView {
    /// The map for a camera at `camera` over a sea at `sea_level`: centred under the camera,
    /// snapped to a coarse grid.
    pub fn new(camera: Vec3, sea_level: f32) -> Self {
        let center = (Vec2::new(camera.x, camera.z) / SNAP).round() * SNAP;
        let top = sea_level + ABOVE;
        let eye = Vec3::new(center.x, top, center.y);
        // Looking straight down, with north (-Z) at the top of the map.
        let view = Mat4::look_to_rh(eye, Vec3::NEG_Y, Vec3::NEG_Z);
        let half = EXTENT * 0.5;
        let projection = Mat4::orthographic_rh(-half, half, -half, half, 0.0, REACH);
        Self {
            view_proj: projection * view,
            top,
            reach: REACH,
        }
    }

    /// The point at the map's centre, at its top.
    fn center(&self) -> Vec3 {
        let inverse = self.view_proj.inverse();
        inverse.project_point3(Vec3::ZERO)
    }
}

/// The seabed map and the pass that draws it.
pub struct SeabedMap {
    pub view: wgpu::TextureView,
    pipeline: wgpu::RenderPipeline,
    /// What the map last drew, to tell when it has gone stale, and whether finer geometry has
    /// arrived since.
    drawn: Option<(SeabedView, f32)>,
    outdated: bool,
    current: SeabedView,
}

impl SeabedMap {
    pub fn new(gpu: &Gpu, rasterizer: &Rasterizer) -> Self {
        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("seabed"),
            size: wgpu::Extent3d {
                width: RESOLUTION,
                height: RESOLUTION,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let layout = gpu
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("seabed"),
                bind_group_layouts: &[Some(&rasterizer.raster_layout)],
                immediate_size: 0,
            });
        let pipeline = gpu
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("seabed"),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &rasterizer.hardware_module,
                    entry_point: Some("vs_depth"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    front_face: wgpu::FrontFace::Ccw,
                    // Facing is decided per instance in `raster_hw.wgsl`.
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: FORMAT,
                    depth_write_enabled: Some(true),
                    // The highest surface under each texel: nearest the map's top.
                    depth_compare: Some(wgpu::CompareFunction::Less),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                fragment: None,
                multiview_mask: None,
                cache: None,
            });
        Self {
            view: texture.create_view(&Default::default()),
            pipeline,
            drawn: None,
            outdated: false,
            current: SeabedView::new(Vec3::ZERO, 0.0),
        }
    }

    /// Makes the map redraw, as finer geometry has arrived.
    pub fn mark_stale(&mut self) {
        self.outdated = true;
    }

    /// Where the map lies this frame.
    pub fn current(&self) -> &SeabedView {
        &self.current
    }

    /// Places the map for a camera at `camera` and, if it has gone stale, writes its view to
    /// `slot`. Returns whether it is due to be drawn.
    pub fn prepare(
        &mut self,
        gpu: &Gpu,
        slots: &ViewSlots,
        slot: u32,
        camera: Vec3,
        sea_level: f32,
        lod_threshold: f32,
    ) -> bool {
        let view = SeabedView::new(camera, sea_level);
        let due = self.outdated || self.drawn != Some((view, lod_threshold));
        if due {
            self.outdated = false;
            self.drawn = Some((view, lod_threshold));
            self.current = view;
            let pass = PassView::orthographic(
                view.view_proj,
                view.center(),
                Vec2::splat(RESOLUTION as f32),
                RESOLUTION as f32 / EXTENT,
                lod_threshold,
            );
            slots.write(gpu, slot, &pass);
        }
        due
    }

    /// Culls and draws the map, when `prepare` found it due.
    #[allow(clippy::too_many_arguments)]
    pub fn record(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        cull: &CullPipelines,
        scene: &GpuScene,
        cull_scene: &wgpu::BindGroup,
        work: &CullWork,
        raster: &RasterBindings,
        slot: u32,
    ) {
        cull.record(encoder, scene, cull_scene, slot, work, CullPhase::First);
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("seabed"),
            color_attachments: &[],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &self.view,
                depth_ops: Some(wgpu::Operations {
                    // Nothing drawn: as deep as the map reaches.
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &raster.group, &[ViewSlots::offset(slot)]);
        pass.draw_indirect(work.args(), CullWork::draw_args_offset());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_map_lies_under_the_camera_and_reads_heights_from_its_top() {
        let camera = Vec3::new(300.0, 12.0, -150.0);
        let view = SeabedView::new(camera, -0.6);
        // The camera stays within the middle of the map, whatever the snapping.
        let at = view
            .view_proj
            .project_point3(Vec3::new(camera.x, 0.0, camera.z));
        assert!(at.x.abs() < 0.2 && at.y.abs() < 0.2, "{at}");
        // Depth runs from the top, just above the sea, down by `REACH`.
        let top = view
            .view_proj
            .project_point3(Vec3::new(camera.x, view.top, camera.z));
        let bottom = view
            .view_proj
            .project_point3(Vec3::new(camera.x, view.top - REACH, camera.z));
        assert!(top.z.abs() < 1.0e-4, "{top}");
        assert!((bottom.z - 1.0).abs() < 1.0e-4, "{bottom}");
        // North (-Z) is at the top of the map: the top of the texture, as the shaders read it.
        let north = view
            .view_proj
            .project_point3(Vec3::new(camera.x, 0.0, camera.z - 100.0));
        assert!(north.y > at.y, "{north} {at}");
    }

    #[test]
    fn the_map_stays_put_while_the_camera_moves_a_little() {
        let a = SeabedView::new(Vec3::new(10.0, 3.0, 20.0), 0.0);
        let b = SeabedView::new(Vec3::new(40.0, 30.0, -20.0), 0.0);
        assert_eq!(a, b);
        assert_ne!(a, SeabedView::new(Vec3::new(10.0, 3.0, 20.0), 1.0));
    }
}
