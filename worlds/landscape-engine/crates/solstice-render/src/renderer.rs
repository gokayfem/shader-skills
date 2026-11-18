//! The top-level renderer: owns all passes and defines the order of a frame.

use crate::clouds::{CloudBindings, Clouds, Weather};
use crate::frame::{
    jitter_offset, jitter_projection, Camera, DebugView, Environment, FrameUniform, NO_SEA,
};
use crate::gi::{GiVolume, ScreenGi, ScreenGiBindings, LEVEL_COUNT};
use crate::gpu::{Gpu, Target};
use crate::lighting::{
    environment_group, environment_layout, DeferredLighting, LightShadows, LightTiles,
};
use crate::lightning::{BoltDraw, Lightning, Thunder};
use crate::ocean::grid::PATCH_CELLS;
use crate::ocean::pond::{Pond, PondWater};
use crate::ocean::surface::{SceneViews, WaterSurface, WaterTargets};
use crate::ocean::waterline::{SeaViews, Waterline};
use crate::ocean::{Ocean, Sea, SeaState};
use crate::particles::{
    LightInputs, ParticleStats, ParticleTargets, Particles, POOL_HIGH, POOL_LOW, POOL_MEDIUM,
};
use crate::post::{PostBindings, PostProcess};
use crate::profiler::{section, Profiler, SECTIONS};
use crate::rain::Rain;
use crate::scene::GpuScene;
use crate::shadow::{ShadowMaps, CASCADE_COUNT, MOVING_SLOTS};
use crate::sky::Sky;
use crate::streaming::{
    choose_tiers, StreamRequest, Streamer, StreamingStats, HOLD_SHADOWS, HOLD_VOXELS,
};
use crate::vg::{
    CullPhase, CullPipelines, CullWork, GBuffer, Hzb, InstanceFilter, PassView, RasterBindings,
    Rasterizer, Resolve, ResolveBindings, ViewSlots, VisibilityBuffer,
};
use crate::wind::{FieldParams, Wind, WindField};
use glam::{Mat4, Vec2, Vec3, Vec4};
use solstice_asset::paged::{PagedScene, PAGE_BYTES};
use solstice_asset::LoadedScene;

/// Scales texture gradients, selecting finer mips than one texel per pixel, about 0.75 of a mip
/// here. Temporal filtering averages over several frames' jitter, which softens the image and
/// also suppresses the shimmer that finer mips would otherwise cause.
const TEXTURE_DETAIL: f32 = 0.6;

const HDR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// Capacity of the main view's traversal queue and selected-cluster list.
const MAIN_QUEUE_CAPACITY: u64 = 1 << 22;
const MAIN_VISIBLE_CAPACITY: u64 = 1 << 21;
/// The same for views that need no history: shadow cascades and voxelization.
const AUX_QUEUE_CAPACITY: u64 = 1 << 22;
const AUX_VISIBLE_CAPACITY: u64 = 1 << 21;
/// Room for cut-out clusters, in a scene that has any, on top of the above.
const MASKED_VISIBLE_CAPACITY: u64 = 1 << 19;

/// Assignment of view slots.
/// The most a flash of lightning lights the scene, against the light already about it, and the
/// least light counted as about by night, as a share of daylight.
const FLASH_OVER_SCENE: f32 = 20.0;
const NIGHT_LIGHT_SHARE: f32 = 0.0003;

const SLOT_MAIN_FIRST: u32 = 0;
const SLOT_MAIN_SECOND: u32 = 1;
const SLOT_SHADOW: u32 = 2;
const SLOT_VOXEL: u32 = SLOT_SHADOW + CASCADE_COUNT as u32;
const SLOT_SEABED: u32 = SLOT_VOXEL + LEVEL_COUNT as u32;
const SLOT_MOVING_SHADOW: u32 = SLOT_SEABED + 1;
const SLOT_COVER: u32 = SLOT_MOVING_SHADOW + MOVING_SLOTS;
const SLOT_COUNT: u32 = SLOT_COVER + 1;

/// A named balance between image quality and GPU cost.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Quality {
    /// Integrated graphics: fewer pixels and triangles, reconstructed by temporal upscaling.
    Low,
    Medium,
    /// Discrete desktop GPUs.
    #[default]
    High,
}

impl Quality {
    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "low" => Self::Low,
            "medium" => Self::Medium,
            "high" => Self::High,
            _ => return None,
        })
    }
}

/// User-tunable quality settings.
#[derive(Clone, Copy, Debug)]
pub struct RendererSettings {
    /// Largest allowed geometric error on screen, in pixels.
    pub lod_threshold: f32,
    /// The same, in shadow map texels. Shadow maps are the costliest pass on small GPUs, and
    /// a shadow's outline hides far more error than a directly visible surface does.
    pub shadow_lod_threshold: f32,
    /// For a streamed scene: how far around the camera, in metres, pages are loaded for what it
    /// isn't facing, at a coarser level of detail, so that turning or moving fast starts from
    /// more than the coarsest geometry. 0 loads only what is seen.
    pub prefetch_radius: f32,
    /// Clusters under this many texels of a shadow cascade, or voxels of the GI, across are
    /// drawn into it as specks: a point each, by one thread, where a rasterizer gives each a
    /// workgroup. For a scene whose coarsest clusters are small, of which a far cascade is
    /// nearly all specks. 0 draws every cluster's triangles.
    pub speck_texels: f32,
    /// Internal resolution relative to the output. Temporal upscaling fills in the rest.
    pub render_scale: f32,
    pub debug_view: DebugView,
    /// Blend each frame with reprojected history. Off, aliasing returns, but every frame
    /// shows exactly what was rendered, which is what diagnostics and tests want.
    pub temporal_accumulation: bool,
    /// Draw volumetric clouds, their shadows and their light in the sky.
    pub clouds: bool,
    /// Texels across each of the ocean's wave tiles: a power of two, at most 256.
    pub ocean_resolution: u32,
    /// Rays each GI probe traces on a side: one of `PROBE_RAY_OPTIONS`. More rays mean less
    /// boiling in indirect light, for more tracing.
    pub probe_rays: u32,
    /// Cells across each of the sea surface's patches: a multiple of four.
    pub ocean_patch_cells: u32,
    /// Particles the pool holds, across every effect.
    pub particle_pool: u32,
    /// Scales every effect's spawn rate: fewer particles for smaller GPUs.
    pub particle_detail: f32,
    /// Each particle's light is worked out afresh once in this many frames, with ambient rays
    /// of this many steps through the GI volume.
    pub particle_light_interval: u32,
    pub particle_ray_steps: u32,
    /// Metres from the camera over which swaying in the wind fades out, where it would no longer
    /// be seen (`docs/weather-effects.md`).
    pub sway_fade: [f32; 2],
    /// The rain's share of drops drawn, and its light volume's cells and their size in metres.
    pub rain: crate::rain::RainQuality,
    /// Rays each pixel of a pond reflects along: more for a smoother reflection on ruffled
    /// water.
    pub pond_reflection_rays: u32,
}

impl Default for RendererSettings {
    fn default() -> Self {
        Self {
            lod_threshold: 1.0,
            shadow_lod_threshold: 1.0,
            prefetch_radius: 0.0,
            speck_texels: 0.0,
            render_scale: 1.0,
            debug_view: DebugView::Lit,
            temporal_accumulation: true,
            clouds: true,
            ocean_resolution: 256,
            probe_rays: 8,
            ocean_patch_cells: PATCH_CELLS,
            particle_pool: POOL_HIGH,
            particle_detail: 1.0,
            particle_light_interval: 1,
            particle_ray_steps: 16,
            sway_fade: [40.0, 60.0],
            rain: crate::rain::RainQuality::default(),
            pond_reflection_rays: 4,
        }
    }
}

impl RendererSettings {
    /// Applies a quality preset, leaving diagnostic options alone.
    pub fn apply_quality(&mut self, quality: Quality) {
        (
            self.render_scale,
            self.lod_threshold,
            self.shadow_lod_threshold,
        ) = match quality {
            Quality::Low => (0.6, 2.0, 4.0),
            Quality::Medium => (0.8, 1.5, 2.0),
            Quality::High => (1.0, 1.0, 1.0),
        };
        self.ocean_resolution = if quality == Quality::Low { 128 } else { 256 };
        self.probe_rays = if quality == Quality::Low { 6 } else { 8 };
        self.ocean_patch_cells = if quality == Quality::Low {
            PATCH_CELLS / 2
        } else {
            PATCH_CELLS
        };
        (
            self.particle_pool,
            self.particle_detail,
            self.particle_light_interval,
            self.particle_ray_steps,
        ) = match quality {
            Quality::Low => (POOL_LOW, 0.35, 4, 8),
            Quality::Medium => (POOL_MEDIUM, 0.6, 2, 12),
            Quality::High => (POOL_HIGH, 1.0, 1, 16),
        };
        self.rain = match quality {
            Quality::Low => crate::rain::RainQuality {
                detail: 0.35,
                light_cells: [32, 16, 32],
                light_cell: 2.0,
            },
            Quality::Medium => crate::rain::RainQuality {
                detail: 0.6,
                light_cells: [48, 24, 48],
                light_cell: 1.33,
            },
            Quality::High => crate::rain::RainQuality::default(),
        };
        self.pond_reflection_rays = match quality {
            Quality::Low => 1,
            Quality::Medium => 2,
            Quality::High => 4,
        };
        self.sway_fade = match quality {
            Quality::Low => [20.0, 30.0],
            Quality::Medium => [30.0, 45.0],
            Quality::High => [40.0, 60.0],
        };
    }
}

/// How much GPU memory a streaming renderer may spend on a scene, in bytes.
#[derive(Clone, Copy, Debug)]
pub struct StreamingBudget {
    pub geometry_bytes: u64,
    pub texture_bytes: u64,
}

impl Default for StreamingBudget {
    /// Enough for a detailed view of the cove in every direction at once.
    fn default() -> Self {
        Self {
            geometry_bytes: 128 << 20,
            texture_bytes: 64 << 20,
        }
    }
}

/// GPU memory holding a scene's own data, in bytes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SceneMemory {
    /// Instances, materials, lights, hierarchy nodes and cluster records. Always resident.
    pub records: u64,
    /// The page pool.
    pub pages: u64,
    pub textures: u64,
}

impl SceneMemory {
    pub fn total(&self) -> u64 {
        self.records + self.pages + self.textures
    }
}

/// Measurements from a recent frame.
#[derive(Clone, Copy, Debug, Default)]
pub struct FrameStats {
    pub clusters: u32,
    pub software_clusters: u32,
    pub triangles: u32,
    pub instances: u32,
    pub source_triangles: u64,
    pub unique_triangles: u64,
    pub streaming: StreamingStats,
    pub particles: ParticleStats,
    /// What the most recently read back shadow cascade redraw selected. Counters take a few
    /// frames to arrive, and a redraw is skipped over while the last is still in flight.
    pub shadow_clusters: u32,
    pub shadow_triangles: u32,
    /// How many of those clusters went to the software rasterizer.
    pub shadow_software_clusters: u32,
    /// Clusters it drew as specks, besides those.
    pub shadow_specks: u32,
    /// Queue entries and clusters the main view, and that cascade, lost for want of room in
    /// their lists. Zero unless the scene has outgrown them, in which case the image, or the
    /// shadow, has holes.
    pub dropped: u32,
    pub shadow_dropped: u32,
}

/// Everything whose size depends on the resolution.
struct SizedResources {
    output_size: (u32, u32),
    render_size: (u32, u32),
    visibility: VisibilityBuffer,
    hzb: Hzb,
    gbuffer: GBuffer,
    cull_scene: wgpu::BindGroup,
    raster: RasterBindings,
    resolve: ResolveBindings,
    /// The [`crate::ruts::RutMap::generation`] whose texture `resolve` binds.
    ruts_generation: u32,
    screen_gi: ScreenGiBindings,
    clouds: CloudBindings,
    deferred: [wgpu::BindGroup; 2],
    /// The bindings of the pass that lists which lights reach each tile of the screen; the
    /// list itself is held by these and the deferred bind groups.
    light_tiles_group: wgpu::BindGroup,
    post: PostBindings,
    hdr: Target,
    /// Where the water pass draws, when there is a sea, or ponds.
    water: Option<WaterTargets>,
    /// The depth particles test against.
    particles: ParticleTargets,
    /// The scene's depth with any water in it, which cards test against.
    scene_depth: wgpu::TextureView,
}

/// Previous-frame camera state, for reprojection and occlusion culling.
#[derive(Clone, Copy)]
struct History {
    view: Mat4,
    view_proj: Mat4,
    projection: Mat4,
}

/// Renders a scene with virtualized geometry and global illumination.
pub struct Renderer {
    /// The lamps of whatever is driven, when they are on.
    lamp: Option<crate::Lamp>,
    /// What whatever is driven wears, and which of the scene's instances are it: the first
    /// of them as the GPU numbers them, and how many.
    coat: Option<(crate::Coat, u32, u32)>,
    /// Trucks that stand about, each in a colour of its own: the colour, the colour it was
    /// made in, and the instances that draw it.
    resprays: Vec<(Vec3, Vec3, Vec<u32>)>,
    /// The ruts tyres have pressed into the ground, round the camera, and the colour of
    /// the soil they churn up.
    ruts: crate::ruts::RutMap,
    rut_soil: Vec3,
    streamer: Streamer,
    settings: RendererSettings,
    scene: GpuScene,
    frame_uniform: wgpu::Buffer,
    view_slots: ViewSlots,
    cull: CullPipelines,
    main_work: CullWork,
    /// The editor's picks; idle, and free, unless one is asked for.
    picker: crate::pick::Picker,
    aux_work: CullWork,
    aux_cull_scene: wgpu::BindGroup,
    aux_raster: RasterBindings,
    rasterizer: Rasterizer,
    resolve: Resolve,
    /// Where the sea has been, for how wet surfaces are.
    waterline: Waterline,
    sky: Sky,
    clouds: Clouds,
    /// The lighting bind group as cloud passes see it: without the clouds they are making.
    cloud_environment: wgpu::BindGroup,
    shadows: ShadowMaps,
    volume: GiVolume,
    screen_gi: ScreenGi,
    environment: wgpu::BindGroup,
    deferred: DeferredLighting,
    light_tiles: LightTiles,
    light_shadows: LightShadows,
    post: PostProcess,
    profiler: Option<Profiler>,
    /// The open sea, if the scene has one, and an ocean waiting to be eased into, with the
    /// seconds to take. The sea is made on the next frame, which has the GPU to hand.
    sea: Option<Sea>,
    ocean_request: Option<(Option<Ocean>, f32)>,
    /// The calm sea's level, from the scene.
    sea_level: f32,
    /// Draws and shades the sea's surface, when there is a sea.
    water: Option<WaterSurface>,
    /// The scene's still water, and what shades it: made when there are ponds and no sea.
    ponds: Vec<Pond>,
    /// What the ponds' water is like, if the scene says: clear otherwise.
    pond_ocean: Option<Ocean>,
    pond_water: Option<PondWater>,
    /// Effects, and the pool their particles live in.
    particles: Particles,
    /// The wind near the ground, and the pass that fills its field for what sways: made only
    /// for a scene with something that does.
    wind: Wind,
    wind_field: Option<WindField>,
    /// Rain in the air, where it lands and what it leaves on surfaces.
    rain: Rain,
    /// Lightning, its bolts, and the thunder queued for the page.
    lightning: Lightning,
    bolt: BoltDraw,
    /// Counts the size-dependent resources made, so what binds them knows when they change.
    sized_generation: usize,
    /// The layout of the lighting bind group, for passes made after the renderer.
    environment_layout: wgpu::BindGroupLayout,
    sized: Option<SizedResources>,
    history: Option<History>,
    frame_index: u32,
    time: f32,
    /// Seconds since the previous frame.
    frame_seconds: f32,
    /// What is to be written over in installed pages before the next frame is drawn: the
    /// page, how far into it, and the bytes. See [`Self::rewrite_page`].
    rewrites: Vec<(u32, u32, Vec<u8>)>,
    /// Materials to be changed before the next frame is drawn. See [`Self::set_material`].
    recoloured: Vec<(u32, solstice_asset::Material)>,
}

impl Renderer {
    /// Creates a renderer for a scene held entirely in memory, with all of it on the GPU. It
    /// draws to textures of `output_format`, which must not be sRGB: the renderer applies its
    /// own display encoding.
    pub fn new(
        gpu: &Gpu,
        loaded: &LoadedScene,
        output_format: wgpu::TextureFormat,
        settings: RendererSettings,
    ) -> Self {
        let scene = GpuScene::new(
            gpu,
            &loaded.scene,
            loaded.scene.pages.len() as u32,
            Some(&loaded.upper_textures),
            None,
        );
        let mut streamer = Streamer::new(
            gpu,
            &loaded.scene,
            loaded.scene.pages.len() as u32,
            None,
            scene.regions,
        );
        for page in 0..loaded.scene.pages.len() as u32 {
            streamer.install_now(gpu, &scene, page, loaded.page(page), &[]);
        }
        streamer.flush(gpu, &scene);
        Self::from_parts(gpu, scene, streamer, output_format, settings)
            .with_sea(&loaded.scene.sea)
            .with_still_water(&loaded.scene.still_water)
    }

    /// Why [`Self::streaming`] can't draw this scene on this GPU within `budget`, if it can't:
    /// a message for the host to show, where `streaming` would panic.
    pub fn streaming_unsupported(
        gpu: &Gpu,
        paged: &PagedScene,
        budget: StreamingBudget,
    ) -> Option<String> {
        crate::regions::RegionPlan::unsupported(
            &gpu.limits,
            paged,
            streaming_slots(gpu, paged, budget),
        )
    }

    /// Creates a renderer that starts with only a scene's pinned pages and smallest textures,
    /// and draws with what it has while the host supplies the rest: see
    /// [`Self::stream_requests`]. GPU memory for the scene stays within `budget`, apart from
    /// what the scene can't do without: its records, pinned pages and smallest textures.
    pub fn streaming(
        gpu: &Gpu,
        paged: &PagedScene,
        output_format: wgpu::TextureFormat,
        settings: RendererSettings,
        budget: StreamingBudget,
    ) -> Self {
        let slots = streaming_slots(gpu, paged, budget);
        let plan = choose_tiers(gpu, paged, budget.texture_bytes);
        let scene = GpuScene::new(gpu, paged, slots, None, Some(plan));
        let mut streamer = Streamer::new(gpu, paged, slots, Some(plan), scene.regions);
        if paged.external.is_none() {
            for page in 0..paged.pinned_pages {
                streamer.install_now(gpu, &scene, page, paged.pinned_page(page), &[]);
            }
            streamer.flush(gpu, &scene);
        }
        Self::from_parts(gpu, scene, streamer, output_format, settings)
            .with_sea(&paged.sea)
            .with_still_water(&paged.still_water)
    }

    /// Writes over part of a page from the next frame on: `data` in place of what is
    /// `offset` bytes into it, both whole words. This is how a thing is made as the game is
    /// played: the scene holds a mesh for it, built with nothing coarser made from it
    /// (`solstice_builder::BuildOptions::flat`) so that it is all in pages that are always
    /// loaded, with bounds wide enough for whatever it will be, and its points are written
    /// here when they are known. Its triangles stay as they are. An instance of it should
    /// be movable ([`Self::set_instance_movable`]), so that it is drawn afresh each frame
    /// rather than kept in a view drawn before.
    pub fn rewrite_page(&mut self, page: u32, offset: u32, data: Vec<u8>) {
        self.rewrites.push((page, offset, data));
    }

    /// Makes one of the scene's materials this one from the next frame on. For materials
    /// with no texture alone: one with a texture is refused, and stays as it is.
    pub fn set_material(&mut self, index: u32, material: solstice_asset::Material) {
        self.recoloured.push((index, material));
    }

    /// For a scene that keeps its tables outside its manifest: takes a pinned page and its
    /// cluster records, in order. All must arrive before the first frame.
    pub fn install_pinned(&mut self, gpu: &Gpu, page: u32, data: &[u8], clusters: &[u8]) {
        self.streamer
            .install_now(gpu, &self.scene, page, data, clusters);
    }

    /// As [`Self::deliver`], for a page of such a scene: with its cluster records, which
    /// [`StreamRequest::clusters`] says where to find.
    pub fn deliver_page(&mut self, id: u32, data: Vec<u8>, clusters: Vec<u8>) -> bool {
        self.streamer.deliver_page(id, data, clusters)
    }

    /// Writes what the pinned pages changed. Call after the last of them.
    pub fn finish_external_tables(&mut self, gpu: &Gpu) {
        self.streamer.flush(gpu, &self.scene);
    }

    /// Gives the renderer the scene's sea, if it has one.
    fn with_sea(mut self, sea: &Option<solstice_asset::Sea>) -> Self {
        if let Some(sea) = sea {
            self.sea_level = sea.level;
            let ocean =
                Ocean::by_name(sea.ocean.trim()).map_or_else(|| Ocean::parse(&sea.ocean), Ok);
            let ocean = ocean.unwrap_or_else(|error| {
                log::warn!("the scene's ocean is unreadable ({error}); using coastal water");
                Ocean::default()
            });
            self.ocean_request = Some((Some(ocean), 0.0));
        }
        self
    }

    /// Gives the renderer the scene's ponds, if it has any, and the water they hold.
    fn with_still_water(mut self, still: &Option<solstice_asset::StillWater>) -> Self {
        if let Some(still) = still {
            self.ponds = still
                .ponds
                .iter()
                .map(|&[x0, z0, x1, z1, level]| Pond {
                    min: [x0.min(x1), z0.min(z1)],
                    max: [x0.max(x1), z0.max(z1)],
                    level,
                })
                .collect();
            let text = still.water.trim();
            if !text.is_empty() {
                match Ocean::by_name(text).map_or_else(|| Ocean::parse(&still.water), Ok) {
                    Ok(water) => self.pond_ocean = Some(water),
                    Err(error) => {
                        log::warn!("the scene's pond water is unreadable ({error}); using clear")
                    }
                }
            }
        }
        self
    }

    /// Sets the calm sea's level, in metres.
    pub fn set_sea_level(&mut self, level: f32) {
        self.sea_level = level;
    }

    /// Sets the scene's ponds: level sheets of still, clear water. A scene with an open sea
    /// draws the sea and not its ponds.
    pub fn set_ponds(&mut self, ponds: &[Pond]) {
        self.ponds = ponds.to_vec();
        // Made again with the new ponds on the next frame, which has the GPU to hand.
        self.pond_water = None;
    }

    /// Whether the frame has water over the lit scene, and whether that water is a sea.
    fn water_kind(&self) -> Option<bool> {
        (self.sea.is_some() || self.has_ponds()).then_some(self.sea.is_some())
    }

    /// Whether the scene's ponds are drawn: it has some, and no sea.
    fn has_ponds(&self) -> bool {
        self.sea.is_none() && !self.ponds.is_empty()
    }

    /// How many instances the scene has. Instances are named by their index in the scene file,
    /// from 0 to this.
    pub fn instance_count(&self) -> u32 {
        self.scene.instance_table.len() as u32
    }

    /// Moves an instance to `world` from the next frame on, as motion over the frame, which
    /// motion blur and temporal anti-aliasing follow. An instance that moves becomes movable, as
    /// [`Self::set_instance_movable`] makes it, if it was not already. Returns false, and does
    /// nothing, for an instance that has no geometry or does not exist.
    ///
    /// `world` must be invertible. To make something vanish, hide it with
    /// [`Self::set_instance_hidden`] rather than scaling it to nothing.
    pub fn move_instance(&mut self, instance: u32, world: Mat4) -> bool {
        self.scene
            .instance_table
            .set_transform(instance, world, false)
    }

    /// Puts an instance at `world` from the next frame on, as a jump rather than a motion, as
    /// for a respawn: it shows no motion getting there. Otherwise as [`Self::move_instance`].
    pub fn place_instance(&mut self, instance: u32, world: Mat4) -> bool {
        self.scene
            .instance_table
            .set_transform(instance, world, true)
    }

    /// Makes an instance movable, or static again. A movable instance is drawn afresh every
    /// frame, with its shadow drawn separately, and so costs more; a static one is drawn into
    /// views kept from frame to frame. Either change redraws those views once. Set an instance
    /// movable before it starts moving, for example when a scene loads, to have that happen at
    /// a quiet moment. A skinned instance can't be made static, and is refused.
    pub fn set_instance_movable(&mut self, instance: u32, movable: bool) -> bool {
        self.scene.instance_table.set_movable(instance, movable)
    }

    /// Keeps a movable instance in the GI voxels, where it stood when they were last drawn, or
    /// leaves it out (the default). Movable instances are left out because the voxels are kept
    /// from frame to frame, and would hold them where they were. Something that moves only a
    /// little, such as a ship riding at anchor, is better kept in: its shade and the light it
    /// bounces, and its reflection in the water, would otherwise be missing. A skinned instance
    /// is refused: the voxels are drawn without poses.
    pub fn set_instance_stays_in_gi(&mut self, instance: u32, stays: bool) -> bool {
        self.scene.instance_table.set_stays_in_gi(instance, stays)
    }

    /// Hides an instance, or shows it again.
    pub fn set_instance_hidden(&mut self, instance: u32, hidden: bool) -> bool {
        self.scene.instance_table.set_hidden(instance, hidden)
    }

    /// Below zero, it glows red, as brightly as it would have glowed above: a tail lamp, whose
    /// model has a white lamp behind what should be red glass.
    ///
    /// Scales how brightly an instance's material glows, as a lamp is lit or put out: 1 as the
    /// material says, 0 dark.
    pub fn set_instance_glow(&mut self, instance: u32, glow: f32) -> bool {
        self.scene.instance_table.set_glow(instance, glow)
    }

    /// Where an instance will be drawn next frame.
    pub fn instance_transform(&self, instance: u32) -> Option<Mat4> {
        self.scene.instance_table.transform(instance)
    }

    /// How many poses the scene can hold at once: one for each placed character.
    pub fn pose_count(&self) -> u32 {
        self.scene.skins.pose_count()
    }

    /// The skeleton of a pose.
    pub fn pose_skeleton(&self, pose: u32) -> Option<&solstice_asset::Skeleton> {
        self.scene.skins.skeleton(pose)
    }

    /// Poses a character from the next frame on: `bones` are its bones' places in the mesh's own
    /// space, in the skeleton's order, with no scale. Returns false, and does nothing, if the
    /// pose doesn't exist, or `bones` is not as long as its skeleton.
    pub fn set_pose(&mut self, pose: u32, bones: &[Mat4]) -> bool {
        self.scene.skins.set(pose, bones, false)
    }

    /// Poses a character as a jump rather than a motion, as for a respawn: it shows no motion
    /// getting there. Otherwise as [`Self::set_pose`].
    pub fn place_pose(&mut self, pose: u32, bones: &[Mat4]) -> bool {
        self.scene.skins.set(pose, bones, true)
    }

    /// Asks what is under a point of the screen, as fractions of its width and height from the
    /// top left, on the next frame. [`Self::take_pick`] gives the answer a frame or two later.
    pub fn request_pick(&mut self, x: f32, y: f32) {
        self.picker.request((x, y));
    }

    /// What the last pick found, once it has been read back.
    pub fn take_pick(&mut self) -> Option<crate::pick::Picked> {
        self.picker.take()
    }

    /// An instance's bounding sphere in the world, as its centre and radius.
    pub fn instance_bounds(&self, instance: u32) -> Option<(Vec3, f32)> {
        self.scene.instance_table.world_sphere(instance)
    }

    fn from_parts(
        gpu: &Gpu,
        scene: GpuScene,
        streamer: Streamer,
        output_format: wgpu::TextureFormat,
        settings: RendererSettings,
    ) -> Self {
        let frame_uniform = gpu.uniform_buffer("frame", std::mem::size_of::<FrameUniform>() as u64);
        let mut view_slots = ViewSlots::new(gpu, SLOT_COUNT);
        let hold = streamer.hold_bases();
        view_slots.hold_requests = [hold[HOLD_SHADOWS], hold[HOLD_VOXELS]];

        let cull = CullPipelines::new(gpu, scene.deform(), scene.regions);
        let masked_capacity = if scene.has_masked {
            MASKED_VISIBLE_CAPACITY
        } else {
            0
        };
        let main_work = CullWork::new(
            gpu,
            &cull,
            "main cull",
            MAIN_QUEUE_CAPACITY,
            MAIN_VISIBLE_CAPACITY,
            masked_capacity,
            streamer.requests_buffer(),
        );
        // A scene too large for one binding has more in its farthest shadow cascade, at its
        // coarsest, than the usual lists hold, and what they drop leaves holes in the shadows.
        let aux_scale = if scene.regions.is_some() { 2 } else { 1 };
        let aux_work = CullWork::new(
            gpu,
            &cull,
            "aux cull",
            AUX_QUEUE_CAPACITY * aux_scale,
            AUX_VISIBLE_CAPACITY * aux_scale,
            masked_capacity,
            streamer.requests_buffer(),
        );
        let rasterizer = Rasterizer::new(gpu, scene.has_masked, scene.deform(), scene.regions);

        // Views without occlusion culling never read the pyramid, but a binding is required.
        let no_hzb = Target::new(
            gpu,
            "no hzb",
            (1, 1),
            wgpu::TextureFormat::R32Float,
            wgpu::TextureUsages::empty(),
        );
        let aux_cull_scene = cull.scene_bind_group(gpu, &view_slots, &scene, &no_hzb.view);
        let aux_raster = rasterizer.bindings(gpu, &view_slots, &scene, &aux_work);

        let sky = Sky::new(gpu);
        let shadows = ShadowMaps::new(gpu, &rasterizer);
        let waterline = Waterline::new(gpu);
        let environment_layout = environment_layout(gpu);
        let light_shadows = LightShadows::new(gpu, &environment_layout, &scene);
        let volume = GiVolume::new(
            gpu,
            &scene,
            &aux_work,
            &environment_layout,
            &light_shadows.view,
        );
        let mut clouds = Clouds::new(gpu, &environment_layout);
        clouds.set_enabled(settings.clouds);
        let environment = environment_group(
            gpu,
            &environment_layout,
            &frame_uniform,
            &volume,
            &sky,
            clouds.sky(),
        );
        let cloud_environment = environment_group(
            gpu,
            &environment_layout,
            &frame_uniform,
            &volume,
            &sky,
            clouds.placeholders(),
        );

        let particles = Particles::new(
            gpu,
            &frame_uniform,
            settings.particle_pool,
            &environment_layout,
            waterline.resolve_layout(),
            LightInputs {
                lights: scene.lights.clone(),
                shadow_uniform: shadows.uniform.clone(),
                shadow_maps: shadows.array_view.clone(),
                shadow_sampler: shadows.comparison_sampler.clone(),
            },
        );
        let wind_field = scene
            .has_sways
            .then(|| WindField::new(gpu, &scene.wind_field));
        let rain = Rain::new(
            gpu,
            &rasterizer,
            &environment_layout,
            crate::particles::COLOR_FORMAT,
            crate::particles::REACTIVE_FORMAT,
        );
        let resolve = Resolve::new(
            gpu,
            waterline.resolve_layout(),
            &rain.surface_layout,
            scene.deform(),
            scene.regions,
            scene.glow_maps.is_some(),
        );
        Self {
            settings,
            wind: Wind::new(),
            wind_field,
            rain,
            lightning: Lightning::default(),
            bolt: BoltDraw::new(gpu, crate::particles::COLOR_FORMAT),
            sized_generation: 0,
            resolve,
            waterline,
            screen_gi: ScreenGi::new(gpu, &environment_layout),
            deferred: DeferredLighting::new(gpu, &environment_layout),
            light_tiles: LightTiles::new(gpu, &environment_layout),
            light_shadows,
            post: PostProcess::new(gpu, output_format),
            particles,
            profiler: Profiler::new(gpu),
            scene,
            lamp: None,
            coat: None,
            resprays: Vec::new(),
            ruts: crate::ruts::RutMap::new(gpu),
            rut_soil: Vec3::new(0.33, 0.15, 0.075),
            streamer,
            frame_uniform,
            view_slots,
            cull,
            main_work,
            picker: Default::default(),
            aux_work,
            aux_cull_scene,
            aux_raster,
            rasterizer,
            sky,
            clouds,
            cloud_environment,
            shadows,
            volume,
            environment,
            sea: None,
            ocean_request: None,
            sea_level: 0.0,
            water: None,
            ponds: Vec::new(),
            pond_ocean: None,
            pond_water: None,
            environment_layout,
            sized: None,
            history: None,
            frame_index: 0,
            time: 0.0,
            frame_seconds: 1.0 / 60.0,
            rewrites: Vec::new(),
            recoloured: Vec::new(),
        }
    }

    /// The ranges of the scene file to start fetching now, most wanted first. Each must later
    /// be handed to [`Self::deliver`] or [`Self::request_failed`]. `max_fetching` caps the
    /// pages outstanding, and a few texture levels may come on top.
    pub fn stream_requests(&mut self, max_fetching: u32) -> Vec<StreamRequest> {
        self.streamer.take_fetches(max_fetching)
    }

    /// Takes the bytes of a request, unpacked if it was deflated. They reach the GPU within a
    /// few frames. Returns false if they aren't wanted any more or are the wrong size.
    pub fn deliver(&mut self, id: u32, data: Vec<u8>) -> bool {
        self.streamer.deliver_request(id, data)
    }

    /// As [`Self::deliver`], for bytes just as a scene file stores them, inflated if they were
    /// deflated. A host reading a file hands them over with this: a stored page has its vertices
    /// filtered to deflate smaller, which this undoes. [`Self::deliver`] takes a page as the GPU
    /// reads it, as a scene held in memory has them.
    pub fn deliver_fetched(&mut self, id: u32, data: Vec<u8>) -> bool {
        self.streamer.deliver_fetched(id, data, Vec::new())
    }

    /// As [`Self::deliver_page`], for a page just as a scene file stores it, inflated.
    pub fn deliver_fetched_page(&mut self, id: u32, data: Vec<u8>, clusters: Vec<u8>) -> bool {
        self.streamer.deliver_fetched(id, data, clusters)
    }

    /// Reports that a request could not be fetched, so what it holds may be requested again.
    pub fn request_failed(&mut self, id: u32) {
        self.streamer.request_failed(id);
    }

    /// Bytes of GPU memory holding this scene's own data, not counting render targets.
    pub fn scene_bytes(&self) -> SceneMemory {
        let buffer_bytes = self.scene.instances.size()
            + self.scene.nodes.size()
            + self.scene.geometry_bytes().0
            + self.scene.materials.size()
            + self.scene.lights.size();
        SceneMemory {
            records: buffer_bytes,
            pages: self.scene.geometry_bytes().1,
            textures: self.scene.textures.bytes,
        }
    }

    /// How the scene's textures ended up on the GPU, for logs and on-screen readouts.
    pub fn texture_summary(&self) -> &str {
        &self.scene.texture_summary
    }

    pub fn settings_mut(&mut self) -> &mut RendererSettings {
        &mut self.settings
    }

    /// The effects running, to start, steer and stop them.
    pub fn particles(&mut self) -> &mut Particles {
        &mut self.particles
    }

    /// The scene's depth and motion as the temporal filter reads them, for tests that check
    /// what was drawn into them.
    #[cfg(test)]
    pub(crate) fn reactive_mask(&self) -> Option<&wgpu::Texture> {
        Some(self.sized.as_ref()?.particles.reactive_texture())
    }

    /// The scene's depth and motion as the temporal filter reads them, for tests that check
    /// what was drawn into them.
    #[cfg(test)]
    pub(crate) fn scene_depth_and_motion(&self) -> Option<(&wgpu::Texture, &wgpu::Texture)> {
        let sized = self.sized.as_ref()?;
        Some(match &sized.water {
            Some(water) => (&water.scene_depth.texture, &water.motion.texture),
            None => (&sized.gbuffer.depth.texture, &sized.gbuffer.motion.texture),
        })
    }

    /// Gives the scene an open sea, easing from the current one over `seconds`, or at once for
    /// 0; `None` takes the sea away. Its waves follow the weather's wind.
    pub fn set_ocean(&mut self, ocean: Option<Ocean>, seconds: f32) {
        self.ocean_request = Some((ocean, seconds));
    }

    /// The ocean as it is now, part way to any new one, or `None` without a sea.
    pub fn ocean(&self) -> Option<Ocean> {
        match (self.ocean_request, &self.sea) {
            // A change not yet begun: none, or one that is at once, or the first sea, is already
            // as good as made; one that eases has not moved yet.
            (Some((None, _)), _) => None,
            (Some((Some(ocean), seconds)), sea) if sea.is_none() || seconds <= 0.0 => Some(ocean),
            (_, sea) => sea.as_ref().map(Sea::ocean),
        }
    }

    /// The wind and swell the waves are drawn from now, which lag the weather's wind.
    pub fn sea_state(&self) -> Option<SeaState> {
        self.sea.as_ref().and_then(Sea::state)
    }

    /// Current counters. They lag a few frames, since readback is asynchronous.
    /// Changes the weather, easing into it over about `seconds`, or at once for 0. Clouds
    /// drift with its wind and change shape as they go.
    pub fn set_weather(&mut self, weather: Weather, seconds: f32) {
        self.clouds.set_weather(weather, seconds);
    }

    /// Lets the weather change by itself, starting from the named weather, or stops it: the
    /// sky drifts, grows and clears through the day.
    pub fn set_weather_changing(&mut self, changing: bool, start: &str) {
        self.clouds.set_changing(changing, start);
    }

    /// Moves the weather on by `seconds` at once, as if that much time had passed: for tools,
    /// tests, and catching up after a pause.
    pub fn advance_weather(&mut self, seconds: f32) {
        self.clouds.advance(seconds);
        // What the rain has left behind, as if it had been falling all that while.
        self.rain
            .advance(&self.clouds.weather(), &self.wind, seconds);
    }

    /// The weather now, part way to its target while it changes.
    /// Thunder from strikes since the last call, each with how long until it is heard: the page
    /// sounds it (`lightning.rs`).
    pub fn take_thunder(&mut self) -> Vec<Thunder> {
        self.lightning.take_thunder()
    }

    /// Makes lightning strike on the next frame, whatever the weather.
    pub fn strike_lightning(&mut self) {
        self.lightning.strike_soon();
    }

    /// The wind near the ground, as it is this frame.
    pub fn wind(&self) -> &Wind {
        &self.wind
    }

    pub fn weather(&self) -> Weather {
        self.clouds.weather()
    }

    /// Says what whatever is driven wears from now on (see [`crate::Coat`]), and which of
    /// the scene's instances are it; or, with `None`, that nothing wears anything. The
    /// instances are taken as one run of the scene's: a car's are, as a bake places them.
    /// Set each frame the car moves, as its lamp is.
    pub fn set_coat(&mut self, coat: Option<(crate::Coat, &[u32])>) {
        self.coat = coat.and_then(|(coat, instances)| {
            let numbered = instances
                .iter()
                .filter_map(|&instance| self.scene.instance_table.gpu_number(instance));
            let (first, last) = numbered.fold((u32::MAX, 0), |(first, last), number| {
                (first.min(number), last.max(number))
            });
            (first <= last).then(|| (coat, first, last - first + 1))
        });
    }

    /// Says which trucks stand about in colours of their own: for each, its colour, the
    /// colour its paint was made in, and the instances that draw it, which are taken as one
    /// run of the scene's. Only the first [`crate::frame::RESPRAYS`] of them are painted.
    /// It holds until it is set again.
    pub fn set_resprays(&mut self, resprays: &[(Vec3, Vec3, Vec<u32>)]) {
        self.resprays = resprays.to_vec();
    }

    /// Brings the ruts that are drawn up to date with the map of them that `source` keeps
    /// (see [`crate::ruts`]), round `centre`: the world's x and z of what is looked at. Call
    /// each frame something may have been driven; on ground nobody has pressed it does
    /// nothing. Returns how many squares of the map were copied to the GPU.
    pub fn sync_ruts(&mut self, centre: glam::Vec2, source: &dyn crate::ruts::RutSource) -> usize {
        self.ruts.sync(centre, source)
    }

    /// Says what colour the soil is that tyres churn up: red clay, unless told.
    pub fn set_rut_soil(&mut self, soil: Vec3) {
        self.rut_soil = soil;
    }

    /// Turns on the lamps of whatever is driven, where they are now, or turns them off. Set
    /// each frame as the car moves.
    pub fn set_lamp(&mut self, lamp: Option<crate::Lamp>) {
        self.lamp = lamp;
    }

    /// What the rain has done so far: how hard it falls now, how wet it has left what is open
    /// to the sky, and how full the puddles are. For a game whose ground the rain changes.
    pub fn rain_state(&self) -> crate::rain::RainState {
        self.rain.state
    }

    /// Whether this scene has cut-out materials, and so the passes that draw them.
    pub fn draws_masked(&self) -> bool {
        self.rasterizer.draws_masked()
    }

    pub fn stats(&self) -> FrameStats {
        let stats = self.main_work.stats();
        FrameStats {
            clusters: stats.clusters(),
            software_clusters: stats.software_clusters(),
            triangles: stats.triangles(),
            instances: self.scene.gpu_instance_count,
            source_triangles: self.scene.source_triangles,
            unique_triangles: self.scene.unique_triangles,
            streaming: self.streamer.stats(),
            particles: self.particles.stats(),
            shadow_clusters: self.aux_work.stats().clusters(),
            shadow_software_clusters: self.aux_work.stats().software_clusters(),
            shadow_specks: self.aux_work.stats().specks(),
            shadow_triangles: self.aux_work.stats().triangles(),
            dropped: self.main_work.stats().dropped(),
            shadow_dropped: self.aux_work.stats().dropped(),
        }
    }

    /// GPU milliseconds per entry of [`SECTIONS`], where the platform can measure them.
    pub fn section_times(&self) -> Option<[f32; SECTIONS.len()]> {
        self.profiler.as_ref().and_then(Profiler::latest_ms)
    }

    /// Renders one frame to `output`, a view of a texture of the size given.
    pub fn render(
        &mut self,
        gpu: &Gpu,
        camera: &Camera,
        environment: &Environment,
        output: &wgpu::TextureView,
        output_size: (u32, u32),
        delta_seconds: f32,
    ) {
        let scale = self.settings.render_scale;
        let render_size = (
            ((output_size.0 as f32 * scale) as u32).max(16),
            ((output_size.1 as f32 * scale) as u32).max(16),
        );
        // The sea first: whether there is one decides what the frame's textures are.
        if let Some((ocean, seconds)) = self.ocean_request.take() {
            match (ocean, &mut self.sea) {
                (None, _) => self.sea = None,
                (Some(ocean), Some(sea)) => sea.set_ocean(ocean, seconds),
                (Some(ocean), None) => {
                    self.sea = Some(Sea::new(gpu, ocean, self.settings.ocean_resolution))
                }
            }
        }
        let cells = self.settings.ocean_patch_cells;
        if self.sea.is_none() {
            self.water = None;
        } else if self.water.as_ref().is_none_or(|w| w.cells() != cells) {
            self.water = Some(WaterSurface::new(
                gpu,
                &self.environment_layout,
                &self.rasterizer,
                cells,
            ));
        }
        if !self.has_ponds() {
            self.pond_water = None;
        } else if self.pond_water.is_none() {
            let water = PondWater::new(gpu, &self.environment_layout);
            let clear = self
                .pond_ocean
                .unwrap_or_else(|| Ocean::by_name("clear").unwrap_or_default());
            water.set(gpu, &self.ponds, &clear);
            self.pond_water = Some(water);
        }
        self.ensure_sized(gpu, render_size, output_size);
        self.time += delta_seconds;
        self.frame_seconds = delta_seconds.clamp(1.0 / 1000.0, 0.25);
        self.particles.resize(gpu, self.settings.particle_pool);
        self.particles
            .set_light_interval(self.settings.particle_light_interval);
        self.particles
            .set_ray_steps(self.settings.particle_ray_steps);
        // The weather's wind is at cloud height; near the ground it is about half that, as the
        // sea takes it, measured from the sea's level where there is a sea.
        let [east, south] = self.clouds.weather().wind_velocity();
        let floor = if self.sea.is_some() {
            self.sea_level
        } else {
            0.0
        };
        self.particles.set_air([east * 0.5, south * 0.5], floor);
        // Particles take the frame's own time, so a host that pauses by passing 0 holds them
        // still, where the passes that filter over time need a step above zero.
        self.particles.prepare(
            gpu,
            camera,
            delta_seconds.clamp(0.0, 0.25),
            self.settings.particle_detail,
        );

        // Everything below draws what's installed from here on, with instances where they are
        // this frame.
        for (page, offset, data) in std::mem::take(&mut self.rewrites) {
            if !self.streamer.rewrite(gpu, &self.scene, page, offset, &data) {
                log::warn!("page {page} could not be written over at {offset}: it is not installed, or is shorter");
            }
        }
        for (index, material) in std::mem::take(&mut self.recoloured) {
            if !self
                .streamer
                .set_material(gpu, &self.scene, index, material)
            {
                log::warn!("material {index} could not be changed: it has a texture, or there is none such");
            }
        }
        self.streamer.update(gpu, &self.scene, delta_seconds);
        let changes = self.scene.update_instances(gpu);
        let streamed = self.streamer.should_refresh_cached_views();
        let shadows_streamed = streamed & (1 << HOLD_SHADOWS) != 0;
        if shadows_streamed {
            self.shadows.mark_stale();
        }
        if changes.shadows_changed {
            self.shadows.mark_stale_now();
        }
        if shadows_streamed || changes.shadows_changed {
            if let Some(water) = &mut self.water {
                water.seabed.mark_stale();
            }
            self.rain.mark_stale();
        }
        if streamed & (1 << HOLD_VOXELS) != 0 || changes.gi_changed {
            self.volume.mark_stale();
        }

        let aspect = render_size.0 as f32 / render_size.1 as f32;
        let parity = (self.frame_index & 1) as usize;

        self.write_frame_data(gpu, camera, environment, render_size);
        self.shadows.speck_texels = self.settings.speck_texels;
        self.volume.speck_voxels = self.settings.speck_texels;
        let shadow_work = self.shadows.prepare(
            gpu,
            &self.view_slots,
            SLOT_SHADOW,
            SLOT_MOVING_SHADOW,
            camera,
            aspect,
            environment.sun_direction,
            self.settings.shadow_lod_threshold,
            0.5 * render_size.1 as f32 * camera.projection(aspect).y_axis.y,
            &self.scene.instance_table.moving_spheres(),
        );
        // A cascade and a level of voxels each cost several times a frame to redraw: not both
        // on one frame.
        self.volume.hold_off = shadow_work.due.contains(&true);
        let volume_work = self
            .volume
            .prepare(gpu, &self.view_slots, SLOT_VOXEL, camera.position);
        self.waterline.prepare(
            gpu,
            camera.position,
            delta_seconds,
            self.sea.is_some() && self.water.is_some(),
        );
        let seabed_due = self.water.as_mut().is_some_and(|water| {
            water.seabed.prepare(
                gpu,
                &self.view_slots,
                SLOT_SEABED,
                camera.position,
                self.sea_level,
                self.settings.shadow_lod_threshold,
            )
        });

        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame"),
            });

        self.streamer.begin_frame(gpu, &self.scene, &mut encoder);

        mark(&self.profiler, &mut encoder, section::SKY);
        self.sky.record(
            gpu,
            &mut encoder,
            environment.sun_direction,
            camera.position.y,
        );

        // Clouds come before anything lit: their shadows and their light in the sky feed it all.
        mark(&self.profiler, &mut encoder, section::CLOUDS);
        // A flash of lightning lights the clouds, the sky and everything under them.
        self.lightning.advance_facing(
            &self.clouds.weather(),
            camera.position,
            camera.forward(),
            delta_seconds.clamp(0.0, 0.25),
        );
        // Against the light already about: a flash by night lights the scene brilliantly, but
        // not to a white sheet, as the eye sees it, and by day it is a sharp flicker.
        let weather = self.clouds.weather();
        let open = 1.0 - 0.8 * weather.coverage;
        let about = (environment.sun_illuminance * environment.sun_direction.y.max(0.05) * open)
            .max(environment.daylight() * NIGHT_LIGHT_SHARE);
        let flash = self.lightning.flash(camera.position).map(|mut flash| {
            flash.illuminance = flash.illuminance.min(FLASH_OVER_SCENE * about);
            flash
        });
        self.clouds.set_flash(flash);
        self.post.hold_exposure = self.lightning.holds_exposure();
        self.clouds.update(gpu, delta_seconds, camera.position);
        self.clouds
            .record_lighting(&mut encoder, &self.cloud_environment);

        // The sea's waves, raised by the weather's wind.
        mark(&self.profiler, &mut encoder, section::OCEAN);
        if let Some(sea) = &mut self.sea {
            let weather = self.clouds.weather();
            sea.update(gpu, &weather, delta_seconds, self.settings.ocean_resolution);
            sea.record(&mut encoder);
        }

        // The wind near the ground, before anything that sways in it is drawn.
        mark(&self.profiler, &mut encoder, section::WIND);
        self.wind
            .advance(&self.clouds.weather(), delta_seconds.clamp(0.0, 0.25));
        if let Some(field) = &self.wind_field {
            let params = FieldParams::around(
                Vec2::new(camera.position.x, camera.position.z),
                &self.wind,
                true,
                self.settings.sway_fade,
            );
            field.record(gpu, &mut encoder, &params);
        }
        // The rain moves on with the weather and the wind.
        self.rain.quality = self.settings.rain;
        let weather = self.clouds.weather();
        self.rain
            .advance(&weather, &self.wind, delta_seconds.clamp(0.0, 0.25));
        let cover_due = self.rain.prepare(
            gpu,
            &self.view_slots,
            SLOT_COVER,
            camera.position,
            &self.wind,
            &weather,
            self.settings.shadow_lod_threshold,
            self.frame_seconds,
        );

        mark(&self.profiler, &mut encoder, section::SHADOWS);
        self.shadows.record(
            &mut encoder,
            &self.cull,
            &self.rasterizer,
            &self.scene,
            &self.aux_cull_scene,
            &self.aux_work,
            &self.aux_raster,
            SLOT_SHADOW,
            SLOT_MOVING_SHADOW,
            &shadow_work,
        );
        // The seabed and voxel views share this work, so the counters are read now, while
        // they are the last cascade's.
        if shadow_work.due.iter().any(|&due| due) {
            self.aux_work.capture_stats(&mut encoder);
        }

        if let Some(water) = self.water.as_ref().filter(|_| seabed_due) {
            water.seabed.record(
                &mut encoder,
                &self.cull,
                &self.scene,
                &self.aux_cull_scene,
                &self.aux_work,
                &self.aux_raster,
                SLOT_SEABED,
            );
        }
        // Where the sea stands and has stood, from the waves over the seabed, before the
        // surfaces it wets are resolved.
        mark(&self.profiler, &mut encoder, section::WATERLINE);
        if let (Some(sea), Some(water)) = (&self.sea, &self.water) {
            let waves = sea.waves();
            self.waterline.record(
                gpu,
                &mut encoder,
                SeaViews {
                    key: water.id() ^ (waves.generation() << 32),
                    surface: water.uniform(),
                    displacement: &waves.displacement.view,
                    sampler: water.wave_sampler(),
                    seabed: &water.seabed.view,
                },
            );
        }

        // Where the rain lands, its light and its ripples, after the shadows that light it and
        // before the surfaces it wets are resolved.
        mark(&self.profiler, &mut encoder, section::RAIN);
        if cover_due {
            self.rain.record_maps(
                &mut encoder,
                &self.cull,
                &self.scene,
                &self.aux_cull_scene,
                &self.aux_work,
                &self.aux_raster,
                SLOT_COVER,
            );
        }
        self.rain.record_light(
            gpu,
            &mut encoder,
            &self.environment,
            &crate::rain::LightInputs {
                shadow_uniform: &self.shadows.uniform,
                shadow_maps: &self.shadows.array_view,
                shadow_sampler: &self.shadows.comparison_sampler,
                lights: &self.scene.lights,
            },
        );

        self.volume.record(
            &mut encoder,
            &self.cull,
            &self.scene,
            &self.aux_cull_scene,
            &self.aux_work,
            &self.environment,
            &self.light_shadows,
            SLOT_VOXEL,
            &volume_work,
            self.profiler.as_ref(),
        );

        let sized = self.sized.as_ref().expect("resources were just created");

        // Geometry: two occlusion phases into one visibility buffer, then its G-buffer.
        sized.visibility.clear(&mut encoder);
        for (phase, slot, cull_section, raster_section) in [
            (
                CullPhase::First,
                SLOT_MAIN_FIRST,
                section::CULL_FIRST,
                section::RASTER_FIRST,
            ),
            (
                CullPhase::Second,
                SLOT_MAIN_SECOND,
                section::CULL_SECOND,
                section::RASTER_SECOND,
            ),
        ] {
            mark(&self.profiler, &mut encoder, cull_section);
            self.cull.record(
                &mut encoder,
                &self.scene,
                &sized.cull_scene,
                slot,
                &self.main_work,
                phase,
            );
            mark(&self.profiler, &mut encoder, raster_section);
            self.rasterizer.record(
                &mut encoder,
                &sized.raster,
                slot,
                &self.main_work,
                &sized.visibility,
            );
            sized.hzb.record(&mut encoder);
        }
        mark(&self.profiler, &mut encoder, section::RESOLVE);
        self.resolve.record(
            &mut encoder,
            &sized.resolve,
            self.waterline.resolve_group(),
            &self.rain.surface_group,
            render_size,
        );
        self.picker.record(
            gpu,
            &mut encoder,
            &sized.visibility.depth,
            &sized.visibility.ids,
            &self.main_work.visible,
            render_size,
        );

        // Lighting.
        mark(&self.profiler, &mut encoder, section::CLOUDS_ON_SCREEN);
        self.clouds
            .record_screen(&mut encoder, &sized.clouds, &self.cloud_environment, parity);

        mark(&self.profiler, &mut encoder, section::SCREEN_GI);
        self.screen_gi
            .record(&mut encoder, &sized.screen_gi, &self.environment, parity);
        mark(&self.profiler, &mut encoder, section::LIGHTING);
        if self.scene.light_count > 0 {
            self.light_tiles.record(
                &mut encoder,
                &sized.light_tiles_group,
                &self.environment,
                render_size,
            );
        }
        self.deferred.record(
            &mut encoder,
            &sized.deferred[parity],
            &self.environment,
            render_size,
        );

        // The sea's surface, over the lit scene.
        mark(&self.profiler, &mut encoder, section::WATER);
        if let (Some(sea), Some(water), Some(targets)) = (&self.sea, &mut self.water, &sized.water)
        {
            water.prepare(
                gpu,
                camera.position,
                self.sea_level,
                &sea.ocean(),
                sea.waves(),
                sea.slopes(),
                self.wind.now(),
            );
            water.record(
                gpu,
                &mut encoder,
                &self.frame_uniform,
                &self.environment,
                sea.waves(),
                targets,
                SceneViews {
                    lit: &sized.hdr.view,
                    depth: &sized.gbuffer.depth.view,
                    motion: &sized.gbuffer.motion.view,
                    depth_pyramid: sized.screen_gi.depth_pyramid_view(),
                    normals: &sized.gbuffer.normal.view,
                },
                &self.shadows,
            );
        } else if let (Some(ponds), Some(targets)) = (&mut self.pond_water, &sized.water) {
            ponds.set_wind(gpu, self.wind.now());
            ponds.set_rays(gpu, self.settings.pond_reflection_rays);
            ponds.record(
                gpu,
                &mut encoder,
                &self.environment,
                targets,
                SceneViews {
                    lit: &sized.hdr.view,
                    depth: &sized.gbuffer.depth.view,
                    motion: &sized.gbuffer.motion.view,
                    depth_pyramid: sized.screen_gi.depth_pyramid_view(),
                    normals: &sized.gbuffer.normal.view,
                },
                &self.shadows,
            );
        }

        // Particles, over the lit scene and the water.
        mark(&self.profiler, &mut encoder, section::PARTICLES);
        self.particles.record_simulation(&mut encoder);
        self.particles.record_collide(
            &mut encoder,
            &self.environment,
            &sized.particles,
            self.waterline.resolve_group(),
        );
        self.particles.record_light(&mut encoder, &self.environment);
        self.particles.record_sort(&mut encoder);
        mark(&self.profiler, &mut encoder, section::PARTICLE_DRAW);
        let debug = self.settings.debug_view;
        let scene_color = sized
            .water
            .as_ref()
            .map_or(&sized.hdr.view, |water| &water.color.view);
        if let Some(cards) = self
            .scene
            .cards
            .as_ref()
            .filter(|_| debug == DebugView::Lit)
        {
            cards.record(
                gpu,
                &mut encoder,
                scene_color,
                &sized.scene_depth,
                &self.frame_uniform,
                camera.position,
            );
        }
        self.particles.record_draw(
            &mut encoder,
            scene_color,
            &sized.particles,
            debug == DebugView::Particles,
        );

        // Rain, over everything, as the particles are.
        mark(&self.profiler, &mut encoder, section::RAIN_DRAW);
        if self.rain.draws() && debug == DebugView::Lit {
            self.rain.record_draw(
                gpu,
                &mut encoder,
                &self.frame_uniform,
                scene_color,
                sized.particles.reactive(),
                false,
                &sized.scene_depth,
                self.sized_generation,
            );
            self.particles.mark_reactive_dirty();
        }
        // A bolt, over everything.
        if let Some(strike) = self.lightning.strike().filter(|_| debug == DebugView::Lit) {
            self.bolt.record(
                gpu,
                &mut encoder,
                &self.frame_uniform,
                strike,
                scene_color,
                &sized.scene_depth,
                self.sized_generation,
            );
        }

        // From the lit scene to the display.
        mark(&self.profiler, &mut encoder, section::POST);
        let passthrough = !matches!(
            debug,
            DebugView::Lit | DebugView::DiffuseGi | DebugView::Reflections | DebugView::Voxels
        );
        self.post.record(
            gpu,
            &mut encoder,
            &sized.post,
            output,
            parity,
            self.frame_index,
            delta_seconds,
            environment.daylight(),
            environment.day(),
            passthrough,
        );

        mark(&self.profiler, &mut encoder, section::END);
        if let Some(profiler) = &mut self.profiler {
            profiler.resolve(&mut encoder);
        }
        self.main_work.capture_stats(&mut encoder);
        self.particles.capture_stats(&mut encoder);
        self.streamer.capture_requests(&mut encoder);
        gpu.queue.submit([encoder.finish()]);
        self.main_work.map_stats();
        self.particles.map_stats();
        self.aux_work.map_stats();
        self.streamer.map_requests();
        self.picker.request_map();
        if let Some(profiler) = &mut self.profiler {
            profiler.request_map();
        }
        let _ = gpu.device.poll(wgpu::PollType::Poll);
        self.frame_index = self.frame_index.wrapping_add(1);
    }

    fn ensure_sized(&mut self, gpu: &Gpu, render_size: (u32, u32), output_size: (u32, u32)) {
        let unchanged = self.sized.as_ref().is_some_and(|s| {
            s.render_size == render_size
                && s.output_size == output_size
                && s.screen_gi.probe_rays() == self.settings.probe_rays
                && s.water.as_ref().map(WaterTargets::for_sea) == self.water_kind()
        });
        if unchanged {
            // The ruts' texture is made whole at the first press: only the resolve pass,
            // which reads it, is bound again.
            let generation = self.ruts.generation();
            if let Some(sized) = self
                .sized
                .as_mut()
                .filter(|sized| sized.ruts_generation != generation)
            {
                sized.resolve = self.resolve.bindings(
                    gpu,
                    &self.frame_uniform,
                    &self.scene,
                    &sized.visibility,
                    &self.main_work,
                    &sized.gbuffer,
                    &self.ruts.view,
                );
                sized.ruts_generation = generation;
            }
            return;
        }
        let visibility = VisibilityBuffer::new(gpu, &self.rasterizer, render_size);
        let hzb = Hzb::new(gpu, &visibility);
        let gbuffer = GBuffer::new(gpu, render_size);
        // Particles are drawn into whichever of this and the water's colour post reads.
        let hdr = Target::new(
            gpu,
            "hdr",
            render_size,
            HDR_FORMAT,
            wgpu::TextureUsages::RENDER_ATTACHMENT,
        );
        let cull_scene = self
            .cull
            .scene_bind_group(gpu, &self.view_slots, &self.scene, &hzb.view);
        let raster = self
            .rasterizer
            .bindings(gpu, &self.view_slots, &self.scene, &self.main_work);
        let resolve = self.resolve.bindings(
            gpu,
            &self.frame_uniform,
            &self.scene,
            &visibility,
            &self.main_work,
            &gbuffer,
            &self.ruts.view,
        );

        let history = PostProcess::history_targets(gpu, output_size);
        // On each frame the GI passes read the history written on the frame before, which is
        // the other half of the pair.
        let screen_gi = self.screen_gi.bindings(
            gpu,
            render_size,
            self.settings.probe_rays,
            &gbuffer,
            [&history[1].view, &history[0].view],
        );
        let clouds = self.clouds.bindings(gpu, render_size, &gbuffer.depth.view);
        let light_tiles = LightTiles::buffer(gpu, render_size);
        let light_tiles_group =
            self.light_tiles
                .bind_group(gpu, &gbuffer.depth.view, &self.scene, &light_tiles);
        let deferred = self.deferred.bind_groups(
            gpu,
            &gbuffer,
            &screen_gi,
            &hdr.view,
            &self.shadows,
            &self.scene,
            &clouds,
            &light_tiles,
            &self.light_shadows.view,
        );
        let water = self
            .water_kind()
            .map(|sea| WaterTargets::new(gpu, render_size, sea));
        let (scene_color, scene_depth, scene_motion) = match &water {
            Some(water) => (
                &water.color.view,
                &water.scene_depth.view,
                &water.motion.view,
            ),
            None => (&hdr.view, &gbuffer.depth.view, &gbuffer.motion.view),
        };
        let particles = self.particles.targets(
            gpu,
            render_size,
            scene_depth,
            scene_motion,
            &gbuffer.depth.view,
            &gbuffer.normal.view,
        );
        let post = self.post.bindings(
            gpu,
            &self.frame_uniform,
            scene_color,
            scene_depth,
            scene_motion,
            particles.reactive(),
            history,
            render_size,
            output_size,
        );

        let scene_depth = scene_depth.clone();
        self.history = None;
        self.frame_index = 0;
        self.sized_generation += 1;
        self.sized = Some(SizedResources {
            output_size,
            render_size,
            visibility,
            hzb,
            gbuffer,
            cull_scene,
            raster,
            resolve,
            ruts_generation: self.ruts.generation(),
            screen_gi,
            clouds,
            deferred,
            light_tiles_group,
            post,
            hdr,
            water,
            particles,
            scene_depth,
        });
    }

    fn write_frame_data(
        &mut self,
        gpu: &Gpu,
        camera: &Camera,
        environment: &Environment,
        size: (u32, u32),
    ) {
        let viewport = Vec2::new(size.0 as f32, size.1 as f32);
        let projection = camera.projection(viewport.x / viewport.y);
        let view = camera.view();
        let unjittered = projection * view;

        let jitter_ndc = jitter_offset(self.frame_index) * 2.0 / viewport;
        let jittered = jitter_projection(projection, jitter_ndc) * view;

        let current = History {
            view,
            view_proj: unjittered,
            projection,
        };
        let previous = self.history.unwrap_or(current);

        // The coat of what is driven: the world as its own space sees it, a row a line.
        let coat = self.coat;
        let coat_rows = coat.map_or([glam::Vec4::ZERO; 3], |(coat, ..)| {
            let to_car = coat.car.inverse();
            [to_car.row(0), to_car.row(1), to_car.row(2)]
        });
        // And of what stands about: each where its instances are numbered now.
        let mut resprays = [crate::frame::Respray::default(); crate::frame::RESPRAYS];
        for (slot, (paint, was, instances)) in resprays.iter_mut().zip(&self.resprays) {
            let numbered = instances
                .iter()
                .filter_map(|&instance| self.scene.instance_table.gpu_number(instance));
            let (first, last) = numbered.fold((u32::MAX, 0), |(first, last), number| {
                (first.min(number), last.max(number))
            });
            if first <= last {
                *slot = crate::frame::Respray {
                    paint: *paint,
                    first,
                    was: *was,
                    count: last - first + 1,
                };
            }
        }
        let frame = FrameUniform {
            view_proj: jittered,
            inv_view_proj: jittered.inverse(),
            unjittered_view_proj: unjittered,
            prev_view_proj: previous.view_proj,
            view,
            camera_position: camera.position,
            frame_index: self.frame_index,
            viewport,
            inv_viewport: viewport.recip(),
            jitter_ndc,
            near: camera.near,
            debug_mode: self.settings.debug_view as u32,
            sun_direction: environment.sun_direction,
            sun_illuminance: environment.sun_illuminance,
            sun_color: environment.sun_color,
            sky_intensity: environment.sky_intensity,
            light_count: self.scene.light_count,
            time: self.time,
            gi_intensity: environment.gi_intensity,
            temporal_accumulation: u32::from(self.settings.temporal_accumulation),
            mip_scale: self.settings.render_scale * TEXTURE_DETAIL,
            night_sky: environment.night_sky,
            stars: environment.stars,
            moon_radiance: if environment.moon_disc.is_some() {
                environment.moon_radiance
            } else {
                0.0
            },
            star_pole: environment.star_pole,
            star_equinox: environment.star_equinox,
            frame_seconds: self.frame_seconds,
            sun_disc: environment.sun_disc.unwrap_or(environment.sun_direction),
            sea_level: if self.sea.is_some() {
                self.sea_level
            } else {
                NO_SEA
            },
            moon_disc: environment.moon_disc.unwrap_or(Vec3::NEG_Y),
            _pad2: 0.0,
            lamp_position: self.lamp.map_or(Vec3::ZERO, |lamp| lamp.position),
            lamp_range: self.lamp.map_or(0.0, |lamp| lamp.range),
            lamp_direction: self.lamp.map_or(Vec3::NEG_Z, |lamp| lamp.direction),
            lamp_spread: self.lamp.map_or(1.0, |lamp| lamp.half_angle.cos()),
            lamp_intensity: self.lamp.map_or(Vec3::ZERO, |lamp| lamp.intensity),
            _pad3: 0.0,
            coat_x: coat_rows[0],
            coat_y: coat_rows[1],
            coat_z: coat_rows[2],
            coat_paint: coat.map_or(Vec3::ZERO, |(coat, ..)| coat.paint),
            coat_rust: coat.map_or(0.0, |(coat, ..)| coat.rust.clamp(0.0, 1.0)),
            coat_soil: coat.map_or(Vec3::ZERO, |(coat, ..)| coat.soil),
            coat_mud: coat.map_or(0.0, |(coat, ..)| coat.mud.clamp(0.0, 1.0)),
            coat_was: coat.map_or(Vec3::ZERO, |(coat, ..)| coat.was),
            coat_dust: coat.map_or(0.0, |(coat, ..)| coat.dust.clamp(0.0, 1.0)),
            coat_first: coat.map_or(0, |(_, first, _)| first),
            coat_count: coat.map_or(0, |(_, _, count)| count),
            coat_seed: coat.map_or(0, |(coat, ..)| coat.seed),
            coat_wet: coat.map_or(0.0, |(coat, ..)| coat.wet.clamp(0.0, 1.0)),
            coat_axles: coat.map_or(glam::Vec4::ZERO, |(coat, ..)| {
                glam::Vec4::from_array(coat.axles)
            }),
            resprays,
            rut_window: self.ruts.window(),
            rut_soil: self.rut_soil,
            rut_cell: self.ruts.cell(),
            daylight_illuminance: environment.daylight(),
        };
        gpu.queue
            .write_buffer(&self.frame_uniform, 0, bytemuck::bytes_of(&frame));

        let sized = self
            .sized
            .as_ref()
            .expect("resources exist before frame data");
        let hzb_size = Vec2::new(sized.hzb.size.0 as f32, sized.hzb.size.1 as f32);
        let main_view = |occlusion: &History, phase: u32, occlusion_enabled: bool| PassView {
            raster_view_proj: jittered,
            occlusion_view: occlusion.view,
            planes: PassView::frustum_planes(unjittered),
            camera_position: camera.position,
            lod_threshold: self.settings.lod_threshold,
            viewport,
            proj_scale: Vec2::new(occlusion.projection.x_axis.x, occlusion.projection.y_axis.y),
            hzb_size,
            near: camera.near,
            lod_scale: 0.5 * viewport.y * projection.y_axis.y,
            is_ortho: 0,
            phase,
            occlusion_enabled: u32::from(occlusion_enabled),
            allow_software: 1,
            eye_position: Vec3::ZERO,
            eye_lod_scale: 0.0,
            eye_near: 0.0,
            receiver_plane_count: 0,
            instance_filter: InstanceFilter::All as u32,
            stamp: 0,
            receiver_planes: [Vec4::ZERO; 6],
            eye_light: Vec3::Y,
            masked_lists: 1,
            prefetch_radius: self.settings.prefetch_radius,
            hold_requests: 0,
            speck_texels: 0.0,
            _pad_prefetch: 0,
        };
        self.view_slots.write(
            gpu,
            SLOT_MAIN_FIRST,
            &main_view(&previous, 0, self.history.is_some()),
        );
        self.view_slots
            .write(gpu, SLOT_MAIN_SECOND, &main_view(&current, 1, true));

        self.history = Some(current);
    }
}

fn mark(profiler: &Option<Profiler>, encoder: &mut wgpu::CommandEncoder, section: usize) {
    if let Some(profiler) = profiler {
        profiler.mark(encoder, section);
    }
}

/// The page pool a streaming renderer gives a scene within `budget`: no less than its pinned
/// pages, no more than all of them, and on a GPU with too few storage buffers for regions,
/// what one binding holds (`Fallbacks::pool_in_one_binding`) unless its pinned pages need more.
fn streaming_slots(gpu: &Gpu, paged: &PagedScene, budget: StreamingBudget) -> u32 {
    let mut slots = u32::try_from(budget.geometry_bytes / PAGE_BYTES as u64).unwrap_or(u32::MAX);
    if gpu.fallbacks.pool_in_one_binding {
        slots = slots.min(crate::regions::binding_slots(&gpu.limits));
    }
    slots.max(paged.pinned_pages).min(paged.pages.len() as u32)
}
