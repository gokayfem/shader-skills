//! Rain: in the air, where it lands, and what it leaves on the surfaces it falls on
//! (`docs/weather-effects.md`, steps 4 and 5).
//!
//! **How hard it rains** follows the weather's `rain`, as a rate in millimetres an hour. The
//! rate sets how many drops there are and how big, by Marshall and Palmer's distribution, and how
//! fast they fall, at the terminal speeds Gunn and Kinzer measured.
//!
//! **In the air**, nothing is simulated: every drop's place follows from a hash and the clock
//! (`rain/draw.wgsl`), so rain costs only its drawing. Drops lean and drift with the wind,
//! splash where they strike, and drip from eaves.
//!
//! **Where it lands**: the cover map is the scene seen from straight above, drawn through the
//! same culling and rasterizing as a shadow cascade, and redrawn only when the camera moves
//! far. The rain map made from it says how high the first surface the rain meets stands over
//! each spot around the camera, how deep a puddle can gather there, and how high an eave beside
//! it drips from.
//!
//! **What it leaves**: surfaces open to the sky grow wet as the rain goes on and dry after it;
//! puddles fill the hollows of level ground more slowly, and drain more slowly still. Resolve
//! applies both through the wet material the sea uses (`rain/surface.wgsl`). Rings spread where
//! drops strike puddles and water (`rain/ripples.wgsl`).
//!
//! **Its light**: a small volume around the camera holds the light a drop receives there, the
//! sky's, the shadowed sun's and the lamps', worked out once a frame (`rain/light.wgsl`).
//!
//! A scene where it has never rained, or that has dried since, runs none of it.

use crate::clouds::Weather;
use crate::gpu::{groups_for, Gpu, LayoutBuilder, Target};
use crate::scene::GpuScene;
use crate::shaders;
use crate::vg::{
    CullPhase, CullPipelines, CullWork, PassView, RasterBindings, Rasterizer, ViewSlots,
};
use crate::wind::Wind;
use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec2, Vec3};

/// Texels along each side of the cover and rain maps. Twin of `RAIN_MAP_TEXELS`.
pub const MAP_TEXELS: u32 = 512;
/// Metres per texel: 128 m around the camera at a quarter of a metre.
pub const MAP_TEXEL: f32 = 0.25;
/// The maps move in steps of this many metres, so they are seldom redrawn.
const MAP_SNAP: f32 = 16.0;
/// How far above the camera the cover map's top lies, and how far down it reaches.
const COVER_ABOVE: f32 = 150.0;
const COVER_REACH: f32 = 400.0;
const COVER_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const MAP_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba32Float;
/// Texels along the ripple tile, a metre across. Twin of `RIPPLE_TEXELS`.
pub const RIPPLE_TEXELS: u32 = 256;
const RIPPLE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const LIGHT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// What a drawn instance is, in the top bits of its number: the three rings of falling drops,
/// then splashes, then drips, in the order of `Rain::counts`. Twin of `rain/draw.wgsl`.
const KIND_SHIFT: u32 = 20;
/// The rings of falling drops: cells along each side. Twin of `RING_CELLS`.
const RING_CELLS: [u32; 3] = [80, 80, 80];
const SPLASH_CELLS: u32 = 28;
const SPLASH_DROPLETS: u32 = 6;
const DRIP_CELLS: u32 = 64;
/// Layers of drops each ring holds in the heaviest rain, at full quality.
pub const MOST_LAYERS: f32 = 4.0;

/// The rain rate, in mm/h, of the weather's heaviest rain, `rain` 1: a downpour.
pub const HEAVIEST_RATE: f32 = 40.0;
/// Marshall and Palmer's intercept, drops per cubic metre per millimetre of diameter.
const MP_INTERCEPT: f32 = 8000.0;
/// Drops smaller than this, in mm, are too small to see as streaks.
pub const SMALLEST_SEEN: f32 = 1.0;
/// Seconds for open surfaces to grow wet in a downpour, and for puddles to fill; how much
/// slower lighter rain is, at its rate in mm/h.
const SOAKING_SECONDS: f32 = 40.0;
const FILLING_SECONDS: f32 = 240.0;
const RATE_HALF_SOAK: f32 = 3.0;
/// Seconds over which a wet surface dries, in still air, and puddles drain.
const DRYING_SECONDS: f32 = 300.0;
const DRAINING_SECONDS: f32 = 900.0;

/// The rain rate in mm/h for the weather's `rain`, 0 to 1. Light rain is a few mm/h, heavy rain
/// tens; it grows faster than linearly, as the weather's heaviest is rare.
pub fn rate_for(rain: f32) -> f32 {
    HEAVIEST_RATE * rain.clamp(0.0, 1.0).powf(1.5)
}

/// Marshall and Palmer's slope of the drop-size distribution at `rate` mm/h, per mm.
fn mp_slope(rate: f32) -> f32 {
    4.1 * rate.max(1.0e-3).powf(-0.21)
}

/// Drops larger than `diameter` mm in a cubic metre of air, at `rate` mm/h (Marshall and Palmer
/// 1948): N(D) = N0 e^(-ΛD).
pub fn drops_above(rate: f32, diameter: f32) -> f32 {
    if rate <= 0.0 {
        return 0.0;
    }
    let slope = mp_slope(rate);
    MP_INTERCEPT / slope * (-slope * diameter).exp()
}

/// The mean diameter, in mm, of the drops larger than `diameter` mm at `rate` mm/h.
pub fn mean_diameter_above(rate: f32, diameter: f32) -> f32 {
    diameter + 1.0 / mp_slope(rate)
}

/// A raindrop's terminal speed in m/s at `diameter` mm, as Gunn and Kinzer (1949) measured it,
/// in Atlas, Srivastava and Sekhon's fit (1973).
pub fn terminal_speed(diameter: f32) -> f32 {
    (9.65 - 10.3 * (-0.6 * diameter).exp()).max(0.0)
}

/// How dense the visible rain is at `rate` mm/h, against the heaviest: the drops big enough to
/// see, by Marshall and Palmer.
pub fn visible_density(rate: f32) -> f32 {
    (drops_above(rate, SMALLEST_SEEN) / drops_above(HEAVIEST_RATE, SMALLEST_SEEN)).clamp(0.0, 1.0)
}

/// The rain as it goes on: how hard it falls, and what it has left on the surfaces.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RainState {
    /// Seconds since the rain began, for where its drops are.
    pub time: f64,
    /// mm/h now.
    pub rate: f32,
    /// How wet surfaces open to the sky are, 0 to 1.
    pub wetness: f32,
    /// How full the puddles are, 0 to 1.
    pub puddles: f32,
}

impl RainState {
    /// Moves the rain on by `seconds` in `weather`, with a wind of `wind_speed` m/s near the
    /// ground, which dries what the rain leaves faster.
    pub fn advance(&mut self, weather: &Weather, wind_speed: f32, seconds: f32) {
        let seconds = seconds.max(0.0);
        // The rain's own clock starts afresh each time it starts, so it stays precise in the
        // shaders, which take it as f32.
        if !self.active() {
            self.time = 0.0;
        }
        self.time += f64::from(seconds);
        self.rate = rate_for(weather.rain);
        let pace = self.rate / (self.rate + RATE_HALF_SOAK);
        // Wet surfaces dry in wind faster, and puddles evaporate faster.
        let airing = 1.0 + wind_speed / 8.0;
        if self.rate > 0.0 {
            let soak = 1.0 - (-seconds * pace / SOAKING_SECONDS).exp();
            self.wetness += (1.0 - self.wetness) * soak;
            let fill = 1.0 - (-seconds * pace / FILLING_SECONDS).exp();
            self.puddles += (pace - self.puddles).max(0.0) * fill;
        } else {
            self.wetness *= (-seconds * airing / DRYING_SECONDS).exp();
            self.puddles *= (-seconds * airing / DRAINING_SECONDS).exp();
            // Nothing left worth drawing.
            if self.wetness < 1.0e-3 {
                self.wetness = 0.0;
            }
            if self.puddles < 1.0e-3 {
                self.puddles = 0.0;
            }
        }
    }

    /// Whether anything of the rain is to be drawn or shown.
    pub fn active(&self) -> bool {
        self.rate > 0.0 || self.wetness > 0.0 || self.puddles > 0.0
    }
}

/// Twin of `RainParams` in `rain/params.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct RainUniform {
    pub map: [f32; 4],
    pub camera: [f32; 4],
    pub wind: [f32; 4],
    pub rain: [f32; 4],
    pub drops: [f32; 4],
    pub extra: [f32; 4],
    pub light: [f32; 4],
}

/// Twin of `MapParams` in `rain/map.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
struct MapUniform {
    top: f32,
    reach: f32,
    _pad: [f32; 2],
}

/// Where the cover map lies.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CoverView {
    pub view_proj: Mat4,
    /// The world's (x, z) at its first texel's corner.
    pub origin: Vec2,
    pub top: f32,
}

impl CoverView {
    /// The map for a camera at `camera`: centred under it, snapped to a coarse grid, its top
    /// well above the camera.
    pub fn new(camera: Vec3) -> Self {
        let centre = (Vec2::new(camera.x, camera.z) / MAP_SNAP).round() * MAP_SNAP;
        let top = (camera.y / MAP_SNAP).round() * MAP_SNAP + COVER_ABOVE;
        let eye = Vec3::new(centre.x, top, centre.y);
        // Looking straight down, with north (-Z) at the top of the map, so texel (0, 0) is its
        // least x and z.
        let view = Mat4::look_to_rh(eye, Vec3::NEG_Y, Vec3::NEG_Z);
        let half = MAP_TEXELS as f32 * MAP_TEXEL * 0.5;
        let projection = Mat4::orthographic_rh(-half, half, -half, half, 0.0, COVER_REACH);
        Self {
            view_proj: projection * view,
            origin: centre - Vec2::splat(half),
            top,
        }
    }
}

/// Quality settings for rain.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RainQuality {
    /// The share of drops drawn.
    pub detail: f32,
    /// Cells of the light volume, and metres per cell.
    pub light_cells: [u32; 3],
    pub light_cell: f32,
}

impl Default for RainQuality {
    fn default() -> Self {
        Self {
            detail: 1.0,
            light_cells: [64, 32, 64],
            light_cell: 1.0,
        }
    }
}

/// The rain's passes and what they keep.
pub struct Rain {
    pub state: RainState,
    uniform: wgpu::Buffer,
    params: RainUniform,
    // Cover and rain maps.
    cover: wgpu::TextureView,
    cover_pipeline: wgpu::RenderPipeline,
    cover_drawn: Option<(CoverView, f32)>,
    cover_outdated: bool,
    cover_current: CoverView,
    map: Target,
    map_pipeline: wgpu::ComputePipeline,
    map_group: wgpu::BindGroup,
    map_uniform: wgpu::Buffer,
    map_ready: bool,
    // Ripples.
    ripples: Target,
    ripple_pipeline: wgpu::ComputePipeline,
    ripple_group: wgpu::BindGroup,
    // The light volume.
    light_layout: wgpu::BindGroupLayout,
    light_pipeline: wgpu::ComputePipeline,
    light_textures: Option<([u32; 3], wgpu::Texture, wgpu::Texture, wgpu::BindGroup)>,
    light_sampler: wgpu::Sampler,
    // Drawing.
    draw_layout: wgpu::BindGroupLayout,
    draw_pipeline: wgpu::RenderPipeline,
    /// The bind group for drawing, made for one scene depth, and what it was made for.
    draw_group: Option<(usize, wgpu::BindGroup)>,
    /// For resolve: the rain, its map and its ripples.
    pub surface_layout: wgpu::BindGroupLayout,
    pub surface_group: wgpu::BindGroup,
    pub quality: RainQuality,
    /// Instances of each kind to draw this frame.
    counts: [u32; 5],
}

/// What the light volume reads besides the rain.
pub struct LightInputs<'a> {
    pub shadow_uniform: &'a wgpu::Buffer,
    pub shadow_maps: &'a wgpu::TextureView,
    pub shadow_sampler: &'a wgpu::Sampler,
    pub lights: &'a wgpu::Buffer,
}

impl Rain {
    pub fn new(
        gpu: &Gpu,
        rasterizer: &Rasterizer,
        environment_layout: &wgpu::BindGroupLayout,
        color_format: wgpu::TextureFormat,
        reactive_format: wgpu::TextureFormat,
    ) -> Self {
        let uniform = gpu.uniform_buffer("rain", std::mem::size_of::<RainUniform>() as u64);

        // The cover map, drawn as the seabed map is.
        let cover_texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("rain cover"),
            size: wgpu::Extent3d {
                width: MAP_TEXELS,
                height: MAP_TEXELS,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: COVER_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let cover = cover_texture.create_view(&Default::default());
        let cover_layout = gpu
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("rain cover"),
                bind_group_layouts: &[Some(&rasterizer.raster_layout)],
                immediate_size: 0,
            });
        let cover_pipeline = gpu
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("rain cover"),
                layout: Some(&cover_layout),
                vertex: wgpu::VertexState {
                    module: &rasterizer.hardware_module,
                    entry_point: Some("vs_depth"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    front_face: wgpu::FrontFace::Ccw,
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: COVER_FORMAT,
                    depth_write_enabled: Some(true),
                    // The highest surface over each texel: nearest the map's top.
                    depth_compare: Some(wgpu::CompareFunction::Less),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                fragment: None,
                multiview_mask: None,
                cache: None,
            });

        let map = Target::new(
            gpu,
            "rain map",
            (MAP_TEXELS, MAP_TEXELS),
            MAP_FORMAT,
            wgpu::TextureUsages::COPY_SRC,
        );
        let map_uniform = gpu.uniform_buffer("rain map", std::mem::size_of::<MapUniform>() as u64);
        let map_layout = LayoutBuilder::new()
            .uniform()
            .depth_texture()
            .storage_texture(MAP_FORMAT)
            .build(gpu, "rain map");
        let map_module = shaders::module(&gpu.device, "rain/map.wgsl");
        let map_pipeline = gpu.compute_pipeline("rain map", &map_module, "main", &[&map_layout]);
        let map_group = gpu.bind_group(
            "rain map",
            &map_layout,
            &[
                map_uniform.as_entire_binding(),
                wgpu::BindingResource::TextureView(&cover),
                wgpu::BindingResource::TextureView(&map.view),
            ],
        );

        let ripples = Target::new(
            gpu,
            "rain ripples",
            (RIPPLE_TEXELS, RIPPLE_TEXELS),
            RIPPLE_FORMAT,
            wgpu::TextureUsages::COPY_SRC,
        );
        let ripple_layout = LayoutBuilder::new()
            .uniform()
            .storage_texture(RIPPLE_FORMAT)
            .build(gpu, "rain ripples");
        let ripple_module = shaders::module(&gpu.device, "rain/ripples.wgsl");
        let ripple_pipeline =
            gpu.compute_pipeline("rain ripples", &ripple_module, "main", &[&ripple_layout]);
        let ripple_group = gpu.bind_group(
            "rain ripples",
            &ripple_layout,
            &[
                uniform.as_entire_binding(),
                wgpu::BindingResource::TextureView(&ripples.view),
            ],
        );

        let light_layout = LayoutBuilder::new()
            .uniform()
            .uniform()
            .depth_texture_array()
            .sampler_comparison()
            .storage_read()
            .storage_texture_3d(LIGHT_FORMAT)
            .storage_texture_3d(LIGHT_FORMAT)
            .build(gpu, "rain light");
        let light_module = shaders::module(&gpu.device, "rain/light.wgsl");
        let light_pipeline = gpu.compute_pipeline(
            "rain light",
            &light_module,
            "main",
            &[&light_layout, environment_layout],
        );
        let light_sampler = gpu.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("rain light"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let draw_layout = LayoutBuilder::new()
            .visibility(wgpu::ShaderStages::VERTEX_FRAGMENT)
            .uniform()
            .uniform()
            .texture_unfilterable()
            .texture_unfilterable()
            .texture_3d()
            .texture_3d()
            .sampler()
            .build(gpu, "rain draw");
        let draw_module = shaders::module(&gpu.device, "rain/draw.wgsl");
        let draw_pipeline_layout =
            gpu.device
                .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("rain draw"),
                    bind_group_layouts: &[Some(&draw_layout)],
                    immediate_size: 0,
                });
        // Premultiplied over the scene, and the most of it kept in the reactive mask.
        let premultiplied = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
            operation: wgpu::BlendOperation::Add,
        };
        let most = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::One,
            operation: wgpu::BlendOperation::Max,
        };
        let draw_pipeline = gpu
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("rain draw"),
                layout: Some(&draw_pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &draw_module,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &draw_module,
                    entry_point: Some("fs_main"),
                    compilation_options: Default::default(),
                    targets: &[
                        Some(wgpu::ColorTargetState {
                            format: color_format,
                            blend: Some(wgpu::BlendState {
                                color: premultiplied,
                                alpha: premultiplied,
                            }),
                            write_mask: wgpu::ColorWrites::ALL,
                        }),
                        Some(wgpu::ColorTargetState {
                            format: reactive_format,
                            blend: Some(wgpu::BlendState {
                                color: most,
                                alpha: most,
                            }),
                            write_mask: wgpu::ColorWrites::ALL,
                        }),
                    ],
                }),
                multiview_mask: None,
                cache: None,
            });

        let surface_layout = LayoutBuilder::new()
            .uniform()
            .texture_unfilterable()
            .texture()
            .build(gpu, "rain on surfaces");
        let surface_group = gpu.bind_group(
            "rain on surfaces",
            &surface_layout,
            &[
                uniform.as_entire_binding(),
                wgpu::BindingResource::TextureView(&map.view),
                wgpu::BindingResource::TextureView(&ripples.view),
            ],
        );

        Self {
            state: RainState::default(),
            uniform,
            params: RainUniform::default(),
            cover,
            cover_pipeline,
            cover_drawn: None,
            cover_outdated: false,
            cover_current: CoverView::new(Vec3::ZERO),
            map,
            map_pipeline,
            map_group,
            map_uniform,
            map_ready: false,
            ripples,
            ripple_pipeline,
            ripple_group,
            light_layout,
            light_pipeline,
            light_textures: None,
            light_sampler,
            draw_layout,
            draw_pipeline,
            draw_group: None,
            surface_layout,
            surface_group,
            quality: RainQuality::default(),
            counts: [0; 5],
        }
    }

    /// The ripple tile, for the water passes.
    pub fn ripples(&self) -> &wgpu::TextureView {
        &self.ripples.view
    }

    /// Makes the cover map redraw, as finer geometry has arrived or the scene has changed.
    pub fn mark_stale(&mut self) {
        self.cover_outdated = true;
    }

    /// Moves the rain on by `seconds`, in `weather` and `wind`.
    pub fn advance(&mut self, weather: &Weather, wind: &Wind, seconds: f32) {
        let mean = Vec2::from_array(wind.now().mean);
        self.state.advance(weather, mean.length(), seconds);
    }

    /// Sets this frame's parameters for a camera at `camera`, and places the cover map. Writes
    /// the cover map's view to `slot` and returns whether it is due to be drawn.
    #[allow(clippy::too_many_arguments)]
    pub fn prepare(
        &mut self,
        gpu: &Gpu,
        slots: &ViewSlots,
        slot: u32,
        camera: Vec3,
        wind: &Wind,
        weather: &Weather,
        lod_threshold: f32,
        frame_seconds: f32,
    ) -> bool {
        let state = self.state;
        let now = wind.now();
        let view = CoverView::new(camera);
        let due = state.active()
            && (self.cover_outdated || self.cover_drawn != Some((view, lod_threshold)));
        if due {
            self.cover_outdated = false;
            self.cover_drawn = Some((view, lod_threshold));
            self.cover_current = view;
            let centre = view.origin + Vec2::splat(MAP_TEXELS as f32 * MAP_TEXEL * 0.5);
            let pass = PassView::orthographic(
                view.view_proj,
                Vec3::new(centre.x, view.top, centre.y),
                Vec2::splat(MAP_TEXELS as f32),
                1.0 / MAP_TEXEL,
                lod_threshold,
            );
            slots.write(gpu, slot, &pass);
            gpu.queue.write_buffer(
                &self.map_uniform,
                0,
                bytemuck::bytes_of(&MapUniform {
                    top: view.top,
                    reach: COVER_REACH,
                    _pad: [0.0; 2],
                }),
            );
        }

        let density = visible_density(state.rate);
        let diameter = mean_diameter_above(state.rate.max(0.1), SMALLEST_SEEN);
        let layers = (MOST_LAYERS * density * self.quality.detail).min(MOST_LAYERS);
        let cells = self.quality.light_cells;
        let cell = self.quality.light_cell;
        let light_origin = (camera / cell).floor() * cell
            - Vec3::new(cells[0] as f32, cells[1] as f32, cells[2] as f32) * cell * 0.5;
        // Streaks are a drop's path over the time an eye or a camera smears it: 1/45 s at any
        // frame rate above 45, and a frame's length below, to 1/30 s. At a frame's own length
        // they would shrink to ticks on a fast display.
        let exposure = (1.0 / 45.0_f32).max(frame_seconds.min(1.0 / 30.0));
        self.params = RainUniform {
            map: [
                self.cover_current.origin.x,
                self.cover_current.origin.y,
                MAP_TEXEL,
                if self.map_ready || due { 1.0 } else { 0.0 },
            ],
            camera: [
                camera.x,
                camera.y,
                camera.z,
                (state.time % 100_000.0) as f32,
            ],
            wind: [now.mean[0], now.mean[1], now.offset[0], now.offset[1]],
            rain: [state.rate, density, state.wetness, state.puddles],
            drops: [diameter * 0.001, terminal_speed(diameter), exposure, layers],
            extra: [state.wetness, weather.gusts, frame_seconds, 0.0],
            light: [light_origin.x, light_origin.y, light_origin.z, cell],
        };
        gpu.queue
            .write_buffer(&self.uniform, 0, bytemuck::bytes_of(&self.params));

        let rings = if state.rate > 0.0 {
            layers.ceil() as u32
        } else {
            0
        };
        self.counts = [
            RING_CELLS[0] * RING_CELLS[0] * rings,
            RING_CELLS[1] * RING_CELLS[1] * rings,
            RING_CELLS[2] * RING_CELLS[2] * rings,
            if state.rate > 0.0 {
                SPLASH_CELLS * SPLASH_CELLS * SPLASH_DROPLETS
            } else {
                0
            },
            if state.wetness > 0.0 {
                DRIP_CELLS * DRIP_CELLS
            } else {
                0
            },
        ];
        due
    }

    /// Draws the cover map and makes the rain map from it, when `prepare` found it due.
    #[allow(clippy::too_many_arguments)]
    pub fn record_maps(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        cull: &CullPipelines,
        scene: &GpuScene,
        cull_scene: &wgpu::BindGroup,
        work: &CullWork,
        raster: &RasterBindings,
        slot: u32,
    ) {
        cull.record(encoder, scene, cull_scene, slot, work, CullPhase::First);
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("rain cover"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.cover,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            pass.set_pipeline(&self.cover_pipeline);
            pass.set_bind_group(0, &raster.group, &[ViewSlots::offset(slot)]);
            pass.draw_indirect(work.args(), CullWork::draw_args_offset());
        }
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("rain map"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.map_pipeline);
        pass.set_bind_group(0, &self.map_group, &[]);
        let groups = groups_for(MAP_TEXELS, 8);
        pass.dispatch_workgroups(groups, groups, 1);
        self.map_ready = true;
    }

    /// Works out the rain's light and its ripples for this frame, while it rains.
    pub fn record_light(
        &mut self,
        gpu: &Gpu,
        encoder: &mut wgpu::CommandEncoder,
        environment: &wgpu::BindGroup,
        inputs: &LightInputs<'_>,
    ) {
        // Lit while anything of it is drawn, drips after the rain included.
        if !self.state.active() {
            return;
        }
        let cells = self.quality.light_cells;
        if self
            .light_textures
            .as_ref()
            .is_none_or(|(c, ..)| *c != cells)
        {
            let make = |label| {
                gpu.device.create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width: cells[0],
                        height: cells[1],
                        depth_or_array_layers: cells[2],
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D3,
                    format: LIGHT_FORMAT,
                    usage: wgpu::TextureUsages::STORAGE_BINDING
                        | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
            };
            let ambient = make("rain ambient light");
            let sun = make("rain sunlight");
            let group = gpu.bind_group(
                "rain light",
                &self.light_layout,
                &[
                    self.uniform.as_entire_binding(),
                    inputs.shadow_uniform.as_entire_binding(),
                    wgpu::BindingResource::TextureView(inputs.shadow_maps),
                    wgpu::BindingResource::Sampler(inputs.shadow_sampler),
                    inputs.lights.as_entire_binding(),
                    wgpu::BindingResource::TextureView(&ambient.create_view(&Default::default())),
                    wgpu::BindingResource::TextureView(&sun.create_view(&Default::default())),
                ],
            );
            self.light_textures = Some((cells, ambient, sun, group));
            self.draw_group = None;
        }
        let (_, _, _, group) = self.light_textures.as_ref().expect("just made");
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("rain light"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.light_pipeline);
        pass.set_bind_group(0, group, &[]);
        pass.set_bind_group(1, environment, &[]);
        pass.dispatch_workgroups(
            groups_for(cells[0], 4),
            groups_for(cells[1], 4),
            groups_for(cells[2], 4),
        );
        // Rings only while it rains: after, puddles lie still.
        if self.state.rate > 0.0 {
            pass.set_pipeline(&self.ripple_pipeline);
            pass.set_bind_group(0, &self.ripple_group, &[]);
            let groups = groups_for(RIPPLE_TEXELS, 8);
            pass.dispatch_workgroups(groups, groups, 1);
        }
    }

    /// Whether any rain is drawn this frame.
    pub fn draws(&self) -> bool {
        self.counts.iter().any(|&c| c > 0) && self.light_textures.is_some()
    }

    /// Draws the rain over `color`, testing against `scene_depth` (`depth_key` names it, so the
    /// bind group is made again when it changes), and marks where it covers in `reactive`, which
    /// it clears first if `clear_reactive`.
    #[allow(clippy::too_many_arguments)]
    pub fn record_draw(
        &mut self,
        gpu: &Gpu,
        encoder: &mut wgpu::CommandEncoder,
        frame: &wgpu::Buffer,
        color: &wgpu::TextureView,
        reactive: &wgpu::TextureView,
        clear_reactive: bool,
        scene_depth: &wgpu::TextureView,
        depth_key: usize,
    ) {
        if !self.draws() {
            return;
        }
        if self
            .draw_group
            .as_ref()
            .is_none_or(|(key, _)| *key != depth_key)
        {
            let (_, ambient, sun, _) = self.light_textures.as_ref().expect("drawn only with light");
            let group = gpu.bind_group(
                "rain draw",
                &self.draw_layout,
                &[
                    self.uniform.as_entire_binding(),
                    frame.as_entire_binding(),
                    wgpu::BindingResource::TextureView(&self.map.view),
                    wgpu::BindingResource::TextureView(scene_depth),
                    wgpu::BindingResource::TextureView(&ambient.create_view(&Default::default())),
                    wgpu::BindingResource::TextureView(&sun.create_view(&Default::default())),
                    wgpu::BindingResource::Sampler(&self.light_sampler),
                ],
            );
            self.draw_group = Some((depth_key, group));
        }
        let (_, group) = self.draw_group.as_ref().expect("just made");
        let attach = |view, load| {
            Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load,
                    store: wgpu::StoreOp::Store,
                },
            })
        };
        let reactive_load = if clear_reactive {
            wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT)
        } else {
            wgpu::LoadOp::Load
        };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("rain draw"),
            color_attachments: &[
                attach(color, wgpu::LoadOp::Load),
                attach(reactive, reactive_load),
            ],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.draw_pipeline);
        pass.set_bind_group(0, group, &[]);
        for (kind, &count) in self.counts.iter().enumerate() {
            if count > 0 {
                let first = (kind as u32) << KIND_SHIFT;
                pass.draw(0..6, first..first + count);
            }
        }
    }

    /// Instances drawn this frame, of each kind.
    pub fn counts(&self) -> [u32; 5] {
        self.counts
    }

    /// The cover map's place this frame.
    pub fn cover_view(&self) -> &CoverView {
        &self.cover_current
    }
}

#[cfg(test)]
mod tests;
