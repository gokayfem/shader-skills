//! The closest-depth pyramid that screen traces march through.
//!
//! Unlike the culling pyramid in [`crate::vg::Hzb`], which keeps the farthest depth on a
//! power-of-two grid, this one keeps the nearest depth and lines up with the screen: mip 0
//! is the G-buffer depth itself and texel `pixel >> n` of mip `n` covers exactly the pixels
//! that shift to it.

use crate::gpu::{groups_for, Gpu, LayoutBuilder};
use crate::shaders;

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R32Float;
/// Mips beyond this cover so much of the screen that they rarely let a ray skip anything.
const MAX_MIPS: u32 = 8;

/// Pipelines that build a [`DepthPyramid`].
pub struct DepthPyramidBuilder {
    layout: wgpu::BindGroupLayout,
    copy: wgpu::ComputePipeline,
    reduce: wgpu::ComputePipeline,
}

/// The pyramid texture for one render size, with one bind group per mip.
pub struct DepthPyramid {
    pub view: wgpu::TextureView,
    /// `(bind group, output size)` per mip; the first copies, the rest reduce.
    passes: Vec<(wgpu::BindGroup, (u32, u32))>,
}

impl DepthPyramidBuilder {
    pub fn new(gpu: &Gpu) -> Self {
        let layout = LayoutBuilder::new()
            .texture_unfilterable()
            .storage_texture(FORMAT)
            .build(gpu, "depth pyramid");
        let module = shaders::module(&gpu.device, "gi/depth_pyramid.wgsl");
        Self {
            copy: gpu.compute_pipeline("depth pyramid copy", &module, "copy", &[&layout]),
            reduce: gpu.compute_pipeline("depth pyramid reduce", &module, "reduce", &[&layout]),
            layout,
        }
    }

    /// Creates the pyramid for a G-buffer `depth` of the given `size`.
    pub fn pyramid(&self, gpu: &Gpu, depth: &wgpu::TextureView, size: (u32, u32)) -> DepthPyramid {
        let (padded, mips) = layout(size);
        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("depth pyramid"),
            size: wgpu::Extent3d {
                width: padded.0,
                height: padded.1,
                depth_or_array_layers: 1,
            },
            mip_level_count: mips,
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
        let passes = (0..mips)
            .map(|mip| {
                let mip_size = (padded.0 >> mip, padded.1 >> mip);
                let input = if mip == 0 {
                    None
                } else {
                    Some(mip_view(mip - 1))
                };
                let group = gpu.bind_group(
                    "depth pyramid",
                    &self.layout,
                    &[
                        wgpu::BindingResource::TextureView(input.as_ref().unwrap_or(depth)),
                        wgpu::BindingResource::TextureView(&mip_view(mip)),
                    ],
                );
                (group, mip_size)
            })
            .collect();
        DepthPyramid {
            view: texture.create_view(&Default::default()),
            passes,
        }
    }

    /// Rebuilds every mip from the current G-buffer depth, inside an open compute pass.
    pub fn record(&self, pass: &mut wgpu::ComputePass, pyramid: &DepthPyramid) {
        for (mip, (group, size)) in pyramid.passes.iter().enumerate() {
            pass.set_pipeline(if mip == 0 { &self.copy } else { &self.reduce });
            pass.set_bind_group(0, group, &[]);
            pass.dispatch_workgroups(groups_for(size.0, 8), groups_for(size.1, 8), 1);
        }
    }
}

/// The texture size and mip count for a screen `size`. Mips of a texture round down, so
/// the texture is padded to a whole number of the coarsest tiles: then texel `pixel >> n` of
/// mip `n` exists for every pixel. The padding holds zero depth, which never occludes.
fn layout(size: (u32, u32)) -> ((u32, u32), u32) {
    let (w, h) = (size.0.max(1), size.1.max(1));
    let mips = (32 - w.min(h).leading_zeros()).min(MAX_MIPS);
    let tile = 1 << (mips - 1);
    ((w.next_multiple_of(tile), h.next_multiple_of(tile)), mips)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_pixel_has_a_texel_at_every_mip() {
        for size in [
            (1920, 1080),
            (1366, 768),
            (17, 17),
            (3, 1),
            (1, 1),
            (2001, 1087),
        ] {
            let ((w, h), mips) = layout(size);
            assert!(w >= size.0 && h >= size.1 && (1..=MAX_MIPS).contains(&mips));
            // WebGPU allows mips down to 1x1 of the largest side.
            assert!(mips <= 32 - w.max(h).leading_zeros());
            for mip in 0..mips {
                assert!((size.0 - 1) >> mip < (w >> mip).max(1));
                assert!((size.1 - 1) >> mip < (h >> mip).max(1));
            }
        }
        assert_eq!(layout((1920, 1080)), ((1920, 1152), MAX_MIPS));
    }
}
