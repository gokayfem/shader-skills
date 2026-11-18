//! The hierarchical depth pyramid used for occlusion culling.

use super::raster::VisibilityBuffer;
use crate::gpu::{groups_for, Gpu, LayoutBuilder};
use crate::shaders;

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R32Float;

/// A power-of-two depth pyramid, rebuilt from the visibility buffer.
pub struct Hzb {
    pub view: wgpu::TextureView,
    pub size: (u32, u32),
    seed_pipeline: wgpu::ComputePipeline,
    down_pipeline: wgpu::ComputePipeline,
    seed_group: wgpu::BindGroup,
    /// One bind group per mip after the first, with that mip's size.
    down_groups: Vec<(wgpu::BindGroup, (u32, u32))>,
}

impl Hzb {
    pub fn new(gpu: &Gpu, visibility: &VisibilityBuffer) -> Self {
        let source = visibility.size();
        let size = (prev_power_of_two(source.0), prev_power_of_two(source.1));
        let mip_count = 32 - size.0.max(size.1).leading_zeros();

        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("hzb"),
            size: wgpu::Extent3d {
                width: size.0,
                height: size.1,
                depth_or_array_layers: 1,
            },
            mip_level_count: mip_count,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let mip_view = |mip: u32| {
            texture.create_view(&wgpu::TextureViewDescriptor {
                base_mip_level: mip,
                mip_level_count: Some(1),
                ..Default::default()
            })
        };

        let seed_layout = LayoutBuilder::new()
            .uniform()
            .storage_read()
            .storage_texture(FORMAT)
            .build(gpu, "hzb seed");
        let down_layout = LayoutBuilder::new()
            .texture_unfilterable()
            .storage_texture(FORMAT)
            .build(gpu, "hzb down");
        let seed_module = shaders::module(&gpu.device, "vg/hzb_seed.wgsl");
        let down_module = shaders::module(&gpu.device, "vg/hzb_down.wgsl");

        let params = gpu.uniform_buffer("hzb params", 16);
        gpu.queue.write_buffer(
            &params,
            0,
            bytemuck::cast_slice(&[source.0, source.1, size.0, size.1]),
        );
        let seed_group = gpu.bind_group(
            "hzb seed",
            &seed_layout,
            &[
                params.as_entire_binding(),
                visibility.depth.as_entire_binding(),
                wgpu::BindingResource::TextureView(&mip_view(0)),
            ],
        );

        let down_groups = (1..mip_count)
            .map(|mip| {
                let group = gpu.bind_group(
                    "hzb down",
                    &down_layout,
                    &[
                        wgpu::BindingResource::TextureView(&mip_view(mip - 1)),
                        wgpu::BindingResource::TextureView(&mip_view(mip)),
                    ],
                );
                (group, ((size.0 >> mip).max(1), (size.1 >> mip).max(1)))
            })
            .collect();

        Self {
            view: texture.create_view(&Default::default()),
            size,
            seed_pipeline: gpu.compute_pipeline("hzb seed", &seed_module, "main", &[&seed_layout]),
            down_pipeline: gpu.compute_pipeline("hzb down", &down_module, "main", &[&down_layout]),
            seed_group,
            down_groups,
        }
    }

    /// Rebuilds the whole pyramid from the visibility buffer's depth.
    pub fn record(&self, encoder: &mut wgpu::CommandEncoder) {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("hzb"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.seed_pipeline);
        pass.set_bind_group(0, &self.seed_group, &[]);
        pass.dispatch_workgroups(groups_for(self.size.0, 8), groups_for(self.size.1, 8), 1);

        pass.set_pipeline(&self.down_pipeline);
        for (group, size) in &self.down_groups {
            pass.set_bind_group(0, group, &[]);
            pass.dispatch_workgroups(groups_for(size.0, 8), groups_for(size.1, 8), 1);
        }
    }
}

fn prev_power_of_two(x: u32) -> u32 {
    1 << (31 - x.max(1).leading_zeros())
}

#[cfg(test)]
mod tests {
    use super::prev_power_of_two;

    #[test]
    fn prev_power_of_two_rounds_down() {
        assert_eq!(prev_power_of_two(1), 1);
        assert_eq!(prev_power_of_two(2), 2);
        assert_eq!(prev_power_of_two(1023), 512);
        assert_eq!(prev_power_of_two(1024), 1024);
        assert_eq!(prev_power_of_two(1920), 1024);
    }
}
