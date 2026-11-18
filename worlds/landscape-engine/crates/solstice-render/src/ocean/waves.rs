//! The waves, on the GPU: three cascades of moving waves, made each frame by inverse FFT.
//!
//! When the sea state changes, `ocean/spectrum.wgsl` draws every wave's starting amplitude from
//! the spectrum. Each frame, `ocean/evolve.wgsl` turns every wave to this moment and makes the
//! spectra of eight fields, then `ocean/fft_rows.wgsl` and `ocean/fft_columns.wgsl` transform
//! them back to the tile: displacement, slopes and the stretch of the surface, from which foam
//! forms where crests fold. Three dispatches a frame, whatever the sea.

use super::spectrum::{band_limits, SeaState, CASCADE_SIZES, REPEAT_SECONDS};
use super::Ocean;
use crate::gpu::{groups_for, Gpu, LayoutBuilder};
use crate::shaders;
use bytemuck::{Pod, Zeroable};
use std::sync::atomic::{AtomicU64, Ordering};

/// Numbers each set of wave textures, so that what binds them knows when they are replaced.
static GENERATION: AtomicU64 = AtomicU64::new(1);

pub const CASCADES: u32 = 3;

const AMPLITUDE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rg32Float;
const SPECTRUM_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba32Float;
const WAVE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const FOAM_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R32Float;

/// While the sea state is easing from one to another, the waves' amplitudes are redrawn at most
/// this often. Each wave keeps its phase, so the sea grows or calms smoothly between.
const REBUILD_SECONDS: f32 = 0.25;

/// Foam forms where the surface is squeezed to less than this share of its area, the start of
/// a crest folding over.
const FOAM_STRETCH: f32 = 0.55;
/// How quickly foam builds as the squeeze deepens.
const FOAM_RATE: f32 = 4.0;
/// Seconds for foam to fade to 1/e of itself.
const FOAM_LIFE: f32 = 3.0;

/// Twin of `OceanParams` in `ocean/common.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct OceanUniform {
    sizes: [f32; 4],
    bands: [f32; 4],
    wind: [f32; 4],
    shape: [f32; 4],
    swell: [f32; 4],
    motion: [f32; 4],
    foam: [f32; 4],
    seed: u32,
    _pad: [u32; 3],
}

/// A texture array with one layer per cascade.
pub struct CascadeTexture {
    pub texture: wgpu::Texture,
    /// Every level, for sampling.
    pub view: wgpu::TextureView,
    /// Each level alone, for writing.
    pub levels: Vec<wgpu::TextureView>,
}

impl CascadeTexture {
    fn new(gpu: &Gpu, label: &str, size: u32, format: wgpu::TextureFormat) -> Self {
        Self::with_levels(gpu, label, size, format, 1)
    }

    /// One with a full mip chain, down to a single texel.
    fn mipmapped(gpu: &Gpu, label: &str, size: u32, format: wgpu::TextureFormat) -> Self {
        Self::with_levels(gpu, label, size, format, size.ilog2() + 1)
    }

    fn with_levels(
        gpu: &Gpu,
        label: &str,
        size: u32,
        format: wgpu::TextureFormat,
        levels: u32,
    ) -> Self {
        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: CASCADES,
            },
            mip_level_count: levels,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        let levels = (0..levels)
            .map(|level| {
                texture.create_view(&wgpu::TextureViewDescriptor {
                    dimension: Some(wgpu::TextureViewDimension::D2Array),
                    base_mip_level: level,
                    mip_level_count: Some(1),
                    ..Default::default()
                })
            })
            .collect();
        Self {
            texture,
            view,
            levels,
        }
    }
}

/// The moving waves of the open sea.
pub struct OceanWaves {
    size: u32,
    uniform: wgpu::Buffer,
    /// Each wave's starting amplitude. The bind groups use it; tests read it back.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) amplitudes: CascadeTexture,
    /// Displacement x, y and z, and ∂Dx/∂z, per cascade.
    pub displacement: CascadeTexture,
    /// Slopes ∂h/∂x and ∂h/∂z, then ∂Dx/∂x and ∂Dz/∂z, per cascade.
    pub derivatives: CascadeTexture,
    /// How much foam each point carries, written alternately into each.
    foam: [CascadeTexture; 2],
    build_spectrum: (wgpu::ComputePipeline, wgpu::BindGroup),
    evolve: (wgpu::ComputePipeline, wgpu::BindGroup),
    rows: (wgpu::ComputePipeline, wgpu::BindGroup),
    columns: (wgpu::ComputePipeline, [wgpu::BindGroup; 2]),
    /// Builds each mip level of the displacement and slopes from the one above.
    mips: (wgpu::ComputePipeline, Vec<wgpu::BindGroup>),
    /// The sea state the amplitudes were last drawn from, and how long ago.
    built: Option<SeaState>,
    since_build: f32,
    /// The amplitudes must be redrawn before the next frame's waves.
    pending_build: bool,
    /// Seconds the sea has run, kept in double precision and folded into the repeat.
    time: f64,
    seed: u32,
    frame: usize,
    uniform_data: OceanUniform,
    generation: u64,
    /// The stretch below which the surface breaks into foam.
    pub(crate) foam_stretch: f32,
}

/// Compiles an ocean shader for tiles of `size` texels.
fn module(gpu: &Gpu, path: &str, size: u32) -> wgpu::ShaderModule {
    const DECLARATION: &str = "const SIZE: u32 = 256u;";
    let source = shaders::expand(path);
    // A reformatted declaration would leave the shader at 256 texels on a smaller tile.
    assert!(
        source.contains(DECLARATION),
        "{path} must declare `{DECLARATION}` for the tile size to be set"
    );
    let source = source.replacen(DECLARATION, &format!("const SIZE: u32 = {size}u;"), 1);
    gpu.device
        .create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some(path),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        })
}

impl OceanWaves {
    /// Waves on tiles of `size` texels: 256 for desktop GPUs, 128 for integrated ones. A power
    /// of two, at most 256.
    pub fn new(gpu: &Gpu, size: u32) -> Self {
        assert!(size.is_power_of_two() && (16..=256).contains(&size));
        let amplitudes = CascadeTexture::new(gpu, "ocean amplitudes", size, AMPLITUDE_FORMAT);
        let spectrum = [
            CascadeTexture::new(gpu, "ocean spectrum a", size, SPECTRUM_FORMAT),
            CascadeTexture::new(gpu, "ocean spectrum b", size, SPECTRUM_FORMAT),
        ];
        let rows_out = [
            CascadeTexture::new(gpu, "ocean rows a", size, SPECTRUM_FORMAT),
            CascadeTexture::new(gpu, "ocean rows b", size, SPECTRUM_FORMAT),
        ];
        let displacement = CascadeTexture::mipmapped(gpu, "ocean displacement", size, WAVE_FORMAT);
        let derivatives = CascadeTexture::mipmapped(gpu, "ocean derivatives", size, WAVE_FORMAT);
        let foam = [
            CascadeTexture::new(gpu, "ocean foam 0", size, FOAM_FORMAT),
            CascadeTexture::new(gpu, "ocean foam 1", size, FOAM_FORMAT),
        ];
        let uniform = gpu.uniform_buffer("ocean", std::mem::size_of::<OceanUniform>() as u64);
        fn tex(t: &CascadeTexture) -> wgpu::BindingResource<'_> {
            wgpu::BindingResource::TextureView(&t.view)
        }
        fn level(t: &CascadeTexture, level: usize) -> wgpu::BindingResource<'_> {
            wgpu::BindingResource::TextureView(&t.levels[level])
        }

        let layout = LayoutBuilder::new()
            .uniform()
            .storage_texture_array(AMPLITUDE_FORMAT)
            .build(gpu, "ocean spectrum");
        let build_spectrum = (
            gpu.compute_pipeline(
                "ocean spectrum",
                &module(gpu, "ocean/spectrum.wgsl", size),
                "main",
                &[&layout],
            ),
            gpu.bind_group(
                "ocean spectrum",
                &layout,
                &[uniform.as_entire_binding(), tex(&amplitudes)],
            ),
        );

        let layout = LayoutBuilder::new()
            .uniform()
            .texture_array_unfilterable()
            .storage_texture_array(SPECTRUM_FORMAT)
            .storage_texture_array(SPECTRUM_FORMAT)
            .build(gpu, "ocean evolve");
        let evolve = (
            gpu.compute_pipeline(
                "ocean evolve",
                &module(gpu, "ocean/evolve.wgsl", size),
                "main",
                &[&layout],
            ),
            gpu.bind_group(
                "ocean evolve",
                &layout,
                &[
                    uniform.as_entire_binding(),
                    tex(&amplitudes),
                    tex(&spectrum[0]),
                    tex(&spectrum[1]),
                ],
            ),
        );

        let layout = LayoutBuilder::new()
            .texture_array_unfilterable()
            .texture_array_unfilterable()
            .storage_texture_array(SPECTRUM_FORMAT)
            .storage_texture_array(SPECTRUM_FORMAT)
            .build(gpu, "ocean rows");
        let rows = (
            gpu.compute_pipeline(
                "ocean rows",
                &module(gpu, "ocean/fft_rows.wgsl", size),
                "main",
                &[&layout],
            ),
            gpu.bind_group(
                "ocean rows",
                &layout,
                &[
                    tex(&spectrum[0]),
                    tex(&spectrum[1]),
                    tex(&rows_out[0]),
                    tex(&rows_out[1]),
                ],
            ),
        );

        let layout = LayoutBuilder::new()
            .uniform()
            .texture_array_unfilterable()
            .texture_array_unfilterable()
            .texture_array_unfilterable()
            .storage_texture_array(WAVE_FORMAT)
            .storage_texture_array(WAVE_FORMAT)
            .storage_texture_array(FOAM_FORMAT)
            .build(gpu, "ocean columns");
        // Each frame reads the foam the last one wrote, and writes the other.
        let columns_group = |read: usize| {
            gpu.bind_group(
                "ocean columns",
                &layout,
                &[
                    uniform.as_entire_binding(),
                    tex(&rows_out[0]),
                    tex(&rows_out[1]),
                    tex(&foam[read]),
                    level(&displacement, 0),
                    level(&derivatives, 0),
                    tex(&foam[1 - read]),
                ],
            )
        };
        let columns = (
            gpu.compute_pipeline(
                "ocean columns",
                &module(gpu, "ocean/fft_columns.wgsl", size),
                "main",
                &[&layout],
            ),
            [columns_group(0), columns_group(1)],
        );

        let layout = LayoutBuilder::new()
            .texture_array()
            .texture_array()
            .storage_texture_array(WAVE_FORMAT)
            .storage_texture_array(WAVE_FORMAT)
            .build(gpu, "ocean mips");
        let mips = (
            gpu.compute_pipeline(
                "ocean mips",
                &shaders::module(&gpu.device, "ocean/mips.wgsl"),
                "main",
                &[&layout],
            ),
            (1..displacement.levels.len())
                .map(|l| {
                    gpu.bind_group(
                        "ocean mips",
                        &layout,
                        &[
                            level(&displacement, l - 1),
                            level(&derivatives, l - 1),
                            level(&displacement, l),
                            level(&derivatives, l),
                        ],
                    )
                })
                .collect(),
        );

        let [first, second] = band_limits();
        Self {
            size,
            uniform,
            amplitudes,
            displacement,
            derivatives,
            foam,
            build_spectrum,
            evolve,
            rows,
            columns,
            mips,
            built: None,
            since_build: 0.0,
            pending_build: false,
            time: 0.0,
            seed: 0x5eed,
            frame: 0,
            foam_stretch: FOAM_STRETCH,
            generation: GENERATION.fetch_add(1, Ordering::Relaxed),
            uniform_data: OceanUniform {
                sizes: [CASCADE_SIZES[0], CASCADE_SIZES[1], CASCADE_SIZES[2], 0.0],
                bands: [first, second, 0.0, 0.0],
                wind: [0.0; 4],
                shape: [0.0; 4],
                swell: [0.0; 4],
                motion: [0.0; 4],
                foam: [FOAM_STRETCH, FOAM_RATE, 1.0, 0.0],
                seed: 0x5eed,
                _pad: [0; 3],
            },
        }
    }

    /// Texels across each cascade's tile.
    pub fn size(&self) -> u32 {
        self.size
    }

    /// Different for every set of wave textures ever made, so bind groups know when to remake.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// How far through the sea's repeat it is, 0..1.
    pub fn tau(&self) -> f32 {
        self.tau_at(self.time)
    }

    /// The foam the last [`Self::record`] wrote, per cascade. Frame `n` reads foam `n % 2` and
    /// writes the other, and `frame` has counted it by then.
    pub fn foam(&self) -> &CascadeTexture {
        &self.foam[self.frame % 2]
    }

    /// Records this frame's waves: the amplitudes if the sea has changed, then the transform.
    pub fn record(&mut self, encoder: &mut wgpu::CommandEncoder) {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("ocean waves"),
            timestamp_writes: None,
        });
        let texels = groups_for(self.size, 8);
        if self.pending_build {
            pass.set_pipeline(&self.build_spectrum.0);
            pass.set_bind_group(0, &self.build_spectrum.1, &[]);
            pass.dispatch_workgroups(texels, texels, CASCADES);
            self.pending_build = false;
        }
        pass.set_pipeline(&self.evolve.0);
        pass.set_bind_group(0, &self.evolve.1, &[]);
        pass.dispatch_workgroups(texels, texels, CASCADES);
        // One workgroup per line of each cascade.
        pass.set_pipeline(&self.rows.0);
        pass.set_bind_group(0, &self.rows.1, &[]);
        pass.dispatch_workgroups(self.size, CASCADES, 1);
        pass.set_pipeline(&self.columns.0);
        pass.set_bind_group(0, &self.columns.1[self.frame % 2], &[]);
        pass.dispatch_workgroups(self.size, CASCADES, 1);
        // Each level from the one above, down to a single texel.
        pass.set_pipeline(&self.mips.0);
        for (level, group) in self.mips.1.iter().enumerate() {
            let texels = groups_for((self.size >> (level + 1)).max(1), 8);
            pass.set_bind_group(0, group, &[]);
            pass.dispatch_workgroups(texels, texels, CASCADES);
        }
        self.frame += 1;
    }

    /// Moves the sea on by `seconds` towards `sea` on `ocean`. Call before [`Self::record`].
    pub fn update(&mut self, gpu: &Gpu, sea: &SeaState, ocean: &Ocean, seconds: f32) {
        let seconds = seconds.max(0.0);
        self.time += seconds as f64;
        self.since_build += seconds;
        let changed = self.built != Some(*sea);
        let rebuild = self.built.is_none() || (changed && self.since_build >= REBUILD_SECONDS);
        let tau = self.tau_at(self.time);
        let data = &mut self.uniform_data;
        // Only the amplitudes need the spectrum, so its parameters, which take an integral to
        // work out, are only worked out when they are redrawn.
        if rebuild {
            let params = sea.params();
            data.wind = [
                params.peak_wavenumber,
                params.inverse_age,
                params.wind_angle,
                params.short_level,
            ];
            data.shape = [
                params.long_level,
                params.peak_enhancement,
                params.peak_width,
                params.peak_speed,
            ];
            data.swell = [
                params.swell_scale,
                params.swell_peak,
                params.swell_angle,
                0.0,
            ];
            self.built = Some(*sea);
            self.since_build = 0.0;
            self.pending_build = true;
        }
        data.motion = [tau, seconds, ocean.choppiness, 0.0];
        // Foam fades to 1/e over its life, whatever the frame rate.
        let kept = (-seconds / FOAM_LIFE).exp();
        data.foam = [self.foam_stretch, FOAM_RATE * ocean.foam, kept, 0.0];
        data.seed = self.seed;
        gpu.queue
            .write_buffer(&self.uniform, 0, bytemuck::bytes_of(&self.uniform_data));
    }

    fn tau_at(&self, time: f64) -> f32 {
        (time.rem_euclid(REPEAT_SECONDS as f64) / REPEAT_SECONDS as f64) as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ocean::spectrum::{direct_sum, frequency_steps, initial_amplitude, wavenumber};
    use std::f32::consts::TAU;
    use std::sync::{Arc, Mutex};

    use crate::gpu::test_gpu as request_gpu;

    fn sea() -> SeaState {
        SeaState {
            wind_speed: 9.0,
            wind_angle: 0.4,
            fetch: 60_000.0,
            swell_height: 0.6,
            swell_period: 10.0,
            swell_angle: -1.1,
        }
    }

    fn f16_to_f32(bits: u16) -> f32 {
        let sign = if bits & 0x8000 != 0 { -1.0 } else { 1.0 };
        let exponent = ((bits >> 10) & 0x1f) as i32;
        let mantissa = (bits & 0x3ff) as f32;
        sign * match exponent {
            0 => mantissa * 2.0_f32.powi(-24),
            31 => f32::INFINITY,
            _ => (1.0 + mantissa / 1024.0) * 2.0_f32.powi(exponent - 15),
        }
    }

    /// Every texel of one cascade's layer, as four floats whatever the format.
    fn read_layer(gpu: &Gpu, texture: &CascadeTexture, size: u32, layer: u32) -> Vec<[f32; 4]> {
        read_level(gpu, texture, size, layer, 0)
    }

    /// Every texel of one level of one cascade's layer; `size` is the level's own.
    fn read_level(
        gpu: &Gpu,
        texture: &CascadeTexture,
        size: u32,
        layer: u32,
        mip_level: u32,
    ) -> Vec<[f32; 4]> {
        let format = texture.texture.format();
        let texel_bytes = format.block_copy_size(None).unwrap();
        let row_bytes = (size * texel_bytes).div_ceil(256) * 256;
        let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ocean readback"),
            size: (row_bytes * size) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &texture.texture,
                mip_level,
                origin: wgpu::Origin3d {
                    x: 0,
                    y: 0,
                    z: layer,
                },
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row_bytes),
                    rows_per_image: Some(size),
                },
            },
            wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: 1,
            },
        );
        gpu.queue.submit([encoder.finish()]);
        buffer.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        gpu.device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
        let bytes = buffer.slice(..).get_mapped_range().unwrap().to_vec();
        let mut texels = Vec::with_capacity((size * size) as usize);
        for y in 0..size {
            for x in 0..size {
                let at = (y * row_bytes + x * texel_bytes) as usize;
                let texel = &bytes[at..at + texel_bytes as usize];
                let mut value = [0.0; 4];
                if format == wgpu::TextureFormat::Rgba16Float {
                    for (c, v) in value.iter_mut().enumerate() {
                        *v = f16_to_f32(u16::from_le_bytes([texel[2 * c], texel[2 * c + 1]]));
                    }
                } else {
                    for (c, chunk) in texel.chunks_exact(4).enumerate() {
                        value[c] = f32::from_le_bytes(chunk.try_into().unwrap());
                    }
                }
                texels.push(value);
            }
        }
        texels
    }

    /// Waves made on the GPU after `seconds`, failing on any GPU error.
    fn run(gpu: &Gpu, size: u32, seconds: f32) -> OceanWaves {
        let errors: Arc<Mutex<Vec<String>>> = Default::default();
        let sink = errors.clone();
        gpu.device
            .on_uncaptured_error(Arc::new(move |error: wgpu::Error| {
                sink.lock().unwrap().push(error.to_string());
            }));
        let mut waves = OceanWaves::new(gpu, size);
        waves.update(gpu, &sea(), &Ocean::default(), seconds);
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        waves.record(&mut encoder);
        gpu.queue.submit([encoder.finish()]);
        gpu.device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
        let errors = errors.lock().unwrap();
        assert!(
            errors.is_empty(),
            "GPU errors: {}",
            errors.join(
                "
"
            )
        );
        waves
    }

    /// A cascade's starting amplitudes, looked up by wave.
    fn amplitudes_of(texels: &[[f32; 4]], size: u32) -> impl Fn(i32, i32) -> [f32; 2] + '_ {
        let half = size as i32 / 2;
        move |nx, nz| {
            let wrap = |n: i32| (n + half).rem_euclid(size as i32) as usize;
            let texel = texels[wrap(nz) * size as usize + wrap(nx)];
            [texel[0], texel[1]]
        }
    }

    #[test]
    fn the_gpu_draws_the_same_waves_as_the_cpu() {
        let Some(gpu) = request_gpu() else { return };
        let params = sea().params();
        for size in [128, 256] {
            let waves = run(&gpu, size, 0.0);
            let half = size as i32 / 2;
            for cascade in 0..CASCADES {
                let found = read_layer(&gpu, &waves.amplitudes, size, cascade);
                let mut largest = 0.0_f32;
                for (i, texel) in found.iter().enumerate() {
                    let (x, z) = (i as i32 % size as i32, i as i32 / size as i32);
                    let expected = initial_amplitude(
                        cascade as usize,
                        size,
                        x - half,
                        z - half,
                        &params,
                        waves.seed,
                    );
                    largest = largest.max(expected[0].abs()).max(expected[1].abs());
                    for c in 0..2 {
                        assert!(
                            (texel[c] - expected[c]).abs() <= 1.0e-3 * expected[c].abs() + 1.0e-9,
                            "{size}², cascade {cascade}, texel ({x}, {z}): {:?} against {expected:?}",
                            &texel[..2]
                        );
                    }
                }
                assert!(largest > 0.0, "cascade {cascade} carries no waves");
            }
        }
    }

    #[test]
    fn the_transform_matches_a_direct_sum_over_every_wave() {
        let Some(gpu) = request_gpu() else { return };
        // At both sizes the quality presets use, so both builds of each shader are checked.
        for size in [128, 256] {
            check_against_direct_sum(&gpu, size);
        }
    }

    fn check_against_direct_sum(gpu: &Gpu, size: u32) {
        // An arbitrary moment, so that every wave has turned.
        let waves = run(gpu, size, 37.3);
        let tau = waves.tau();
        for (cascade, tile) in CASCADE_SIZES.iter().enumerate() {
            let texels = read_layer(gpu, &waves.amplitudes, size, cascade as u32);
            let amplitude = amplitudes_of(&texels, size);
            let displacement = read_layer(gpu, &waves.displacement, size, cascade as u32);
            let derivatives = read_layer(gpu, &waves.derivatives, size, cascade as u32);
            // Each field's own scale, for a tolerance in keeping with half floats.
            let mut scale = [0.0_f32; 8];
            for (a, b) in displacement.iter().zip(&derivatives) {
                for c in 0..4 {
                    scale[c] = scale[c].max(a[c].abs());
                    scale[c + 4] = scale[c + 4].max(b[c].abs());
                }
            }
            let texel_metres = tile / size as f32;
            // Texels spread over the tile, wherever its size puts them.
            for (u, v) in [
                (0.0, 0.0),
                (0.07, 0.79),
                (0.5, 0.25),
                (0.999, 0.999),
                (0.35, 0.004),
                (0.79, 0.55),
            ] {
                let (x, z) = ((u * size as f32) as usize, (v * size as f32) as usize);
                let expected = direct_sum(
                    cascade,
                    size,
                    &amplitude,
                    tau,
                    x as f32 * texel_metres,
                    z as f32 * texel_metres,
                );
                let at = z * size as usize + x;
                let (a, b) = (displacement[at], derivatives[at]);
                let found = [a[0], a[1], a[2], a[3], b[0], b[1], b[2], b[3]];
                for field in 0..8 {
                    let tolerance = 4.0e-3 * scale[field] + 1.0e-5;
                    assert!(
                        (found[field] - expected[field]).abs() <= tolerance,
                        "{size}², cascade {cascade}, texel ({x}, {z}), field {field}: {} against {} (scale {})",
                        found[field],
                        expected[field],
                        scale[field]
                    );
                }
            }
        }
    }

    #[test]
    fn the_sea_holds_exactly_the_energy_of_its_waves() {
        // Parseval: the height's variance across a tile is the sum of the squared amplitudes
        // of its waves at that moment. This pins the transform's scale.
        let Some(gpu) = request_gpu() else { return };
        for size in [128, 256] {
            let waves = run(&gpu, size, 12.0);
            check_energy(&gpu, &waves, size);
        }
    }

    fn check_energy(gpu: &Gpu, waves: &OceanWaves, size: u32) {
        let tau = waves.tau();
        let half = size as i32 / 2;
        for cascade in 0..CASCADES as usize {
            let texels = read_layer(gpu, &waves.amplitudes, size, cascade as u32);
            let amplitude = amplitudes_of(&texels, size);
            let mut energy = 0.0_f64;
            for nz in -half..half {
                for nx in -half..half {
                    let [kx, kz] = wavenumber(cascade, nx, nz);
                    let phase =
                        TAU as f64 * (frequency_steps(kx.hypot(kz)) as f64 * tau as f64).fract();
                    let (s, c) = phase.sin_cos();
                    let [ar, ai] = amplitude(nx, nz).map(|v| v as f64);
                    let [mr, mi] = amplitude(-nx, -nz).map(|v| v as f64);
                    let hr = ar * c - ai * s + mr * c - mi * s;
                    let hi = ar * s + ai * c - mr * s - mi * c;
                    energy += hr * hr + hi * hi;
                }
            }
            let heights = read_layer(gpu, &waves.displacement, size, cascade as u32);
            let variance =
                heights.iter().map(|t| (t[1] as f64).powi(2)).sum::<f64>() / heights.len() as f64;
            assert!(
                (variance / energy - 1.0).abs() < 0.01,
                "{size}², cascade {cascade}: variance {variance} against energy {energy}"
            );
        }
    }

    #[test]
    fn each_mip_level_is_the_mean_of_the_one_above() {
        let Some(gpu) = request_gpu() else { return };
        let size = 128;
        let waves = run(&gpu, size, 5.0);
        for texture in [&waves.displacement, &waves.derivatives] {
            assert_eq!(texture.levels.len(), 8);
            for cascade in 0..CASCADES {
                for level in [1, 4, 7] {
                    let above = read_level(&gpu, texture, size >> (level - 1), cascade, level - 1);
                    let here = read_level(&gpu, texture, size >> level, cascade, level);
                    let width = (size >> level) as usize;
                    for z in 0..width {
                        for x in 0..width {
                            let at =
                                |dx: usize, dz: usize| above[(2 * z + dz) * 2 * width + 2 * x + dx];
                            for (c, found) in here[z * width + x].iter().copied().enumerate() {
                                let mean =
                                    (at(0, 0)[c] + at(1, 0)[c] + at(0, 1)[c] + at(1, 1)[c]) / 4.0;
                                assert!(
                                    (found - mean).abs() <= mean.abs() * 2.0e-3 + 1.0e-4,
                                    "level {level}, cascade {cascade}, ({x}, {z}), channel {c}: {found} against {mean}"
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn foam_builds_where_crests_fold_and_fades_over_time() {
        // Each frame's foam is the last frame's, faded, or the new breaking of this frame's own
        // surface, whichever is more; and `foam` hands out this frame's.
        let Some(gpu) = request_gpu() else { return };
        let size = 128;
        let gale = SeaState {
            wind_speed: 16.0,
            fetch: 300_000.0,
            ..sea()
        };
        // Foam forms readily, so that many texels break.
        let ocean = Ocean {
            foam: 30.0,
            ..Ocean::default()
        };
        let seconds = 0.5;
        let mut waves = OceanWaves::new(&gpu, size);
        // Single cascades rarely fold by themselves, so count anything squeezed at all.
        waves.foam_stretch = 1.0;
        let frame = |waves: &mut OceanWaves| {
            waves.update(&gpu, &gale, &ocean, seconds);
            let mut encoder = gpu.device.create_command_encoder(&Default::default());
            waves.record(&mut encoder);
            gpu.queue.submit([encoder.finish()]);
        };
        frame(&mut waves);
        frame(&mut waves);
        let previous: Vec<Vec<[f32; 4]>> = (0..CASCADES)
            .map(|c| read_layer(&gpu, waves.foam(), size, c))
            .collect();
        frame(&mut waves);
        let kept = (-seconds / FOAM_LIFE).exp();
        let rate = FOAM_RATE * ocean.foam;
        let lambda = ocean.choppiness;
        let (mut breaking_texels, mut faded_texels) = (0, 0);
        for cascade in 0..CASCADES {
            let foam = read_layer(&gpu, waves.foam(), size, cascade);
            let displacement = read_layer(&gpu, &waves.displacement, size, cascade);
            let derivatives = read_layer(&gpu, &waves.derivatives, size, cascade);
            for i in 0..foam.len() {
                let (a, b) = (displacement[i], derivatives[i]);
                let stretch =
                    (1.0 + lambda * b[2]) * (1.0 + lambda * b[3]) - lambda * lambda * a[3] * a[3];
                let breaking = ((waves.foam_stretch - stretch) * rate).clamp(0.0, 1.0);
                let faded = previous[cascade as usize][i][0] * kept;
                if stretch > waves.foam_stretch + 0.01 {
                    // Clearly not breaking: the foam is exactly last frame's, faded. Nothing
                    // here passes through half floats, so this holds tightly.
                    assert!(
                        (foam[i][0] - faded).abs() <= 1.0e-5,
                        "cascade {cascade}, texel {i}: {} should have faded to {faded}",
                        foam[i][0]
                    );
                    faded_texels += (faded > 0.01) as usize;
                } else {
                    // Near or past the threshold, the stretch worked out here from half floats
                    // differs a little from the GPU's.
                    let expected = breaking.max(faded);
                    let tolerance = rate * 2.0e-3 + 1.0e-4;
                    assert!(
                        (foam[i][0] - expected).abs() <= tolerance,
                        "cascade {cascade}, texel {i}: {} against {expected}",
                        foam[i][0]
                    );
                    breaking_texels += (breaking > 0.0) as usize;
                }
            }
        }
        assert!(breaking_texels > 0, "no crest broke");
        assert!(faded_texels > 0, "no foam lingered");
    }

    /// Writes pictures of the sea for a person to judge: shaded relief of the three cascades
    /// summed, and foam, for a calm sea and a rough one, each 128 m across. Run with
    /// `SOLSTICE_OCEAN_IMAGES=DIR cargo test -p solstice-render --lib -- --ignored ocean_images`.
    #[test]
    #[ignore]
    fn ocean_images() {
        let Ok(dir) = std::env::var("SOLSTICE_OCEAN_IMAGES") else {
            return;
        };
        let Some(gpu) = request_gpu() else { return };
        let size = 256;
        for (name, wind, fetch) in [
            ("calm", 3.0, 30_000.0),
            ("fresh", 9.0, 60_000.0),
            ("gale", 16.0, 300_000.0),
        ] {
            let state = SeaState {
                wind_speed: wind,
                fetch,
                ..sea()
            };
            let mut waves = OceanWaves::new(&gpu, size);
            // Two seconds of frames, so foam has built up and faded as it would.
            for _ in 0..120 {
                waves.update(&gpu, &state, &Ocean::default(), 1.0 / 60.0);
                let mut encoder = gpu.device.create_command_encoder(&Default::default());
                waves.record(&mut encoder);
                gpu.queue.submit([encoder.finish()]);
            }
            let layers: Vec<_> = (0..CASCADES)
                .map(|c| {
                    (
                        read_layer(&gpu, &waves.displacement, size, c),
                        read_layer(&gpu, &waves.derivatives, size, c),
                        read_layer(&gpu, waves.foam(), size, c),
                    )
                })
                .collect();
            // Bilinear, wrapping: one field `field` of cascade `c` at (x, z) metres.
            let sample = |c: usize, layer: &[[f32; 4]], field: usize, x: f32, z: f32| {
                let u = x / CASCADE_SIZES[c] * size as f32 - 0.5;
                let v = z / CASCADE_SIZES[c] * size as f32 - 0.5;
                let (u0, v0) = (u.floor(), v.floor());
                let (fu, fv) = (u - u0, v - v0);
                let at = |du: i32, dv: i32| {
                    let wrap = |t: i32| t.rem_euclid(size as i32) as usize;
                    layer[wrap(v0 as i32 + dv) * size as usize + wrap(u0 as i32 + du)][field]
                };
                let top = at(0, 0) * (1.0 - fu) + at(1, 0) * fu;
                let bottom = at(0, 1) * (1.0 - fu) + at(1, 1) * fu;
                top * (1.0 - fv) + bottom * fv
            };
            let light = glam::Vec3::new(-0.6, 0.5, -0.4).normalize();
            let choppiness = Ocean::default().choppiness;
            let wave_height = state.wind_sea_height();
            // Close up: 32 m across, every cascade, pixels about the finest texels' size.
            // Far off: 1 km across, the two larger cascades, shaded by height.
            for (view, metres, pixels, cascades) in [
                ("near", 32.0_f32, 512usize, 3usize),
                ("far", 1000.0, 512, 2),
            ] {
                let mut image = Vec::with_capacity(pixels * pixels * 3);
                let (mut folded, mut squeezed, mut lowest) = (0usize, 0usize, f32::MAX);
                for py in 0..pixels {
                    for px in 0..pixels {
                        let (x, z) = (
                            px as f32 / pixels as f32 * metres,
                            py as f32 / pixels as f32 * metres,
                        );
                        let mut d = [0.0_f32; 8];
                        for (c, (displacement, derivatives, _)) in
                            layers.iter().enumerate().take(cascades)
                        {
                            for f in 0..4 {
                                d[f] += sample(c, displacement, f, x, z);
                                d[f + 4] += sample(c, derivatives, f, x, z);
                            }
                        }
                        // The whole surface's stretch, from every cascade's derivatives together.
                        let stretch = (1.0 + choppiness * d[6]) * (1.0 + choppiness * d[7])
                            - choppiness * choppiness * d[3] * d[3];
                        lowest = lowest.min(stretch);
                        folded += (stretch < 0.0) as usize;
                        squeezed += (stretch < 0.55) as usize;
                        let normal = glam::Vec3::new(-d[4], 1.0, -d[5]).normalize();
                        let shade = normal.dot(light).max(0.0) * 0.8 + 0.2;
                        let height =
                            (d[1] / (wave_height.max(0.3) + 0.6) * 0.5 + 0.5).clamp(0.0, 1.0);
                        let base = if view == "far" {
                            0.35 + 0.65 * height
                        } else {
                            1.0
                        };
                        let white = if stretch < 0.55 { 1.0 } else { 0.0 };
                        let rgb = [0.25, 0.5, 0.85].map(|c| {
                            ((c * base * shade * (1.0 - white) + white) * 255.0).min(255.0) as u8
                        });
                        image.extend(rgb);
                    }
                }
                let share = |n: usize| n as f32 / (pixels * pixels) as f32 * 100.0;
                println!(
                    "{name} {view}: stretch at least {lowest:.2}; {:.2}% squeezed below 0.55, {:.3}% folded",
                    share(squeezed),
                    share(folded)
                );
                let mut ppm = format!(
                    "P6
{pixels} {pixels}
255
"
                )
                .into_bytes();
                ppm.extend(image);
                std::fs::write(format!("{dir}/{name}_{view}.ppm"), ppm).unwrap();
            }
            println!("{name}: {wind} m/s, Hs {wave_height:.2} m");
        }
    }
}
