use super::*;
use crate::frame::FrameUniform;
use glam::Mat4;

#[test]
fn diffuse_history_rejects_invalid_samples_instead_of_spreading_them() {
    let Some(gpu) = gpu() else { return };
    const SIZE: u32 = 32;
    let make = |name, format| {
        Target::new(
            &gpu,
            name,
            (SIZE, SIZE),
            format,
            wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::COPY_SRC,
        )
    };
    let normal = make("GI stress normal", FORMAT);
    let depth = make("GI stress depth", wgpu::TextureFormat::R32Float);
    let motion = make("GI stress motion", wgpu::TextureFormat::Rg32Float);
    let out_depth = make("GI stress output depth", wgpu::TextureFormat::R32Float);
    let history = [
        make("GI stress history 0", FORMAT),
        make("GI stress history 1", FORMAT),
    ];
    let layout = LayoutBuilder::new()
        .texture()
        .texture_unfilterable()
        .texture_unfilterable()
        .texture()
        .storage_read()
        .storage_texture(FORMAT)
        .texture_unfilterable()
        .storage_texture(wgpu::TextureFormat::R32Float)
        .build(&gpu, "GI stress");
    let frame_layout = LayoutBuilder::new()
        .uniform()
        .build(&gpu, "GI stress frame");
    let module = shaders::module(&gpu.device, "gi/probe_integrate.wgsl");
    let pipeline = gpu.compute_pipeline("GI stress", &module, "main", &[&layout, &frame_layout]);
    let frame_buffer = gpu.uniform_buffer(
        "GI stress frame",
        std::mem::size_of::<FrameUniform>() as u64,
    );
    let frame_group = gpu.bind_group(
        "GI stress frame",
        &frame_layout,
        &[frame_buffer.as_entire_binding()],
    );
    let probes = gpu.storage_buffer_init("GI stress probes", &[0.0f32; 4 * 44]);
    let groups: Vec<_> = (0..2)
        .map(|p| {
            gpu.bind_group(
                "GI stress",
                &layout,
                &[
                    wgpu::BindingResource::TextureView(&normal.view),
                    wgpu::BindingResource::TextureView(&depth.view),
                    wgpu::BindingResource::TextureView(&motion.view),
                    wgpu::BindingResource::TextureView(&history[1 - p].view),
                    probes.as_entire_binding(),
                    wgpu::BindingResource::TextureView(&history[p].view),
                    wgpu::BindingResource::TextureView(&depth.view),
                    wgpu::BindingResource::TextureView(&out_depth.view),
                ],
            )
        })
        .collect();
    write(
        &gpu,
        &depth,
        bytemuck::cast_slice(&vec![1.0f32; (SIZE * SIZE) as usize]),
        4,
        SIZE,
    );
    let white = vec![0x3c00u16; (SIZE * SIZE * 4) as usize];
    let mut frame = FrameUniform::zeroed();
    frame.viewport = Vec2::splat(SIZE as f32);
    frame.inv_viewport = frame.viewport.recip();
    frame.inv_view_proj = Mat4::IDENTITY;
    frame.prev_view_proj = Mat4::IDENTITY;
    frame.near = 1.0;
    frame.frame_seconds = 1.0 / 60.0;
    let mut failures = Vec::new();
    for bad in [0x7e00u16, 0x7c00, 0xfc00] {
        for target in &history {
            write(&gpu, target, bytemuck::cast_slice(&white), 8, SIZE);
        }
        let mut damaged = white.clone();
        damaged[((SIZE / 2 * SIZE + SIZE / 2) * 4) as usize] = bad;
        write(&gpu, &history[1], bytemuck::cast_slice(&damaged), 8, SIZE);
        let mut counts = Vec::new();
        for tick in 0..12 {
            frame.frame_index = tick + 1;
            gpu.queue
                .write_buffer(&frame_buffer, 0, bytemuck::bytes_of(&frame));
            let mut encoder = gpu.device.create_command_encoder(&Default::default());
            {
                let mut pass = encoder.begin_compute_pass(&Default::default());
                pass.set_pipeline(&pipeline);
                pass.set_bind_group(0, &groups[tick as usize % 2], &[]);
                pass.set_bind_group(1, &frame_group, &[]);
                pass.dispatch_workgroups(SIZE.div_ceil(8), SIZE.div_ceil(8), 1);
            }
            gpu.queue.submit([encoder.finish()]);
            let pixels = read(&gpu, &history[tick as usize % 2], SIZE);
            counts.push(
                pixels
                    .chunks_exact(4)
                    .filter(|p| p.iter().any(|v| v & 0x7c00 == 0x7c00))
                    .count(),
            );
        }
        eprintln!("diffuse bad={bad:#x}, invalid pixels: {counts:?}");
        failures.extend(counts);
    }
    assert!(
        failures.iter().all(|&n| n == 0),
        "GI history must recover on the first healthy frame"
    );
}

fn gpu() -> Option<Gpu> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::PRIMARY,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    pollster::block_on(Gpu::request(
        &instance,
        None,
        wgpu::PowerPreference::HighPerformance,
    ))
    .map_err(|error| eprintln!("skipping GPU stress test: {error}"))
    .ok()
}

fn write(gpu: &Gpu, target: &Target, bytes: &[u8], bytes_per_pixel: u32, size: u32) {
    gpu.queue.write_texture(
        target.texture.as_image_copy(),
        bytes,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(size * bytes_per_pixel),
            rows_per_image: Some(size),
        },
        wgpu::Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
    );
}

fn read(gpu: &Gpu, target: &Target, size: u32) -> Vec<u16> {
    let row = (size * 8).next_multiple_of(256);
    let staging = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("HDR stress readback"),
        size: u64::from(row * size),
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        target.texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &staging,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row),
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
    staging
        .slice(..)
        .map_async(wgpu::MapMode::Read, |r| r.unwrap());
    gpu.device
        .poll(wgpu::PollType::wait_indefinitely())
        .unwrap();
    let bytes = staging.slice(..).get_mapped_range().unwrap();
    bytes
        .chunks_exact(row as usize)
        .flat_map(|line| {
            bytemuck::cast_slice::<u8, u16>(&line[..size as usize * 8])
                .iter()
                .copied()
        })
        .collect()
}

#[test]
fn an_invalid_hdr_pixel_does_not_spread_through_temporal_history() {
    let Some(gpu) = gpu() else { return };
    const SIZE: u32 = 32;
    let make = |name, format| {
        Target::new(
            &gpu,
            name,
            (SIZE, SIZE),
            format,
            wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::COPY_SRC,
        )
    };
    let scene = make("stress scene", FORMAT);
    let depth = make("stress depth", wgpu::TextureFormat::R32Float);
    let motion = make("stress motion", wgpu::TextureFormat::Rg32Float);
    let reactive = make("stress reactive", wgpu::TextureFormat::Rgba16Float);
    let history = [
        make("stress history 0", FORMAT),
        make("stress history 1", FORMAT),
    ];
    let post = PostProcess::new(&gpu, wgpu::TextureFormat::Rgba8Unorm);
    let frame_buffer =
        gpu.uniform_buffer("stress frame", std::mem::size_of::<FrameUniform>() as u64);
    let groups: Vec<_> = (0..2)
        .map(|parity| {
            gpu.bind_group(
                "stress TAA",
                &post.taa_layout,
                &[
                    frame_buffer.as_entire_binding(),
                    wgpu::BindingResource::TextureView(&scene.view),
                    wgpu::BindingResource::TextureView(&depth.view),
                    wgpu::BindingResource::TextureView(&motion.view),
                    wgpu::BindingResource::TextureView(&history[1 - parity].view),
                    wgpu::BindingResource::Sampler(&post.linear_sampler),
                    wgpu::BindingResource::TextureView(&history[parity].view),
                    wgpu::BindingResource::TextureView(&reactive.view),
                ],
            )
        })
        .collect();
    write(
        &gpu,
        &depth,
        bytemuck::cast_slice(&vec![1.0f32; (SIZE * SIZE) as usize]),
        4,
        SIZE,
    );
    let white = vec![0x3c00u16; (SIZE * SIZE * 4) as usize];
    let mut frame = FrameUniform::zeroed();
    frame.viewport = Vec2::splat(SIZE as f32);
    frame.inv_viewport = frame.viewport.recip();
    frame.inv_view_proj = Mat4::IDENTITY;
    frame.prev_view_proj = Mat4::IDENTITY;
    frame.temporal_accumulation = 1;
    // Both new lighting and old history can carry a non-finite half-float. Each must recover.
    for (bad, in_history, whole_image) in [
        (0x7e00u16, false, false),
        (0x7c00, false, false),
        (0xfc00, false, false),
        (0x7e00, true, false),
        (0x7c00, true, false),
        (0xfc00, true, false),
        (0x7e00, false, true),
        (0x7e00, true, true),
    ] {
        write(&gpu, &scene, bytemuck::cast_slice(&white), 8, SIZE);
        for target in &history {
            write(&gpu, target, bytemuck::cast_slice(&white), 8, SIZE);
        }
        let mut damaged = white.clone();
        damaged[((SIZE / 2 * SIZE + SIZE / 2) * 4) as usize] = bad;
        if whole_image {
            for pixel in damaged.chunks_exact_mut(4) {
                pixel[0] = bad;
            }
        }
        write(
            &gpu,
            if in_history { &history[1] } else { &scene },
            bytemuck::cast_slice(&damaged),
            8,
            SIZE,
        );
        let mut invalid_per_frame = Vec::new();
        let mut dark_per_frame = Vec::new();
        let mut final_pixels = Vec::new();
        for tick in 0..12 {
            frame.frame_index = tick + 1;
            gpu.queue
                .write_buffer(&frame_buffer, 0, bytemuck::bytes_of(&frame));
            let parity = tick as usize % 2;
            let mut encoder = gpu.device.create_command_encoder(&Default::default());
            {
                let mut pass = encoder.begin_compute_pass(&Default::default());
                pass.set_pipeline(&post.taa);
                pass.set_bind_group(0, &groups[parity], &[]);
                pass.dispatch_workgroups(SIZE.div_ceil(8), SIZE.div_ceil(8), 1);
            }
            gpu.queue.submit([encoder.finish()]);
            final_pixels = read(&gpu, &history[parity], SIZE);
            invalid_per_frame.push(
                final_pixels
                    .chunks_exact(4)
                    .filter(|p| p[..3].iter().any(|v| v & 0x7c00 == 0x7c00))
                    .count(),
            );
            dark_per_frame.push(
                final_pixels
                    .chunks_exact(4)
                    .filter(|p| p[..3].iter().any(|&v| v < 0x3b00 || v & 0x8000 != 0))
                    .count(),
            );
            // Only the first frame is damaged; every subsequent lighting input is healthy.
            write(&gpu, &scene, bytemuck::cast_slice(&white), 8, SIZE);
        }
        eprintln!("bad={bad:#x}, in_history={in_history}, whole_image={whole_image}, non-finite: {invalid_per_frame:?}, dark: {dark_per_frame:?}");
        assert!(
            invalid_per_frame
                .iter()
                .chain(&dark_per_frame)
                .all(|&n| n == 0),
            "an invalid sample must not contaminate the white scene"
        );
        assert!(
            final_pixels.iter().all(|&v| (0x3bf0..=0x3c10).contains(&v)),
            "the white scene must recover without a spreading dark patch"
        );
    }
}
