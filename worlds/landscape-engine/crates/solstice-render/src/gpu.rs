//! Device acquisition and small helpers that remove wgpu boilerplate.

use std::fmt;

/// Why the GPU could not be initialised.
#[derive(Debug)]
pub enum GpuError {
    NoAdapter(String),
    Device(wgpu::RequestDeviceError),
    /// The adapter lacks something this renderer cannot work without.
    Unsupported(String),
}

impl fmt::Display for GpuError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoAdapter(reason) => write!(f, "no suitable GPU adapter: {reason}"),
            Self::Device(error) => write!(f, "could not create a GPU device: {error}"),
            Self::Unsupported(what) => write!(f, "this GPU is not supported: {what}"),
        }
    }
}

impl std::error::Error for GpuError {}

/// Storage buffers the heaviest pass binds in a single shader stage.
const REQUIRED_STORAGE_BUFFERS: u32 = 8;
/// Storage textures the busiest pass binds in a single shader stage, with every fallback.
const REQUIRED_STORAGE_TEXTURES: u32 = 4;
/// Storage textures the G-buffer resolve binds when it writes depth too.
const RESOLVE_STORAGE_TEXTURES: u32 = 5;
/// Sampled textures the ocean's shading binds in one stage, with the scene's depth apart from
/// its depth pyramid.
const OCEAN_SAMPLED_TEXTURES: u32 = 17;

/// What the renderer does differently on a device that offers less, chosen once from the
/// device's limits. A device with all it asks for has none, and pays nothing for them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Fallbacks {
    /// The G-buffer's depth is written by a pass of its own, so that the resolve binds four
    /// storage textures, as the WebGPU defaults and many phones allow, rather than five.
    pub separate_depth: bool,
    /// The ocean's shading reads the opaque scene's depth from mip 0 of its depth pyramid,
    /// which is the same depth, binding 16 sampled textures per stage, as the WebGPU defaults
    /// allow, rather than 17.
    pub ocean_depth_from_pyramid: bool,
    /// A streamed scene's page pool is kept to what one storage binding holds: a larger pool
    /// is split into regions, whose bindings take more storage buffers per stage than this
    /// device offers.
    pub pool_in_one_binding: bool,
}

impl Fallbacks {
    /// The fallbacks a device with these limits needs.
    pub fn for_limits(limits: &wgpu::Limits) -> Self {
        Self {
            separate_depth: limits.max_storage_textures_per_shader_stage < RESOLVE_STORAGE_TEXTURES,
            ocean_depth_from_pyramid: limits.max_sampled_textures_per_shader_stage
                < OCEAN_SAMPLED_TEXTURES,
            pool_in_one_binding: limits.max_storage_buffers_per_shader_stage
                < crate::regions::required_storage_buffers(crate::regions::FEWEST_POOLS),
        }
    }

    /// Whether any fallback is in use.
    pub fn any(&self) -> bool {
        *self != Self::default()
    }
}

/// The device and queue, plus what was learned about them.
pub struct Gpu {
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub adapter_info: wgpu::AdapterInfo,
    pub limits: wgpu::Limits,
    /// Whether scene textures may stay in BC formats. Without this they are unpacked on load.
    pub block_compressed_textures: bool,
    /// What the renderer does differently for these limits.
    pub fallbacks: Fallbacks,
}

impl Gpu {
    /// Requests a device, asking for the adapter's full limits.
    ///
    /// The WebGPU default limits are far below what browsers actually expose on
    /// desktop GPUs, and this renderer needs large storage buffers.
    pub async fn request(
        instance: &wgpu::Instance,
        surface: Option<&wgpu::Surface<'_>>,
        power_preference: wgpu::PowerPreference,
    ) -> Result<Self, GpuError> {
        Self::request_within(instance, surface, power_preference, None).await
    }

    /// Like [`Gpu::request`], but asks for no more than `cap` of any limit: to see on a large
    /// GPU what a small one is given.
    pub async fn request_within(
        instance: &wgpu::Instance,
        surface: Option<&wgpu::Surface<'_>>,
        power_preference: wgpu::PowerPreference,
        cap: Option<&wgpu::Limits>,
    ) -> Result<Self, GpuError> {
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference,
                force_fallback_adapter: false,
                compatible_surface: surface,
                apply_limit_buckets: false,
            })
            .await
            .map_err(|error| GpuError::NoAdapter(error.to_string()))?;

        let limits = match cap {
            Some(cap) => adapter.limits().or_worse_values_from(cap),
            None => adapter.limits(),
        };
        if limits.max_storage_buffers_per_shader_stage < REQUIRED_STORAGE_BUFFERS {
            return Err(GpuError::Unsupported(format!(
                "needs {REQUIRED_STORAGE_BUFFERS} storage buffers per stage, has {}",
                limits.max_storage_buffers_per_shader_stage
            )));
        }
        if limits.max_storage_textures_per_shader_stage < REQUIRED_STORAGE_TEXTURES {
            return Err(GpuError::Unsupported(format!(
                "needs {REQUIRED_STORAGE_TEXTURES} storage textures per stage, has {}",
                limits.max_storage_textures_per_shader_stage
            )));
        }

        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("solstice"),
                // All optional. Timestamps let the profiler time passes where the platform
                // allows it, and BC textures save most of the memory that scenes' textures use.
                required_features: adapter.features()
                    & (wgpu::Features::TIMESTAMP_QUERY
                        | wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS
                        | wgpu::Features::TEXTURE_COMPRESSION_BC),
                required_limits: limits.clone(),
                ..Default::default()
            })
            .await
            .map_err(GpuError::Device)?;

        let block_compressed_textures = device
            .features()
            .contains(wgpu::Features::TEXTURE_COMPRESSION_BC);
        let adapter_info = adapter.get_info();
        log::info!("using {} ({:?})", adapter_info.name, adapter_info.backend);
        let fallbacks = Fallbacks::for_limits(&limits);
        if fallbacks.any() {
            log::info!("with fallbacks for its limits: {fallbacks:?}");
        }
        Ok(Self {
            adapter,
            device,
            queue,
            adapter_info,
            limits,
            block_compressed_textures,
            fallbacks,
        })
    }

    /// Creates a compute pipeline.
    pub fn compute_pipeline(
        &self,
        label: &str,
        module: &wgpu::ShaderModule,
        entry_point: &str,
        layouts: &[&wgpu::BindGroupLayout],
    ) -> wgpu::ComputePipeline {
        self.compute_pipeline_with(label, module, entry_point, layouts, &[])
    }

    /// Creates a compute pipeline, setting the shader's `override` constants by name.
    pub fn compute_pipeline_with(
        &self,
        label: &str,
        module: &wgpu::ShaderModule,
        entry_point: &str,
        layouts: &[&wgpu::BindGroupLayout],
        constants: &[(&str, f64)],
    ) -> wgpu::ComputePipeline {
        let layouts: Vec<Option<&wgpu::BindGroupLayout>> =
            layouts.iter().map(|l| Some(*l)).collect();
        let layout = self
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(label),
                bind_group_layouts: &layouts,
                immediate_size: 0,
            });
        self.device
            .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(label),
                layout: Some(&layout),
                module,
                entry_point: Some(entry_point),
                compilation_options: wgpu::PipelineCompilationOptions {
                    constants,
                    ..Default::default()
                },
                cache: None,
            })
    }

    /// Creates a storage buffer of `size` bytes with no initial contents.
    pub fn storage_buffer(
        &self,
        label: &str,
        size: u64,
        extra: wgpu::BufferUsages,
    ) -> wgpu::Buffer {
        self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: size.max(16),
            usage: wgpu::BufferUsages::STORAGE | extra,
            mapped_at_creation: false,
        })
    }

    /// Creates a storage buffer holding `data`. Bindings cannot be empty and must fit one
    /// element, so an empty slice gets a zeroed buffer the size of one element.
    pub fn storage_buffer_init<T: bytemuck::Pod>(&self, label: &str, data: &[T]) -> wgpu::Buffer {
        let placeholder = vec![0u8; std::mem::size_of::<T>().max(16)];
        let bytes: &[u8] = if data.is_empty() {
            &placeholder
        } else {
            bytemuck::cast_slice(data)
        };
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: bytes.len() as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: true,
        });
        buffer
            .slice(..)
            .get_mapped_range_mut()
            .expect("a buffer mapped at creation is writable")
            .copy_from_slice(bytes);
        buffer.unmap();
        buffer
    }

    /// Creates a uniform buffer of `size` bytes, updated with `Queue::write_buffer`.
    pub fn uniform_buffer(&self, label: &str, size: u64) -> wgpu::Buffer {
        self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    /// Creates a bind group, numbering `resources` from binding 0 in order.
    pub fn bind_group(
        &self,
        label: &str,
        layout: &wgpu::BindGroupLayout,
        resources: &[wgpu::BindingResource<'_>],
    ) -> wgpu::BindGroup {
        let entries: Vec<wgpu::BindGroupEntry<'_>> = resources
            .iter()
            .enumerate()
            .map(|(binding, resource)| wgpu::BindGroupEntry {
                binding: binding as u32,
                resource: resource.clone(),
            })
            .collect();
        self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(label),
            layout,
            entries: &entries,
        })
    }

    /// Creates a bind group like [`Gpu::bind_group`] for a layout made with
    /// [`LayoutBuilder::skip`] at binding `skipped`: resources from there on take the binding
    /// after their place in the list.
    pub fn bind_group_skipping(
        &self,
        label: &str,
        layout: &wgpu::BindGroupLayout,
        resources: &[wgpu::BindingResource<'_>],
        skipped: u32,
    ) -> wgpu::BindGroup {
        let entries: Vec<wgpu::BindGroupEntry<'_>> = resources
            .iter()
            .enumerate()
            .map(|(place, resource)| {
                let place = place as u32;
                wgpu::BindGroupEntry {
                    binding: if place < skipped { place } else { place + 1 },
                    resource: resource.clone(),
                }
            })
            .collect();
        self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(label),
            layout,
            entries: &entries,
        })
    }
}

/// Builds a bind group layout, numbering entries from binding 0 in call order.
///
/// Calls read like the shader's binding list, which makes the two easy to compare.
pub struct LayoutBuilder {
    entries: Vec<wgpu::BindGroupLayoutEntry>,
    visibility: wgpu::ShaderStages,
    /// The binding the next entry takes.
    next: u32,
}

impl LayoutBuilder {
    /// Entries visible to compute shaders until [`Self::visibility`] says otherwise.
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            visibility: wgpu::ShaderStages::COMPUTE,
            next: 0,
        }
    }

    /// Sets the stages for entries added after this call.
    pub fn visibility(mut self, visibility: wgpu::ShaderStages) -> Self {
        self.visibility = visibility;
        self
    }

    fn push(mut self, ty: wgpu::BindingType) -> Self {
        self.entries.push(wgpu::BindGroupLayoutEntry {
            binding: self.next,
            visibility: self.visibility,
            ty,
            count: None,
        });
        self.next += 1;
        self
    }

    /// Leaves a binding out, for a variant of a shader that does without it: later entries
    /// keep their numbers. Its bind groups are made with [`Gpu::bind_group_skipping`].
    pub fn skip(mut self) -> Self {
        self.next += 1;
        self
    }

    fn buffer(self, ty: wgpu::BufferBindingType, dynamic: bool) -> Self {
        self.push(wgpu::BindingType::Buffer {
            ty,
            has_dynamic_offset: dynamic,
            min_binding_size: None,
        })
    }

    pub fn uniform(self) -> Self {
        self.buffer(wgpu::BufferBindingType::Uniform, false)
    }

    pub fn uniform_dynamic(self) -> Self {
        self.buffer(wgpu::BufferBindingType::Uniform, true)
    }

    pub fn storage_read(self) -> Self {
        self.buffer(wgpu::BufferBindingType::Storage { read_only: true }, false)
    }

    pub fn storage_write(self) -> Self {
        self.buffer(wgpu::BufferBindingType::Storage { read_only: false }, false)
    }

    fn texture_of(
        self,
        sample_type: wgpu::TextureSampleType,
        dimension: wgpu::TextureViewDimension,
    ) -> Self {
        self.push(wgpu::BindingType::Texture {
            sample_type,
            view_dimension: dimension,
            multisampled: false,
        })
    }

    /// A filterable floating-point 2D texture.
    pub fn texture(self) -> Self {
        self.texture_of(
            wgpu::TextureSampleType::Float { filterable: true },
            wgpu::TextureViewDimension::D2,
        )
    }

    /// A non-filterable floating-point 2D texture, such as `r32float`.
    pub fn texture_unfilterable(self) -> Self {
        self.texture_of(
            wgpu::TextureSampleType::Float { filterable: false },
            wgpu::TextureViewDimension::D2,
        )
    }

    /// An unsigned integer 2D texture, such as `rgba32uint`.
    pub fn texture_uint(self) -> Self {
        self.texture_of(
            wgpu::TextureSampleType::Uint,
            wgpu::TextureViewDimension::D2,
        )
    }

    pub fn texture_array(self) -> Self {
        self.texture_of(
            wgpu::TextureSampleType::Float { filterable: true },
            wgpu::TextureViewDimension::D2Array,
        )
    }

    /// A non-filterable floating-point 2D texture array, such as `rgba32float`.
    pub fn texture_array_unfilterable(self) -> Self {
        self.texture_of(
            wgpu::TextureSampleType::Float { filterable: false },
            wgpu::TextureViewDimension::D2Array,
        )
    }

    pub fn texture_3d(self) -> Self {
        self.texture_of(
            wgpu::TextureSampleType::Float { filterable: true },
            wgpu::TextureViewDimension::D3,
        )
    }

    pub fn depth_texture(self) -> Self {
        self.texture_of(
            wgpu::TextureSampleType::Depth,
            wgpu::TextureViewDimension::D2,
        )
    }

    pub fn depth_texture_array(self) -> Self {
        self.texture_of(
            wgpu::TextureSampleType::Depth,
            wgpu::TextureViewDimension::D2Array,
        )
    }

    pub fn storage_texture(self, format: wgpu::TextureFormat) -> Self {
        self.push(wgpu::BindingType::StorageTexture {
            access: wgpu::StorageTextureAccess::WriteOnly,
            format,
            view_dimension: wgpu::TextureViewDimension::D2,
        })
    }

    pub fn storage_texture_array(self, format: wgpu::TextureFormat) -> Self {
        self.push(wgpu::BindingType::StorageTexture {
            access: wgpu::StorageTextureAccess::WriteOnly,
            format,
            view_dimension: wgpu::TextureViewDimension::D2Array,
        })
    }

    pub fn storage_texture_3d(self, format: wgpu::TextureFormat) -> Self {
        self.push(wgpu::BindingType::StorageTexture {
            access: wgpu::StorageTextureAccess::WriteOnly,
            format,
            view_dimension: wgpu::TextureViewDimension::D3,
        })
    }

    pub fn sampler(self) -> Self {
        self.push(wgpu::BindingType::Sampler(
            wgpu::SamplerBindingType::Filtering,
        ))
    }

    pub fn sampler_comparison(self) -> Self {
        self.push(wgpu::BindingType::Sampler(
            wgpu::SamplerBindingType::Comparison,
        ))
    }

    pub fn build(self, gpu: &Gpu, label: &str) -> wgpu::BindGroupLayout {
        gpu.device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some(label),
                entries: &self.entries,
            })
    }
}

impl Default for LayoutBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// A sampled, storage-writable 2D render target.
pub struct Target {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
}

impl Target {
    /// Creates a target usable as storage and as a texture. `extra` adds usages.
    pub fn new(
        gpu: &Gpu,
        label: &str,
        size: (u32, u32),
        format: wgpu::TextureFormat,
        extra: wgpu::TextureUsages,
    ) -> Self {
        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: size.0.max(1),
                height: size.1.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::TEXTURE_BINDING
                | extra,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        Self { texture, view }
    }
}

/// Number of workgroups needed to cover `count` items.
pub fn groups_for(count: u32, group_size: u32) -> u32 {
    count.div_ceil(group_size)
}

/// A GPU for the crate's own tests, or `None`, said on stderr, where there is none to have.
///
/// Devices are made one at a time: made at once from many test threads, the Vulkan loader,
/// validation layer and driver on Windows can crash or deadlock.
#[cfg(test)]
pub(crate) fn test_gpu() -> Option<Gpu> {
    static MAKING: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _one_at_a_time = MAKING
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::PRIMARY,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    pollster::block_on(Gpu::request(
        &instance,
        None,
        wgpu::PowerPreference::HighPerformance,
    ))
    .map_err(|error| eprintln!("skipping GPU test: {error}"))
    .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What Chrome gives a desktop GPU, such as the RTX 5080 and the Radeon 780M.
    fn desktop() -> wgpu::Limits {
        wgpu::Limits {
            max_storage_buffers_per_shader_stage: 16,
            max_storage_textures_per_shader_stage: 8,
            max_sampled_textures_per_shader_stage: 48,
            ..wgpu::Limits::defaults()
        }
    }

    #[test]
    fn a_desktop_gpu_needs_no_fallbacks() {
        assert_eq!(Fallbacks::for_limits(&desktop()), Fallbacks::default());
        assert!(!Fallbacks::for_limits(&desktop()).any());
    }

    /// What many phones, and Chrome's own software adapter, are given.
    #[test]
    fn the_webgpu_defaults_need_every_fallback() {
        let fallbacks = Fallbacks::for_limits(&wgpu::Limits::defaults());
        assert_eq!(
            fallbacks,
            Fallbacks {
                separate_depth: true,
                ocean_depth_from_pyramid: true,
                pool_in_one_binding: true,
            }
        );
        assert!(fallbacks.any());
    }

    /// Each fallback turns on exactly where its pass would ask for more than the device has.
    #[test]
    fn each_fallback_starts_one_below_what_its_pass_binds() {
        let with = |change: fn(&mut wgpu::Limits)| {
            let mut limits = desktop();
            change(&mut limits);
            Fallbacks::for_limits(&limits)
        };
        assert!(!with(|l| l.max_storage_textures_per_shader_stage = 5).separate_depth);
        assert!(with(|l| l.max_storage_textures_per_shader_stage = 4).separate_depth);
        assert!(!with(|l| l.max_sampled_textures_per_shader_stage = 17).ocean_depth_from_pyramid);
        assert!(with(|l| l.max_sampled_textures_per_shader_stage = 16).ocean_depth_from_pyramid);
        assert!(!with(|l| l.max_storage_buffers_per_shader_stage = 11).pool_in_one_binding);
        assert!(with(|l| l.max_storage_buffers_per_shader_stage = 10).pool_in_one_binding);
        // Each alone: the others stay off.
        assert_eq!(
            with(|l| l.max_storage_textures_per_shader_stage = 4),
            Fallbacks {
                separate_depth: true,
                ..Default::default()
            }
        );
    }
}
