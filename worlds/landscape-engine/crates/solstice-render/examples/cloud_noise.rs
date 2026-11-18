//! Generates the cloud noise on the GPU and writes it out as images, to check it by eye: the
//! weather map's channels, a slice through each noise volume, and the coverage table.
//!
//! ```text
//! cargo run --release -p solstice-render --example cloud_noise -- OUT_DIR
//! ```

use solstice_render::clouds::{Clouds, NOISE_SIZES};
use solstice_render::lighting::environment_layout;
use solstice_render::Gpu;
use std::io::Write;
use std::path::Path;

/// Reads back one layer of a texture of 4-byte texels.
fn read_layer(gpu: &Gpu, texture: &wgpu::Texture, size: u32, layer: u32) -> Vec<u8> {
    let row = (size * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
    let staging = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size: u64::from(row) * u64::from(size),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d {
                x: 0,
                y: 0,
                z: layer,
            },
            aspect: wgpu::TextureAspect::All,
        },
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
    let data = staging.slice(..).get_mapped_range().expect("mapped");
    let mut texels = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        let start = (y * row) as usize;
        texels.extend_from_slice(&data[start..start + (size * 4) as usize]);
    }
    texels
}

/// Writes each channel of RGBA texels as its own grey image, side by side.
fn write_channels(path: &Path, texels: &[u8], size: u32, channels: usize) {
    let mut file = std::fs::File::create(path).unwrap();
    write!(file, "P5\n{} {}\n255\n", size as usize * channels, size).unwrap();
    for y in 0..size as usize {
        for c in 0..channels {
            for x in 0..size as usize {
                file.write_all(&[texels[(y * size as usize + x) * 4 + c]])
                    .unwrap();
            }
        }
    }
    println!("wrote {}", path.display());
}

fn main() {
    let out = std::env::args().nth(1).expect("usage: cloud_noise OUT_DIR");
    let out = Path::new(&out);
    std::fs::create_dir_all(out).unwrap();
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::PRIMARY,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let gpu = pollster::block_on(Gpu::request(
        &instance,
        None,
        wgpu::PowerPreference::HighPerformance,
    ))
    .expect("a GPU");
    let mut clouds = Clouds::new(&gpu, &environment_layout(&gpu));
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    clouds.generate_noise(&mut encoder);
    gpu.queue.submit([encoder.finish()]);

    let noise = clouds.noise();
    let [shape, detail, weather] = NOISE_SIZES;
    write_channels(
        &out.join("weather.pgm"),
        &read_layer(&gpu, noise.weather_map, weather, 0),
        weather,
        3,
    );
    write_channels(
        &out.join("shape.pgm"),
        &read_layer(&gpu, noise.shape, shape, shape / 2),
        shape,
        4,
    );
    write_channels(
        &out.join("detail.pgm"),
        &read_layer(&gpu, noise.detail, detail, detail / 2),
        detail,
        3,
    );

    let staging = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("cdf readback"),
        size: 1024,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    encoder.copy_buffer_to_buffer(noise.coverage_cdf, 0, &staging, 0, 1024);
    gpu.queue.submit([encoder.finish()]);
    staging
        .slice(..)
        .map_async(wgpu::MapMode::Read, |r| r.unwrap());
    gpu.device
        .poll(wgpu::PollType::wait_indefinitely())
        .unwrap();
    let cdf: Vec<f32> =
        bytemuck::cast_slice(&staging.slice(..).get_mapped_range().expect("mapped")).to_vec();
    for q in [0.1, 0.25, 0.5, 0.75, 0.9] {
        let value = cdf.iter().position(|&c| c >= q).unwrap_or(255);
        println!(
            "{:.0}% of the weather map's coverage values are at or below {:.2}",
            q * 100.0,
            value as f32 / 255.0
        );
    }
}
