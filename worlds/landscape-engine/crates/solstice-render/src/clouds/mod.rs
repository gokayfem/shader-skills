//! Volumetric clouds, driven by the weather.
//!
//! Clouds are ray marched through noise in a layer above a curved planet, as Unreal's volumetric
//! clouds are. See `shaders/clouds/common.wgsl` for the model. Each frame:
//!
//! * the [`Weather`] eases towards its target, and the wind carries the clouds along;
//! * a shadow map records how much sunlight they let through to the ground around the camera;
//! * a low-resolution panorama of the whole sky lets skylight, reflections and haze see them;
//! * the clouds on screen are marched at half resolution and blended with last frame's.
//!
//! The noise they are carved from is generated on the GPU once, when first drawn, along with a
//! table of how the weather map's values are spread, so that coverage means what it says.

mod weather;

pub use weather::{Observation, Weather, WeatherCycle};

/// Edge of the shape noise volume, the detail noise volume and the weather map, in texels.
pub const NOISE_SIZES: [u32; 3] = [SHAPE_SIZE, DETAIL_SIZE, WEATHER_SIZE];

use crate::gpu::{groups_for, Gpu, LayoutBuilder, Target};
use crate::shaders;
use bytemuck::{Pod, Zeroable};
use glam::{Vec2, Vec3};

const COLOR: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const NOISE: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const SHAPE_SIZE: u32 = 128;
const DETAIL_SIZE: u32 = 32;
const WEATHER_SIZE: u32 = 512;
const SHADOW_SIZE: u32 = 256;
/// Ground covered by the shadow map, in metres along each side.
const SHADOW_EXTENT: f32 = 12_000.0;
/// The panorama, laid out like the sky-view table.
const PANORAMA_SIZE: (u32, u32) = (192, 108);
/// Every noise repeat divides this, in metres, so wrapping the drift by it changes nothing
/// while keeping the numbers small enough for single precision.
const DRIFT_PERIOD: f32 = 252_000.0;
const EVOLUTION_PERIOD: f32 = 7_000.0;
/// How fast cloud shapes boil upwards at evolution 1, in metres per second.
const BOIL_SPEED: f32 = 4.0;

/// Twin of `CloudParams` in `shaders/clouds/params.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct CloudUniform {
    wind_offset: [f32; 2],
    evolution: f32,
    coverage: f32,
    cloud_type: f32,
    base: f32,
    top: f32,
    density: f32,
    cirrus: f32,
    rain: f32,
    shadow_extent: f32,
    enabled: f32,
    shadow_origin: [f32; 2],
    wind_direction: [f32; 2],
    /// A flash of lightning: towards it from the camera, and the illuminance it gives the scene.
    flash: [f32; 4],
}

/// What lighting passes need to see the clouds.
pub struct CloudSky<'a> {
    pub sky: &'a wgpu::TextureView,
    pub shadow: &'a wgpu::TextureView,
    /// A single texel: the shadow map's mean, which stands in for it beyond its edge.
    pub shadow_mean: &'a wgpu::TextureView,
    pub params: &'a wgpu::Buffer,
}

/// Size-dependent targets: the clouds on screen, one per frame parity, each written on its
/// frame and read as history on the next.
pub struct CloudBindings {
    pub screen: [Target; 2],
    groups: [wgpu::BindGroup; 2],
    size: (u32, u32),
}

/// The cloud passes and the weather that drives them.
pub struct Clouds {
    shape_texture: wgpu::Texture,
    detail_texture: wgpu::Texture,
    coverage_cdf: wgpu::Buffer,
    params: wgpu::Buffer,
    sampler: wgpu::Sampler,
    shape: wgpu::TextureView,
    detail: wgpu::TextureView,
    weather_map: Target,
    noise: [(wgpu::ComputePipeline, wgpu::BindGroup, (u32, u32, u32)); 5],
    noise_ready: bool,
    shadow: Target,
    shadow_mean: Target,
    panorama: Target,
    no_clouds: Target,
    no_shadow: Target,
    screen_layout: wgpu::BindGroupLayout,
    shadow_pass: (wgpu::ComputePipeline, wgpu::BindGroup),
    mean_pass: (wgpu::ComputePipeline, wgpu::BindGroup),
    panorama_pass: (wgpu::ComputePipeline, wgpu::BindGroup),
    screen_pipeline: wgpu::ComputePipeline,
    current: Weather,
    target: Weather,
    transition_seconds: f32,
    wind_offset: Vec2,
    evolution: f32,
    enabled: bool,
    /// When set, the weather changes by itself.
    cycle: Option<WeatherCycle>,
    /// The flash of lightning this frame, if any (`lightning.rs`).
    flash: [f32; 4],
}

fn volume(gpu: &Gpu, label: &str, size: u32) -> wgpu::Texture {
    gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: size,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D3,
        format: NOISE,
        usage: wgpu::TextureUsages::STORAGE_BINDING
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    })
}

/// The textures and table the clouds are carved from, for tools that inspect them.
pub struct CloudNoise<'a> {
    pub shape: &'a wgpu::Texture,
    pub detail: &'a wgpu::Texture,
    pub weather_map: &'a wgpu::Texture,
    pub coverage_cdf: &'a wgpu::Buffer,
}

/// A one-texel target holding `value`, each channel 0 or 1, for a pass that must see "no
/// clouds".
fn constant(gpu: &Gpu, label: &str, value: [f32; 4]) -> Target {
    let target = Target::new(gpu, label, (1, 1), COLOR, wgpu::TextureUsages::COPY_DST);
    // Half-precision 1.0 is 0x3c00.
    let halves: [u16; 4] = value.map(|v| if v > 0.5 { 0x3c00 } else { 0 });
    gpu.queue.write_texture(
        target.texture.as_image_copy(),
        bytemuck::cast_slice(&halves),
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(8),
            rows_per_image: None,
        },
        wgpu::Extent3d::default(),
    );
    target
}

impl Clouds {
    pub fn new(gpu: &Gpu, environment_layout: &wgpu::BindGroupLayout) -> Self {
        let params = gpu.uniform_buffer("cloud params", std::mem::size_of::<CloudUniform>() as u64);
        let sampler = gpu.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("cloud noise"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let none = wgpu::TextureUsages::empty();

        // Noise, made once.
        let shape_texture = volume(gpu, "cloud shape noise", SHAPE_SIZE);
        let detail_texture = volume(gpu, "cloud detail noise", DETAIL_SIZE);
        let shape = shape_texture.create_view(&Default::default());
        let detail = detail_texture.create_view(&Default::default());
        let no_volume = volume(gpu, "no volume", 1).create_view(&Default::default());
        let weather_map = Target::new(
            gpu,
            "cloud weather map",
            (WEATHER_SIZE, WEATHER_SIZE),
            NOISE,
            wgpu::TextureUsages::COPY_SRC,
        );
        let no_map = Target::new(gpu, "no map", (1, 1), NOISE, none);
        let noise_layout = LayoutBuilder::new()
            .storage_texture_3d(NOISE)
            .storage_texture(NOISE)
            .build(gpu, "cloud noise");
        let noise_module = shaders::module(&gpu.device, "clouds/noise.wgsl");
        let noise_pass =
            |entry: &str, volume: &wgpu::TextureView, map: &wgpu::TextureView, size| {
                let group = gpu.bind_group(
                    entry,
                    &noise_layout,
                    &[
                        wgpu::BindingResource::TextureView(volume),
                        wgpu::BindingResource::TextureView(map),
                    ],
                );
                (
                    gpu.compute_pipeline(entry, &noise_module, entry, &[&noise_layout]),
                    group,
                    size,
                )
            };
        // How the weather map's coverage values are spread, so that coverage means what it says.
        let bins = gpu.storage_buffer("cloud coverage bins", 256 * 4, wgpu::BufferUsages::empty());
        let coverage_cdf =
            gpu.storage_buffer("cloud coverage cdf", 256 * 4, wgpu::BufferUsages::COPY_SRC);
        let coverage_layout = LayoutBuilder::new()
            .texture()
            .storage_write()
            .storage_write()
            .build(gpu, "cloud coverage");
        let coverage_module = shaders::module(&gpu.device, "clouds/coverage.wgsl");
        let coverage_group = || {
            gpu.bind_group(
                "cloud coverage",
                &coverage_layout,
                &[
                    wgpu::BindingResource::TextureView(&weather_map.view),
                    bins.as_entire_binding(),
                    coverage_cdf.as_entire_binding(),
                ],
            )
        };
        let coverage_pass = |entry: &str, size| {
            (
                gpu.compute_pipeline(entry, &coverage_module, entry, &[&coverage_layout]),
                coverage_group(),
                size,
            )
        };
        let noise = [
            noise_pass(
                "shape",
                &shape,
                &no_map.view,
                (
                    groups_for(SHAPE_SIZE, 4),
                    groups_for(SHAPE_SIZE, 4),
                    groups_for(SHAPE_SIZE, 4),
                ),
            ),
            noise_pass(
                "detail",
                &detail,
                &no_map.view,
                (
                    groups_for(DETAIL_SIZE, 4),
                    groups_for(DETAIL_SIZE, 4),
                    groups_for(DETAIL_SIZE, 4),
                ),
            ),
            noise_pass(
                "weather",
                &no_volume,
                &weather_map.view,
                (groups_for(WEATHER_SIZE, 8), groups_for(WEATHER_SIZE, 8), 1),
            ),
            coverage_pass(
                "histogram",
                (groups_for(WEATHER_SIZE, 8), groups_for(WEATHER_SIZE, 8), 1),
            ),
            coverage_pass("accumulate", (1, 1, 1)),
        ];

        // The passes that march the clouds.
        let common = || {
            LayoutBuilder::new()
                .texture_3d()
                .texture_3d()
                .texture()
                .sampler()
                .storage_texture(COLOR)
        };
        let pass_layout = common().storage_read().build(gpu, "clouds");
        let screen_layout = common()
            .texture()
            .texture_unfilterable()
            .storage_read()
            .build(gpu, "cloud screen");
        let shadow = Target::new(gpu, "cloud shadow", (SHADOW_SIZE, SHADOW_SIZE), COLOR, none);
        let panorama = Target::new(gpu, "cloud panorama", PANORAMA_SIZE, COLOR, none);
        let pass = |path: &str, output: &Target| {
            let module = shaders::module(&gpu.device, path);
            let group = gpu.bind_group(
                path,
                &pass_layout,
                &[
                    wgpu::BindingResource::TextureView(&shape),
                    wgpu::BindingResource::TextureView(&detail),
                    wgpu::BindingResource::TextureView(&weather_map.view),
                    wgpu::BindingResource::Sampler(&sampler),
                    wgpu::BindingResource::TextureView(&output.view),
                    coverage_cdf.as_entire_binding(),
                ],
            );
            (
                gpu.compute_pipeline(path, &module, "main", &[&pass_layout, environment_layout]),
                group,
            )
        };
        let shadow_pass = pass("clouds/shadow.wgsl", &shadow);
        let panorama_pass = pass("clouds/panorama.wgsl", &panorama);
        let shadow_mean = Target::new(gpu, "cloud shadow mean", (1, 1), COLOR, none);
        let mean_layout = LayoutBuilder::new()
            .texture()
            .storage_texture(COLOR)
            .build(gpu, "cloud shadow mean");
        let mean_module = shaders::module(&gpu.device, "clouds/shadow_mean.wgsl");
        let mean_pass = (
            gpu.compute_pipeline("cloud shadow mean", &mean_module, "main", &[&mean_layout]),
            gpu.bind_group(
                "cloud shadow mean",
                &mean_layout,
                &[
                    wgpu::BindingResource::TextureView(&shadow.view),
                    wgpu::BindingResource::TextureView(&shadow_mean.view),
                ],
            ),
        );
        let screen_module = shaders::module(&gpu.device, "clouds/screen.wgsl");
        let screen_pipeline = gpu.compute_pipeline(
            "cloud screen",
            &screen_module,
            "main",
            &[&screen_layout, environment_layout],
        );

        let weather = Weather::default();
        Self {
            shape_texture,
            detail_texture,
            coverage_cdf,
            params,
            sampler,
            shape,
            detail,
            weather_map,
            noise,
            noise_ready: false,
            shadow,
            shadow_mean,
            panorama,
            no_clouds: constant(gpu, "no clouds", [0.0, 0.0, 0.0, 1.0]),
            no_shadow: constant(gpu, "no cloud shadow", [1.0; 4]),
            screen_layout,
            shadow_pass,
            mean_pass,
            panorama_pass,
            screen_pipeline,
            current: weather,
            flash: [0.0; 4],
            target: weather,
            transition_seconds: 0.0,
            wind_offset: Vec2::ZERO,
            evolution: 0.0,
            enabled: true,
            cycle: None,
        }
    }

    /// The clouds as lighting passes see them.
    pub fn sky(&self) -> CloudSky<'_> {
        CloudSky {
            sky: &self.panorama.view,
            shadow: &self.shadow.view,
            shadow_mean: &self.shadow_mean.view,
            params: &self.params,
        }
    }

    /// No clouds and no shadow, for the cloud passes themselves, which must not read what they
    /// write.
    pub fn placeholders(&self) -> CloudSky<'_> {
        CloudSky {
            sky: &self.no_clouds.view,
            shadow: &self.no_shadow.view,
            shadow_mean: &self.no_shadow.view,
            params: &self.params,
        }
    }

    /// Lets the weather change by itself, starting from the named weather, or stops it.
    pub fn set_changing(&mut self, changing: bool, start: &str) {
        self.cycle = changing.then(|| WeatherCycle::new(start, 1));
        if let Some(cycle) = &self.cycle {
            if let Some(weather) = Weather::by_name(cycle.current()) {
                self.target = weather;
                self.transition_seconds = WeatherCycle::TRANSITION;
            }
        }
    }

    /// Changes the weather, easing into it over about `seconds`, or at once for 0. This stops
    /// the weather changing by itself.
    pub fn set_weather(&mut self, weather: Weather, seconds: f32) {
        self.cycle = None;
        self.target = weather;
        self.transition_seconds = seconds.max(0.0);
        if seconds <= 0.0 {
            self.current = weather;
        }
    }

    /// The weather now, part way to its target while it changes.
    pub fn weather(&self) -> Weather {
        self.current
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// Moves the weather on by `seconds`: easing towards its target, drifting with the wind and
    /// changing shape as it goes. Long spans are taken a second at a time, so that the wind
    /// follows the weather as it changes.
    pub fn advance(&mut self, seconds: f32) {
        let mut left = seconds.max(0.0);
        while left > 0.0 {
            let step = left.min(1.0);
            left -= step;
            if let Some(next) = self.cycle.as_mut().and_then(|cycle| cycle.advance(step)) {
                if let Some(weather) = Weather::by_name(next) {
                    self.target = weather;
                    self.transition_seconds = WeatherCycle::TRANSITION;
                }
            }
            let ease = if self.transition_seconds <= 0.0 {
                1.0
            } else {
                1.0 - (-step / self.transition_seconds).exp()
            };
            self.current = self.current.lerp(&self.target, ease);
            let wind = Vec2::from(self.current.wind_velocity());
            self.wind_offset =
                (self.wind_offset + wind * step).rem_euclid(Vec2::splat(DRIFT_PERIOD));
            self.evolution = (self.evolution + step * BOIL_SPEED * self.current.evolution)
                .rem_euclid(EVOLUTION_PERIOD);
        }
    }

    /// Advances the weather by one frame of `seconds`, and writes this frame's parameters.
    pub fn update(&mut self, gpu: &Gpu, seconds: f32, camera: Vec3) {
        // A long pause, such as a hidden tab, should not make the clouds leap.
        self.advance(seconds.clamp(0.0, 0.25));
        let wind = Vec2::from(self.current.wind_velocity());

        // Centred on the camera and snapped to whole texels, so the shadows do not crawl.
        let texel = SHADOW_EXTENT / SHADOW_SIZE as f32;
        let corner = (Vec2::new(camera.x, camera.z) / texel).floor() * texel - SHADOW_EXTENT * 0.5;
        let w = self.current;
        let uniform = CloudUniform {
            wind_offset: self.wind_offset.into(),
            evolution: self.evolution,
            coverage: w.coverage,
            cloud_type: w.cloud_type,
            base: w.base,
            top: w.base + w.thickness,
            density: w.density,
            cirrus: w.cirrus,
            rain: w.rain,
            shadow_extent: SHADOW_EXTENT,
            enabled: if self.enabled { 1.0 } else { 0.0 },
            shadow_origin: corner.into(),
            wind_direction: wind.normalize_or(Vec2::X).into(),
            flash: self.flash,
        };
        gpu.queue
            .write_buffer(&self.params, 0, bytemuck::bytes_of(&uniform));
    }

    /// Sets the flash of lightning for this frame, from the next `update`.
    pub fn set_flash(&mut self, flash: Option<crate::lightning::Flash>) {
        self.flash = flash.map_or([0.0; 4], |f| {
            [f.direction.x, f.direction.y, f.direction.z, f.illuminance]
        });
    }

    /// Creates the on-screen targets for a render `size`, which skip pixels `depth` covers.
    pub fn bindings(
        &self,
        gpu: &Gpu,
        size: (u32, u32),
        depth: &wgpu::TextureView,
    ) -> CloudBindings {
        let half = (size.0.div_ceil(2), size.1.div_ceil(2));
        let none = wgpu::TextureUsages::empty();
        let screen = [
            Target::new(gpu, "clouds 0", half, COLOR, none),
            Target::new(gpu, "clouds 1", half, COLOR, none),
        ];
        let group = |parity: usize| {
            gpu.bind_group(
                "cloud screen",
                &self.screen_layout,
                &[
                    wgpu::BindingResource::TextureView(&self.shape),
                    wgpu::BindingResource::TextureView(&self.detail),
                    wgpu::BindingResource::TextureView(&self.weather_map.view),
                    wgpu::BindingResource::Sampler(&self.sampler),
                    wgpu::BindingResource::TextureView(&screen[parity].view),
                    wgpu::BindingResource::TextureView(&screen[1 - parity].view),
                    wgpu::BindingResource::TextureView(depth),
                    self.coverage_cdf.as_entire_binding(),
                ],
            )
        };
        CloudBindings {
            groups: [group(0), group(1)],
            screen,
            size: half,
        }
    }

    /// Generates the noise and the coverage table, if not done already.
    pub fn generate_noise(&mut self, encoder: &mut wgpu::CommandEncoder) {
        if self.noise_ready {
            return;
        }
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("cloud noise"),
            timestamp_writes: None,
        });
        for (pipeline, group, size) in &self.noise {
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, group, &[]);
            pass.dispatch_workgroups(size.0, size.1, size.2);
        }
        self.noise_ready = true;
    }

    /// The noise and coverage table, for inspection.
    pub fn noise(&self) -> CloudNoise<'_> {
        CloudNoise {
            shape: &self.shape_texture,
            detail: &self.detail_texture,
            weather_map: &self.weather_map.texture,
            coverage_cdf: &self.coverage_cdf,
        }
    }

    /// Records the passes that lighting needs: the shadow map and the panorama. `environment`
    /// must be the lighting bind group made with [`Clouds::placeholders`].
    pub fn record_lighting(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        environment: &wgpu::BindGroup,
    ) {
        self.generate_noise(encoder);
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("clouds"),
            timestamp_writes: None,
        });
        pass.set_bind_group(1, environment, &[]);
        let passes = [
            (&self.shadow_pass, (SHADOW_SIZE, SHADOW_SIZE)),
            (&self.panorama_pass, PANORAMA_SIZE),
        ];
        for ((pipeline, group), size) in passes {
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, group, &[]);
            pass.dispatch_workgroups(groups_for(size.0, 8), groups_for(size.1, 8), 1);
        }
        // The shadow map's mean, for lighting beyond its edge.
        pass.set_pipeline(&self.mean_pass.0);
        pass.set_bind_group(0, &self.mean_pass.1, &[]);
        pass.dispatch_workgroups(1, 1, 1);
    }

    /// Records the clouds on screen, once this frame's depth is known. Afterwards
    /// `bindings.screen[parity]` holds them.
    pub fn record_screen(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        bindings: &CloudBindings,
        environment: &wgpu::BindGroup,
        parity: usize,
    ) {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("clouds on screen"),
            timestamp_writes: None,
        });
        pass.set_bind_group(1, environment, &[]);
        pass.set_pipeline(&self.screen_pipeline);
        pass.set_bind_group(0, &bindings.groups[parity], &[]);
        pass.dispatch_workgroups(
            groups_for(bindings.size.0, 8),
            groups_for(bindings.size.1, 8),
            1,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_uniform_matches_its_shader_twin() {
        crate::shaders::assert_twin!(
            "lighting/deferred.wgsl",
            "CloudParams",
            CloudUniform,
            [
                wind_offset,
                evolution,
                coverage,
                cloud_type,
                base,
                top,
                density,
                cirrus,
                rain,
                shadow_extent,
                enabled,
                shadow_origin,
                wind_direction,
                flash,
            ]
        );
    }

    #[test]
    fn drift_wraps_on_a_period_every_noise_repeat_divides() {
        // Weather map, shape noise, detail noise and the cirrus, which drifts 2.5 times faster
        // across 0.7 of the weather map.
        for tile in [36_000.0, 7_000.0, 700.0, 36_000.0 * 0.7 / 2.5] {
            let repeats = DRIFT_PERIOD / tile;
            assert!((repeats - repeats.round()).abs() < 1.0e-3, "{tile}");
        }
        assert!((EVOLUTION_PERIOD / 700.0).fract() < 1.0e-6);
    }
}
