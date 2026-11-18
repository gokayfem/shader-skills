//! Atmosphere lookup tables. See `shaders/sky/luts.wgsl`.

use crate::gpu::{groups_for, Gpu, LayoutBuilder, Target};
use crate::shaders;
use bytemuck::{Pod, Zeroable};
use glam::Vec3;

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const TRANSMITTANCE_SIZE: (u32, u32) = (256, 64);
const MULTI_SCATTER_SIZE: (u32, u32) = (32, 32);
const SKY_VIEW_SIZE: (u32, u32) = (256, 144);

/// Height of the viewer in kilometres above the planet centre at sea level.
const GROUND_RADIUS_KM: f32 = 6360.0;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct SkyParams {
    sun_direction: Vec3,
    viewer_radius: f32,
}

/// The sky's precomputed data and the passes that fill it.
pub struct Sky {
    pub transmittance: Target,
    pub sky_view: Target,
    params: wgpu::Buffer,
    build_transmittance: (wgpu::ComputePipeline, wgpu::BindGroup),
    build_multi_scatter: (wgpu::ComputePipeline, wgpu::BindGroup),
    build_sky_view: (wgpu::ComputePipeline, wgpu::BindGroup),
    static_tables_built: bool,
    last_params: Option<SkyParams>,
}

impl Sky {
    pub fn new(gpu: &Gpu) -> Self {
        let layout = LayoutBuilder::new()
            .uniform()
            .texture()
            .texture()
            .sampler()
            .storage_texture(FORMAT)
            .build(gpu, "sky");
        let module = shaders::module(&gpu.device, "sky/luts.wgsl");
        let params = gpu.uniform_buffer("sky params", std::mem::size_of::<SkyParams>() as u64);
        let sampler = gpu.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("sky"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let none = wgpu::TextureUsages::empty();
        let transmittance = Target::new(gpu, "sky transmittance", TRANSMITTANCE_SIZE, FORMAT, none);
        let multi_scatter = Target::new(gpu, "sky multi scatter", MULTI_SCATTER_SIZE, FORMAT, none);
        let sky_view = Target::new(gpu, "sky view", SKY_VIEW_SIZE, FORMAT, none);
        // Bind groups must be complete, but no entry point reads the table it is writing.
        let placeholder = Target::new(gpu, "sky placeholder", (1, 1), FORMAT, none);

        let pipeline = |entry: &str, input_a: &Target, input_b: &Target, output: &Target| {
            let group = gpu.bind_group(
                entry,
                &layout,
                &[
                    params.as_entire_binding(),
                    wgpu::BindingResource::TextureView(&input_a.view),
                    wgpu::BindingResource::TextureView(&input_b.view),
                    wgpu::BindingResource::Sampler(&sampler),
                    wgpu::BindingResource::TextureView(&output.view),
                ],
            );
            (
                gpu.compute_pipeline(entry, &module, entry, &[&layout]),
                group,
            )
        };

        Self {
            build_transmittance: pipeline(
                "build_transmittance",
                &placeholder,
                &placeholder,
                &transmittance,
            ),
            build_multi_scatter: pipeline(
                "build_multi_scatter",
                &transmittance,
                &placeholder,
                &multi_scatter,
            ),
            build_sky_view: pipeline("build_sky_view", &transmittance, &multi_scatter, &sky_view),
            transmittance,
            sky_view,
            params,
            static_tables_built: false,
            last_params: None,
        }
    }

    /// Updates the tables for the current sun and viewer height.
    pub fn record(
        &mut self,
        gpu: &Gpu,
        encoder: &mut wgpu::CommandEncoder,
        sun_direction: Vec3,
        viewer_height_m: f32,
    ) {
        let params = SkyParams {
            sun_direction,
            viewer_radius: GROUND_RADIUS_KM + viewer_height_m.max(1.0) * 0.001,
        };
        // No time or camera orientation enters the LUT. Compare the exact shader inputs
        // after height conversion; no tolerance or stale lighting is introduced.
        if self
            .last_params
            .is_some_and(|last| bytemuck::bytes_of(&last) == bytemuck::bytes_of(&params))
        {
            return;
        }
        self.last_params = Some(params);
        gpu.queue
            .write_buffer(&self.params, 0, bytemuck::bytes_of(&params));

        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("sky"),
            timestamp_writes: None,
        });
        let mut run = |(pipeline, group): &(wgpu::ComputePipeline, wgpu::BindGroup),
                       size: (u32, u32)| {
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, group, &[]);
            pass.dispatch_workgroups(groups_for(size.0, 8), groups_for(size.1, 8), 1);
        };
        if !self.static_tables_built {
            run(&self.build_transmittance, TRANSMITTANCE_SIZE);
            run(&self.build_multi_scatter, MULTI_SCATTER_SIZE);
            self.static_tables_built = true;
        }
        run(&self.build_sky_view, SKY_VIEW_SIZE);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cached_sky_matches_rebuilding_after_sun_and_height_changes() {
        let Some(gpu) = crate::gpu::test_gpu() else {
            return;
        };
        let mut sky = Sky::new(&gpu);
        let layout = LayoutBuilder::new()
            .texture_unfilterable()
            .storage_write()
            .build(&gpu, "sky readback");
        let module = gpu
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: None,
                source: wgpu::ShaderSource::Wgsl(
                    r#"
@group(0) @binding(0) var sky: texture_2d<f32>;
@group(0) @binding(1) var<storage, read_write> pixels: array<vec4f>;
@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3u) {
    let size = textureDimensions(sky);
    if all(id.xy < size) { pixels[id.y * size.x + id.x] = textureLoad(sky, id.xy, 0); }
}
"#
                    .into(),
                ),
            });
        let pipeline = gpu.compute_pipeline("sky readback", &module, "main", &[&layout]);
        let size = u64::from(SKY_VIEW_SIZE.0 * SKY_VIEW_SIZE.1) * 16;
        let output = gpu.storage_buffer("sky pixels", size, wgpu::BufferUsages::COPY_SRC);
        let group = gpu.bind_group(
            "sky readback",
            &layout,
            &[
                wgpu::BindingResource::TextureView(&sky.sky_view.view),
                output.as_entire_binding(),
            ],
        );
        let staging = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let read = |sky: &mut Sky, sun, height| {
            let mut encoder = gpu.device.create_command_encoder(&Default::default());
            sky.record(&gpu, &mut encoder, sun, height);
            {
                let mut pass = encoder.begin_compute_pass(&Default::default());
                pass.set_pipeline(&pipeline);
                pass.set_bind_group(0, &group, &[]);
                pass.dispatch_workgroups(SKY_VIEW_SIZE.0 / 8, SKY_VIEW_SIZE.1 / 8, 1);
            }
            encoder.copy_buffer_to_buffer(&output, 0, &staging, 0, size);
            gpu.queue.submit([encoder.finish()]);
            staging
                .slice(..)
                .map_async(wgpu::MapMode::Read, |r| r.unwrap());
            gpu.device
                .poll(wgpu::PollType::wait_indefinitely())
                .unwrap();
            let data = staging.slice(..).get_mapped_range().unwrap().to_vec();
            staging.unmap();
            data
        };
        let mut previous = Vec::new();
        for (sun, height) in [
            (Vec3::Y, 1.0),
            (Vec3::Y, 1000.0),
            (Vec3::new(1.0, 0.1, 0.0).normalize(), 1000.0),
        ] {
            let changed = read(&mut sky, sun, height);
            assert_ne!(changed, previous, "changed input must refresh the LUT");
            assert_eq!(changed, read(&mut sky, sun, height));
            sky.last_params = None;
            assert_eq!(
                changed,
                read(&mut sky, sun, height),
                "cached and rebuilt pixels differ"
            );
            previous = changed;
        }
    }
}
