//! GPU-driven hierarchy traversal. See `shaders/vg/cull.wgsl` for the algorithm.

use crate::gpu::{groups_for, Gpu, LayoutBuilder};
use crate::regions::RegionPlan;
use crate::scene::GpuScene;
use crate::shaders;
use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec2, Vec3, Vec4};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

/// Twin of `PassView` in `vg/pass_view.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct PassView {
    pub raster_view_proj: Mat4,
    pub occlusion_view: Mat4,
    pub planes: [Vec4; 6],
    pub camera_position: Vec3,
    pub lod_threshold: f32,
    pub viewport: Vec2,
    pub proj_scale: Vec2,
    pub hzb_size: Vec2,
    pub near: f32,
    pub lod_scale: f32,
    pub is_ortho: u32,
    pub phase: u32,
    pub occlusion_enabled: u32,
    pub allow_software: u32,
    /// For an orthographic view drawn on behalf of a camera: where that camera is, its pixels
    /// per world unit at unit distance, and the nearest distance from it at which what this view
    /// draws can show. Error too small for the camera to see is then not drawn here either, so
    /// a far shadow cascade stops short of detail finer than the camera's pixels. A scale of 0
    /// turns this off.
    pub eye_position: Vec3,
    pub eye_lod_scale: f32,
    pub eye_near: f32,
    /// How many of `receiver_planes` are in use.
    pub receiver_plane_count: u32,
    /// Which instances the view draws: an [`InstanceFilter`].
    pub instance_filter: u32,
    /// For a depth map drawn by the software rasterizer: which drawing this is, 1 to 255.
    /// Depth is kept under the stamp, so entries from earlier drawings lose to this one's
    /// and are ignored when the depth is read, and the buffer never needs clearing.
    pub stamp: u32,
    /// Further planes a cluster must lie inside: for a shadow view, the bounds of the volume
    /// whose shadows this view supplies, swept towards the light, so that casters whose shadow
    /// falls outside it are not drawn.
    pub receiver_planes: [Vec4; 6],
    /// Towards the light, for a shadow view: a caster's error shows where its shadow lands,
    /// anywhere along the line from it away from the light.
    pub eye_light: Vec3,
    /// 1 if this view sends cut-out clusters through their own lists and rasterizers; 0 if it
    /// draws them with everything else, solid or testing the mask itself.
    pub masked_lists: u32,
    /// For the camera's own view of a streamed scene: how far around the camera pages are asked
    /// for that it isn't facing, at a coarser level of detail. 0 for none.
    pub prefetch_radius: f32,
    /// For a view kept from frame to frame, in a streamed scene: where in the page requests the
    /// bits begin that it sets, one for each page it asks for, or 0 for none. Such a view asks
    /// only on the frames it is redrawn, so what it asks for is told apart from what the camera
    /// asks for every frame: held longer, and the view redrawn when it comes. [`ViewSlots`]
    /// fills this in.
    pub hold_requests: u32,
    /// For a depth map or a voxelization: clusters under this many texels or voxels across are
    /// drawn as specks, a point each, by one thread, where a rasterizer gives each a workgroup.
    /// 0 for none.
    pub speck_texels: f32,
    pub _pad_prefetch: u32,
}

/// Which instances a view draws. Twin of the `INSTANCES_` constants in `vg/pass_view.wgsl`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum InstanceFilter {
    All = 0,
    /// Only those that cannot move, for views kept from frame to frame, which would otherwise
    /// keep a moving instance where it was.
    Static = 1,
    /// Only those that can move, drawn each frame over a cached view.
    Movable = 2,
    /// Static instances, and movable ones that stay in the GI voxels.
    Gi = 3,
}

impl PassView {
    /// Extracts normalised frustum planes from a view-projection matrix, with depth
    /// running 0 to 1. An infinite far plane yields a degenerate plane that passes all.
    pub fn frustum_planes(view_proj: Mat4) -> [Vec4; 6] {
        let (r0, r1, r2, r3) = (
            view_proj.row(0),
            view_proj.row(1),
            view_proj.row(2),
            view_proj.row(3),
        );
        [r3 + r0, r3 - r0, r3 + r1, r3 - r1, r2, r3 - r2].map(|plane| {
            let length = plane.truncate().length();
            if length > 1.0e-6 {
                plane / length
            } else {
                Vec4::new(0.0, 0.0, 0.0, 1.0)
            }
        })
    }

    /// A view with no occlusion testing and no software rasterization, for orthographic
    /// volumes such as shadow cascades. `pixels_per_unit` converts LOD error to pixels. These
    /// views are kept from frame to frame, so they draw only static instances; see
    /// [`Self::drawing`].
    pub fn orthographic(
        view_proj: Mat4,
        center: Vec3,
        viewport: Vec2,
        pixels_per_unit: f32,
        lod_threshold: f32,
    ) -> Self {
        Self {
            raster_view_proj: view_proj,
            occlusion_view: Mat4::IDENTITY,
            planes: Self::frustum_planes(view_proj),
            camera_position: center,
            lod_threshold,
            viewport,
            proj_scale: Vec2::ONE,
            hzb_size: Vec2::ONE,
            near: 0.0,
            lod_scale: pixels_per_unit,
            is_ortho: 1,
            phase: 0,
            occlusion_enabled: 0,
            allow_software: 0,
            eye_position: Vec3::ZERO,
            eye_lod_scale: 0.0,
            eye_near: 0.0,
            receiver_plane_count: 0,
            instance_filter: InstanceFilter::Static as u32,
            stamp: 0,
            receiver_planes: [Vec4::ZERO; 6],
            eye_light: Vec3::Y,
            masked_lists: 0,
            prefetch_radius: 0.0,
            hold_requests: 0,
            speck_texels: 0.0,
            _pad_prefetch: 0,
        }
    }

    /// The same view, drawing clusters under this many texels across as specks.
    pub fn with_specks(mut self, texels: f32) -> Self {
        self.speck_texels = texels;
        self
    }

    /// The same view, cutting out masked clusters through their own lists.
    pub fn cutting_out(mut self) -> Self {
        self.masked_lists = 1;
        self
    }

    /// Restricts a light's view to casters that can shadow the volume inside `planes`, given
    /// light arriving along `-light` (`light` points at the light). Anything whose shadow lands
    /// in the volume lies in it or towards the light from it: that is, inside every plane of
    /// the volume that faces the light. The others are dropped, which leaves the volume open
    /// in the light's direction. Planes face inwards, with inside positive.
    pub fn shadowing(mut self, planes: &[Vec4; 6], light: Vec3) -> Self {
        let mut count = 0;
        for plane in planes {
            if plane.truncate().dot(light) >= 0.0 {
                self.receiver_planes[count] = *plane;
                count += 1;
            }
        }
        self.receiver_plane_count = count as u32;
        self
    }

    /// Bounds a light's view's level of detail by what a camera at `eye`, with `lod_scale`
    /// pixels per world unit at unit distance, can see of the shadows it draws: a caster's
    /// error shows wherever its shadow lands, along the line from it away from `light`, and
    /// nothing this view draws is seen nearer than `nearest` from that camera.
    pub fn seen_from(mut self, eye: Vec3, light: Vec3, lod_scale: f32, nearest: f32) -> Self {
        self.eye_position = eye;
        // The distance to the shadow's line needs a unit direction.
        self.eye_light = light.normalize_or(Vec3::Y);
        self.eye_lod_scale = lod_scale;
        self.eye_near = nearest.max(1.0e-3);
        self
    }

    /// The same view, drawing other instances.
    pub fn drawing(mut self, filter: InstanceFilter) -> Self {
        self.instance_filter = filter as u32;
        self
    }
}

/// Bytes between view slots. Comfortably above `PassView` and any device's offset alignment.
const SLOT_STRIDE: u32 = 512;
const _: () = assert!(std::mem::size_of::<PassView>() <= SLOT_STRIDE as usize);

/// A uniform buffer holding several [`PassView`]s, selected by dynamic offset.
pub struct ViewSlots {
    pub buffer: wgpu::Buffer,
    /// See [`PassView::hold_requests`]: what views kept from frame to frame are given, those
    /// of static instances and then those of the GI's.
    pub hold_requests: [u32; 2],
}

impl ViewSlots {
    pub fn new(gpu: &Gpu, count: u32) -> Self {
        Self {
            buffer: gpu.uniform_buffer("view slots", u64::from(count * SLOT_STRIDE)),
            hold_requests: [0; 2],
        }
    }

    pub fn write(&self, gpu: &Gpu, slot: u32, view: &PassView) {
        let mut view = *view;
        // The views kept from frame to frame are those that leave out what may move.
        if view.instance_filter == InstanceFilter::Static as u32 {
            view.hold_requests = self.hold_requests[0];
        } else if view.instance_filter == InstanceFilter::Gi as u32 {
            view.hold_requests = self.hold_requests[1];
        }
        gpu.queue.write_buffer(
            &self.buffer,
            u64::from(Self::offset(slot)),
            bytemuck::bytes_of(&view),
        );
    }

    pub fn offset(slot: u32) -> u32 {
        slot * SLOT_STRIDE
    }

    pub fn binding(&self) -> wgpu::BindingResource<'_> {
        wgpu::BindingResource::Buffer(wgpu::BufferBinding {
            buffer: &self.buffer,
            offset: 0,
            size: wgpu::BufferSize::new(std::mem::size_of::<PassView>() as u64),
        })
    }
}

/// Byte offsets into the indirect arguments buffer. See `Args` in `cull_control.wgsl`.
const ARGS_QUEUE_OFFSET: u64 = 0;
const ARGS_SOFTWARE_OFFSET: u64 = 12;
const ARGS_DRAW_OFFSET: u64 = 24;
const ARGS_MERGE_OFFSET: u64 = 40;
const ARGS_MASKED_SOFTWARE_OFFSET: u64 = 56;
const ARGS_MASKED_DRAW_OFFSET: u64 = 68;
const ARGS_SPECK_OFFSET: u64 = 84;
const ARGS_VOXEL_SPECK_OFFSET: u64 = 96;
const ARGS_SIZE: u64 = 108;
/// Size of `CullState` in bytes.
const STATE_SIZE: u64 = 72;
/// Where in `CullState` the opaque lists' end is. A frame's clear stops short of it.
const STATE_OPAQUE_END_OFFSET: u64 = 68;

#[cfg(test)]
#[path = "culling_stress.rs"]
mod stress;

/// Which occlusion phase a cull runs. See the module docs of [`crate::vg`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CullPhase {
    /// Starts a frame's cull: tests the previous depth and collects retries. Views
    /// without occlusion simply run this phase alone.
    First,
    /// Re-tests the retries against this frame's depth.
    Second,
}

/// The transient buffers of one culling run.
pub struct CullWork {
    state: wgpu::Buffer,
    pub visible: wgpu::Buffer,
    /// A depth map's specks: the first phase's retry list, which a view without occlusion has
    /// no other use for.
    pub specks: wgpu::Buffer,
    args: wgpu::Buffer,
    first_phase: wgpu::BindGroup,
    second_phase: wgpu::BindGroup,
    control: wgpu::BindGroup,
    stats: CullStats,
}

impl CullWork {
    pub fn new(
        gpu: &Gpu,
        pipelines: &CullPipelines,
        label: &str,
        queue_capacity: u64,
        visible_capacity: u64,
        masked_capacity: u64,
        page_requests: &wgpu::Buffer,
    ) -> Self {
        let state = gpu.storage_buffer(
            label,
            STATE_SIZE,
            wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC,
        );
        let queue_a = gpu.storage_buffer(label, queue_capacity * 8, wgpu::BufferUsages::empty());
        let queue_b = gpu.storage_buffer(label, queue_capacity * 8, wgpu::BufferUsages::empty());
        let visible = gpu.storage_buffer(
            label,
            (visible_capacity + masked_capacity) * 8,
            wgpu::BufferUsages::empty(),
        );
        gpu.queue.write_buffer(
            &state,
            STATE_OPAQUE_END_OFFSET,
            bytemuck::bytes_of(&(visible_capacity as u32)),
        );
        let args = gpu.storage_buffer(label, ARGS_SIZE, wgpu::BufferUsages::INDIRECT);

        let bind = |queue: &wgpu::Buffer, retry: &wgpu::Buffer| {
            gpu.bind_group(
                label,
                &pipelines.work_layout,
                &[
                    state.as_entire_binding(),
                    queue.as_entire_binding(),
                    retry.as_entire_binding(),
                    visible.as_entire_binding(),
                    page_requests.as_entire_binding(),
                ],
            )
        };
        let first_phase = bind(&queue_a, &queue_b);
        let second_phase = bind(&queue_b, &queue_a);
        let control = gpu.bind_group(
            label,
            &pipelines.control_layout,
            &[state.as_entire_binding(), args.as_entire_binding()],
        );
        let stats = CullStats::new(gpu);

        Self {
            state,
            visible,
            specks: queue_b,
            args,
            first_phase,
            second_phase,
            control,
            stats,
        }
    }

    pub fn state(&self) -> &wgpu::Buffer {
        &self.state
    }

    pub fn args(&self) -> &wgpu::Buffer {
        &self.args
    }

    pub const fn software_args_offset() -> u64 {
        ARGS_SOFTWARE_OFFSET
    }

    pub const fn draw_args_offset() -> u64 {
        ARGS_DRAW_OFFSET
    }

    pub const fn masked_software_args_offset() -> u64 {
        ARGS_MASKED_SOFTWARE_OFFSET
    }

    pub const fn masked_draw_args_offset() -> u64 {
        ARGS_MASKED_DRAW_OFFSET
    }

    /// Offset of the dispatch that draws a depth map's specks.
    pub const fn speck_args_offset() -> u64 {
        ARGS_SPECK_OFFSET
    }

    /// Offset of the dispatch that voxelizes a voxelization's specks.
    pub const fn voxel_speck_args_offset() -> u64 {
        ARGS_VOXEL_SPECK_OFFSET
    }

    /// Offset of the draw arguments for merging software-rasterized depth into a depth map:
    /// one triangle when the software list holds something, nothing otherwise.
    pub const fn merge_args_offset() -> u64 {
        ARGS_MERGE_OFFSET
    }

    /// Queues a copy of the counters. Call at the end of a frame's encoding.
    pub fn capture_stats(&self, encoder: &mut wgpu::CommandEncoder) {
        self.stats.capture(encoder, &self.state);
    }

    /// Takes in any counters that have arrived and starts reading back any captured this
    /// frame. Call after submitting, every frame, even ones that captured nothing.
    pub fn map_stats(&self) {
        self.stats.collect();
        self.stats.request_map();
    }

    pub fn stats(&self) -> &CullStats {
        &self.stats
    }
}

/// Asynchronous readback of how much recent culling selected.
///
/// A buffer can only map while no command uses it, so a capture cycle takes several
/// frames: copy, map, wait for the callback, read, unmap. Numbers lag slightly behind,
/// which is fine for statistics, and the render loop never blocks on them.
pub struct CullStats {
    staging: wgpu::Buffer,
    shared: Arc<StatsShared>,
}

#[derive(Default)]
struct StatsShared {
    clusters: AtomicU32,
    software_clusters: AtomicU32,
    triangles: AtomicU32,
    dropped: AtomicU32,
    specks: AtomicU32,
    /// Set when a copy was recorded and still needs its map request.
    copied: AtomicBool,
    /// Set from the map request until the data has been consumed.
    busy: AtomicBool,
    /// Set by the map callback once the buffer is readable.
    mapped: AtomicBool,
}

impl CullStats {
    fn new(gpu: &Gpu) -> Self {
        let staging = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("cull stats"),
            size: STATE_SIZE,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            staging,
            shared: Default::default(),
        }
    }

    fn capture(&self, encoder: &mut wgpu::CommandEncoder, state: &wgpu::Buffer) {
        self.collect();
        if !self.shared.busy.load(Ordering::Acquire) {
            encoder.copy_buffer_to_buffer(state, 0, &self.staging, 0, STATE_SIZE);
            self.shared.copied.store(true, Ordering::Release);
        }
    }

    /// Reads the counters once the buffer has mapped, freeing it for the next capture.
    fn collect(&self) {
        if self.shared.mapped.swap(false, Ordering::AcqRel) {
            self.consume_mapped_data();
            self.shared.busy.store(false, Ordering::Release);
        }
    }

    fn consume_mapped_data(&self) {
        {
            let data = self
                .staging
                .slice(..)
                .get_mapped_range()
                .expect("the map has completed");
            let words: &[u32] = bytemuck::cast_slice(&data);
            // Word positions follow `CullState` in `vg/pass_view.wgsl`.
            let (software, hardware, triangles, dropped) = (
                words[4] + words[12],
                words[5] + words[13],
                words[8],
                words[9],
            );
            self.shared
                .clusters
                .store(software + hardware, Ordering::Release);
            self.shared
                .software_clusters
                .store(software, Ordering::Release);
            self.shared.triangles.store(triangles, Ordering::Release);
            self.shared.dropped.store(dropped, Ordering::Release);
            self.shared.specks.store(words[11], Ordering::Release);
        }
        self.staging.unmap();
    }

    fn request_map(&self) {
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

    /// Clusters selected in the most recently read-back frame.
    pub fn clusters(&self) -> u32 {
        self.shared.clusters.load(Ordering::Acquire)
    }

    /// Triangles selected in the most recently read-back frame.
    pub fn triangles(&self) -> u32 {
        self.shared.triangles.load(Ordering::Acquire)
    }

    /// Clusters sent to the software rasterizer in the most recently read-back frame.
    pub fn software_clusters(&self) -> u32 {
        self.shared.software_clusters.load(Ordering::Acquire)
    }

    /// Clusters drawn into a depth map as specks in the most recently read-back frame. They are
    /// not among [`Self::clusters`].
    pub fn specks(&self) -> u32 {
        self.shared.specks.load(Ordering::Acquire)
    }

    /// Queue entries and clusters lost for want of room in the most recently read-back frame.
    /// Anything but zero means that view's lists are too small for the scene.
    pub fn dropped(&self) -> u32 {
        self.shared.dropped.load(Ordering::Acquire)
    }
}

/// The compute pipelines of culling, shared by every view.
pub struct CullPipelines {
    pub scene_layout: wgpu::BindGroupLayout,
    work_layout: wgpu::BindGroupLayout,
    control_layout: wgpu::BindGroupLayout,
    seed: wgpu::ComputePipeline,
    process: wgpu::ComputePipeline,
    advance: wgpu::ComputePipeline,
    finish: wgpu::ComputePipeline,
    begin_second_phase: wgpu::ComputePipeline,
}

impl CullPipelines {
    pub fn new(gpu: &Gpu, deform: crate::shaders::Deform, regions: Option<RegionPlan>) -> Self {
        let mut scene_layout = LayoutBuilder::new()
            .uniform_dynamic()
            .storage_read()
            .storage_read()
            .storage_read()
            .texture_unfilterable()
            .texture_unfilterable()
            .texture_uint();
        if regions.is_some() {
            scene_layout = scene_layout.storage_read();
        }
        let scene_layout = scene_layout.build(gpu, "cull scene");
        let work_layout = LayoutBuilder::new()
            .storage_write()
            .storage_write()
            .storage_write()
            .storage_write()
            .storage_write()
            .build(gpu, "cull work");
        let control_layout = LayoutBuilder::new()
            .storage_write()
            .storage_write()
            .build(gpu, "cull control");

        let cull = shaders::module_for(&gpu.device, "vg/cull.wgsl", deform, regions);
        let control = shaders::module(&gpu.device, "vg/cull_control.wgsl");
        let cull_layouts = [&scene_layout, &work_layout];
        Self {
            seed: gpu.compute_pipeline("cull seed", &cull, "seed", &cull_layouts),
            process: gpu.compute_pipeline("cull process", &cull, "process", &cull_layouts),
            advance: gpu.compute_pipeline("cull advance", &control, "advance", &[&control_layout]),
            finish: gpu.compute_pipeline("cull finish", &control, "finish", &[&control_layout]),
            begin_second_phase: gpu.compute_pipeline(
                "cull second phase",
                &control,
                "begin_second_phase",
                &[&control_layout],
            ),
            scene_layout,
            work_layout,
            control_layout,
        }
    }

    /// Binds a scene and depth pyramid for culling.
    pub fn scene_bind_group(
        &self,
        gpu: &Gpu,
        slots: &ViewSlots,
        scene: &GpuScene,
        hzb: &wgpu::TextureView,
    ) -> wgpu::BindGroup {
        let mut resources = vec![
            slots.binding(),
            scene.instances.as_entire_binding(),
            scene.nodes.as_entire_binding(),
            scene.clusters.as_entire_binding(),
            wgpu::BindingResource::TextureView(hzb),
            wgpu::BindingResource::TextureView(&scene.skins.bones_view),
            wgpu::BindingResource::TextureView(&scene.skins.lists_view),
        ];
        resources.extend(
            scene
                .clusters_more
                .iter()
                .map(wgpu::Buffer::as_entire_binding),
        );
        gpu.bind_group("cull scene", &self.scene_layout, &resources)
    }

    /// Records one culling run. On return, `work` holds indirect arguments for
    /// rasterizing what this phase selected.
    pub fn record(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        scene: &GpuScene,
        scene_group: &wgpu::BindGroup,
        slot: u32,
        work: &CullWork,
        phase: CullPhase,
    ) {
        if phase == CullPhase::First {
            encoder.clear_buffer(&work.state, 0, Some(STATE_OPAQUE_END_OFFSET));
        }
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("cull"),
            timestamp_writes: None,
        });
        let work_group = match phase {
            CullPhase::First => &work.first_phase,
            CullPhase::Second => &work.second_phase,
        };
        let offset = [ViewSlots::offset(slot)];

        match phase {
            CullPhase::First => {
                pass.set_pipeline(&self.seed);
                pass.set_bind_group(0, scene_group, &offset);
                pass.set_bind_group(1, work_group, &[]);
                let groups = groups_for(scene.gpu_instance_count, 64);
                let rows = groups.div_ceil(65_535).max(1);
                pass.dispatch_workgroups(groups.div_ceil(rows), rows, 1);
            }
            CullPhase::Second => {
                pass.set_pipeline(&self.begin_second_phase);
                pass.set_bind_group(0, &work.control, &[]);
                pass.dispatch_workgroups(1, 1, 1);
            }
        }

        for _ in 0..scene.max_hierarchy_depth {
            pass.set_pipeline(&self.advance);
            pass.set_bind_group(0, &work.control, &[]);
            pass.dispatch_workgroups(1, 1, 1);

            pass.set_pipeline(&self.process);
            pass.set_bind_group(0, scene_group, &offset);
            pass.set_bind_group(1, work_group, &[]);
            pass.dispatch_workgroups_indirect(&work.args, ARGS_QUEUE_OFFSET);
        }

        pass.set_pipeline(&self.finish);
        pass.set_bind_group(0, &work.control, &[]);
        pass.dispatch_workgroups(1, 1, 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pass_view_lays_out_as_its_shader_twin() {
        crate::shaders::assert_twin!(
            "vg/cull.wgsl",
            "PassView",
            PassView,
            [
                raster_view_proj,
                occlusion_view,
                planes,
                camera_position,
                lod_threshold,
                viewport,
                proj_scale,
                hzb_size,
                near,
                lod_scale,
                is_ortho,
                phase,
                occlusion_enabled,
                allow_software,
                eye_position,
                eye_lod_scale,
                eye_near,
                receiver_plane_count,
                instance_filter,
                stamp,
                receiver_planes,
                eye_light,
                masked_lists,
                prefetch_radius,
                hold_requests,
                speck_texels,
            ]
        );
    }

    #[test]
    fn orthographic_views_draw_only_what_stands_still() {
        let view = PassView::orthographic(Mat4::IDENTITY, Vec3::ZERO, Vec2::ONE, 1.0, 1.0);
        assert_eq!(view.instance_filter, InstanceFilter::Static as u32);
        let moving = view.drawing(InstanceFilter::Movable);
        assert_eq!(moving.instance_filter, InstanceFilter::Movable as u32);
    }

    #[test]
    fn frustum_planes_accept_points_inside_and_reject_points_outside() {
        let projection = Mat4::perspective_infinite_reverse_rh(1.0, 1.0, 0.1);
        let planes = PassView::frustum_planes(projection);
        let is_inside = |p: Vec3| {
            planes
                .iter()
                .all(|plane| plane.truncate().dot(p) + plane.w >= 0.0)
        };
        assert!(is_inside(Vec3::new(0.0, 0.0, -5.0)));
        assert!(is_inside(Vec3::new(0.0, 0.0, -5000.0)));
        assert!(!is_inside(Vec3::new(0.0, 0.0, 5.0)), "behind the camera");
        assert!(
            !is_inside(Vec3::new(50.0, 0.0, -5.0)),
            "right of the frustum"
        );
        assert!(
            !is_inside(Vec3::new(0.0, 0.0, -0.01)),
            "nearer than the near plane"
        );
    }

    #[test]
    fn orthographic_planes_bound_all_six_sides() {
        let projection = Mat4::orthographic_rh(-1.0, 1.0, -1.0, 1.0, 0.0, 10.0);
        let planes = PassView::frustum_planes(projection);
        let is_inside = |p: Vec3| {
            planes
                .iter()
                .all(|plane| plane.truncate().dot(p) + plane.w >= 0.0)
        };
        assert!(is_inside(Vec3::new(0.5, 0.5, -5.0)));
        assert!(!is_inside(Vec3::new(0.0, 0.0, -11.0)));
        assert!(!is_inside(Vec3::new(0.0, 0.0, 1.0)));
        assert!(!is_inside(Vec3::new(1.5, 0.0, -5.0)));
    }
}
