//! A scene's transparent glowing cards, drawn over the lit scene.
//!
//! There are few of them, so they are kept out of the cluster pipeline: one small vertex buffer,
//! and a draw for each card, farthest first, so that they cover one another properly.

use crate::gpu::{Gpu, LayoutBuilder};
use crate::shaders;
use bytemuck::{Pod, Zeroable};
use glam::Vec3;
use solstice_asset::{CardImage, PagedScene};

const COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const IMAGE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Bc7RgbaUnormSrgb;

/// Mirrored by `Vertex` in `cards/draw.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuVertex {
    position: [f32; 3],
    opacity: f32,
    emissive: [f32; 3],
    phase: f32,
    uv: [f32; 2],
}

struct Item {
    first_vertex: u32,
    vertex_count: u32,
    image: u32,
    center: Vec3,
}

pub struct Cards {
    pipeline: wgpu::RenderPipeline,
    frame_layout: wgpu::BindGroupLayout,
    images: Vec<wgpu::BindGroup>,
    vertices: wgpu::Buffer,
    items: Vec<Item>,
}

/// Which way round to draw: the farthest from `camera` first.
fn back_to_front(centers: impl Iterator<Item = Vec3>, camera: Vec3) -> Vec<usize> {
    let distances: Vec<f32> = centers.map(|c| c.distance_squared(camera)).collect();
    let mut order: Vec<usize> = (0..distances.len()).collect();
    order.sort_by(|&a, &b| distances[b].total_cmp(&distances[a]));
    order
}

/// What a scene's cards are drawn from: their vertices with what is the same for each card, in
/// one buffer of their own, and where each card's are in it.
fn gather(source: &solstice_asset::Cards) -> (Vec<GpuVertex>, Vec<Item>) {
    let mut vertices = Vec::with_capacity(source.vertices.len());
    let mut items = Vec::with_capacity(source.cards.len());
    for card in &source.cards {
        let range = card.first_vertex as usize..(card.first_vertex + card.vertex_count) as usize;
        let positions = source.vertices[range.clone()]
            .iter()
            .map(|v| Vec3::from(v.position));
        let center = positions.sum::<Vec3>() / card.vertex_count.max(1) as f32;
        let phase = phase_of(center);
        let first_vertex = vertices.len() as u32;
        vertices.extend(source.vertices[range].iter().map(|v| GpuVertex {
            position: v.position,
            opacity: card.opacity,
            emissive: card.emissive,
            phase,
            uv: v.uv,
        }));
        items.push(Item {
            first_vertex,
            vertex_count: card.vertex_count,
            image: card.image,
            center,
        });
    }
    (vertices, items)
}

/// A number from a card's place in the world, so that neighbours don't flicker together.
fn phase_of(center: Vec3) -> f32 {
    let wave = (center.x * 12.9898 + center.z * 78.233).sin() * 43_758.547;
    wave.fract().abs() * std::f32::consts::TAU
}

fn upload(gpu: &Gpu, label: &str, image: &CardImage, mips: &[Vec<u8>]) -> wgpu::TextureView {
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: image.width,
            height: image.height,
            depth_or_array_layers: 1,
        },
        mip_level_count: mips.len() as u32,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: IMAGE_FORMAT,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    for (level, data) in mips.iter().enumerate() {
        let blocks_wide = (image.width >> level).max(1).div_ceil(4);
        let blocks_high = (image.height >> level).max(1).div_ceil(4);
        gpu.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: level as u32,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(blocks_wide * 16),
                rows_per_image: Some(blocks_high),
            },
            wgpu::Extent3d {
                width: blocks_wide * 4,
                height: blocks_high * 4,
                depth_or_array_layers: 1,
            },
        );
    }
    texture.create_view(&Default::default())
}

impl Cards {
    /// `None` for a scene without cards, or on a GPU that can't read their images.
    pub fn new(gpu: &Gpu, scene: &PagedScene) -> Option<Self> {
        let block = gpu
            .device
            .features()
            .contains(wgpu::Features::TEXTURE_COMPRESSION_BC);
        if scene.cards.cards.is_empty() {
            return None;
        }
        if !block {
            log::warn!("this GPU can't read block-compressed cards, so they aren't drawn");
            return None;
        }
        let source = &scene.cards;
        let (vertices, items) = gather(source);

        let stages = wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT;
        let frame_layout = LayoutBuilder::new()
            .visibility(stages)
            .uniform()
            .texture_unfilterable()
            .build(gpu, "cards frame");
        let image_layout = LayoutBuilder::new()
            .visibility(stages)
            .sampler()
            .texture()
            .texture()
            .build(gpu, "cards image");
        let sampler = gpu.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("cards"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            // A card's picture can be far from the card's shape: Zorah's flames are an image
            // eight times taller than it is wide, on a card wider than it is tall.
            anisotropy_clamp: 16,
            ..Default::default()
        });
        let images = source
            .images
            .iter()
            .map(|image| {
                let color = upload(gpu, "card color", image, &image.color_mips);
                let glow = upload(gpu, "card glow", image, &image.glow_mips);
                gpu.bind_group(
                    "cards image",
                    &image_layout,
                    &[
                        wgpu::BindingResource::Sampler(&sampler),
                        wgpu::BindingResource::TextureView(&color),
                        wgpu::BindingResource::TextureView(&glow),
                    ],
                )
            })
            .collect();

        let module = shaders::module(&gpu.device, "cards/draw.wgsl");
        let layout = gpu
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("cards"),
                bind_group_layouts: &[Some(&frame_layout), Some(&image_layout)],
                immediate_size: 0,
            });
        let premultiplied = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
            operation: wgpu::BlendOperation::Add,
        };
        let attributes = wgpu::vertex_attr_array![
            0 => Float32x3, 1 => Float32, 2 => Float32x3, 3 => Float32, 4 => Float32x2
        ];
        let pipeline = gpu
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("cards"),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[Some(wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<GpuVertex>() as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &attributes,
                    })],
                },
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some("fs_main"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: COLOR_FORMAT,
                        blend: Some(wgpu::BlendState {
                            color: premultiplied,
                            alpha: premultiplied,
                        }),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            });
        let vertices = {
            use wgpu::util::DeviceExt;
            gpu.device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("card vertices"),
                    contents: bytemuck::cast_slice(&vertices),
                    usage: wgpu::BufferUsages::VERTEX,
                })
        };
        Some(Self {
            pipeline,
            frame_layout,
            images,
            vertices,
            items,
        })
    }

    pub fn record(
        &self,
        gpu: &Gpu,
        encoder: &mut wgpu::CommandEncoder,
        color: &wgpu::TextureView,
        scene_depth: &wgpu::TextureView,
        frame: &wgpu::Buffer,
        camera: Vec3,
    ) {
        let group = gpu.bind_group(
            "cards frame",
            &self.frame_layout,
            &[
                frame.as_entire_binding(),
                wgpu::BindingResource::TextureView(scene_depth),
            ],
        );
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("cards"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: color,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.set_vertex_buffer(0, self.vertices.slice(..));
        for index in back_to_front(self.items.iter().map(|item| item.center), camera) {
            let item = &self.items[index];
            pass.set_bind_group(1, &self.images[item.image as usize], &[]);
            pass.draw(
                item.first_vertex..item.first_vertex + item.vertex_count,
                0..1,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_farthest_card_is_drawn_first() {
        let centers = [Vec3::X, Vec3::X * 9.0, Vec3::X * 4.0];
        assert_eq!(
            back_to_front(centers.into_iter(), Vec3::ZERO),
            [1, 2, 0],
            "seen from the origin"
        );
        assert_eq!(
            back_to_front(centers.into_iter(), Vec3::X * 10.0),
            [0, 2, 1],
            "seen from the other end"
        );
    }

    /// A file may keep its cards' vertices in any order, or leave some out.
    #[test]
    fn each_card_is_drawn_from_where_its_vertices_went() {
        use solstice_asset::{Card, CardVertex, Cards};
        let at = |x: f32| CardVertex {
            position: [x, 0.0, 0.0],
            uv: [0.0; 2],
        };
        let card = |first_vertex: u32| Card {
            first_vertex,
            vertex_count: 3,
            ..Default::default()
        };
        let source = Cards {
            vertices: (0..9).map(|i| at(i as f32)).collect(),
            cards: vec![card(6), card(3)],
            images: Vec::new(),
        };
        let (vertices, items) = gather(&source);
        assert_eq!(vertices.len(), 6);
        assert_eq!(items[0].first_vertex, 0);
        assert_eq!(items[1].first_vertex, 3);
        assert_eq!(vertices[0].position[0], 6.0);
        assert_eq!(vertices[3].position[0], 3.0);
        assert_eq!(items[0].center.x, 7.0);
    }

    #[test]
    fn neighbouring_cards_flicker_apart() {
        let (a, b) = (phase_of(Vec3::ZERO), phase_of(Vec3::new(1.3, 0.0, 0.7)));
        assert_ne!(a, b);
        for phase in [a, b] {
            assert!((0.0..=std::f32::consts::TAU).contains(&phase));
        }
    }
}
