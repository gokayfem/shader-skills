//! Converts the visibility buffer into a G-buffer.

use super::culling::CullWork;
#[cfg(test)]
#[path = "resolve_stress.rs"]
mod stress;
use super::raster::VisibilityBuffer;
use crate::gpu::{groups_for, Gpu, LayoutBuilder, Target};
use crate::regions::RegionPlan;
use crate::scene::GpuScene;
use crate::shaders;

const ALBEDO_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const NORMAL_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const EMISSIVE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const MOTION_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rg32Float;
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R32Float;

/// Surface properties per pixel.
pub struct GBuffer {
    /// RGB base colour, A cavity.
    pub albedo: Target,
    /// RG octahedral normal, B roughness, A metallic.
    pub normal: Target,
    pub emissive: Target,
    /// UV offset from this pixel to its position in the previous frame.
    pub motion: Target,
    /// Reversed-Z depth; zero where there is no geometry.
    pub depth: Target,
}

impl GBuffer {
    pub fn new(gpu: &Gpu, size: (u32, u32)) -> Self {
        let none = wgpu::TextureUsages::empty();
        // Solid particles draw their depth and motion into these when there is no water, and
        // tests read them back.
        let render = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC;
        Self {
            albedo: Target::new(gpu, "gbuffer albedo", size, ALBEDO_FORMAT, none),
            normal: Target::new(gpu, "gbuffer normal", size, NORMAL_FORMAT, none),
            emissive: Target::new(gpu, "gbuffer emissive", size, EMISSIVE_FORMAT, none),
            motion: Target::new(gpu, "gbuffer motion", size, MOTION_FORMAT, render),
            depth: Target::new(gpu, "gbuffer depth", size, DEPTH_FORMAT, render),
        }
    }
}

/// Where the resolve's output group binds depth.
const DEPTH_BINDING: u32 = 4;

/// The resolve compute pass.
pub struct Resolve {
    pipeline: wgpu::ComputePipeline,
    data_layout: wgpu::BindGroupLayout,
    output_layout: wgpu::BindGroupLayout,
    /// On a GPU allowing too few storage textures per stage for the resolve to write depth
    /// too ([`crate::Fallbacks::separate_depth`]), the pass that writes it, and its layout.
    depth: Option<(wgpu::ComputePipeline, wgpu::BindGroupLayout)>,
}

/// The bind groups of one resolve configuration, rebuilt on resize.
pub struct ResolveBindings {
    data: wgpu::BindGroup,
    output: wgpu::BindGroup,
    depth: Option<wgpu::BindGroup>,
}

impl Resolve {
    /// `wetness` is the layout of the third bind group, from [`crate::ocean::waterline`].
    pub fn new(
        gpu: &Gpu,
        wetness: &wgpu::BindGroupLayout,
        rain: &wgpu::BindGroupLayout,
        deform: crate::shaders::Deform,
        regions: Option<RegionPlan>,
        glow: bool,
    ) -> Self {
        let mut data_layout = LayoutBuilder::new()
            .uniform()
            .storage_read()
            .storage_read()
            .storage_read()
            .storage_read()
            .storage_read()
            .storage_read()
            .storage_read()
            .storage_read()
            .texture_unfilterable()
            // The wind, for what sways in it.
            .texture_unfilterable();
        if regions.is_some() {
            // The second cluster binding and the pool's far band.
            for _ in 0..regions.map_or(0, |plan| plan.pools() - 1) {
                data_layout = data_layout.storage_read();
            }
        }
        let data_layout = data_layout.build(gpu, "resolve data");
        let separate_depth = gpu.fallbacks.separate_depth;
        let output_layout = LayoutBuilder::new()
            .storage_texture(ALBEDO_FORMAT)
            .storage_texture(NORMAL_FORMAT)
            .storage_texture(EMISSIVE_FORMAT)
            .storage_texture(MOTION_FORMAT);
        let mut output_layout = if separate_depth {
            output_layout.skip()
        } else {
            output_layout.storage_texture(DEPTH_FORMAT)
        }
        .sampler()
        .texture_array()
        .texture_array()
        .texture_array()
        .texture_array()
        .texture_array()
        .texture_array()
        .texture_array()
        .texture_array()
        .texture_array()
        // The ruts pressed into the ground.
        .texture_array();
        if glow {
            output_layout = output_layout.texture_array();
        }
        let output_layout = output_layout.build(gpu, "resolve output");
        let mut source = shaders::expand_for("vg/resolve.wgsl", deform, regions);
        if !glow {
            source = shaders::without_glow(&source);
        }
        if separate_depth {
            source = shaders::without_section(&source, "depth");
        }
        let module = gpu
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("vg/resolve.wgsl"),
                source: wgpu::ShaderSource::Wgsl(source.into()),
            });
        Self {
            pipeline: gpu.compute_pipeline(
                "resolve",
                &module,
                "main",
                &[&data_layout, &output_layout, wetness, rain],
            ),
            data_layout,
            output_layout,
            depth: separate_depth.then(|| {
                let layout = LayoutBuilder::new()
                    .uniform()
                    .storage_read()
                    .storage_texture(DEPTH_FORMAT)
                    .build(gpu, "resolve depth");
                let module = gpu
                    .device
                    .create_shader_module(wgpu::ShaderModuleDescriptor {
                        label: Some("vg/resolve_depth.wgsl"),
                        source: wgpu::ShaderSource::Wgsl(
                            shaders::expand("vg/resolve_depth.wgsl").into(),
                        ),
                    });
                let pipeline = gpu.compute_pipeline("resolve depth", &module, "main", &[&layout]);
                (pipeline, layout)
            }),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn bindings(
        &self,
        gpu: &Gpu,
        frame: &wgpu::Buffer,
        scene: &GpuScene,
        visibility: &VisibilityBuffer,
        work: &CullWork,
        gbuffer: &GBuffer,
        ruts: &wgpu::TextureView,
    ) -> ResolveBindings {
        ResolveBindings {
            data: {
                let mut resources = vec![
                    frame.as_entire_binding(),
                    visibility.depth.as_entire_binding(),
                    visibility.ids.as_entire_binding(),
                    work.visible.as_entire_binding(),
                    scene.instances.as_entire_binding(),
                    scene.clusters.as_entire_binding(),
                    scene.vertices.as_entire_binding(),
                    scene.triangles.as_entire_binding(),
                    scene.materials.as_entire_binding(),
                    wgpu::BindingResource::TextureView(&scene.skins.bones_view),
                    wgpu::BindingResource::TextureView(&scene.wind_field.view),
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
                gpu.bind_group("resolve data", &self.data_layout, &resources)
            },
            output: {
                let mut resources = vec![
                    wgpu::BindingResource::TextureView(&gbuffer.albedo.view),
                    wgpu::BindingResource::TextureView(&gbuffer.normal.view),
                    wgpu::BindingResource::TextureView(&gbuffer.emissive.view),
                    wgpu::BindingResource::TextureView(&gbuffer.motion.view),
                ];
                if self.depth.is_none() {
                    resources.push(wgpu::BindingResource::TextureView(&gbuffer.depth.view));
                }
                resources.extend([
                    wgpu::BindingResource::Sampler(&scene.material_sampler),
                    wgpu::BindingResource::TextureView(&scene.textures.base.color_view),
                    wgpu::BindingResource::TextureView(&scene.textures.base.normal_view),
                    wgpu::BindingResource::TextureView(&scene.textures.base.material_view),
                    wgpu::BindingResource::TextureView(&scene.textures.middle.color_view),
                    wgpu::BindingResource::TextureView(&scene.textures.middle.normal_view),
                    wgpu::BindingResource::TextureView(&scene.textures.middle.material_view),
                    wgpu::BindingResource::TextureView(&scene.textures.top.color_view),
                    wgpu::BindingResource::TextureView(&scene.textures.top.normal_view),
                    wgpu::BindingResource::TextureView(&scene.textures.top.material_view),
                    wgpu::BindingResource::TextureView(ruts),
                ]);
                if let Some(glow) = &scene.glow_maps {
                    resources.push(wgpu::BindingResource::TextureView(glow));
                }
                if self.depth.is_some() {
                    gpu.bind_group_skipping(
                        "resolve output",
                        &self.output_layout,
                        &resources,
                        DEPTH_BINDING,
                    )
                } else {
                    gpu.bind_group("resolve output", &self.output_layout, &resources)
                }
            },
            depth: self.depth.as_ref().map(|(_, layout)| {
                gpu.bind_group(
                    "resolve depth",
                    layout,
                    &[
                        frame.as_entire_binding(),
                        visibility.depth.as_entire_binding(),
                        wgpu::BindingResource::TextureView(&gbuffer.depth.view),
                    ],
                )
            }),
        }
    }

    pub fn record(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        bindings: &ResolveBindings,
        wetness: &wgpu::BindGroup,
        rain: &wgpu::BindGroup,
        size: (u32, u32),
    ) {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("resolve"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &bindings.data, &[]);
        pass.set_bind_group(1, &bindings.output, &[]);
        pass.set_bind_group(2, wetness, &[]);
        pass.set_bind_group(3, rain, &[]);
        pass.dispatch_workgroups(groups_for(size.0, 8), groups_for(size.1, 8), 1);
        if let (Some((pipeline, _)), Some(group)) = (&self.depth, &bindings.depth) {
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, group, &[]);
            pass.dispatch_workgroups(groups_for(size.0, 8), groups_for(size.1, 8), 1);
        }
    }
}
