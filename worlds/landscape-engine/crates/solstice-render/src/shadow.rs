//! Cascaded sun shadow maps, drawn through the same cluster culling as the main view.
//!
//! Shadow maps are the most expensive pass on small GPUs, and most of a scene stands still, so
//! a cascade holds only static instances and is only redrawn when its view changes. Views move
//! in coarse steps to make that rare: each cascade covers slightly more than it must, so it can
//! stay put while the camera moves. When several cascades are stale, the far ones take turns.
//!
//! Movable instances are drawn every frame into a map of their own, through the same view as
//! the cascade they fall in, and shading multiplies the light the two let through. The four
//! cascades share
//! one more layer of the array for this, a quarter each, at half their resolution: a moving
//! thing's shadow is a little softer, but it costs one layer rather than four, and no new
//! binding. Shaders that sample the shadows are already at WebGPU's limit of 16 textures. A
//! cascade with nothing movable in it draws nothing, and is not sampled twice.

use crate::frame::Camera;
use crate::gpu::{Gpu, LayoutBuilder};
use crate::scene::GpuScene;
use crate::vg::{
    CullPhase, CullPipelines, CullWork, DepthBuffer, InstanceFilter, PassView, RasterBindings,
    Rasterizer, ViewSlots, DEPTH_STAMPS,
};
use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec2, Vec3, Vec4};

pub const CASCADE_COUNT: usize = 4;
const RESOLUTION: u32 = 2048;
/// The stamps for `count` cascades drawn this frame through the software depth buffer, after
/// `last` was the last one used: whether the buffer must be cleared before them, and the first
/// of them, which the rest follow one by one. A frame's cascades never straddle a round, since
/// the clear is recorded once before the frame's first cascade: a cascade stamped 1 after one
/// stamped `DEPTH_STAMPS` in the same frame would lose every pixel the other had written.
fn next_stamps(last: u32, count: u32) -> (bool, u32) {
    if last + count > DEPTH_STAMPS {
        (true, 1)
    } else {
        (false, last + 1)
    }
}

/// Each cascade's quarter of the layer for movable instances, the last layer of the array.
const MOVING_RESOLUTION: u32 = RESOLUTION / 2;
const MOVING_LAYER: u32 = CASCADE_COUNT as u32;
const FORMAT: wgpu::TextureFormat = crate::vg::DEPTH_FORMAT;
/// Far end of each cascade, in metres from the camera.
const SPLITS: [f32; CASCADE_COUNT] = [12.0, 38.0, 120.0, 420.0];
/// How far casters may lie towards the sun beyond a cascade's bounds.
const DEPTH_REACH: f32 = 900.0;
/// Cascades move in steps of this many texels. A whole number keeps shadow edges from
/// crawling as they move, and a large one means they rarely need to move at all.
const SNAP_TEXELS: f32 = 64.0;
/// Extra radius that covers the furthest a slice can be from its snapped centre, which is
/// half a snap step on each of three axes. The bounds test in this module checks that it is enough.
const SNAP_PADDING: f32 = 1.07;
/// Where along a cascade receivers begin to dither into the next one, as a fraction of its
/// span. Twin of the blend in `lighting/sun_shadow.wgsl`.
const BLEND_START: f32 = 0.85;
/// How far the camera may turn, on each side, before a cascade's receiver planes are redrawn:
/// a little more than the turn that moves its fitted sphere a snap step and refits it anyway,
/// so that turning seldom costs a redraw of its own.
const ROTATION_MARGIN: f32 = 0.1;

/// How far the shadow lookup reaches from the receiver it is for, in metres, for a cascade
/// with texels `texel` across, as `lighting/sun_shadow.wgsl` samples: an offset along the
/// normal of 1.2 + 0.8 × a slope of up to 8 texels, then a filter disc of radius 1.5 texels,
/// and the diagonal of the 2×2 bilinear comparison. The disc's radius is held to 1.8 cm at
/// least, which in the nearest cascade is more than a texel. Receivers themselves are found
/// through a jittered projection, which reaches up to half a pixel past the slice: at 720p
/// and 420 m, 0.34 m, which `JITTER_REACH` covers at any distance the cascades span.
fn lookup_reach(texel: f32) -> f32 {
    texel * (7.6 + 1.5 * (0.018 / texel).max(1.0) + std::f32::consts::SQRT_2) + JITTER_REACH
}
/// Half a pixel at the far end of the last cascade, at 720p, in metres.
const JITTER_REACH: f32 = 0.35;

/// How far the light may move, in radians, before each cascade is redrawn for it. A shadow
/// moves by its caster's height times this, which for casters tens of metres off is about a
/// texel of each cascade, and the far cascades' texels are large: they keep the light they were
/// drawn with through several of its steps, where redrawing all four at every step is most of
/// what moving light costs. The nearest follows every step.
const LIGHT_LAG: [f32; CASCADE_COUNT] = [0.0, 1.3e-3, 3.0e-3, 6.5e-3];

/// What a cascade was drawn with, to tell when it has gone stale.
#[derive(Clone, Copy, PartialEq)]
struct Drawn {
    view_proj: Mat4,
    /// Towards the light it was drawn for.
    light: Vec3,
    lod_threshold: f32,
    eye_lod_scale: f32,
    /// The receiver planes its casters were culled with, `receiver_count` of them in use.
    receivers: [Vec4; 6],
    receiver_count: usize,
    /// How far receivers may lie outside those planes and still find their casters.
    slack: f32,
}

impl Drawn {
    /// Whether the receivers in the frustum slice with these `corners` all have their casters
    /// in this cascade.
    fn covers(&self, corners: &[Vec3; 8]) -> bool {
        self.receivers[..self.receiver_count].iter().all(|plane| {
            corners
                .iter()
                .all(|&corner| plane.truncate().dot(corner) + plane.w - self.slack >= 0.0)
        })
    }

    /// Whether a sphere could shadow this cascade's receivers: whether it lies in the volume
    /// the cascade's casters were culled to, the slice swept towards the light.
    fn can_be_shadowed_by(&self, (center, radius): (Vec3, f32)) -> bool {
        self.receivers[..self.receiver_count]
            .iter()
            .all(|plane| plane.truncate().dot(center) + plane.w >= -radius)
    }
}

/// Twin of `Shadows` in `lighting/deferred.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ShadowUniform {
    view_proj: [Mat4; CASCADE_COUNT],
    splits: Vec4,
    texel_size: Vec4,
    /// 1 for each cascade whose movable instances were drawn this frame, else 0.
    moving: Vec4,
}

/// What [`ShadowMaps::record`] draws this frame.
pub struct ShadowWork {
    /// Cascades whose static contents are redrawn.
    pub due: [bool; CASCADE_COUNT],
    /// Whether the software depth buffer must be cleared before this frame's cascades, as the
    /// stamps have started a new round.
    pub clear_software_depth: bool,
    /// Cascades with movable instances to draw.
    pub moving: [bool; CASCADE_COUNT],
    /// Whether the movable instances are culled once, through the slot after the cascades'
    /// own, and that one list drawn into each touched cascade's quarter; otherwise each
    /// cascade culls them through its own view.
    pub shared_cull: bool,
}

/// Up to this many movable instances, one cull serves every touched cascade: a traversal's
/// fixed cost outweighs drawing a few things into quarters they fall outside of. Beyond it,
/// drawing the whole list into every quarter costs more than the traversals saved. Measured
/// with bobbing instances of the stress scene: 2,031 were cheaper shared (0.45 against 0.68
/// ms), 3,615 the same either way (0.55), 13,431 far dearer shared (19 against 3).
const SHARED_CULL_LIMIT: usize = 2048;

/// Slots a [`ShadowMaps`] writes from `first_moving_slot`: one per cascade for drawing movable
/// instances into its quarter, then one for culling them all at once.
pub const MOVING_SLOTS: u32 = CASCADE_COUNT as u32 + 1;

/// Shadow map resources and passes.
pub struct ShadowMaps {
    /// Draws cut-out clusters into a map, in a scene that has any.
    masked_pipeline: Option<wgpu::RenderPipeline>,
    pub array_view: wgpu::TextureView,
    pub comparison_sampler: wgpu::Sampler,
    pub uniform: wgpu::Buffer,
    layer_views: Vec<wgpu::TextureView>,
    /// The layer movable instances are drawn into.
    moving_view: wgpu::TextureView,
    pipeline: wgpu::RenderPipeline,
    /// Where the software rasterizer draws a cascade's small clusters before they are laid
    /// over its map. Most of a wood's clusters are pixel-sized, and the hardware rasterizer
    /// draws those slowly, as the main view found long ago.
    software_depth: DepthBuffer,
    /// The stamp of the last cascade drawn through it, 1 to `DEPTH_STAMPS`; 0 before any.
    /// Before the stamps start a new round the buffer is cleared, or the words the last round
    /// left, with their higher stamps, would win the atomic max against the new round's. See
    /// [`next_stamps`].
    software_stamp: u32,
    data: ShadowUniform,
    /// What each cascade last drew, to tell when it has gone stale.
    drawn: [Option<Drawn>; CASCADE_COUNT],
    /// The centre and radius of each cascade's view as last drawn.
    extents: [(Vec3, f32); CASCADE_COUNT],
    next_far_cascade: usize,
    /// See [`crate::RendererSettings::speck_texels`].
    pub speck_texels: f32,
    /// Cascades that must redraw because the geometry on the GPU has improved.
    outdated: [bool; CASCADE_COUNT],
    /// Redraw every stale cascade next frame, rather than the far ones taking turns.
    redraw_all: bool,
}

impl ShadowMaps {
    pub fn new(gpu: &Gpu, rasterizer: &Rasterizer) -> Self {
        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("shadow maps"),
            size: wgpu::Extent3d {
                width: RESOLUTION,
                height: RESOLUTION,
                depth_or_array_layers: CASCADE_COUNT as u32 + 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let layer_view = |layer| {
            texture.create_view(&wgpu::TextureViewDescriptor {
                dimension: Some(wgpu::TextureViewDimension::D2),
                base_array_layer: layer,
                array_layer_count: Some(1),
                ..Default::default()
            })
        };
        let layer_views = (0..CASCADE_COUNT as u32).map(layer_view).collect();
        let moving_view = layer_view(MOVING_LAYER);
        let array_view = texture.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });

        let layout = gpu
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("shadow"),
                bind_group_layouts: &[Some(&rasterizer.raster_layout)],
                immediate_size: 0,
            });
        let pipeline = gpu
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("shadow"),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &rasterizer.hardware_module,
                    entry_point: Some("vs_depth"),
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
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: FORMAT,
                    depth_write_enabled: Some(true),
                    depth_compare: Some(wgpu::CompareFunction::Less),
                    stencil: Default::default(),
                    bias: wgpu::DepthBiasState {
                        constant: 0,
                        slope_scale: 1.5,
                        clamp: 0.0,
                    },
                }),
                multisample: Default::default(),
                fragment: None,
                multiview_mask: None,
                cache: None,
            });

        let masked_pipeline = rasterizer
            .masked_depth_parts()
            .map(|(module, mask_layout)| {
                let layout = gpu
                    .device
                    .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                        label: Some("masked shadow"),
                        bind_group_layouts: &[
                            Some(&rasterizer.raster_layout),
                            None,
                            Some(mask_layout),
                        ],
                        immediate_size: 0,
                    });
                gpu.device
                    .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                        label: Some("masked shadow"),
                        layout: Some(&layout),
                        vertex: wgpu::VertexState {
                            module,
                            entry_point: Some("vs_masked"),
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
                        depth_stencil: Some(wgpu::DepthStencilState {
                            format: FORMAT,
                            depth_write_enabled: Some(true),
                            depth_compare: Some(wgpu::CompareFunction::Less),
                            stencil: Default::default(),
                            bias: wgpu::DepthBiasState {
                                constant: 0,
                                slope_scale: 1.5,
                                clamp: 0.0,
                            },
                        }),
                        multisample: Default::default(),
                        fragment: Some(wgpu::FragmentState {
                            module,
                            entry_point: Some("fs_masked_depth"),
                            compilation_options: Default::default(),
                            targets: &[],
                        }),
                        multiview_mask: None,
                        cache: None,
                    })
            });

        Self {
            masked_pipeline,
            array_view,
            comparison_sampler: gpu.device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("shadow comparison"),
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                compare: Some(wgpu::CompareFunction::LessEqual),
                ..Default::default()
            }),
            uniform: gpu.uniform_buffer("shadow", std::mem::size_of::<ShadowUniform>() as u64),
            layer_views,
            moving_view,
            pipeline,
            software_depth: rasterizer.depth_buffer(gpu, (RESOLUTION, RESOLUTION)),
            software_stamp: 0,
            data: ShadowUniform {
                view_proj: [Mat4::IDENTITY; CASCADE_COUNT],
                splits: Vec4::from_array(SPLITS),
                texel_size: Vec4::ONE,
                moving: Vec4::ZERO,
            },
            drawn: [None; CASCADE_COUNT],
            extents: [(Vec3::ZERO, 1.0); CASCADE_COUNT],
            next_far_cascade: 1,
            speck_texels: 0.0,
            outdated: [false; CASCADE_COUNT],
            redraw_all: false,
        }
    }

    /// Makes every cascade redraw in turn, as finer geometry has arrived.
    pub fn mark_stale(&mut self) {
        self.outdated = [true; CASCADE_COUNT];
    }

    /// Makes every cascade redraw next frame, as instances in them have changed: shown or
    /// hidden, or made movable or static. Taking turns would leave the far cascades showing the
    /// change late, as a shadow left behind or a missing one.
    pub fn mark_stale_now(&mut self) {
        self.mark_stale();
        self.redraw_all = true;
    }

    /// Finds the stale cascades and writes the views of those to redraw to slots from
    /// `first_slot`. Then finds the cascades that `moving`, the bounding spheres of movable
    /// instances, fall in, and writes their views to slots from `first_moving_slot`.
    #[allow(clippy::too_many_arguments)]
    pub fn prepare(
        &mut self,
        gpu: &Gpu,
        slots: &ViewSlots,
        first_slot: u32,
        first_moving_slot: u32,
        camera: &Camera,
        aspect: f32,
        sun_direction: Vec3,
        lod_threshold: f32,
        eye_lod_scale: f32,
        moving: &[(Vec3, f32)],
    ) -> ShadowWork {
        let fit = |cascade: usize, light: Vec3| {
            let near = if cascade == 0 {
                camera.near
            } else {
                SPLITS[cascade - 1]
            };
            fit_cascade(camera, aspect, light, near, SPLITS[cascade])
        };
        // Where each cascade's receivers begin: where the previous cascade's start to dither
        // into it, since the lookup blends across the last of each cascade.
        let nearest: [f32; CASCADE_COUNT] = std::array::from_fn(|cascade| match cascade {
            0 => camera.near,
            1 => SPLITS[0] * BLEND_START,
            _ => SPLITS[cascade - 2] + (SPLITS[cascade - 1] - SPLITS[cascade - 2]) * BLEND_START,
        });
        // A cascade is still good while it is fitted as it would be now, for the light it was
        // drawn with, and that light is near enough to where the light is now.
        let fitted: [bool; CASCADE_COUNT] = std::array::from_fn(|cascade| {
            self.drawn[cascade].is_some_and(|drawn| {
                drawn.light.angle_between(sun_direction) <= LIGHT_LAG[cascade]
                    && drawn.view_proj == fit(cascade, drawn.light).0
                    && drawn.lod_threshold == lod_threshold
                    && drawn.eye_lod_scale == eye_lod_scale
                    && drawn.covers(&slice_corners(
                        camera,
                        aspect,
                        nearest[cascade],
                        SPLITS[cascade],
                    ))
            })
        });
        let stale: [bool; CASCADE_COUNT] =
            std::array::from_fn(|cascade| self.outdated[cascade] || !fitted[cascade]);
        // What is redrawn is redrawn for the light as it is now.
        let fits: [(Mat4, Vec3, f32); CASCADE_COUNT] =
            std::array::from_fn(|cascade| fit(cascade, sun_direction));
        let never_drawn: [bool; CASCADE_COUNT] =
            std::array::from_fn(|cascade| self.drawn[cascade].is_none());
        let mut due = choose_due(stale, never_drawn, &mut self.next_far_cascade);
        if std::mem::take(&mut self.redraw_all) {
            due = stale;
        }
        // Every cascade drawn this frame takes a stamp of its own, and all of them come from
        // one round: the clear is recorded once, before the frame's first cascade.
        let (clear_software_depth, first_stamp) = next_stamps(
            self.software_stamp,
            due.iter().filter(|&&due| due).count() as u32,
        );
        self.software_stamp = first_stamp;

        for cascade in (0..CASCADE_COUNT).filter(|&c| due[c]) {
            self.outdated[cascade] = false;
            let (view_proj, center, radius) = fits[cascade];
            let texel = 2.0 * radius / RESOLUTION as f32;
            self.extents[cascade] = (center, radius);
            self.data.view_proj[cascade] = view_proj;
            self.data.texel_size[cascade] = texel;
            // Only casters whose shadow can fall on this cascade's slice of the view are drawn:
            // the sphere the cascade is fitted to covers far more than the slice, especially
            // beside and behind the camera. The slice is widened by the turn and the step the
            // camera can make before the cascade is refitted, so that the cached map serves
            // every pose it is kept for, and by the reach of the shadow lookup.
            let slack = lookup_reach(texel);
            let planes = receiver_planes(
                camera,
                aspect,
                nearest[cascade],
                SPLITS[cascade],
                texel * SNAP_TEXELS + slack,
            );
            let mut view = PassView::orthographic(
                view_proj,
                center,
                Vec2::splat(RESOLUTION as f32),
                RESOLUTION as f32 / (2.0 * radius),
                lod_threshold,
            )
            // Detail the camera could not see from where the shadows land is left out.
            .seen_from(
                camera.position,
                sun_direction,
                eye_lod_scale,
                nearest[cascade],
            )
            .shadowing(&planes, sun_direction)
            .with_specks(self.speck_texels);
            // Small clusters go to the software rasterizer, as in the main view.
            view.allow_software = 1;
            view = view.cutting_out();
            view.stamp = self.software_stamp;
            self.software_stamp += 1;
            self.drawn[cascade] = Some(Drawn {
                view_proj,
                light: sun_direction,
                lod_threshold,
                eye_lod_scale,
                receivers: view.receiver_planes,
                receiver_count: view.receiver_plane_count as usize,
                slack,
            });
            slots.write(gpu, first_slot + cascade as u32, &view);
        }

        // Movable instances go through each cascade's view as it stands, drawn this frame or
        // before, so that the two maps line up. A cascade is touched when one of them lies in
        // its view and could shadow its slice. A cascade waiting its turn to be fitted again has
        // receiver planes that may no longer hold the slice, so until it is redrawn its view
        // alone decides. One waiting only for finer geometry is fitted as it was: were it
        // taken for touched, every page's coming would have the movable drawn into cascades
        // they cast nothing on, at those cascades' detail, and their pages asked for.
        let moving_due: [bool; CASCADE_COUNT] = std::array::from_fn(|cascade| {
            let (_, radius) = self.extents[cascade];
            let view_proj = self.data.view_proj[cascade];
            let receivers_current = due[cascade] || fitted[cascade];
            self.drawn[cascade].is_some_and(|drawn| {
                moving.iter().any(|&sphere| {
                    cascade_touches(view_proj, radius, sphere)
                        && (!receivers_current || drawn.can_be_shadowed_by(sphere))
                })
            })
        });
        // Movable things may be anywhere, so the camera's own distance to each bounds the
        // detail that is worth drawing of it, as for the cascades' static casters.
        let seen = |view: PassView| {
            view.seen_from(camera.position, sun_direction, eye_lod_scale, camera.near)
        };
        for cascade in (0..CASCADE_COUNT).filter(|&c| moving_due[c]) {
            let (center, radius) = self.extents[cascade];
            let view = seen(PassView::orthographic(
                self.data.view_proj[cascade],
                center,
                Vec2::splat(MOVING_RESOLUTION as f32),
                MOVING_RESOLUTION as f32 / (2.0 * radius),
                lod_threshold,
            ))
            .drawing(InstanceFilter::Movable)
            .cutting_out();
            slots.write(gpu, first_moving_slot + cascade as u32, &view);
        }
        // One cull serves every touched cascade, since the traversal's fixed cost outweighs the
        // few movable instances: through the union of their views, at the finest touched
        // cascade's level of detail, which satisfies the rest.
        let shared_cull = moving.len() <= SHARED_CULL_LIMIT;
        let touched: Vec<(Vec3, f32)> = (0..CASCADE_COUNT)
            .filter(|&c| moving_due[c])
            .map(|c| self.extents[c])
            .collect();
        if shared_cull && !touched.is_empty() {
            let finest = touched.iter().map(|e| e.1).fold(f32::MAX, f32::min);
            let view = seen(moving_cull_view(
                &touched,
                sun_direction,
                finest,
                lod_threshold,
            ));
            slots.write(gpu, first_moving_slot + CASCADE_COUNT as u32, &view);
        }
        // Back to the last stamp used.
        self.software_stamp -= 1;
        self.data.moving = Vec4::from_array(moving_due.map(|m| f32::from(u8::from(m))));

        gpu.queue
            .write_buffer(&self.uniform, 0, bytemuck::bytes_of(&self.data));
        ShadowWork {
            due,
            clear_software_depth,
            moving: moving_due,
            shared_cull,
        }
    }

    /// Culls and draws what `shadow_work` says is due: stale cascades, then movable instances.
    #[allow(clippy::too_many_arguments)]
    pub fn record(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        cull: &CullPipelines,
        rasterizer: &Rasterizer,
        scene: &GpuScene,
        cull_scene: &wgpu::BindGroup,
        work: &CullWork,
        raster: &RasterBindings,
        first_slot: u32,
        first_moving_slot: u32,
        shadow_work: &ShadowWork,
    ) {
        if shadow_work.clear_software_depth {
            encoder.clear_buffer(&self.software_depth.buffer, 0, None);
        }
        for cascade in (0..CASCADE_COUNT).filter(|&c| shadow_work.due[c]) {
            let slot = first_slot + cascade as u32;
            cull.record(encoder, scene, cull_scene, slot, work, CullPhase::First);
            rasterizer.record_software_depth(encoder, raster, slot, work, &self.software_depth);

            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("shadow cascade"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.layer_views[cascade],
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &raster.group, &[ViewSlots::offset(slot)]);
            pass.draw_indirect(work.args(), CullWork::draw_args_offset());
            self.draw_masked(&mut pass, raster, work);
            rasterizer.merge_depth(&mut pass, raster, slot, work, &self.software_depth);
        }

        if shadow_work.shared_cull && shadow_work.moving.iter().any(|&moving| moving) {
            let slot = first_moving_slot + CASCADE_COUNT as u32;
            cull.record(encoder, scene, cull_scene, slot, work, CullPhase::First);
        }
        let mut cleared = false;
        for cascade in (0..CASCADE_COUNT).filter(|&c| shadow_work.moving[c]) {
            let slot = first_moving_slot + cascade as u32;
            if !shadow_work.shared_cull {
                cull.record(encoder, scene, cull_scene, slot, work, CullPhase::First);
            }

            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("moving shadows"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.moving_view,
                    depth_ops: Some(wgpu::Operations {
                        // The whole layer is cleared once a frame, before its first quarter.
                        load: if cleared {
                            wgpu::LoadOp::Load
                        } else {
                            wgpu::LoadOp::Clear(1.0)
                        },
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            cleared = true;
            let [x, y] = moving_quarter(cascade);
            let size = MOVING_RESOLUTION as f32;
            pass.set_viewport(x as f32 * size, y as f32 * size, size, size, 0.0, 1.0);
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &raster.group, &[ViewSlots::offset(slot)]);
            pass.draw_indirect(work.args(), CullWork::draw_args_offset());
            self.draw_masked(&mut pass, raster, work);
        }
    }

    /// Adds the cut-out clusters the most recent cull sent to hardware to `pass`, which has
    /// the view's bind group set already.
    fn draw_masked(&self, pass: &mut wgpu::RenderPass, raster: &RasterBindings, work: &CullWork) {
        if let (Some(pipeline), Some(mask)) = (&self.masked_pipeline, &raster.mask) {
            pass.set_pipeline(pipeline);
            pass.set_bind_group(2, mask, &[]);
            pass.draw_indirect(work.args(), CullWork::masked_draw_args_offset());
        }
    }

    /// Layout entries for sampling the maps: uniform, depth array, comparison sampler.
    pub fn add_to_layout(builder: LayoutBuilder) -> LayoutBuilder {
        builder.uniform().depth_texture_array().sampler_comparison()
    }
}

/// Which quarter of the movable layer a cascade has, as column and row. Twin of
/// `moving_quarter` in `lighting/sun_shadow.wgsl`.
fn moving_quarter(cascade: usize) -> [u32; 2] {
    [cascade as u32 % 2, cascade as u32 / 2]
}

/// The view that culls movable instances for every touched cascade at once: the box holding
/// every touched cascade's box, which all share the light's orientation (`extents` gives each
/// one's centre and radius, as [`fit_cascade`] does), with the level of detail of the finest
/// touched cascade, `finest_radius` across.
fn moving_cull_view(
    extents: &[(Vec3, f32)],
    sun: Vec3,
    finest_radius: f32,
    lod_threshold: f32,
) -> PassView {
    let up = light_up(sun);
    let light_view = Mat4::look_to_rh(Vec3::ZERO, -sun, up);
    // Each cascade's box, in a view looking along the light from the origin, where +z is
    // towards the light: its square about its centre, from `DEPTH_REACH` towards the light to
    // twice its radius beyond its centre, as `fit_cascade` projects it.
    let (mut low, mut high) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
    for &(center, radius) in extents {
        let c = light_view.transform_point3(center);
        low = low.min(c - Vec3::new(radius, radius, 2.0 * radius));
        high = high.max(c + Vec3::new(radius, radius, DEPTH_REACH));
    }
    let middle = (low + high) * 0.5;
    let half = (high - low) * 0.5;
    let from_light = light_view.inverse();
    let eye = from_light.transform_point3(Vec3::new(middle.x, middle.y, high.z));
    let view = Mat4::look_to_rh(eye, -sun, up);
    let projection = Mat4::orthographic_rh(-half.x, half.x, -half.y, half.y, 0.0, high.z - low.z);
    PassView::orthographic(
        projection * view,
        from_light.transform_point3(middle),
        Vec2::splat(MOVING_RESOLUTION as f32),
        MOVING_RESOLUTION as f32 / (2.0 * finest_radius),
        lod_threshold,
    )
    .drawing(InstanceFilter::Movable)
    .cutting_out()
}

/// The up vector of a view along the light: anything but the light's own direction.
fn light_up(sun: Vec3) -> Vec3 {
    if sun.y.abs() > 0.99 {
        Vec3::Z
    } else {
        Vec3::Y
    }
}

/// Whether a sphere could cast a shadow into a cascade drawn through `view_proj`, an
/// orthographic view `radius` across each way from its centre, with depth reaching back
/// [`DEPTH_REACH`] towards the sun.
fn cascade_touches(view_proj: Mat4, radius: f32, (center, sphere_radius): (Vec3, f32)) -> bool {
    let clip = view_proj.project_point3(center);
    let across = sphere_radius / radius;
    let deep = sphere_radius / (DEPTH_REACH + radius * 2.0);
    clip.x.abs() <= 1.0 + across
        && clip.y.abs() <= 1.0 + across
        && clip.z >= -deep
        && clip.z <= 1.0 + deep
}

/// Picks what to redraw. Anything never drawn goes now, since it has no usable contents.
/// Otherwise the nearest cascade goes whenever stale, as errors there are the most visible,
/// and at most one stale far cascade joins it, which bounds the cost of a frame.
fn choose_due(
    stale: [bool; CASCADE_COUNT],
    never_drawn: [bool; CASCADE_COUNT],
    next_far: &mut usize,
) -> [bool; CASCADE_COUNT] {
    let mut due = never_drawn;
    due[0] |= stale[0];
    let far_count = CASCADE_COUNT - 1;
    for offset in 0..far_count {
        let cascade = (*next_far - 1 + offset) % far_count + 1;
        if stale[cascade] {
            due[cascade] = true;
            *next_far = cascade % far_count + 1;
            break;
        }
    }
    due
}

/// The camera with its field of view widened by `margin` radians on every side.
fn widened(camera: &Camera, margin: f32) -> Camera {
    Camera {
        fov_y: camera.fov_y + 2.0 * margin,
        ..*camera
    }
}

/// The aspect ratio that widens the horizontal field of view by `margin` on each side too.
fn widened_aspect(camera: &Camera, aspect: f32, margin: f32) -> f32 {
    let half_width = (aspect * (camera.fov_y * 0.5).tan()).atan() + margin;
    half_width.tan() / (camera.fov_y * 0.5 + margin).tan()
}

/// How far along the camera's axis the nearest point at `distance` from it lies: the frustum's
/// corners are the furthest off the axis.
fn corner_depth(camera: &Camera, aspect: f32, distance: f32) -> f32 {
    let tan_y = (camera.fov_y * 0.5).tan();
    distance / (1.0 + (1.0 + aspect * aspect) * tan_y * tan_y).sqrt()
}

/// The planes of the camera's frustum holding every point between `near` and `far` from the
/// camera, facing inwards, each moved outwards by `padding` metres. Cascades are chosen by
/// distance from the camera, not depth along its axis, so the near plane is drawn back to
/// where the nearest point at that distance, in the frustum's corner, lies in depth.
fn slice_planes(camera: &Camera, aspect: f32, near: f32, far: f32, padding: f32) -> [Vec4; 6] {
    // A finite, forward-depth projection: `frustum_planes` reads a far plane from it too.
    let projection = Mat4::perspective_rh(
        camera.fov_y,
        aspect,
        corner_depth(camera, aspect, near),
        far,
    );
    PassView::frustum_planes(projection * camera.view()).map(|plane| plane + Vec4::W * padding)
}

/// The planes of the slice of the frustum between `near` and `far`, widened to hold the slice
/// for every turn of the camera up to `ROTATION_MARGIN` on each side, and moved out by
/// `padding` metres besides. Turning widens the slice's sides and swings its flat far face
/// about the camera, carrying its corners further along the axis by their distance from the
/// axis times the turn. The near plane needs no such allowance: widening the frustum draws
/// it back to the wider frustum's corner depth, nearer than any corner the turn can bring.
fn receiver_planes(camera: &Camera, aspect: f32, near: f32, far: f32, padding: f32) -> [Vec4; 6] {
    let mut planes = slice_planes(
        &widened(camera, ROTATION_MARGIN),
        widened_aspect(camera, aspect, ROTATION_MARGIN),
        near,
        far,
        padding,
    );
    let tan_y = (camera.fov_y * 0.5).tan();
    let off_axis = tan_y * (1.0 + aspect * aspect).sqrt();
    // `frustum_planes` puts the far plane last.
    planes[5].w += far * (off_axis * ROTATION_MARGIN.sin() + 1.0 - ROTATION_MARGIN.cos());
    planes
}

/// The eight corners of that same slice of the frustum.
fn slice_corners(camera: &Camera, aspect: f32, near: f32, far: f32) -> [Vec3; 8] {
    let tan_y = (camera.fov_y * 0.5).tan();
    let forward = camera.forward();
    let right = camera.right();
    let up = right.cross(forward);
    let depths = [corner_depth(camera, aspect, near), far];
    std::array::from_fn(|i| {
        let depth = depths[i / 4];
        let sx = if i % 2 == 0 { -1.0 } else { 1.0 };
        let sy = if (i / 2) % 2 == 0 { -1.0 } else { 1.0 };
        camera.position
            + forward * depth
            + right * (sx * depth * tan_y * aspect)
            + up * (sy * depth * tan_y)
    })
}

/// Fits an orthographic view around a slice of the camera frustum.
///
/// Bounding the slice with a sphere keeps the projection's size constant as the camera
/// rotates. Snapping its centre to whole shadow texels then keeps shadow edges from
/// crawling as the camera translates.
fn fit_cascade(camera: &Camera, aspect: f32, sun: Vec3, near: f32, far: f32) -> (Mat4, Vec3, f32) {
    let tan_y = (camera.fov_y * 0.5).tan();
    let k = (1.0 + aspect * aspect) * tan_y * tan_y;
    // Centre and radius of the sphere through the slice's corners, along the view axis.
    let center_distance = ((far + near) * 0.5 * (1.0 + k)).min(far);
    let radius = (((far - center_distance).powi(2) + far * far * k)
        .max((center_distance - near).powi(2) + near * near * k))
    .sqrt()
        * SNAP_PADDING;
    let center = camera.position + camera.forward() * center_distance;

    let up = light_up(sun);
    let light_view = Mat4::look_to_rh(Vec3::ZERO, -sun, up);

    let step = 2.0 * radius / RESOLUTION as f32 * SNAP_TEXELS;
    let in_light = light_view.transform_point3(center);
    let snapped = Vec3::new(
        (in_light.x / step).round() * step,
        (in_light.y / step).round() * step,
        // Along the light too: otherwise any camera motion changes the view, and every
        // frame counts as stale even though the map's contents would be the same.
        (in_light.z / step).round() * step,
    );
    let snapped_center = light_view.inverse().transform_point3(snapped);

    let eye = snapped_center + sun * DEPTH_REACH;
    let view = Mat4::look_to_rh(eye, -sun, up);
    let projection = Mat4::orthographic_rh(
        -radius,
        radius,
        -radius,
        radius,
        0.0,
        DEPTH_REACH + radius * 2.0,
    );
    (projection * view, snapped_center, radius)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_camera() -> Camera {
        Camera {
            position: Vec3::new(3.0, 2.0, 1.0),
            yaw: 0.8,
            pitch: -0.2,
            ..Default::default()
        }
    }

    fn sun() -> Vec3 {
        Vec3::new(0.6, 0.4, 0.5).normalize()
    }

    #[test]
    fn cascade_contains_the_frustum_slice() {
        let camera = test_camera();
        let aspect = 16.0 / 9.0;
        let (view_proj, _, _) = fit_cascade(&camera, aspect, sun(), 5.0, 30.0);
        let tan_y = (camera.fov_y * 0.5).tan();
        let right = camera.right();
        let up = right.cross(camera.forward());
        for distance in [5.0_f32, 30.0] {
            for (sx, sy) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
                let corner = camera.position
                    + camera.forward() * distance
                    + right * sx * distance * tan_y * aspect
                    + up * sy * distance * tan_y;
                let clip = view_proj.project_point3(corner);
                assert!(
                    clip.x.abs() <= 1.0 && clip.y.abs() <= 1.0,
                    "corner outside: {clip}"
                );
                assert!(
                    (0.0..=1.0).contains(&clip.z),
                    "corner outside depth range: {clip}"
                );
            }
        }
    }

    #[test]
    fn small_moves_leave_the_cascade_where_it_is() {
        let aspect = 1.5;
        let mut camera = test_camera();
        let (first, _, _) = fit_cascade(&camera, aspect, sun(), 5.0, 30.0);
        let mut unchanged = 0;
        for _ in 0..50 {
            camera.position.x += 0.001;
            let (next, _, _) = fit_cascade(&camera, aspect, sun(), 5.0, 30.0);
            unchanged += u32::from(next == first);
        }
        assert!(
            unchanged >= 40,
            "only {unchanged} of 50 small moves kept the cascade"
        );
    }

    /// Points at `distance` from the camera, across its field of view: the centre, the four
    /// corners, and a few between.
    fn points_at(camera: &Camera, aspect: f32, distance: f32) -> Vec<Vec3> {
        let tan_y = (camera.fov_y * 0.5).tan();
        let right = camera.right();
        let up = right.cross(camera.forward());
        let mut points = Vec::new();
        for (sx, sy) in [
            (0.0, 0.0),
            (-1.0, -1.0),
            (1.0, -1.0),
            (-1.0, 1.0),
            (1.0, 1.0),
            (0.5, -0.3),
            (-0.7, 0.9),
        ] {
            let direction =
                (camera.forward() + right * sx * tan_y * aspect + up * sy * tan_y).normalize();
            points.push(camera.position + direction * distance);
        }
        points
    }

    fn inside(planes: &[Vec4], point: Vec3) -> bool {
        planes
            .iter()
            .all(|plane| plane.truncate().dot(point) + plane.w >= -1.0e-3)
    }

    #[test]
    fn a_slice_holds_every_point_between_its_distances() {
        let camera = test_camera();
        let aspect = 16.0 / 9.0;
        let planes = slice_planes(&camera, aspect, 38.0, 120.0, 0.0);
        for distance in [38.0, 50.0, 100.0, 119.9] {
            for point in points_at(&camera, aspect, distance) {
                assert!(inside(&planes, point), "{point} at {distance} is outside");
            }
        }
        // Nearer than the slice, in its corner, is outside; so is beyond it.
        let corner = points_at(&camera, aspect, 20.0)[1];
        assert!(!inside(&planes, corner), "{corner} at 20 m is inside");
        let beyond = points_at(&camera, aspect, 130.0)[0];
        assert!(!inside(&planes, beyond), "{beyond} at 130 m is inside");
    }

    #[test]
    fn padding_moves_every_plane_outwards() {
        let camera = test_camera();
        let planes = slice_planes(&camera, 1.5, 38.0, 120.0, 0.0);
        let padded = slice_planes(&camera, 1.5, 38.0, 120.0, 3.0);
        for (plane, wide) in planes.iter().zip(&padded) {
            assert_eq!(plane.truncate(), wide.truncate());
            assert!((wide.w - plane.w - 3.0).abs() < 1.0e-4);
        }
        // Just beyond the far end is now inside.
        let beyond = points_at(&camera, 1.5, 122.0)[0];
        assert!(!inside(&planes, beyond) && inside(&padded, beyond));
    }

    #[test]
    fn every_caster_that_can_shadow_the_slice_passes_the_receiver_planes() {
        let camera = test_camera();
        let aspect = 1.5;
        let planes = slice_planes(&camera, aspect, 38.0, 120.0, 0.0);
        for sun in [sun(), Vec3::new(-0.7, 0.3, 0.1).normalize(), Vec3::Y] {
            let view = PassView::orthographic(Mat4::IDENTITY, Vec3::ZERO, Vec2::ONE, 1.0, 1.0)
                .shadowing(&planes, sun);
            let kept = &view.receiver_planes[..view.receiver_plane_count as usize];
            assert!(
                !kept.is_empty() && kept.len() < 6,
                "{} planes kept",
                kept.len()
            );
            // A caster anywhere between a receiver in the slice and the sun passes.
            for distance in [38.0, 80.0, 119.0] {
                for receiver in points_at(&camera, aspect, distance) {
                    for t in [0.0, 1.0, 50.0, 900.0] {
                        let caster = receiver + sun * t;
                        assert!(
                            inside(kept, caster),
                            "{caster} fails, {t} m towards the sun"
                        );
                    }
                }
            }
            // One well beyond the slice, away from the sun, cannot shadow it and does not pass.
            let shadowed_side = points_at(&camera, aspect, 80.0)[0] - sun * 400.0;
            assert!(
                !inside(kept, shadowed_side),
                "{shadowed_side} passes for sun {sun}"
            );
        }
    }

    /// What a far cascade would be drawn with for this camera and light, as `prepare` builds it.
    fn drawn_for(camera: &Camera, aspect: f32, light: Vec3) -> Drawn {
        let (near, far) = (SPLITS[1] + (SPLITS[2] - SPLITS[1]) * BLEND_START, SPLITS[3]);
        let (view_proj, _, radius) = fit_cascade(camera, aspect, light, SPLITS[2], far);
        let texel = 2.0 * radius / RESOLUTION as f32;
        let slack = lookup_reach(texel);
        let planes = receiver_planes(camera, aspect, near, far, texel * SNAP_TEXELS + slack);
        let view = PassView::orthographic(view_proj, Vec3::ZERO, Vec2::ONE, 1.0, 1.0)
            .shadowing(&planes, light);
        Drawn {
            view_proj,
            light,
            lod_threshold: 1.0,
            eye_lod_scale: 1.0,
            receivers: view.receiver_planes,
            receiver_count: view.receiver_plane_count as usize,
            slack,
        }
    }

    fn covers_now(drawn: &Drawn, camera: &Camera, aspect: f32) -> bool {
        let near = SPLITS[1] + (SPLITS[2] - SPLITS[1]) * BLEND_START;
        drawn.covers(&slice_corners(camera, aspect, near, SPLITS[3]))
    }

    #[test]
    fn a_cascade_covers_the_camera_turning_within_its_margin_and_not_beyond() {
        let aspect = 16.0 / 9.0;
        // Under the test sun only the far plane faces the light, so turning never leaves the
        // cascade's volume; a level camera under a light from overhead keeps the side, far
        // and bottom planes, so turning too far shows.
        let level = Camera {
            pitch: 0.0,
            ..test_camera()
        };
        for (camera, light) in [(test_camera(), sun()), (level, Vec3::Y)] {
            let drawn = drawn_for(&camera, aspect, light);
            assert!(
                covers_now(&drawn, &camera, aspect),
                "not even where it was drawn"
            );
            for (yaw, pitch) in [(0.095, 0.0), (-0.095, 0.0), (0.0, 0.095), (0.06, -0.06)] {
                let turned = Camera {
                    yaw: camera.yaw + yaw,
                    pitch: camera.pitch + pitch,
                    ..camera
                };
                assert!(
                    covers_now(&drawn, &turned, aspect),
                    "turned by {yaw}, {pitch}"
                );
            }
        }
        let drawn = drawn_for(&level, aspect, Vec3::Y);
        for (yaw, pitch) in [(0.3, 0.0), (-0.3, 0.0), (0.0, -0.3)] {
            let turned = Camera {
                yaw: level.yaw + yaw,
                pitch: level.pitch + pitch,
                ..level
            };
            assert!(
                !covers_now(&drawn, &turned, aspect),
                "still covered after {yaw}, {pitch}"
            );
        }
    }

    #[test]
    fn the_widened_planes_alone_hold_the_slice_through_a_turn_of_the_margin() {
        let aspect = 16.0 / 9.0;
        let camera = test_camera();
        let near = SPLITS[1] + (SPLITS[2] - SPLITS[1]) * BLEND_START;
        // No padding: every plane, near and far included, must hold the turned slice by the
        // widening alone.
        let drawn = Drawn {
            view_proj: Mat4::IDENTITY,
            light: Vec3::Y,
            lod_threshold: 1.0,
            eye_lod_scale: 1.0,
            receivers: receiver_planes(&camera, aspect, near, SPLITS[3], 0.0),
            receiver_count: 6,
            slack: 0.0,
        };
        for (yaw, pitch) in [
            (0.0, 0.0),
            (0.095, 0.0),
            (-0.095, 0.0),
            (0.0, 0.095),
            (0.0, -0.095),
        ] {
            let turned = Camera {
                yaw: camera.yaw + yaw,
                pitch: camera.pitch + pitch,
                ..camera
            };
            assert!(
                drawn.covers(&slice_corners(&turned, aspect, near, SPLITS[3])),
                "turned by {yaw}, {pitch}"
            );
        }
    }

    #[test]
    fn a_cascade_covers_the_camera_moving_less_than_a_snap_step_and_not_further() {
        let aspect = 1.5;
        // A level camera under a light from straight overhead: then the side, far and bottom
        // planes all face the light and are kept, so moving past any of them shows.
        let camera = Camera {
            pitch: 0.0,
            ..test_camera()
        };
        let drawn = drawn_for(&camera, aspect, Vec3::Y);
        let (_, _, radius) = fit_cascade(&camera, aspect, Vec3::Y, SPLITS[2], SPLITS[3]);
        let step = 2.0 * radius / RESOLUTION as f32 * SNAP_TEXELS;
        for direction in [
            camera.right(),
            -camera.right(),
            camera.forward(),
            Vec3::NEG_Y,
        ] {
            let moved = Camera {
                position: camera.position + direction * step * 0.9,
                ..camera
            };
            assert!(
                covers_now(&drawn, &moved, aspect),
                "moved {direction} by a step"
            );
            let far = Camera {
                position: camera.position + direction * step * 6.0,
                ..camera
            };
            assert!(
                !covers_now(&drawn, &far, aspect),
                "still covered {direction} by six"
            );
        }
    }

    #[test]
    fn the_lookup_reach_grows_with_the_texel_but_never_falls_below_the_filter_floor() {
        // Far cascades: about ten texels. The nearest, with 1.5 cm texels, keeps the 1.8 cm
        // disc, so more than ten.
        assert!(((lookup_reach(0.5) - JITTER_REACH) / 0.5 - 10.514).abs() < 1.0e-3);
        assert!((lookup_reach(0.015) - JITTER_REACH) / 0.015 > 10.7);
        assert!(lookup_reach(0.2) > lookup_reach(0.1));
    }

    #[test]
    fn a_cascade_is_touched_by_what_stands_in_it_and_not_by_what_stands_aside() {
        let camera = test_camera();
        let (view_proj, center, radius) = fit_cascade(&camera, 1.5, sun(), 5.0, 30.0);
        assert!(cascade_touches(view_proj, radius, (center, 0.5)));
        // The light's own sideways axis, which runs along the cascade's x.
        let across = (-sun()).cross(Vec3::Y).normalize();
        // Out to one side, beyond the cascade's edge by more than its own radius.
        let aside = center + across * radius * 1.5;
        assert!(!cascade_touches(view_proj, radius, (aside, 1.0)));
        // As far out, but large enough to reach in.
        assert!(cascade_touches(view_proj, radius, (aside, radius * 0.6)));
        // Up towards the sun, a caster can be far beyond the cascade and still shade it.
        assert!(cascade_touches(
            view_proj,
            radius,
            (center + sun() * 500.0, 1.0)
        ));
        // Behind the cascade, away from the sun, it can't.
        assert!(!cascade_touches(
            view_proj,
            radius,
            (center - sun() * (radius * 2.0 + 5.0), 1.0)
        ));
    }

    #[test]
    fn the_uniform_lays_out_as_its_shader_twin() {
        crate::shaders::assert_twin!(
            "lighting/deferred.wgsl",
            "Shadows",
            ShadowUniform,
            [view_proj, splits, texel_size, moving]
        );
    }

    #[test]
    fn each_cascade_has_its_own_quarter() {
        let quarters: Vec<_> = (0..CASCADE_COUNT).map(moving_quarter).collect();
        assert_eq!(quarters, vec![[0, 0], [1, 0], [0, 1], [1, 1]]);
    }

    #[test]
    fn a_movable_sphere_touches_a_cascade_only_where_its_shadow_can_land() {
        let aspect = 16.0 / 9.0;
        let camera = test_camera();
        let drawn = drawn_for(&camera, aspect, sun());
        let receiver = points_at(&camera, aspect, 200.0)[0];
        // On the way to the sun from a receiver in the slice: can shadow it.
        assert!(drawn.can_be_shadowed_by((receiver + sun() * 300.0, 2.0)));
        assert!(drawn.can_be_shadowed_by((receiver, 2.0)));
        // Well beyond the slice on the side away from the sun: cannot.
        assert!(!drawn.can_be_shadowed_by((receiver - sun() * 600.0, 2.0)));
    }

    /// The eight corners of a cascade's box in the world, as `fit_cascade` projects it.
    fn box_corners(center: Vec3, radius: f32, sun: Vec3) -> Vec<Vec3> {
        let light_view = Mat4::look_to_rh(Vec3::ZERO, -sun, light_up(sun));
        let c = light_view.transform_point3(center);
        let from_light = light_view.inverse();
        let mut corners = Vec::new();
        for sx in [-1.0, 1.0] {
            for sy in [-1.0, 1.0] {
                for z in [c.z - 2.0 * radius, c.z + DEPTH_REACH] {
                    corners.push(from_light.transform_point3(Vec3::new(
                        c.x + sx * radius,
                        c.y + sy * radius,
                        z,
                    )));
                }
            }
        }
        corners
    }

    #[test]
    fn the_shared_moving_cull_holds_every_touched_cascade_at_any_aspect() {
        let camera = test_camera();
        // Across the view, where the cascades' squares spread apart the most.
        let light = (camera.right() + Vec3::Y * 0.6).normalize();
        for aspect in [1.0, 4.0 / 3.0, 16.0 / 9.0, 0.6] {
            let extents: Vec<(Vec3, f32)> = (0..CASCADE_COUNT)
                .map(|cascade| {
                    let near = if cascade == 0 {
                        camera.near
                    } else {
                        SPLITS[cascade - 1]
                    };
                    let (_, center, radius) =
                        fit_cascade(&camera, aspect, light, near, SPLITS[cascade]);
                    (center, radius)
                })
                .collect();
            let finest = extents[0].1;
            let view = moving_cull_view(&extents, light, finest, 1.0);
            for (cascade, &(center, radius)) in extents.iter().enumerate() {
                for corner in box_corners(center, radius, light) {
                    assert!(
                        inside(&view.planes, corner),
                        "aspect {aspect}: cascade {cascade}'s corner {corner} is left out"
                    );
                }
            }
            // Beyond every box on the side away from the light: nothing there can shadow.
            let (far_center, far_radius) = extents[CASCADE_COUNT - 1];
            assert!(!inside(&view.planes, far_center - light * far_radius * 4.0));
            // Towards the light, the union reaches as far as any box and no further: casters
            // past that would only be clipped by every quarter.
            assert!(inside(
                &view.planes,
                far_center + light * (DEPTH_REACH - 1.0)
            ));
            assert!(!inside(
                &view.planes,
                far_center + light * (DEPTH_REACH + far_radius * 3.0)
            ));
            // The finest cascade's level of detail.
            assert!((view.lod_scale - MOVING_RESOLUTION as f32 / (2.0 * finest)).abs() < 1.0e-3);
        }
    }

    #[test]
    fn nothing_is_redrawn_when_nothing_is_stale() {
        let mut next = 1;
        assert_eq!(choose_due([false; 4], [false; 4], &mut next), [false; 4]);
    }

    #[test]
    fn never_drawn_cascades_all_go_at_once() {
        let mut next = 1;
        assert_eq!(choose_due([true; 4], [true; 4], &mut next), [true; 4]);
    }

    #[test]
    fn stale_far_cascades_take_turns() {
        let mut next = 1;
        let never = [false; 4];
        assert_eq!(
            choose_due([true; 4], never, &mut next),
            [true, true, false, false]
        );
        assert_eq!(
            choose_due([true; 4], never, &mut next),
            [true, false, true, false]
        );
        assert_eq!(
            choose_due([true; 4], never, &mut next),
            [true, false, false, true]
        );
        assert_eq!(
            choose_due([true; 4], never, &mut next),
            [true, true, false, false]
        );
    }

    #[test]
    fn a_single_stale_far_cascade_is_found_whatever_the_turn() {
        let mut next = 2;
        let stale = [false, true, false, false];
        assert_eq!(choose_due(stale, [false; 4], &mut next), stale);
    }

    #[test]
    fn cascade_size_does_not_change_with_rotation() {
        let aspect = 1.5;
        let mut camera = test_camera();
        let (_, _, a) = fit_cascade(&camera, aspect, sun(), 5.0, 30.0);
        camera.yaw += 1.3;
        camera.pitch += 0.4;
        let (_, _, b) = fit_cascade(&camera, aspect, sun(), 5.0, 30.0);
        assert!((a - b).abs() < 1.0e-3);
    }

    #[test]
    fn small_camera_moves_shift_shadows_by_whole_texels() {
        let aspect = 1.5;
        let mut camera = test_camera();
        let (first, _, _) = fit_cascade(&camera, aspect, sun(), 5.0, 30.0);
        camera.position += Vec3::new(0.013, 0.0, 0.0);
        let (second, _, _) = fit_cascade(&camera, aspect, sun(), 5.0, 30.0);
        let point = Vec3::new(10.0, 3.0, -4.0);
        let shift =
            (second.project_point3(point) - first.project_point3(point)) * 0.5 * RESOLUTION as f32;
        for value in [shift.x, shift.y] {
            assert!(
                (value - value.round()).abs() < 0.02,
                "shift of {value} texels"
            );
        }
    }

    #[test]
    fn a_frame_of_cascades_never_straddles_a_round_of_stamps() {
        // Stamps continue from the last one used while the round has room for the frame.
        assert_eq!(next_stamps(0, 1), (false, 1));
        assert_eq!(next_stamps(7, 4), (false, 8));
        assert_eq!(next_stamps(DEPTH_STAMPS - 4, 4), (false, DEPTH_STAMPS - 3));
        // A frame that would run past the last stamp starts a new round, cleared first, even
        // when its first cascade alone would still have fitted.
        assert_eq!(next_stamps(DEPTH_STAMPS - 3, 4), (true, 1));
        assert_eq!(next_stamps(DEPTH_STAMPS, 1), (true, 1));
        // A frame with no cascade due changes nothing.
        assert_eq!(next_stamps(DEPTH_STAMPS, 0), (false, DEPTH_STAMPS + 1));
    }
}
