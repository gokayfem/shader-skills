//! Still water: ponds, pools and basins.
//!
//! A pond is a level sheet of water over a rectangle of the scene, for scenes whose basins came
//! without their water. Nothing is drawn or simulated: one pass over the lit scene finds, for
//! each pixel, where its view ray meets a pond, and shades the water there as the sea's is
//! shaded, with what it reflects and what is seen through it (`ocean/pond.wgsl`). A basin's own
//! walls, standing above the water, hide the sheet's edges.
//!
//! A scene without ponds makes none of this and pays nothing. A scene with an open sea draws
//! the sea and not its ponds.

use super::surface::{SceneViews, WaterTargets, COLOR_FORMAT, MOTION_FORMAT, SCENE_DEPTH_FORMAT};
use super::Ocean;
use crate::gpu::{groups_for, Gpu, LayoutBuilder};
use crate::shaders;
use crate::shadow::ShadowMaps;
use bytemuck::{Pod, Zeroable};

/// The most ponds a scene has. Twin of `MAX_PONDS` in `ocean/pond.wgsl`.
pub const MAX_PONDS: usize = 8;

/// A level sheet of still water over a rectangle of the scene.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pond {
    /// The rectangle's least x and z, in metres.
    pub min: [f32; 2],
    /// Its greatest x and z.
    pub max: [f32; 2],
    /// The water's height.
    pub level: f32,
}

impl Pond {
    /// Reads a pond written `x0,z0,x1,z1,level`: two opposite corners, in either order, and the
    /// water's height.
    pub fn parse(text: &str) -> Result<Self, String> {
        let numbers: Vec<f32> = text
            .split(',')
            .map(|word| {
                word.trim()
                    .parse::<f32>()
                    .ok()
                    .filter(|v| v.is_finite())
                    .ok_or_else(|| format!("{:?} is not a number", word.trim()))
            })
            .collect::<Result<_, _>>()?;
        let [x0, z0, x1, z1, level] = numbers[..] else {
            return Err(format!(
                "a pond is x0,z0,x1,z1,level; found {} numbers",
                numbers.len()
            ));
        };
        Ok(Self {
            min: [x0.min(x1), z0.min(z1)],
            max: [x0.max(x1), z0.max(z1)],
            level,
        })
    }

    /// Reads ponds written one after another with `;` between them.
    pub fn parse_list(text: &str) -> Result<Vec<Self>, String> {
        text.split(';')
            .filter(|pond| !pond.trim().is_empty())
            .map(Self::parse)
            .collect()
    }
}

/// Twin of `Ponds` in `ocean/pond.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct PondUniform {
    count: [u32; 4],
    absorption: [f32; 4],
    scattering: [f32; 4],
    wind: [[f32; 4]; 2],
    ponds: [[[f32; 4]; 2]; MAX_PONDS],
}

/// Where the wind lies in `PondUniform`, so that it can be written each frame alone.
const WIND_OFFSET: u64 = 48;

/// Shades the scene's ponds over the lit scene.
pub struct PondWater {
    uniform: wgpu::Buffer,
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::ComputePipeline,
    /// The bind group, and the targets it was made for.
    group: Option<(u64, wgpu::BindGroup)>,
}

impl PondWater {
    pub fn new(gpu: &Gpu, environment: &wgpu::BindGroupLayout) -> Self {
        let layout = ShadowMaps::add_to_layout(
            LayoutBuilder::new()
                .texture()
                .texture_unfilterable()
                .texture_unfilterable()
                .uniform()
                .storage_texture(COLOR_FORMAT)
                .storage_texture(SCENE_DEPTH_FORMAT)
                .storage_texture(MOTION_FORMAT),
        )
        .texture_unfilterable()
        .texture()
        .build(gpu, "pond water");
        let pipeline = gpu.compute_pipeline(
            "pond water",
            &shaders::module(&gpu.device, "ocean/pond.wgsl"),
            "main",
            &[&layout, environment],
        );
        Self {
            uniform: gpu.uniform_buffer("ponds", std::mem::size_of::<PondUniform>() as u64),
            layout,
            pipeline,
            group: None,
        }
    }

    /// Sets the ponds, and the water they hold. Those past [`MAX_PONDS`] are left out.
    pub fn set(&self, gpu: &Gpu, ponds: &[Pond], water: &Ocean) {
        let [ar, ag, ab] = water.absorption;
        let [sr, sg, sb] = water.scattering;
        let mut uniform = PondUniform {
            count: [ponds.len().min(MAX_PONDS) as u32, 1, 0, 0],
            absorption: [ar, ag, ab, 0.0],
            scattering: [sr, sg, sb, 0.0],
            wind: [[0.0; 4]; 2],
            ponds: [[[0.0; 4]; 2]; MAX_PONDS],
        };
        for (slot, pond) in uniform.ponds.iter_mut().zip(ponds) {
            *slot = [
                [pond.min[0], pond.min[1], pond.max[0], pond.max[1]],
                [pond.level, 0.0, 0.0, 0.0],
            ];
        }
        gpu.queue
            .write_buffer(&self.uniform, 0, bytemuck::bytes_of(&uniform));
    }

    /// Sets how many rays each pixel of water reflects along: more for a smoother reflection
    /// where the wind ruffles the water. Twin of `ponds.count.y` in `ocean/pond.wgsl`.
    pub fn set_rays(&self, gpu: &Gpu, rays: u32) {
        gpu.queue
            .write_buffer(&self.uniform, 4, bytemuck::bytes_of(&rays.max(1)));
    }

    /// Sets the wind over the ponds this frame.
    pub fn set_wind(&self, gpu: &Gpu, wind: &crate::wind::WindSnapshot) {
        gpu.queue.write_buffer(
            &self.uniform,
            WIND_OFFSET,
            bytemuck::cast_slice(&wind.packed()),
        );
    }

    /// Shades the ponds over `scene` into `targets`.
    pub fn record(
        &mut self,
        gpu: &Gpu,
        encoder: &mut wgpu::CommandEncoder,
        environment: &wgpu::BindGroup,
        targets: &WaterTargets,
        scene: SceneViews,
        shadows: &ShadowMaps,
    ) {
        if self.group.as_ref().map(|(made_for, _)| *made_for) != Some(targets.generation) {
            let tex = |view| wgpu::BindingResource::TextureView(view);
            let group = gpu.bind_group(
                "pond water",
                &self.layout,
                &[
                    tex(scene.lit),
                    tex(scene.depth),
                    tex(scene.motion),
                    self.uniform.as_entire_binding(),
                    tex(&targets.color.view),
                    tex(&targets.scene_depth.view),
                    tex(&targets.motion.view),
                    shadows.uniform.as_entire_binding(),
                    tex(&shadows.array_view),
                    wgpu::BindingResource::Sampler(&shadows.comparison_sampler),
                    tex(scene.depth_pyramid),
                    tex(scene.normals),
                ],
            );
            self.group = Some((targets.generation, group));
        }
        let (_, group) = self.group.as_ref().expect("made above");
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("pond water"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, group, &[]);
        pass.set_bind_group(1, environment, &[]);
        pass.dispatch_workgroups(
            groups_for(targets.size.0, 8),
            groups_for(targets.size.1, 8),
            1,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pond_reads_from_its_corners_in_either_order() {
        let pond = Pond::parse("59.4, 5.5, 24.1, -5.5, 1.9").unwrap();
        assert_eq!(pond.min, [24.1, -5.5]);
        assert_eq!(pond.max, [59.4, 5.5]);
        assert_eq!(pond.level, 1.9);
    }

    #[test]
    fn ponds_read_as_a_list() {
        let ponds = Pond::parse_list("0,0,1,1,2; 3,3,4,4,5;").unwrap();
        assert_eq!(ponds.len(), 2);
        assert_eq!(ponds[1].level, 5.0);
        assert!(Pond::parse_list("").unwrap().is_empty());
    }

    #[test]
    fn a_pond_written_wrongly_says_how() {
        assert!(Pond::parse("1,2,3,4").unwrap_err().contains("4 numbers"));
        assert!(Pond::parse("1,2,3,4,deep")
            .unwrap_err()
            .contains("not a number"));
        assert!(Pond::parse("1,2,3,4,inf").is_err());
    }
}
