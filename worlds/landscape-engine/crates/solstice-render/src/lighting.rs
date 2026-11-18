//! The deferred lighting pass and the bind group shared by all lighting passes.

use crate::clouds::{CloudBindings, CloudSky};
use crate::gi::{GiVolume, ScreenGiBindings};
use crate::gpu::{groups_for, Gpu, LayoutBuilder, Target};
use crate::scene::GpuScene;
use crate::shaders;
use crate::shadow::ShadowMaps;
use crate::sky::Sky;
use crate::vg::GBuffer;

const HDR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// The Milky Way's map of the celestial sphere: a fifth of a degree per texel.
const MILKY_WAY_SIZE: (u32, u32) = (2048, 1024);
const MILKY_WAY_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// Creates the layout of the shared lighting bind group. See `common/environment.wgsl`.
pub fn environment_layout(gpu: &Gpu) -> wgpu::BindGroupLayout {
    LayoutBuilder::new()
        .uniform()
        .uniform()
        .texture_3d()
        .sampler()
        .texture()
        .texture()
        .sampler()
        .texture()
        .texture()
        .uniform()
        .texture()
        .build(gpu, "environment")
}

/// Creates the shared lighting bind group, seeing the clouds through `clouds`.
pub fn environment_group(
    gpu: &Gpu,
    layout: &wgpu::BindGroupLayout,
    frame: &wgpu::Buffer,
    volume: &GiVolume,
    sky: &Sky,
    clouds: CloudSky,
) -> wgpu::BindGroup {
    let clamp_sampler = gpu.device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("environment clamp"),
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });
    let repeat_sampler = gpu.device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("environment repeat"),
        address_mode_u: wgpu::AddressMode::Repeat,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });
    gpu.bind_group(
        "environment",
        layout,
        &[
            frame.as_entire_binding(),
            volume.uniform.as_entire_binding(),
            wgpu::BindingResource::TextureView(&volume.atlas_view),
            wgpu::BindingResource::Sampler(&clamp_sampler),
            wgpu::BindingResource::TextureView(&sky.transmittance.view),
            wgpu::BindingResource::TextureView(&sky.sky_view.view),
            wgpu::BindingResource::Sampler(&repeat_sampler),
            wgpu::BindingResource::TextureView(clouds.sky),
            wgpu::BindingResource::TextureView(clouds.shadow),
            clouds.params.as_entire_binding(),
            wgpu::BindingResource::TextureView(clouds.shadow_mean),
        ],
    )
}

/// Pixels along each side of a light tile, and words each tile holds in the tile buffer: its
/// count of lights, then their indices. Twins of `lighting/light_tiles_common.wgsl`.
pub const TILE_SIZE: u32 = 16;
pub const TILE_WORDS: u32 = 128;

/// Texels along each side of a local light's shadow map, an octahedral map of its directions.
/// Twin of `lighting/light_shadows_common.wgsl`.
pub const LIGHT_SHADOW_SIZE: u32 = 64;

/// Each local light's shadow map: how far its light reaches in every direction, traced
/// through the GI volume once a frame by `lighting/light_shadows.wgsl`, so that the passes
/// lighting a point look each light's shadow up instead of tracing it per pixel per light,
/// whose cost grew with pixels times the lights overlapping them. One layer per light, of
/// distances; a scene with no lights keeps one unused layer, so that the bindings exist, and
/// a scene with more lights than the device allows layers leaves the rest unshadowed.
pub struct LightShadows {
    pipeline: wgpu::ComputePipeline,
    group: wgpu::BindGroup,
    pub view: wgpu::TextureView,
    /// Lights with a map: the scene's, up to the device's layers.
    mapped: u32,
}

impl LightShadows {
    pub fn new(gpu: &Gpu, environment: &wgpu::BindGroupLayout, scene: &GpuScene) -> Self {
        let format = wgpu::TextureFormat::R32Float;
        let layers = gpu.device.limits().max_texture_array_layers;
        if scene.light_count > layers {
            log::warn!(
                "{} lights, but the device allows {layers} shadow maps: the rest cast none",
                scene.light_count
            );
        }
        let mapped = scene.light_count.min(layers);
        let layout = LayoutBuilder::new()
            .storage_read()
            .storage_texture_array(format)
            .build(gpu, "light shadows");
        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("light shadows"),
            size: wgpu::Extent3d {
                width: LIGHT_SHADOW_SIZE,
                height: LIGHT_SHADOW_SIZE,
                depth_or_array_layers: mapped.max(1),
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        let module = shaders::module(&gpu.device, "lighting/light_shadows.wgsl");
        Self {
            pipeline: gpu.compute_pipeline(
                "light shadows",
                &module,
                "main",
                &[&layout, environment],
            ),
            group: gpu.bind_group(
                "light shadows",
                &layout,
                &[
                    scene.lights.as_entire_binding(),
                    wgpu::BindingResource::TextureView(&view),
                ],
            ),
            view,
            mapped,
        }
    }

    /// Traces every light's map afresh, after the volume's passes and before the passes that
    /// read the maps.
    pub fn record(&self, encoder: &mut wgpu::CommandEncoder, environment: &wgpu::BindGroup) {
        if self.mapped == 0 {
            return;
        }
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("light shadows"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.group, &[]);
        pass.set_bind_group(1, environment, &[]);
        let groups = groups_for(LIGHT_SHADOW_SIZE, 8);
        pass.dispatch_workgroups(groups, groups, self.mapped);
    }
}

/// The pass that finds which local lights reach each tile of the screen, so that lighting
/// loops over those alone. With no lights in the scene it is skipped, and the tiles, zero
/// from creation, say so.
pub struct LightTiles {
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::ComputePipeline,
}

impl LightTiles {
    pub fn new(gpu: &Gpu, environment: &wgpu::BindGroupLayout) -> Self {
        let layout = LayoutBuilder::new()
            .texture_unfilterable()
            .storage_read()
            .storage_write()
            .build(gpu, "light tiles");
        let module = shaders::module(&gpu.device, "lighting/light_tiles.wgsl");
        Self {
            pipeline: gpu.compute_pipeline("light tiles", &module, "main", &[&layout, environment]),
            layout,
        }
    }

    /// How many tiles cover a viewport of `size`.
    pub fn tiles(size: (u32, u32)) -> (u32, u32) {
        (groups_for(size.0, TILE_SIZE), groups_for(size.1, TILE_SIZE))
    }

    /// The buffer holding the tiles of a viewport of `size`, empty until the pass has run.
    pub fn buffer(gpu: &Gpu, size: (u32, u32)) -> wgpu::Buffer {
        let (across, down) = Self::tiles(size);
        gpu.storage_buffer(
            "light tiles",
            u64::from(across) * u64::from(down) * u64::from(TILE_WORDS) * 4,
            wgpu::BufferUsages::empty(),
        )
    }

    pub fn bind_group(
        &self,
        gpu: &Gpu,
        depth: &wgpu::TextureView,
        scene: &GpuScene,
        tiles: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        gpu.bind_group(
            "light tiles",
            &self.layout,
            &[
                wgpu::BindingResource::TextureView(depth),
                scene.lights.as_entire_binding(),
                tiles.as_entire_binding(),
            ],
        )
    }

    pub fn record(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        group: &wgpu::BindGroup,
        environment: &wgpu::BindGroup,
        size: (u32, u32),
    ) {
        let (across, down) = Self::tiles(size);
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("light tiles"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, group, &[]);
        pass.set_bind_group(1, environment, &[]);
        pass.dispatch_workgroups(across, down, 1);
    }
}

/// The pass that combines all lighting into the HDR scene colour.
pub struct DeferredLighting {
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::ComputePipeline,
    /// The Milky Way, which never changes on the celestial sphere, so is worked out once.
    milky_way: Target,
}

impl DeferredLighting {
    pub fn new(gpu: &Gpu, environment: &wgpu::BindGroupLayout) -> Self {
        let builder = LayoutBuilder::new()
            .texture()
            .texture()
            .texture()
            .texture_unfilterable()
            .texture()
            .texture()
            .storage_texture(HDR_FORMAT);
        let layout = ShadowMaps::add_to_layout(builder)
            .storage_read()
            .texture()
            .texture()
            .storage_read()
            .texture_array_unfilterable()
            .build(gpu, "deferred");
        let module = shaders::module(&gpu.device, "lighting/deferred.wgsl");
        Self {
            pipeline: gpu.compute_pipeline("deferred", &module, "main", &[&layout, environment]),
            layout,
            milky_way: bake_milky_way(gpu),
        }
    }

    /// One bind group per frame parity, reading that frame's GI output and clouds.
    #[allow(clippy::too_many_arguments)]
    pub fn bind_groups(
        &self,
        gpu: &Gpu,
        gbuffer: &GBuffer,
        gi: &ScreenGiBindings,
        hdr: &wgpu::TextureView,
        shadows: &ShadowMaps,
        scene: &GpuScene,
        clouds: &CloudBindings,
        light_tiles: &wgpu::Buffer,
        light_shadows: &wgpu::TextureView,
    ) -> [wgpu::BindGroup; 2] {
        let tex = |view| wgpu::BindingResource::TextureView(view);
        let group = |parity: usize| {
            gpu.bind_group(
                "deferred",
                &self.layout,
                &[
                    tex(&gbuffer.albedo.view),
                    tex(&gbuffer.normal.view),
                    tex(&gbuffer.emissive.view),
                    tex(&gbuffer.depth.view),
                    tex(&gi.diffuse[parity].view),
                    tex(&gi.reflections[parity].view),
                    tex(hdr),
                    shadows.uniform.as_entire_binding(),
                    tex(&shadows.array_view),
                    wgpu::BindingResource::Sampler(&shadows.comparison_sampler),
                    scene.lights.as_entire_binding(),
                    tex(&clouds.screen[parity].view),
                    tex(&self.milky_way.view),
                    light_tiles.as_entire_binding(),
                    tex(light_shadows),
                ],
            )
        };
        [group(0), group(1)]
    }

    pub fn record(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        group: &wgpu::BindGroup,
        environment: &wgpu::BindGroup,
        size: (u32, u32),
    ) {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("deferred"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, group, &[]);
        pass.set_bind_group(1, environment, &[]);
        pass.dispatch_workgroups(groups_for(size.0, 8), groups_for(size.1, 8), 1);
    }
}

/// Works out the Milky Way's map of the celestial sphere, once.
fn bake_milky_way(gpu: &Gpu) -> Target {
    let none = wgpu::TextureUsages::empty();
    let map = Target::new(gpu, "milky way", MILKY_WAY_SIZE, MILKY_WAY_FORMAT, none);
    let layout = LayoutBuilder::new()
        .storage_texture(MILKY_WAY_FORMAT)
        .build(gpu, "milky way bake");
    let module = shaders::module(&gpu.device, "sky/milky_way_bake.wgsl");
    let pipeline = gpu.compute_pipeline("milky way bake", &module, "main", &[&layout]);
    let group = gpu.bind_group(
        "milky way bake",
        &layout,
        &[wgpu::BindingResource::TextureView(&map.view)],
    );
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("milky way bake"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.dispatch_workgroups(
            groups_for(MILKY_WAY_SIZE.0, 8),
            groups_for(MILKY_WAY_SIZE.1, 8),
            1,
        );
    }
    gpu.queue.submit([encoder.finish()]);
    map
}
