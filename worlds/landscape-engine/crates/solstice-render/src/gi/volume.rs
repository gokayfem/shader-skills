//! The world-space scene representation that indirect rays trace.

use crate::gpu::{groups_for, Gpu, LayoutBuilder};
use crate::lighting::LightShadows;
use crate::profiler::{section, Profiler};
use crate::scene::GpuScene;
use crate::shaders;
use crate::vg::{CullPhase, CullPipelines, CullWork, InstanceFilter, PassView, ViewSlots};
use bytemuck::{Pod, Zeroable};
use glam::{IVec3, Mat4, Vec2, Vec3, Vec4};

pub const LEVEL_COUNT: usize = 5;
const RESOLUTION: u32 = 96;
const RESOLUTION_I: i32 = RESOLUTION as i32;
const VOXELS_PER_LEVEL: u64 = (RESOLUTION as u64).pow(3);
const TOTAL_VOXELS: u64 = VOXELS_PER_LEVEL * LEVEL_COUNT as u64;
/// Voxel size of the finest level, in metres. Each further level doubles it.
const BASE_VOXEL_SIZE: f32 = 0.2;
/// A level moves only when the camera has moved this many voxels. Moving costs a rebuild.
const SNAP_VOXELS: i32 = 8;
const ATLAS_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// Jump-flood step lengths. Together they must span half the grid's width.
const FLOOD_STEPS: [i32; 6] = [48, 24, 12, 6, 3, 1];
/// Finer levels get refreshed more often, since they sit closest to the viewer.
const LIGHT_SCHEDULE: [u32; 8] = [0, 1, 0, 2, 0, 3, 0, 4];
/// How much of a rebuild's distance field to compute per frame. Spreading it keeps one level's
/// move from becoming one long frame. It takes three frames, too short to see.
const FLOOD_PASSES_PER_FRAME: usize = 6;
/// The most frames in a row a rebuild waits for other heavy drawing.
const MAX_HELD_OFF: u32 = 8;

/// Twin of `VolumeParams` in `common/environment.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct VolumeUniform {
    levels: [Vec4; LEVEL_COUNT],
    resolution: f32,
    inv_resolution: f32,
    _pad: [f32; 2],
}

/// Twin of `Params` in `gi/voxelize.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct VoxelizeParams {
    origin: Vec3,
    voxel_size: f32,
    resolution: u32,
    level_offset: u32,
    _pad: [u32; 2],
}

/// Twin of `Params` in `gi/jump_flood.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct FloodParams {
    resolution: u32,
    level_offset: u32,
    step: IVec3,
    _pad: [u32; 3],
}

/// Twin of `Params` in `gi/voxel_light.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct LightParams {
    level: u32,
    frame: u32,
    resolution: u32,
    light_count: u32,
    wrap_origin: IVec3,
    _pad: u32,
}

/// Twin of `Params` in `gi/volume_write.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct WriteParams {
    level: u32,
    resolution: u32,
    voxel_size: f32,
    _pad0: u32,
    wrap_origin: IVec3,
    _pad1: u32,
}

/// Per-level uniforms and bind groups.
struct Level {
    voxel_size: f32,
    /// Grid corner, in whole voxels from the world origin.
    origin_voxels: Option<IVec3>,
    /// Lighting updates so far. The checkerboard and ray sequence advance on this, not on
    /// the frame number: a level updated every other frame would otherwise only ever
    /// see even frame numbers, and half of its voxels would never be lit.
    light_updates: u32,
    voxelize_params: wgpu::Buffer,
    light_params: wgpu::Buffer,
    write_params: wgpu::Buffer,
    voxelize_group: wgpu::BindGroup,
    seed_group: wgpu::BindGroup,
    light_group: wgpu::BindGroup,
    write_group: wgpu::BindGroup,
}

/// The GI volume and the passes that maintain it.
pub struct GiVolume {
    pub atlas_view: wgpu::TextureView,
    pub uniform: wgpu::Buffer,
    voxels: wgpu::Buffer,
    seeds: wgpu::Buffer,
    flood_a: wgpu::Buffer,
    levels: Vec<Level>,
    voxelize_pipeline: wgpu::ComputePipeline,
    /// Voxelizes the clusters under a voxel across, a thread each.
    voxelize_specks_pipeline: wgpu::ComputePipeline,
    seed_pipeline: wgpu::ComputePipeline,
    flood_pipeline: wgpu::ComputePipeline,
    light_pipeline: wgpu::ComputePipeline,
    write_pipeline: wgpu::ComputePipeline,
    flood_groups: Vec<wgpu::BindGroup>,
    light_count: u32,
    schedule_index: usize,
    pending: Option<PendingRebuild>,
    /// Levels still to voxelize again where they stand, as what they hold has changed.
    outdated: [bool; LEVEL_COUNT],
    /// A change that came while levels were still outdated, to start over once they are done.
    refresh_waiting: bool,
    /// Set for a frame that already has heavy drawing to do, such as a shadow cascade: no
    /// level starts to rebuild on it, as voxelizing one costs several times a frame.
    pub hold_off: bool,
    /// Frames in a row that rebuilding has been held off.
    held_off: u32,
    /// See [`crate::RendererSettings::speck_texels`].
    pub speck_voxels: f32,
}

/// What [`GiVolume::prepare`] decided should happen this frame.
pub struct VolumeWork {
    /// Levels rebuilt completely this frame, which only happens when nothing exists yet.
    rebuild: Vec<usize>,
    /// The part of a spread-out rebuild that falls in this frame.
    slice: Option<RebuildSlice>,
    relight: usize,
}

/// A level being rebuilt across several frames. Until it finishes, shaders keep using the
/// level's old contents and position, so nothing is ever seen half built.
#[derive(Clone, Copy, Debug)]
struct PendingRebuild {
    level: usize,
    origin: IVec3,
    next_pass: usize,
}

#[derive(Clone, Debug)]
struct RebuildSlice {
    level: usize,
    /// True in the first frame, which voxelizes and seeds the flood.
    begin: bool,
    passes: std::ops::Range<usize>,
    /// True in the last frame, which publishes the finished level.
    commit: bool,
}

impl GiVolume {
    /// Creates the volume. `environment_layout` is the shared lighting bind group layout.
    pub fn new(
        gpu: &Gpu,
        scene: &GpuScene,
        work: &CullWork,
        environment_layout: &wgpu::BindGroupLayout,
        light_shadows: &wgpu::TextureView,
    ) -> Self {
        let voxels =
            gpu.storage_buffer("gi voxels", TOTAL_VOXELS * 12, wgpu::BufferUsages::COPY_DST);
        let seeds = gpu.storage_buffer("gi seeds", TOTAL_VOXELS * 4, wgpu::BufferUsages::COPY_DST);
        let radiance =
            gpu.storage_buffer("gi radiance", TOTAL_VOXELS * 8, wgpu::BufferUsages::empty());
        let flood_a = gpu.storage_buffer(
            "gi flood a",
            VOXELS_PER_LEVEL * 4,
            wgpu::BufferUsages::COPY_SRC,
        );
        let flood_b = gpu.storage_buffer(
            "gi flood b",
            VOXELS_PER_LEVEL * 4,
            wgpu::BufferUsages::empty(),
        );

        let atlas = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("gi atlas"),
            size: wgpu::Extent3d {
                width: RESOLUTION,
                height: RESOLUTION,
                depth_or_array_layers: RESOLUTION * LEVEL_COUNT as u32,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D3,
            format: ATLAS_FORMAT,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let atlas_view = atlas.create_view(&Default::default());

        let mut voxelize_layout = LayoutBuilder::new()
            .uniform()
            .storage_read()
            .storage_read()
            .storage_read()
            .storage_read()
            .storage_read()
            .storage_read()
            .storage_read()
            .storage_write()
            .texture_array()
            .sampler();
        if scene.regions.is_some() {
            // The second cluster binding and the pool's far band.
            for _ in 0..scene.regions.map_or(0, |plan| plan.pools() - 1) {
                voxelize_layout = voxelize_layout.storage_read();
            }
        }
        let voxelize_layout = voxelize_layout.build(gpu, "gi voxelize");
        let flood_layout = LayoutBuilder::new()
            .uniform()
            .storage_read()
            .storage_read()
            .storage_write()
            .build(gpu, "gi flood");
        let light_layout = LayoutBuilder::new()
            .uniform()
            .storage_read()
            .storage_write()
            .storage_read()
            .texture_array_unfilterable()
            .build(gpu, "gi light");
        let write_layout = LayoutBuilder::new()
            .uniform()
            .storage_read()
            .storage_read()
            .storage_texture_3d(ATLAS_FORMAT)
            .build(gpu, "gi write");

        let voxelize_module =
            shaders::module_regions(&gpu.device, "gi/voxelize.wgsl", scene.regions);
        let flood_module = shaders::module(&gpu.device, "gi/jump_flood.wgsl");
        let light_module = shaders::module(&gpu.device, "gi/voxel_light.wgsl");
        let write_module = shaders::module(&gpu.device, "gi/volume_write.wgsl");

        let uniform_of = |label: &str, size: usize| gpu.uniform_buffer(label, size as u64);

        let levels = (0..LEVEL_COUNT)
            .map(|index| {
                let voxel_size = BASE_VOXEL_SIZE * (1u32 << index) as f32;
                let level_offset = index as u64 * VOXELS_PER_LEVEL;

                let voxelize_params =
                    uniform_of("gi voxelize params", std::mem::size_of::<VoxelizeParams>());
                let light_params =
                    uniform_of("gi light params", std::mem::size_of::<LightParams>());
                let write_params =
                    uniform_of("gi write params", std::mem::size_of::<WriteParams>());

                let seed_params = uniform_of("gi seed params", std::mem::size_of::<FloodParams>());
                gpu.queue.write_buffer(
                    &seed_params,
                    0,
                    bytemuck::bytes_of(&FloodParams {
                        resolution: RESOLUTION,
                        level_offset: level_offset as u32,
                        step: IVec3::ZERO,
                        _pad: [0; 3],
                    }),
                );

                Level {
                    voxel_size,
                    origin_voxels: None,
                    light_updates: 0,
                    voxelize_group: {
                        let mut resources = vec![
                            voxelize_params.as_entire_binding(),
                            scene.instances.as_entire_binding(),
                            scene.clusters.as_entire_binding(),
                            scene.vertices.as_entire_binding(),
                            scene.triangles.as_entire_binding(),
                            work.visible.as_entire_binding(),
                            work.state().as_entire_binding(),
                            scene.materials.as_entire_binding(),
                            voxels.as_entire_binding(),
                            wgpu::BindingResource::TextureView(&scene.textures.base.color_view),
                            wgpu::BindingResource::Sampler(&scene.material_sampler),
                        ];
                        if let (Some(more), Some(far)) = (&scene.clusters_more, &scene.pool_far) {
                            resources.extend([more.as_entire_binding(), far.as_entire_binding()]);
                            resources.extend(
                                scene
                                    .pool_spares
                                    .iter()
                                    .map(|pool| pool.as_entire_binding()),
                            );
                        }
                        gpu.bind_group("gi voxelize", &voxelize_layout, &resources)
                    },
                    seed_group: gpu.bind_group(
                        "gi seed",
                        &flood_layout,
                        &[
                            seed_params.as_entire_binding(),
                            voxels.as_entire_binding(),
                            flood_b.as_entire_binding(),
                            flood_a.as_entire_binding(),
                        ],
                    ),
                    light_group: gpu.bind_group(
                        "gi light",
                        &light_layout,
                        &[
                            light_params.as_entire_binding(),
                            voxels.as_entire_binding(),
                            radiance.as_entire_binding(),
                            scene.lights.as_entire_binding(),
                            wgpu::BindingResource::TextureView(light_shadows),
                        ],
                    ),
                    write_group: gpu.bind_group(
                        "gi write",
                        &write_layout,
                        &[
                            write_params.as_entire_binding(),
                            seeds.as_entire_binding(),
                            radiance.as_entire_binding(),
                            wgpu::BindingResource::TextureView(&atlas_view),
                        ],
                    ),
                    voxelize_params,
                    light_params,
                    write_params,
                }
            })
            .collect();

        // One pass per axis per step. Passes alternate buffers, starting from A, where the
        // seed pass wrote. The even total means the result ends up back in A.
        let mut flood_groups = Vec::new();
        let axes = [IVec3::X, IVec3::Y, IVec3::Z];
        for (pass, step) in FLOOD_STEPS
            .iter()
            .flat_map(|&s| axes.map(|axis| axis * s))
            .enumerate()
        {
            let params = uniform_of("gi flood params", std::mem::size_of::<FloodParams>());
            gpu.queue.write_buffer(
                &params,
                0,
                bytemuck::bytes_of(&FloodParams {
                    resolution: RESOLUTION,
                    level_offset: 0,
                    step,
                    _pad: [0; 3],
                }),
            );
            let (from, to) = if pass % 2 == 0 {
                (&flood_a, &flood_b)
            } else {
                (&flood_b, &flood_a)
            };
            flood_groups.push(gpu.bind_group(
                "gi flood",
                &flood_layout,
                &[
                    params.as_entire_binding(),
                    voxels.as_entire_binding(),
                    from.as_entire_binding(),
                    to.as_entire_binding(),
                ],
            ));
        }
        assert!(
            flood_groups.len() % 2 == 0,
            "the flood result must finish in buffer A"
        );

        Self {
            atlas_view,
            uniform: uniform_of("gi volume", std::mem::size_of::<VolumeUniform>()),
            voxelize_pipeline: gpu.compute_pipeline(
                "gi voxelize",
                &voxelize_module,
                "main",
                &[&voxelize_layout],
            ),
            voxelize_specks_pipeline: gpu.compute_pipeline(
                "gi voxelize specks",
                &voxelize_module,
                "specks",
                &[&voxelize_layout],
            ),
            seed_pipeline: gpu.compute_pipeline("gi seed", &flood_module, "seed", &[&flood_layout]),
            flood_pipeline: gpu.compute_pipeline(
                "gi flood",
                &flood_module,
                "flood",
                &[&flood_layout],
            ),
            light_pipeline: gpu.compute_pipeline(
                "gi light",
                &light_module,
                "main",
                &[&light_layout, environment_layout],
            ),
            write_pipeline: gpu.compute_pipeline(
                "gi write",
                &write_module,
                "main",
                &[&write_layout],
            ),
            voxels,
            seeds,
            flood_a,
            levels,
            flood_groups,
            light_count: scene.light_count,
            schedule_index: 0,
            pending: None,
            outdated: [false; LEVEL_COUNT],
            refresh_waiting: false,
            hold_off: false,
            held_off: 0,
            speck_voxels: 0.0,
        }
    }

    /// Follows the camera and decides this frame's work. Writes the views of any levels to
    /// rebuild to view slots starting at `first_slot`.
    pub fn prepare(
        &mut self,
        gpu: &Gpu,
        slots: &ViewSlots,
        first_slot: u32,
        camera_position: Vec3,
    ) -> VolumeWork {
        self.start_waiting_refresh();
        let first_build = self.levels[0].origin_voxels.is_none();
        let mut rebuild = Vec::new();
        let mut slice = None;
        let speck_voxels = self.speck_voxels;
        if first_build {
            for (index, level) in self.levels.iter_mut().enumerate() {
                let wanted = snapped_origin(camera_position, level.voxel_size);
                level.origin_voxels = Some(wanted);
                rebuild.push(index);
                write_level_view(
                    gpu,
                    slots,
                    first_slot + index as u32,
                    wanted,
                    level.voxel_size,
                    speck_voxels,
                );
            }
        } else {
            // Held off for a few frames at most, so that the voxels can't fall far behind.
            let hold_off = std::mem::take(&mut self.hold_off) && self.held_off < MAX_HELD_OFF;
            self.held_off = if hold_off { self.held_off + 1 } else { 0 };
            if self.pending.is_none() && !hold_off {
                self.pending = choose_rebuild(&self.levels, camera_position)
                    .or_else(|| self.choose_outdated());
                if let Some(pending) = self.pending {
                    let level = &self.levels[pending.level];
                    write_level_view(
                        gpu,
                        slots,
                        first_slot + pending.level as u32,
                        pending.origin,
                        level.voxel_size,
                        speck_voxels,
                    );
                }
            }
            if let Some(pending) = &mut self.pending {
                let end = (pending.next_pass + FLOOD_PASSES_PER_FRAME).min(flood_pass_count());
                let commit = end == flood_pass_count();
                slice = Some(RebuildSlice {
                    level: pending.level,
                    begin: pending.next_pass == 0,
                    passes: pending.next_pass..end,
                    commit,
                });
                pending.next_pass = end;
                if commit {
                    self.levels[pending.level].origin_voxels = Some(pending.origin);
                    self.outdated[pending.level] = false;
                }
            }
        }
        let committed = slice.as_ref().is_some_and(|s: &RebuildSlice| s.commit);
        let being_rebuilt = self.pending.filter(|_| !committed).map(|p| p.level);

        // A level being rebuilt has new voxels but its old position, so it must not relight
        // until it is finished. The others take its turn.
        let mut relight = LIGHT_SCHEDULE[self.schedule_index] as usize;
        self.schedule_index = (self.schedule_index + 1) % LIGHT_SCHEDULE.len();
        if being_rebuilt == Some(relight) {
            relight = LIGHT_SCHEDULE[self.schedule_index] as usize;
            self.schedule_index = (self.schedule_index + 1) % LIGHT_SCHEDULE.len();
            if being_rebuilt == Some(relight) {
                relight = (relight + 1) % LEVEL_COUNT;
            }
        }
        self.levels[relight].light_updates = self.levels[relight].light_updates.wrapping_add(1);

        let mut uniform = VolumeUniform {
            levels: [Vec4::ZERO; LEVEL_COUNT],
            resolution: RESOLUTION as f32,
            inv_resolution: 1.0 / RESOLUTION as f32,
            _pad: [0.0; 2],
        };
        for (index, level) in self.levels.iter().enumerate() {
            let origin = level.origin_voxels.unwrap_or(IVec3::ZERO);
            uniform.levels[index] = (origin.as_vec3() * level.voxel_size).extend(level.voxel_size);
            let voxelize_origin = self
                .pending
                .filter(|pending| pending.level == index)
                .map_or(origin, |pending| pending.origin);
            gpu.queue.write_buffer(
                &level.voxelize_params,
                0,
                bytemuck::bytes_of(&VoxelizeParams {
                    origin: voxelize_origin.as_vec3() * level.voxel_size,
                    voxel_size: level.voxel_size,
                    resolution: RESOLUTION,
                    level_offset: (index as u64 * VOXELS_PER_LEVEL) as u32,
                    _pad: [0; 2],
                }),
            );
            gpu.queue.write_buffer(
                &level.light_params,
                0,
                bytemuck::bytes_of(&LightParams {
                    level: index as u32,
                    frame: level.light_updates,
                    resolution: RESOLUTION,
                    light_count: self.light_count,
                    wrap_origin: origin,
                    _pad: 0,
                }),
            );
            gpu.queue.write_buffer(
                &level.write_params,
                0,
                bytemuck::bytes_of(&WriteParams {
                    level: index as u32,
                    resolution: RESOLUTION,
                    voxel_size: level.voxel_size,
                    _pad0: 0,
                    wrap_origin: origin,
                    _pad1: 0,
                }),
            );
        }
        gpu.queue
            .write_buffer(&self.uniform, 0, bytemuck::bytes_of(&uniform));

        if committed {
            self.pending = None;
        }
        VolumeWork {
            rebuild,
            slice,
            relight,
        }
    }

    /// Records rebuilds of moved levels, then a lighting update.
    #[allow(clippy::too_many_arguments)]
    pub fn record(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        cull: &CullPipelines,
        scene: &GpuScene,
        cull_scene: &wgpu::BindGroup,
        cull_work: &CullWork,
        environment: &wgpu::BindGroup,
        light_shadows: &LightShadows,
        first_slot: u32,
        work: &VolumeWork,
        profiler: Option<&Profiler>,
    ) {
        let mark = |encoder: &mut wgpu::CommandEncoder, section: usize| {
            if let Some(profiler) = profiler {
                profiler.mark(encoder, section);
            }
        };
        mark(encoder, section::GI_CULL);
        for &index in &work.rebuild {
            self.record_slice(
                encoder,
                cull,
                scene,
                cull_scene,
                cull_work,
                first_slot,
                &RebuildSlice {
                    level: index,
                    begin: true,
                    passes: 0..flood_pass_count(),
                    commit: true,
                },
                &mark,
            );
        }
        if let Some(slice) = &work.slice {
            self.record_slice(
                encoder, cull, scene, cull_scene, cull_work, first_slot, slice, &mark,
            );
        }
        if work.rebuild.is_empty() && work.slice.is_none() {
            mark(encoder, section::GI_VOXELIZE);
            mark(encoder, section::GI_FLOOD);
            mark(encoder, section::GI_WRITE);
        }
        // The lights' shadow maps are traced through the volume as it now stands, for the
        // voxel light pass and the lighting pass after it.
        mark(encoder, section::LIGHT_SHADOWS);
        light_shadows.record(encoder, environment);
        mark(encoder, section::GI_LIGHT);

        let groups = groups_for(RESOLUTION, 4);
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("gi light"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.light_pipeline);
            pass.set_bind_group(0, &self.levels[work.relight].light_group, &[]);
            pass.set_bind_group(1, environment, &[]);
            pass.dispatch_workgroups(groups, groups, groups);
        }
        self.record_write(encoder, work.relight);
    }

    #[allow(clippy::too_many_arguments)]
    fn record_slice(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        cull: &CullPipelines,
        scene: &GpuScene,
        cull_scene: &wgpu::BindGroup,
        cull_work: &CullWork,
        first_slot: u32,
        slice: &RebuildSlice,
        mark: &dyn Fn(&mut wgpu::CommandEncoder, usize),
    ) {
        if slice.begin {
            cull.record(
                encoder,
                scene,
                cull_scene,
                first_slot + slice.level as u32,
                cull_work,
                CullPhase::First,
            );
        }
        mark(encoder, section::GI_VOXELIZE);
        if slice.begin {
            self.record_voxelize(encoder, slice.level, cull_work);
        }
        mark(encoder, section::GI_FLOOD);
        self.record_flood(encoder, slice);
        mark(encoder, section::GI_WRITE);
        if slice.commit {
            let level_offset = slice.level as u64 * VOXELS_PER_LEVEL;
            encoder.copy_buffer_to_buffer(
                &self.flood_a,
                0,
                &self.seeds,
                level_offset * 4,
                VOXELS_PER_LEVEL * 4,
            );
            self.record_write(encoder, slice.level);
        }
    }

    fn record_voxelize(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        index: usize,
        cull_work: &CullWork,
    ) {
        let level_offset = index as u64 * VOXELS_PER_LEVEL;
        encoder.clear_buffer(&self.voxels, level_offset * 12, Some(VOXELS_PER_LEVEL * 12));
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("gi voxelize"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.voxelize_pipeline);
            pass.set_bind_group(0, &self.levels[index].voxelize_group, &[]);
            pass.dispatch_workgroups_indirect(cull_work.args(), CullWork::software_args_offset());
            // Nothing, unless the level's view draws specks.
            pass.set_pipeline(&self.voxelize_specks_pipeline);
            pass.dispatch_workgroups_indirect(
                cull_work.args(),
                CullWork::voxel_speck_args_offset(),
            );
        }
    }

    fn record_flood(&self, encoder: &mut wgpu::CommandEncoder, slice: &RebuildSlice) {
        let groups = groups_for(RESOLUTION, 4);
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("gi jump flood"),
            timestamp_writes: None,
        });
        if slice.begin {
            pass.set_pipeline(&self.seed_pipeline);
            pass.set_bind_group(0, &self.levels[slice.level].seed_group, &[]);
            pass.dispatch_workgroups(groups, groups, groups);
        }
        pass.set_pipeline(&self.flood_pipeline);
        for group in &self.flood_groups[slice.passes.clone()] {
            pass.set_bind_group(0, group, &[]);
            pass.dispatch_workgroups(groups, groups, groups);
        }
    }

    fn record_write(&self, encoder: &mut wgpu::CommandEncoder, index: usize) {
        let groups = groups_for(RESOLUTION, 4);
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("gi volume write"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.write_pipeline);
        pass.set_bind_group(0, &self.levels[index].write_group, &[]);
        pass.dispatch_workgroups(groups, groups, groups);
    }
}

impl GiVolume {
    /// Makes every level voxelize again where it stands, as what the levels hold has changed:
    /// finer geometry has arrived, or instances changed.
    ///
    /// Levels are rebuilt one at a time, coarsest first, over several frames. A change that
    /// comes while some are still outdated waits until they are all done, and then starts the
    /// whole round again. Starting over at once would rebuild the coarsest level for ever while
    /// changes kept coming, and never reach the finer ones.
    pub fn mark_stale(&mut self) {
        mark_outdated(&mut self.outdated, &mut self.refresh_waiting);
    }

    /// Starts a round of rebuilds that was waiting, once the last round is done.
    fn start_waiting_refresh(&mut self) {
        start_waiting(&mut self.outdated, &mut self.refresh_waiting);
    }

    /// An outdated level to rebuild where it stands, coarsest first: those cover the most
    /// and were the most wrong.
    fn choose_outdated(&self) -> Option<PendingRebuild> {
        let level = (0..LEVEL_COUNT).rev().find(|&level| self.outdated[level])?;
        Some(PendingRebuild {
            level,
            origin: self.levels[level].origin_voxels?,
            next_pass: 0,
        })
    }
}

/// One pass per axis per step.
const fn flood_pass_count() -> usize {
    FLOOD_STEPS.len() * 3
}

/// Picks the level that has fallen furthest behind the camera, measured in its own voxels.
/// Always taking the finest instead would starve the coarser levels during fast flight.
fn choose_rebuild(levels: &[Level], camera_position: Vec3) -> Option<PendingRebuild> {
    let offsets = levels.iter().map(|level| {
        let wanted = snapped_origin(camera_position, level.voxel_size);
        let current = level.origin_voxels.unwrap_or(wanted);
        (wanted, (wanted - current).abs().max_element())
    });
    let (index, (origin, offset)) = offsets
        .enumerate()
        .max_by_key(|&(index, (_, offset))| (offset, std::cmp::Reverse(index)))?;
    (offset > 0).then_some(PendingRebuild {
        level: index,
        origin,
        next_pass: 0,
    })
}

/// The grid corner for a level centred near `position`, in whole voxels.
fn snapped_origin(position: Vec3, voxel_size: f32) -> IVec3 {
    let center = (position / voxel_size).floor().as_ivec3();
    let snapped = IVec3::new(
        center.x.div_euclid(SNAP_VOXELS),
        center.y.div_euclid(SNAP_VOXELS),
        center.z.div_euclid(SNAP_VOXELS),
    ) * SNAP_VOXELS;
    snapped - IVec3::splat(RESOLUTION_I / 2)
}

/// Writes the orthographic view that selects a level's clusters.
fn write_level_view(
    gpu: &Gpu,
    slots: &ViewSlots,
    slot: u32,
    origin_voxels: IVec3,
    voxel_size: f32,
    speck_voxels: f32,
) {
    let half = voxel_size * RESOLUTION as f32 * 0.5;
    let center = (origin_voxels.as_vec3() + Vec3::splat(RESOLUTION as f32 * 0.5)) * voxel_size;
    let view = Mat4::look_to_rh(center + Vec3::Z * half, Vec3::NEG_Z, Vec3::Y);
    let projection = Mat4::orthographic_rh(-half, half, -half, half, 0.0, half * 2.0);
    let mut pass_view = PassView::orthographic(
        projection * view,
        center,
        Vec2::splat(RESOLUTION as f32),
        1.0 / voxel_size,
        // Half a voxel of geometric error is invisible at this resolution.
        0.5,
    )
    .drawing(InstanceFilter::Gi)
    .with_specks(speck_voxels);
    pass_view.allow_software = 2;
    slots.write(gpu, slot, &pass_view);
}

/// Marks every level outdated, or, while some still are, notes that another round is waiting.
fn mark_outdated(outdated: &mut [bool; LEVEL_COUNT], waiting: &mut bool) {
    if outdated.contains(&true) {
        *waiting = true;
    } else {
        *outdated = [true; LEVEL_COUNT];
    }
}

/// Starts a waiting round once no level is outdated.
fn start_waiting(outdated: &mut [bool; LEVEL_COUNT], waiting: &mut bool) {
    if *waiting && !outdated.contains(&true) {
        *waiting = false;
        *outdated = [true; LEVEL_COUNT];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changes_during_a_round_of_rebuilds_wait_for_it_to_finish() {
        let (mut outdated, mut waiting) = ([false; LEVEL_COUNT], false);
        mark_outdated(&mut outdated, &mut waiting);
        assert_eq!((outdated, waiting), ([true; LEVEL_COUNT], false));
        // The coarsest level is rebuilt, then a change comes, every frame for a while: the
        // finer levels must still get their turn.
        outdated[LEVEL_COUNT - 1] = false;
        for _ in 0..10 {
            mark_outdated(&mut outdated, &mut waiting);
            start_waiting(&mut outdated, &mut waiting);
        }
        assert!(!outdated[LEVEL_COUNT - 1] && outdated[0] && waiting);
        // Once the round is done, the waiting one starts.
        outdated = [false; LEVEL_COUNT];
        start_waiting(&mut outdated, &mut waiting);
        assert_eq!((outdated, waiting), ([true; LEVEL_COUNT], false));
        // And with nothing waiting, a finished round stays finished.
        outdated = [false; LEVEL_COUNT];
        start_waiting(&mut outdated, &mut waiting);
        assert_eq!(outdated, [false; LEVEL_COUNT]);
    }

    #[test]
    fn origin_keeps_the_camera_near_the_centre_of_the_grid() {
        for position in [
            Vec3::ZERO,
            Vec3::new(13.7, -4.2, 100.1),
            Vec3::new(-99.0, 7.0, -0.1),
        ] {
            let origin = snapped_origin(position, BASE_VOXEL_SIZE);
            let local = position / BASE_VOXEL_SIZE - origin.as_vec3();
            let middle = RESOLUTION as f32 * 0.5;
            assert!((local - Vec3::splat(middle)).abs().max_element() <= SNAP_VOXELS as f32 + 1.0);
        }
    }

    #[test]
    fn origin_stays_put_for_small_moves() {
        let origin = snapped_origin(Vec3::new(0.5, 0.5, 0.5), BASE_VOXEL_SIZE);
        assert_eq!(
            origin,
            snapped_origin(Vec3::new(0.51, 0.52, 0.53), BASE_VOXEL_SIZE)
        );
    }

    #[test]
    fn flood_steps_span_half_the_grid() {
        assert!(FLOOD_STEPS.iter().sum::<i32>() >= RESOLUTION_I / 2);
        assert_eq!(FLOOD_STEPS[FLOOD_STEPS.len() - 1], 1);
    }

    #[test]
    fn every_level_is_scheduled_for_lighting() {
        for level in 0..LEVEL_COUNT as u32 {
            assert!(LIGHT_SCHEDULE.contains(&level));
        }
    }
}
