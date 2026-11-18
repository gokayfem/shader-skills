//! Everything after lighting: temporal resolve, exposure, bloom and the final image.

use crate::gpu::{groups_for, Gpu, LayoutBuilder, Target};
use crate::shaders;
use bytemuck::{Pod, Zeroable};
use glam::Vec2;

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

#[cfg(test)]
#[path = "post_stress.rs"]
mod stress;
/// Matches `BLOOM_LEVELS` in `post/tonemap.wgsl`.
const BLOOM_LEVELS: u32 = 6;

/// The brightest average luminance exposure adapts to, per unit of sun. Without a limit, exposure
/// maps anything that fills the frame to middle grey, as a camera turns snow grey. Sunlit snow
/// measures about 0.08 per unit of sun. This sits just above the demo's pale desert, so that only
/// scenes truly brighter than any bare ground stay bright. Any lower and that desert brightens too.
const BRIGHTEST_AVERAGE_PER_SUN: f32 = 0.06;
/// The darkest average luminance exposure adapts to, per unit of daylight sun. Below it, as at
/// night, the image darkens rather than being brightened to grey, as an eye adapted to the dark
/// still sees night as night: about seven stops below the brightest.
const DARKEST_AVERAGE_PER_SUN: f32 = BRIGHTEST_AVERAGE_PER_SUN / 128.0;
/// How much further exposure adapts by day. The dark is then a room, not the night, and an eye
/// that has walked in from the sun sees a room plainly: a hall lit through its windows measures
/// eleven stops below the sunlit ground outside, four below where night stops. Five more stops
/// reach it and a little beyond; a cellar with no window stays dark.
const DARKEST_BY_DAY: f32 = 1.0 / 32.0;
/// Matches `BINS` in `post/exposure.wgsl`.
const EXPOSURE_BINS: u64 = 192;

/// Twin of `Params` in `post/exposure.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ExposureParams {
    delta_seconds: f32,
    first_frame: u32,
    compensation: f32,
    brightest_average: f32,
    darkest_average: f32,
    _pad: [f32; 3],
}

/// Twin of `Params` in `post/bloom.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct BloomParams {
    source_texel: Vec2,
    is_first: u32,
    _pad: u32,
}

/// Twin of `Params` in `post/tonemap.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct TonemapParams {
    bloom_strength: f32,
    sharpen: f32,
    vignette: f32,
    grain: f32,
    frame_index: u32,
    debug_passthrough: u32,
    _pad: [u32; 2],
}

/// Post-processing pipelines, created once per output format.
pub struct PostProcess {
    /// Holds exposure where it is, as an eye stays dazzled through a flash of lightning.
    pub hold_exposure: bool,
    taa_layout: wgpu::BindGroupLayout,
    taa: wgpu::ComputePipeline,
    bloom_layout: wgpu::BindGroupLayout,
    bloom_down: wgpu::ComputePipeline,
    bloom_up: wgpu::ComputePipeline,
    exposure_layout: wgpu::BindGroupLayout,
    exposure_histogram: wgpu::ComputePipeline,
    exposure_resolve: wgpu::ComputePipeline,
    exposure_params: wgpu::Buffer,
    histogram: wgpu::Buffer,
    exposure: wgpu::Buffer,
    tonemap_layout: wgpu::BindGroupLayout,
    tonemap: wgpu::RenderPipeline,
    tonemap_params: wgpu::Buffer,
    linear_sampler: wgpu::Sampler,
}

/// Size-dependent targets and bind groups for one output resolution.
pub struct PostBindings {
    pub history: [Target; 2],
    taa_groups: [wgpu::BindGroup; 2],
    exposure_group: wgpu::BindGroup,
    bloom_steps: [Vec<BloomStep>; 2],
    tonemap_groups: [wgpu::BindGroup; 2],
    output_size: (u32, u32),
    render_size: (u32, u32),
}

struct BloomStep {
    down: bool,
    group: wgpu::BindGroup,
    size: (u32, u32),
}

/// A texture with one storage/sampled view per mip.
struct MipChain {
    views: Vec<wgpu::TextureView>,
    sizes: Vec<(u32, u32)>,
}

impl MipChain {
    fn new(gpu: &Gpu, label: &str, size: (u32, u32), levels: u32) -> Self {
        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: size.0.max(1),
                height: size.1.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: levels,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let views = (0..levels)
            .map(|mip| {
                texture.create_view(&wgpu::TextureViewDescriptor {
                    base_mip_level: mip,
                    mip_level_count: Some(1),
                    ..Default::default()
                })
            })
            .collect();
        let sizes = (0..levels)
            .map(|mip| ((size.0 >> mip).max(1), (size.1 >> mip).max(1)))
            .collect();
        Self { views, sizes }
    }
}

impl PostProcess {
    /// `output_format` must not be sRGB, since the final pass applies display encoding.
    pub fn new(gpu: &Gpu, output_format: wgpu::TextureFormat) -> Self {
        let taa_layout = LayoutBuilder::new()
            .uniform()
            .texture()
            .texture_unfilterable()
            .texture_unfilterable()
            .texture()
            .sampler()
            .storage_texture(FORMAT)
            .texture()
            .build(gpu, "taa");
        let bloom_layout = LayoutBuilder::new()
            .uniform()
            .texture()
            .texture()
            .sampler()
            .storage_texture(FORMAT)
            .build(gpu, "bloom");
        let exposure_layout = LayoutBuilder::new()
            .uniform()
            .texture()
            .storage_write()
            .storage_write()
            .build(gpu, "exposure");
        let tonemap_layout = LayoutBuilder::new()
            .visibility(wgpu::ShaderStages::FRAGMENT)
            .uniform()
            .texture()
            .texture()
            .sampler()
            .storage_read()
            .build(gpu, "tonemap");

        let taa_module = shaders::module(&gpu.device, "post/taa.wgsl");
        let bloom_module = shaders::module(&gpu.device, "post/bloom.wgsl");
        let exposure_module = shaders::module(&gpu.device, "post/exposure.wgsl");
        let tonemap_module = shaders::module(&gpu.device, "post/tonemap.wgsl");

        let pipeline_layout = gpu
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("tonemap"),
                bind_group_layouts: &[Some(&tonemap_layout)],
                immediate_size: 0,
            });
        let tonemap = gpu
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("tonemap"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &tonemap_module,
                    entry_point: Some("vs_fullscreen"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &tonemap_module,
                    entry_point: Some("fs_main"),
                    compilation_options: Default::default(),
                    targets: &[Some(output_format.into())],
                }),
                multiview_mask: None,
                cache: None,
            });

        Self {
            taa: gpu.compute_pipeline("taa", &taa_module, "main", &[&taa_layout]),
            bloom_down: gpu.compute_pipeline(
                "bloom down",
                &bloom_module,
                "downsample",
                &[&bloom_layout],
            ),
            bloom_up: gpu.compute_pipeline("bloom up", &bloom_module, "upsample", &[&bloom_layout]),
            exposure_histogram: gpu.compute_pipeline(
                "exposure histogram",
                &exposure_module,
                "build_histogram",
                &[&exposure_layout],
            ),
            exposure_resolve: gpu.compute_pipeline(
                "exposure resolve",
                &exposure_module,
                "resolve",
                &[&exposure_layout],
            ),
            exposure_params: gpu.uniform_buffer(
                "exposure params",
                std::mem::size_of::<ExposureParams>() as u64,
            ),
            histogram: gpu.storage_buffer(
                "exposure histogram",
                EXPOSURE_BINS * 4,
                wgpu::BufferUsages::empty(),
            ),
            exposure: gpu.storage_buffer("exposure", 16, wgpu::BufferUsages::empty()),
            tonemap_params: gpu.uniform_buffer(
                "tonemap params",
                std::mem::size_of::<TonemapParams>() as u64,
            ),
            linear_sampler: gpu.device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("post linear"),
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            }),
            taa_layout,
            bloom_layout,
            exposure_layout,
            hold_exposure: false,
            tonemap_layout,
            tonemap,
        }
    }

    /// Creates the history targets, which other passes also read. Finish the rest of the
    /// setup with [`Self::bindings`] once the HDR scene target exists.
    pub fn history_targets(gpu: &Gpu, output_size: (u32, u32)) -> [Target; 2] {
        let none = wgpu::TextureUsages::empty();
        [
            Target::new(gpu, "taa history 0", output_size, FORMAT, none),
            Target::new(gpu, "taa history 1", output_size, FORMAT, none),
        ]
    }

    #[allow(clippy::too_many_arguments)]
    pub fn bindings(
        &self,
        gpu: &Gpu,
        frame_uniform: &wgpu::Buffer,
        hdr: &wgpu::TextureView,
        depth: &wgpu::TextureView,
        motion: &wgpu::TextureView,
        reactive: &wgpu::TextureView,
        history: [Target; 2],
        render_size: (u32, u32),
        output_size: (u32, u32),
    ) -> PostBindings {
        let tex = |view| wgpu::BindingResource::TextureView(view);
        let sampler = wgpu::BindingResource::Sampler(&self.linear_sampler);

        let taa_group = |parity: usize| {
            gpu.bind_group(
                "taa",
                &self.taa_layout,
                &[
                    frame_uniform.as_entire_binding(),
                    tex(hdr),
                    tex(depth),
                    tex(motion),
                    tex(&history[1 - parity].view),
                    sampler.clone(),
                    tex(&history[parity].view),
                    tex(reactive),
                ],
            )
        };

        let exposure_group = gpu.bind_group(
            "exposure",
            &self.exposure_layout,
            &[
                self.exposure_params.as_entire_binding(),
                tex(hdr),
                self.histogram.as_entire_binding(),
                self.exposure.as_entire_binding(),
            ],
        );

        // Bloom: the source of the first down step depends on which history was written.
        // A full chain needs a minimum size. Below that it is simply wider than the output,
        // which is harmless, since these passes resample by UV rather than by texel.
        let min_edge = 1 << (BLOOM_LEVELS - 1);
        let half = (
            (output_size.0 / 2).max(min_edge),
            (output_size.1 / 2).max(min_edge),
        );
        let down = MipChain::new(gpu, "bloom down", half, BLOOM_LEVELS);
        let up = MipChain::new(gpu, "bloom up", half, BLOOM_LEVELS - 1);
        let bloom_params = |source_size: (u32, u32), is_first: bool| {
            let buffer =
                gpu.uniform_buffer("bloom params", std::mem::size_of::<BloomParams>() as u64);
            let params = BloomParams {
                source_texel: Vec2::new(1.0 / source_size.0 as f32, 1.0 / source_size.1 as f32),
                is_first: u32::from(is_first),
                _pad: 0,
            };
            gpu.queue
                .write_buffer(&buffer, 0, bytemuck::bytes_of(&params));
            buffer
        };
        // Bind groups must be complete, and `lower` is unused while downsampling.
        let placeholder = Target::new(
            gpu,
            "bloom placeholder",
            (1, 1),
            FORMAT,
            wgpu::TextureUsages::empty(),
        );
        let bloom_steps_for = |parity: usize| {
            let mut steps = Vec::new();
            for mip in 0..BLOOM_LEVELS as usize {
                let (source, source_size) = if mip == 0 {
                    (&history[parity].view, output_size)
                } else {
                    (&down.views[mip - 1], down.sizes[mip - 1])
                };
                let params = bloom_params(source_size, mip == 0);
                steps.push(BloomStep {
                    down: true,
                    size: down.sizes[mip],
                    group: gpu.bind_group(
                        "bloom down",
                        &self.bloom_layout,
                        &[
                            params.as_entire_binding(),
                            tex(source),
                            tex(&placeholder.view),
                            sampler.clone(),
                            tex(&down.views[mip]),
                        ],
                    ),
                });
            }
            for mip in (0..BLOOM_LEVELS as usize - 1).rev() {
                let lower = if mip == BLOOM_LEVELS as usize - 2 {
                    &down.views[mip + 1]
                } else {
                    &up.views[mip + 1]
                };
                let params = bloom_params(up.sizes[mip], false);
                steps.push(BloomStep {
                    down: false,
                    size: up.sizes[mip],
                    group: gpu.bind_group(
                        "bloom up",
                        &self.bloom_layout,
                        &[
                            params.as_entire_binding(),
                            tex(&down.views[mip]),
                            tex(lower),
                            sampler.clone(),
                            tex(&up.views[mip]),
                        ],
                    ),
                });
            }
            steps
        };

        let tonemap_group = |parity: usize| {
            gpu.bind_group(
                "tonemap",
                &self.tonemap_layout,
                &[
                    self.tonemap_params.as_entire_binding(),
                    tex(&history[parity].view),
                    tex(&up.views[0]),
                    sampler.clone(),
                    self.exposure.as_entire_binding(),
                ],
            )
        };

        PostBindings {
            taa_groups: [taa_group(0), taa_group(1)],
            exposure_group,
            bloom_steps: [bloom_steps_for(0), bloom_steps_for(1)],
            tonemap_groups: [tonemap_group(0), tonemap_group(1)],
            history,
            output_size,
            render_size,
        }
    }

    /// Records everything from the lit scene to the final output image.
    #[allow(clippy::too_many_arguments)]
    pub fn record(
        &self,
        gpu: &Gpu,
        encoder: &mut wgpu::CommandEncoder,
        bindings: &PostBindings,
        output: &wgpu::TextureView,
        parity: usize,
        frame_index: u32,
        delta_seconds: f32,
        daylight_illuminance: f32,
        day: f32,
        debug_passthrough: bool,
    ) {
        gpu.queue.write_buffer(
            &self.exposure_params,
            0,
            bytemuck::bytes_of(&ExposureParams {
                delta_seconds: if self.hold_exposure {
                    0.0
                } else {
                    delta_seconds
                },
                first_frame: u32::from(frame_index == 0),
                compensation: 1.2,
                brightest_average: daylight_illuminance * BRIGHTEST_AVERAGE_PER_SUN,
                darkest_average: daylight_illuminance
                    * DARKEST_AVERAGE_PER_SUN
                    * DARKEST_BY_DAY.powf(day),
                _pad: [0.0; 3],
            }),
        );
        gpu.queue.write_buffer(
            &self.tonemap_params,
            0,
            bytemuck::bytes_of(&TonemapParams {
                bloom_strength: 0.045,
                sharpen: 0.9,
                vignette: 0.18,
                grain: 0.012,
                frame_index,
                debug_passthrough: u32::from(debug_passthrough),
                _pad: [0; 2],
            }),
        );

        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("post compute"),
                timestamp_writes: None,
            });

            pass.set_pipeline(&self.exposure_histogram);
            pass.set_bind_group(0, &bindings.exposure_group, &[]);
            pass.dispatch_workgroups(
                groups_for(bindings.render_size.0.div_ceil(4), 16),
                groups_for(bindings.render_size.1.div_ceil(4), 16),
                1,
            );
            pass.set_pipeline(&self.exposure_resolve);
            pass.dispatch_workgroups(1, 1, 1);

            pass.set_pipeline(&self.taa);
            pass.set_bind_group(0, &bindings.taa_groups[parity], &[]);
            pass.dispatch_workgroups(
                groups_for(bindings.output_size.0, 8),
                groups_for(bindings.output_size.1, 8),
                1,
            );

            for step in &bindings.bloom_steps[parity] {
                pass.set_pipeline(if step.down {
                    &self.bloom_down
                } else {
                    &self.bloom_up
                });
                pass.set_bind_group(0, &step.group, &[]);
                pass.dispatch_workgroups(groups_for(step.size.0, 8), groups_for(step.size.1, 8), 1);
            }
        }

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("tonemap"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: output,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        pass.set_pipeline(&self.tonemap);
        pass.set_bind_group(0, &bindings.tonemap_groups[parity], &[]);
        pass.draw(0..3, 0..1);
    }
}
