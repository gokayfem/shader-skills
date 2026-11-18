//! The sea's surface on screen: the patches of `grid` drawn into a water layer of their own, and
//! the water pass that shades them over the lit scene.
//!
//! The opaque scene is lit as it always is. Then the patches the camera needs are drawn by the
//! hardware rasterizer into the water layer, its own depth and where each pixel's water lay on
//! the calm sea. The water pass shades the water wherever it is nearer than the opaque scene, and
//! writes the result, with the water's depth and motion, into textures of its own that post then
//! reads. The lit opaque scene stays untouched, for refraction to see through the water.

use super::grid::{self, Patch, FINEST_PATCH};
use super::seabed::SeabedMap;
use super::spectrum::{CASCADE_SIZES, SLOPE_TABLE_FIRST, SLOPE_TABLE_SIZE, SLOPE_TABLE_STEP};
use super::waves::OceanWaves;
use super::Ocean;
use crate::gpu::{groups_for, Gpu, LayoutBuilder, Target};
use crate::shaders;
use crate::shadow::ShadowMaps;
use crate::vg::Rasterizer;
use bytemuck::{Pod, Zeroable};
use glam::Vec3;

const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const CALM_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rg32Float;
pub(super) const COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
pub(super) const SCENE_DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R32Float;
pub(super) const MOTION_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rg32Float;

/// The smoothest the water's surface ever is, as GGX alpha: still water is not a perfect
/// mirror, for the drawn waves' own slopes vary across a pixel.
const SMOOTHEST: f32 = 0.02;

/// Patches the instance buffer holds. The tests in `grid` keep the selection well under this.
const MAX_PATCHES: usize = 1024;

/// Twin of `SurfaceParams` in `ocean/surface_params.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct SurfaceUniform {
    sizes: [f32; 4],
    grid: [f32; 4],
    morph: [f32; 4],
    absorption: [f32; 4],
    scattering: [f32; 4],
    slope_range: [f32; 4],
    slopes: [[f32; 4]; 8],
    seabed_view_proj: [[f32; 4]; 4],
    seabed: [f32; 4],
    wind: [[f32; 4]; 2],
}

/// One patch, as the vertex shader reads it.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct PatchInstance {
    origin: [f32; 2],
    size: f32,
    level: u32,
    cells: u32,
}

/// A square grid of vertices and its triangles, for patches `cells` across.
struct PatchMesh {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    index_count: u32,
}

impl PatchMesh {
    fn new(gpu: &Gpu, cells: u32) -> Self {
        let mut vertices = Vec::new();
        for z in 0..=cells {
            for x in 0..=cells {
                vertices.push([x, z]);
            }
        }
        let mut indices = Vec::new();
        let at = |x: u32, z: u32| z * (cells + 1) + x;
        for z in 0..cells {
            for x in 0..cells {
                // Each square's diagonal runs the same way, so neighbouring patches split
                // their shared cells alike.
                indices.extend([at(x, z), at(x, z + 1), at(x + 1, z)]);
                indices.extend([at(x + 1, z), at(x, z + 1), at(x + 1, z + 1)]);
            }
        }
        Self {
            vertices: buffer(
                gpu,
                "ocean patch vertices",
                bytemuck::cast_slice(&vertices),
                wgpu::BufferUsages::VERTEX,
            ),
            indices: buffer(
                gpu,
                "ocean patch indices",
                bytemuck::cast_slice(&indices),
                wgpu::BufferUsages::INDEX,
            ),
            index_count: indices.len() as u32,
        }
    }
}

fn buffer(gpu: &Gpu, label: &str, bytes: &[u8], usage: wgpu::BufferUsages) -> wgpu::Buffer {
    let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: bytes.len() as u64,
        usage,
        mapped_at_creation: true,
    });
    buffer
        .slice(..)
        .get_mapped_range_mut()
        .expect("a buffer mapped at creation is writable")
        .copy_from_slice(bytes);
    buffer.unmap();
    buffer
}

/// The textures the water layer and pass draw into, at the render size.
pub struct WaterTargets {
    /// The layer the sea's patches are drawn into: its depth, and where each pixel's water lay
    /// on the calm sea. Still water has no need of it.
    layer: Option<(wgpu::TextureView, Target)>,
    /// The lit scene with the water over it, its depth and its motion: what post reads.
    pub color: Target,
    pub scene_depth: Target,
    pub motion: Target,
    pub(super) size: (u32, u32),
    pub(super) generation: u64,
}

impl WaterTargets {
    /// Targets for water over the scene; with a `sea`, the layer its surface is drawn into too.
    pub fn new(gpu: &Gpu, size: (u32, u32), sea: bool) -> Self {
        static GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let render = wgpu::TextureUsages::RENDER_ATTACHMENT;
        let layer = sea.then(|| {
            let depth = gpu.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("water depth"),
                size: wgpu::Extent3d {
                    width: size.0,
                    height: size.1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: DEPTH_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            (
                depth.create_view(&Default::default()),
                Target::new(gpu, "water calm", size, CALM_FORMAT, render),
            )
        });
        Self {
            layer,
            // Particles are drawn over it.
            color: Target::new(gpu, "scene with water", size, COLOR_FORMAT, render),
            // Solid particles draw their depth and motion into these two, and tests read them.
            scene_depth: Target::new(
                gpu,
                "scene depth with water",
                size,
                SCENE_DEPTH_FORMAT,
                render | wgpu::TextureUsages::COPY_SRC,
            ),
            motion: Target::new(
                gpu,
                "scene motion with water",
                size,
                MOTION_FORMAT,
                render | wgpu::TextureUsages::COPY_SRC,
            ),
            size,
            generation: GENERATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        }
    }

    /// Whether these have the layer the sea's surface is drawn into.
    pub fn for_sea(&self) -> bool {
        self.layer.is_some()
    }
}

/// What the water pass reads from the rest of the frame.
pub struct SceneViews<'a> {
    pub lit: &'a wgpu::TextureView,
    pub depth: &'a wgpu::TextureView,
    pub motion: &'a wgpu::TextureView,
    /// The opaque scene's closest-depth pyramid and normals, for tracing reflections.
    pub depth_pyramid: &'a wgpu::TextureView,
    pub normals: &'a wgpu::TextureView,
}

/// Draws and shades the sea's surface.
pub struct WaterSurface {
    full: PatchMesh,
    quarter: PatchMesh,
    instances: wgpu::Buffer,
    uniform: wgpu::Buffer,
    sampler: wgpu::Sampler,
    raster_layout: wgpu::BindGroupLayout,
    raster: wgpu::RenderPipeline,
    shade_layout: wgpu::BindGroupLayout,
    /// See [`crate::Fallbacks::ocean_depth_from_pyramid`].
    depth_from_pyramid: bool,
    shade: wgpu::ComputePipeline,
    cells: u32,
    /// This frame's patches: the whole ones, then the quarters.
    whole_count: u32,
    quarter_count: u32,
    /// Bind groups, and the waves and targets they were made for.
    groups: Option<(u64, u64, wgpu::BindGroup, wgpu::BindGroup)>,
    /// The seabed under the sea, which the waves shrink over as it shoals.
    pub seabed: SeabedMap,
    /// Tells this surface's resources from those of any other made before it.
    id: u64,
}

impl WaterSurface {
    /// Patches of `cells` cells across: [`grid::PATCH_CELLS`] for desktop GPUs, half that for
    /// integrated ones.
    pub fn new(
        gpu: &Gpu,
        environment: &wgpu::BindGroupLayout,
        rasterizer: &Rasterizer,
        cells: u32,
    ) -> Self {
        let raster_layout = LayoutBuilder::new()
            .visibility(wgpu::ShaderStages::VERTEX)
            .uniform()
            .uniform()
            .texture_array()
            .sampler()
            .depth_texture()
            .build(gpu, "ocean surface");
        let module = shaders::module(&gpu.device, "ocean/surface.wgsl");
        let layout = gpu
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("ocean surface"),
                bind_group_layouts: &[Some(&raster_layout)],
                immediate_size: 0,
            });
        let raster = gpu
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("ocean surface"),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[
                        Some(wgpu::VertexBufferLayout {
                            array_stride: 8,
                            step_mode: wgpu::VertexStepMode::Vertex,
                            attributes: &wgpu::vertex_attr_array![0 => Uint32x2],
                        }),
                        Some(wgpu::VertexBufferLayout {
                            array_stride: std::mem::size_of::<PatchInstance>() as u64,
                            step_mode: wgpu::VertexStepMode::Instance,
                            attributes: &wgpu::vertex_attr_array![
                                1 => Float32x2,
                                2 => Float32,
                                3 => Uint32x2,
                            ],
                        }),
                    ],
                },
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    // The sea is seen from above and, near the shore or in rough water, its
                    // waves' backs; draw both sides.
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH_FORMAT,
                    depth_write_enabled: Some(true),
                    // Reversed depth, as everywhere else: nearer is larger.
                    depth_compare: Some(wgpu::CompareFunction::Greater),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some("fs_main"),
                    compilation_options: Default::default(),
                    targets: &[Some(CALM_FORMAT.into())],
                }),
                multiview_mask: None,
                cache: None,
            });

        // See `Fallbacks::ocean_depth_from_pyramid`.
        let from_pyramid = gpu.fallbacks.ocean_depth_from_pyramid;
        let opaque_depth = LayoutBuilder::new().texture();
        let opaque_depth = if from_pyramid {
            opaque_depth.skip()
        } else {
            opaque_depth.texture_unfilterable()
        };
        let shade_layout = ShadowMaps::add_to_layout(
            opaque_depth
                .texture_unfilterable()
                .depth_texture()
                .texture_unfilterable()
                .texture_array()
                .texture_array()
                .sampler()
                .uniform()
                .storage_texture(COLOR_FORMAT)
                .storage_texture(SCENE_DEPTH_FORMAT)
                .storage_texture(MOTION_FORMAT),
        )
        .texture_unfilterable()
        .texture()
        .depth_texture()
        .build(gpu, "ocean shade");
        let mut source = shaders::expand("ocean/shade.wgsl");
        if from_pyramid {
            source = shaders::depth_from_pyramid(&source);
        }
        let shade = gpu.compute_pipeline(
            "ocean shade",
            &gpu.device
                .create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some("ocean/shade.wgsl"),
                    source: wgpu::ShaderSource::Wgsl(source.into()),
                }),
            "main",
            &[&shade_layout, environment],
        );

        Self {
            full: PatchMesh::new(gpu, cells),
            quarter: PatchMesh::new(gpu, cells / 2),
            instances: gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("ocean patches"),
                size: (MAX_PATCHES * std::mem::size_of::<PatchInstance>()) as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            uniform: gpu.uniform_buffer(
                "ocean surface",
                std::mem::size_of::<SurfaceUniform>() as u64,
            ),
            sampler: gpu.device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("ocean waves"),
                address_mode_u: wgpu::AddressMode::Repeat,
                address_mode_v: wgpu::AddressMode::Repeat,
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                mipmap_filter: wgpu::MipmapFilterMode::Linear,
                ..Default::default()
            }),
            raster_layout,
            raster,
            shade_layout,
            depth_from_pyramid: from_pyramid,
            shade,
            cells,
            whole_count: 0,
            quarter_count: 0,
            groups: None,
            seabed: SeabedMap::new(gpu, rasterizer),
            id: {
                static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            },
        }
    }

    /// Cells across each whole patch.
    pub fn cells(&self) -> u32 {
        self.cells
    }

    /// Tells this surface's resources from those of any other made before it.
    pub fn id(&self) -> u64 {
        self.id
    }

    /// The sea's surface parameters, as `ocean/surface_params.wgsl` reads them.
    pub fn uniform(&self) -> &wgpu::Buffer {
        &self.uniform
    }

    /// The sampler the waves are read through.
    pub fn wave_sampler(&self) -> &wgpu::Sampler {
        &self.sampler
    }

    /// Chooses this frame's patches for a camera at `camera`, over a sea at `sea_level`. The
    /// seabed map must have been placed for this frame first.
    /// `slopes` is the sea's table of unresolved slopes, from `spectrum::unresolved_slopes`.
    #[allow(clippy::too_many_arguments)]
    pub fn prepare(
        &mut self,
        gpu: &Gpu,
        camera: Vec3,
        sea_level: f32,
        ocean: &Ocean,
        waves: &OceanWaves,
        slopes: &[f32; SLOPE_TABLE_SIZE],
        wind: &crate::wind::WindSnapshot,
    ) {
        let mut patches = grid::select(camera, sea_level, self.cells);
        patches.truncate(MAX_PATCHES);
        // Whole patches first, then quarters, each drawn with its own mesh.
        patches.sort_by_key(|p| p.cells != self.cells);
        let instance = |p: &Patch| PatchInstance {
            origin: p.origin.to_array(),
            size: p.size,
            level: p.level,
            cells: p.cells,
        };
        let data: Vec<PatchInstance> = patches.iter().map(instance).collect();
        self.whole_count = patches.iter().filter(|p| p.cells == self.cells).count() as u32;
        self.quarter_count = patches.len() as u32 - self.whole_count;
        gpu.queue
            .write_buffer(&self.instances, 0, bytemuck::cast_slice(&data));

        let [ar, ag, ab] = ocean.absorption;
        let [sr, sg, sb] = ocean.scattering;
        let uniform = SurfaceUniform {
            sizes: [
                CASCADE_SIZES[0],
                CASCADE_SIZES[1],
                CASCADE_SIZES[2],
                sea_level,
            ],
            grid: [
                waves.size() as f32,
                ocean.choppiness,
                FINEST_PATCH,
                grid::range(0) / FINEST_PATCH,
            ],
            morph: [grid::morph_range(0).0 / grid::range(0), 0.0, 0.0, 0.0],
            absorption: [ar, ag, ab, 0.0],
            scattering: [sr, sg, sb, 0.0],
            slope_range: [
                SLOPE_TABLE_FIRST,
                SLOPE_TABLE_STEP,
                // The finest cascade's Nyquist wavenumber.
                std::f32::consts::PI * waves.size() as f32 / CASCADE_SIZES[2],
                SMOOTHEST,
            ],
            slopes: std::array::from_fn(|i| std::array::from_fn(|j| slopes[i * 4 + j])),
            seabed_view_proj: self.seabed.current().view_proj.to_cols_array_2d(),
            seabed: [
                self.seabed.current().top,
                self.seabed.current().reach,
                0.0,
                0.0,
            ],
            wind: wind.packed(),
        };
        gpu.queue
            .write_buffer(&self.uniform, 0, bytemuck::bytes_of(&uniform));
    }

    /// Draws the water layer, then shades the water over `scene` into `targets`.
    #[allow(clippy::too_many_arguments)]
    pub fn record(
        &mut self,
        gpu: &Gpu,
        encoder: &mut wgpu::CommandEncoder,
        frame_uniform: &wgpu::Buffer,
        environment: &wgpu::BindGroup,
        waves: &OceanWaves,
        targets: &WaterTargets,
        scene: SceneViews,
        shadows: &ShadowMaps,
    ) {
        let fresh = self
            .groups
            .as_ref()
            .is_some_and(|(w, t, _, _)| *w == waves.generation() && *t == targets.generation);
        let (layer_depth, calm) = targets
            .layer
            .as_ref()
            .expect("targets made for a sea have its layer");
        if !fresh {
            let tex = |view| wgpu::BindingResource::TextureView(view);
            let raster = gpu.bind_group(
                "ocean surface",
                &self.raster_layout,
                &[
                    frame_uniform.as_entire_binding(),
                    self.uniform.as_entire_binding(),
                    tex(&waves.displacement.view),
                    wgpu::BindingResource::Sampler(&self.sampler),
                    tex(&self.seabed.view),
                ],
            );
            let mut resources = vec![tex(scene.lit)];
            if !self.depth_from_pyramid {
                resources.push(tex(scene.depth));
            }
            resources.extend([
                tex(scene.motion),
                tex(layer_depth),
                tex(&calm.view),
                tex(&waves.derivatives.view),
                tex(&waves.displacement.view),
                wgpu::BindingResource::Sampler(&self.sampler),
                self.uniform.as_entire_binding(),
                tex(&targets.color.view),
                tex(&targets.scene_depth.view),
                tex(&targets.motion.view),
                shadows.uniform.as_entire_binding(),
                tex(&shadows.array_view),
                wgpu::BindingResource::Sampler(&shadows.comparison_sampler),
                tex(scene.depth_pyramid),
                tex(scene.normals),
                tex(&self.seabed.view),
            ]);
            let shade = if self.depth_from_pyramid {
                gpu.bind_group_skipping("ocean shade", &self.shade_layout, &resources, 1)
            } else {
                gpu.bind_group("ocean shade", &self.shade_layout, &resources)
            };
            self.groups = Some((waves.generation(), targets.generation, raster, shade));
        }
        let (_, _, raster, shade) = self.groups.as_ref().expect("made above");

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("ocean surface"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &calm.view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: layer_depth,
                    depth_ops: Some(wgpu::Operations {
                        // Reversed depth: zero is infinitely far, so no water anywhere yet.
                        load: wgpu::LoadOp::Clear(0.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.raster);
            pass.set_bind_group(0, raster, &[]);
            pass.set_vertex_buffer(1, self.instances.slice(..));
            for (mesh, first, count) in [
                (&self.full, 0, self.whole_count),
                (&self.quarter, self.whole_count, self.quarter_count),
            ] {
                if count > 0 {
                    pass.set_vertex_buffer(0, mesh.vertices.slice(..));
                    pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(0..mesh.index_count, 0, first..first + count);
                }
            }
        }

        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("ocean shade"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.shade);
        pass.set_bind_group(0, shade, &[]);
        pass.set_bind_group(1, environment, &[]);
        pass.dispatch_workgroups(
            groups_for(targets.size.0, 8),
            groups_for(targets.size.1, 8),
            1,
        );
    }
}
