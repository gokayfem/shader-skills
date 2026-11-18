//! Rasterizes selected clusters into the visibility buffer.

use super::culling::{CullWork, ViewSlots};
use crate::gpu::{Gpu, LayoutBuilder};
use crate::regions::RegionPlan;
use crate::scene::GpuScene;
use crate::shaders::{self, Deform};

/// Format of the colour attachment that exists only to satisfy WebGPU's rule that a
/// render pass have at least one attachment. Nothing is ever written to it.
const DUMMY_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R8Unorm;
/// Format of the depth maps the depth-only rasterization draws into: the shadow maps.
pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

/// A depth map's worth of software-rasterized depth, and the bind groups that write and read
/// it. See [`Rasterizer::record_software_depth`].
pub struct DepthBuffer {
    pub buffer: wgpu::Buffer,
    write: wgpu::BindGroup,
    read: wgpu::BindGroup,
}

/// Stamps a drawing into a depth buffer can carry: a byte, with 0 meaning never drawn. Twin
/// of `vg/depth_stamp.wgsl`.
pub const DEPTH_STAMPS: u32 = 255;

/// Per-pixel depth and triangle ID, as storage buffers both rasterizers can write.
pub struct VisibilityBuffer {
    pub depth: wgpu::Buffer,
    pub ids: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    dummy_target: wgpu::TextureView,
    size: (u32, u32),
}

impl VisibilityBuffer {
    pub fn new(gpu: &Gpu, rasterizer: &Rasterizer, size: (u32, u32)) -> Self {
        let bytes = u64::from(size.0) * u64::from(size.1) * 4;
        let depth = gpu.storage_buffer("visibility depth", bytes, wgpu::BufferUsages::COPY_DST);
        let ids = gpu.storage_buffer("visibility ids", bytes, wgpu::BufferUsages::COPY_DST);
        let bind_group = gpu.bind_group(
            "visibility",
            &rasterizer.vis_layout,
            &[depth.as_entire_binding(), ids.as_entire_binding()],
        );
        let dummy_target = gpu
            .device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("visibility dummy target"),
                size: wgpu::Extent3d {
                    width: size.0,
                    height: size.1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: DUMMY_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            })
            .create_view(&Default::default());
        Self {
            depth,
            ids,
            bind_group,
            dummy_target,
            size,
        }
    }

    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    /// Resets every pixel to "no geometry". Depth 0 is the far plane in reversed Z,
    /// and resolve checks depth before reading an ID, so IDs need no clearing.
    pub fn clear(&self, encoder: &mut wgpu::CommandEncoder) {
        encoder.clear_buffer(&self.depth, 0, None);
    }
}

/// Bindings for rasterizing the output of one [`CullWork`].
pub struct RasterBindings {
    pub group: wgpu::BindGroup,
    /// What the pass that draws a depth map's specks reads.
    specks: wgpu::BindGroup,
    /// What the masked rasterizers read, in a scene that has any.
    pub mask: Option<wgpu::BindGroup>,
}

/// The rasterizers of cut-out clusters. Made only for scenes that have some, so that others
/// spend nothing on them.
struct Masked {
    layout: wgpu::BindGroupLayout,
    software: wgpu::ComputePipeline,
    hardware: wgpu::RenderPipeline,
    software_depth: wgpu::ComputePipeline,
    module: wgpu::ShaderModule,
}

/// The software and hardware rasterization pipelines.
pub struct Rasterizer {
    pub raster_layout: wgpu::BindGroupLayout,
    vis_layout: wgpu::BindGroupLayout,
    software: wgpu::ComputePipeline,
    hardware: wgpu::RenderPipeline,
    pub hardware_module: wgpu::ShaderModule,
    /// Depth-only software rasterization into a [`DepthBuffer`], and the pass that lays the
    /// result over a depth map.
    depth_write_layout: wgpu::BindGroupLayout,
    depth_read_layout: wgpu::BindGroupLayout,
    software_depth: wgpu::ComputePipeline,
    merge_depth: wgpu::RenderPipeline,
    /// Draws a depth map's specks into a [`DepthBuffer`]: see `vg/speck_depth.wgsl`.
    speck_layout: wgpu::BindGroupLayout,
    speck_depth: wgpu::ComputePipeline,
    masked: Option<Masked>,
}

/// A visibility-buffer pipeline: rasterization with no attachments in use.
fn visibility_pipeline(
    gpu: &Gpu,
    label: &str,
    module: &wgpu::ShaderModule,
    (vertex, fragment): (&str, &str),
    bind_group_layouts: &[Option<&wgpu::BindGroupLayout>],
) -> wgpu::RenderPipeline {
    let layout = gpu
        .device
        .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some(label),
            bind_group_layouts,
            immediate_size: 0,
        });
    gpu.device
        .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(label),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module,
                entry_point: Some(vertex),
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
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module,
                entry_point: Some(fragment),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: DUMMY_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::empty(),
                })],
            }),
            multiview_mask: None,
            cache: None,
        })
}

impl Rasterizer {
    pub fn new(gpu: &Gpu, has_masked: bool, deform: Deform, regions: Option<RegionPlan>) -> Self {
        let mut raster_layout = LayoutBuilder::new()
            .visibility(wgpu::ShaderStages::COMPUTE | wgpu::ShaderStages::VERTEX_FRAGMENT)
            .uniform_dynamic()
            .storage_read()
            .storage_read()
            .storage_read()
            .storage_read()
            .storage_read()
            .texture_unfilterable()
            // The wind, for what sways in it.
            .texture_unfilterable();
        if regions.is_some() {
            // The second cluster binding, and the pool's middle and far bands.
            for _ in 0..regions.map_or(0, |plan| plan.pools()) {
                raster_layout = raster_layout.storage_read();
            }
        }
        let raster_layout = raster_layout.build(gpu, "raster");
        let vis_layout = LayoutBuilder::new()
            .visibility(wgpu::ShaderStages::COMPUTE | wgpu::ShaderStages::FRAGMENT)
            .storage_write()
            .storage_write()
            .build(gpu, "visibility");

        let software_module =
            shaders::module_for(&gpu.device, "vg/raster_sw.wgsl", deform, regions);
        let software = gpu.compute_pipeline(
            "software raster",
            &software_module,
            "main",
            &[&raster_layout, &vis_layout],
        );

        let hardware_module =
            shaders::module_for(&gpu.device, "vg/raster_hw_fragment.wgsl", deform, regions);
        let hardware = visibility_pipeline(
            gpu,
            "hardware raster",
            &hardware_module,
            ("vs_visibility", "fs_visibility"),
            &[Some(&raster_layout), Some(&vis_layout)],
        );

        let depth_write_layout = LayoutBuilder::new()
            .storage_write()
            .build(gpu, "software depth");
        let depth_read_layout = LayoutBuilder::new()
            .visibility(wgpu::ShaderStages::FRAGMENT)
            .storage_read()
            .build(gpu, "software depth merge");
        let depth_module =
            shaders::module_for(&gpu.device, "vg/raster_sw_depth.wgsl", deform, regions);
        let software_depth = gpu.compute_pipeline(
            "software depth raster",
            &depth_module,
            "main",
            &[&raster_layout, &depth_write_layout],
        );
        let speck_layout = LayoutBuilder::new()
            .uniform_dynamic()
            .storage_read()
            .storage_read()
            .build(gpu, "depth specks");
        let speck_module = shaders::module(&gpu.device, "vg/speck_depth.wgsl");
        let speck_depth = gpu.compute_pipeline(
            "depth specks",
            &speck_module,
            "main",
            &[&speck_layout, &depth_write_layout],
        );
        let merge_module = shaders::module(&gpu.device, "vg/shadow_merge.wgsl");
        let merge_layout = gpu
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("software depth merge"),
                bind_group_layouts: &[Some(&raster_layout), Some(&depth_read_layout)],
                immediate_size: 0,
            });
        let merge_depth = gpu
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("software depth merge"),
                layout: Some(&merge_layout),
                vertex: wgpu::VertexState {
                    module: &merge_module,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH_FORMAT,
                    depth_write_enabled: Some(true),
                    depth_compare: Some(wgpu::CompareFunction::Less),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &merge_module,
                    entry_point: Some("fs_main"),
                    compilation_options: Default::default(),
                    targets: &[],
                }),
                multiview_mask: None,
                cache: None,
            });

        let masked = has_masked.then(|| {
            let layout = LayoutBuilder::new()
                .visibility(wgpu::ShaderStages::COMPUTE | wgpu::ShaderStages::VERTEX_FRAGMENT)
                .storage_read()
                .texture_array()
                .texture_array()
                .texture_array()
                .sampler()
                .build(gpu, "masked raster");
            let software_module =
                shaders::module_for(&gpu.device, "vg/raster_sw_masked.wgsl", deform, regions);
            let depth_module = shaders::module_for(
                &gpu.device,
                "vg/raster_sw_depth_masked.wgsl",
                deform,
                regions,
            );
            let module =
                shaders::module_for(&gpu.device, "vg/raster_hw_masked.wgsl", deform, regions);
            Masked {
                software: gpu.compute_pipeline(
                    "masked software raster",
                    &software_module,
                    "main",
                    &[&raster_layout, &vis_layout, &layout],
                ),
                software_depth: gpu.compute_pipeline(
                    "masked software depth raster",
                    &depth_module,
                    "main",
                    &[&raster_layout, &depth_write_layout, &layout],
                ),
                hardware: visibility_pipeline(
                    gpu,
                    "masked hardware raster",
                    &module,
                    ("vs_masked", "fs_masked"),
                    &[Some(&raster_layout), Some(&vis_layout), Some(&layout)],
                ),
                module,
                layout,
            }
        });

        Self {
            raster_layout,
            vis_layout,
            software,
            hardware,
            hardware_module,
            depth_write_layout,
            depth_read_layout,
            software_depth,
            merge_depth,
            speck_layout,
            speck_depth,
            masked,
        }
    }

    /// What a pipeline drawing cut-out clusters into a depth map needs: the shader, with its
    /// `vs_masked` and `fs_masked_depth`, and the layout of bind group 2.
    pub fn masked_depth_parts(&self) -> Option<(&wgpu::ShaderModule, &wgpu::BindGroupLayout)> {
        self.masked.as_ref().map(|m| (&m.module, &m.layout))
    }

    /// Whether this rasterizer has the pipelines for cut-out clusters.
    pub fn draws_masked(&self) -> bool {
        self.masked.is_some()
    }

    /// A buffer for the software-rasterized depth of a `size` map, with its bindings.
    pub fn depth_buffer(&self, gpu: &Gpu, size: (u32, u32)) -> DepthBuffer {
        let buffer = gpu.storage_buffer(
            "software depth",
            u64::from(size.0) * u64::from(size.1) * 4,
            wgpu::BufferUsages::COPY_DST,
        );
        DepthBuffer {
            write: gpu.bind_group(
                "software depth",
                &self.depth_write_layout,
                &[buffer.as_entire_binding()],
            ),
            read: gpu.bind_group(
                "software depth merge",
                &self.depth_read_layout,
                &[buffer.as_entire_binding()],
            ),
            buffer,
        }
    }

    /// Rasterizes into `depth` the clusters the most recent cull of `work` sent to software,
    /// as depth alone, under the stamp the view at `slot` carries. [`Self::merge_depth`] then
    /// lays them over a depth map. Nothing is cleared per drawing: a drawing's entries outrank
    /// any left by earlier drawings, the merge reads only its own stamp, and the caller clears
    /// the buffer once each time the stamps start a new round.
    pub fn record_software_depth(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        bindings: &RasterBindings,
        slot: u32,
        work: &CullWork,
        depth: &DepthBuffer,
    ) {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("software depth raster"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.software_depth);
        pass.set_bind_group(0, &bindings.group, &[ViewSlots::offset(slot)]);
        pass.set_bind_group(1, &depth.write, &[]);
        pass.dispatch_workgroups_indirect(work.args(), CullWork::software_args_offset());
        drop(pass);
        // In a pass of its own, for the reason in `record`.
        if let (Some(masked), Some(mask)) = (&self.masked, &bindings.mask) {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("masked software depth raster"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&masked.software_depth);
            pass.set_bind_group(0, &bindings.group, &[ViewSlots::offset(slot)]);
            pass.set_bind_group(1, &depth.write, &[]);
            pass.set_bind_group(2, mask, &[]);
            pass.dispatch_workgroups_indirect(work.args(), CullWork::masked_software_args_offset());
        }
        // The clusters under a texel across, which culling listed apart: nothing, for a view
        // that draws none as specks.
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("depth specks"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.speck_depth);
        pass.set_bind_group(0, &bindings.specks, &[ViewSlots::offset(slot)]);
        pass.set_bind_group(1, &depth.write, &[]);
        pass.dispatch_workgroups_indirect(work.args(), CullWork::speck_args_offset());
    }

    /// Draws the depth in `depth` into the depth attachment of `pass`, under its depth test:
    /// one triangle over the map, which the cull's arguments drop when nothing went to
    /// software, so an empty list costs nothing.
    pub fn merge_depth(
        &self,
        pass: &mut wgpu::RenderPass,
        bindings: &RasterBindings,
        slot: u32,
        work: &CullWork,
        depth: &DepthBuffer,
    ) {
        pass.set_pipeline(&self.merge_depth);
        pass.set_bind_group(0, &bindings.group, &[ViewSlots::offset(slot)]);
        pass.set_bind_group(1, &depth.read, &[]);
        pass.draw_indirect(work.args(), CullWork::merge_args_offset());
    }

    /// Binds a scene and a culling run's output for rasterization.
    pub fn bindings(
        &self,
        gpu: &Gpu,
        slots: &ViewSlots,
        scene: &GpuScene,
        work: &CullWork,
    ) -> RasterBindings {
        RasterBindings {
            group: {
                let mut resources = vec![
                    slots.binding(),
                    scene.instances.as_entire_binding(),
                    scene.clusters.as_entire_binding(),
                    scene.vertices.as_entire_binding(),
                    work.visible.as_entire_binding(),
                    work.state().as_entire_binding(),
                    wgpu::BindingResource::TextureView(&scene.skins.bones_view),
                    wgpu::BindingResource::TextureView(&scene.wind_field.view),
                ];
                if let (Some(more), Some(far)) = (&scene.clusters_more, &scene.pool_far) {
                    resources.extend([
                        more.as_entire_binding(),
                        scene.triangles.as_entire_binding(),
                        far.as_entire_binding(),
                    ]);
                    resources.extend(
                        scene
                            .pool_spares
                            .iter()
                            .map(|pool| pool.as_entire_binding()),
                    );
                }
                gpu.bind_group("raster", &self.raster_layout, &resources)
            },
            specks: gpu.bind_group(
                "depth specks",
                &self.speck_layout,
                &[
                    slots.binding(),
                    work.specks.as_entire_binding(),
                    work.state().as_entire_binding(),
                ],
            ),
            mask: self.masked.as_ref().map(|masked| {
                gpu.bind_group(
                    "masked raster",
                    &masked.layout,
                    &[
                        scene.materials.as_entire_binding(),
                        wgpu::BindingResource::TextureView(&scene.textures.base.color_view),
                        wgpu::BindingResource::TextureView(&scene.textures.middle.color_view),
                        wgpu::BindingResource::TextureView(&scene.textures.top.color_view),
                        wgpu::BindingResource::Sampler(&scene.material_sampler),
                    ],
                )
            }),
        }
    }

    /// Rasterizes everything the most recent cull of `work` selected.
    pub fn record(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        bindings: &RasterBindings,
        slot: u32,
        work: &CullWork,
        target: &VisibilityBuffer,
    ) {
        let offset = [ViewSlots::offset(slot)];
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("software raster"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.software);
            pass.set_bind_group(0, &bindings.group, &offset);
            pass.set_bind_group(1, &target.bind_group, &[]);
            pass.dispatch_workgroups_indirect(work.args(), CullWork::software_args_offset());
        }
        // In a pass of its own: following the opaque dispatch in one pass loses the device on
        // integrated Radeon graphics under Vulkan, a few frames in.
        if let (Some(masked), Some(mask)) = (&self.masked, &bindings.mask) {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("masked software raster"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&masked.software);
            pass.set_bind_group(0, &bindings.group, &offset);
            pass.set_bind_group(1, &target.bind_group, &[]);
            pass.set_bind_group(2, mask, &[]);
            pass.dispatch_workgroups_indirect(work.args(), CullWork::masked_software_args_offset());
        }

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("hardware raster"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &target.dummy_target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Discard,
                },
            })],
            ..Default::default()
        });
        pass.set_pipeline(&self.hardware);
        pass.set_bind_group(0, &bindings.group, &offset);
        pass.set_bind_group(1, &target.bind_group, &[]);
        pass.draw_indirect(work.args(), CullWork::draw_args_offset());
        if let (Some(masked), Some(mask)) = (&self.masked, &bindings.mask) {
            pass.set_pipeline(&masked.hardware);
            pass.set_bind_group(2, mask, &[]);
            pass.draw_indirect(work.args(), CullWork::masked_draw_args_offset());
        }
    }
}
