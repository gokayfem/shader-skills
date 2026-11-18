//! Deterministic, fixed-quality vegetation benchmark with per-pass GPU timings.
//! Arguments: grass grid side, tree count, measured frames, camera mode, output directory.
//! Example: cargo run --release -p solstice-render --example vegetation_stress -- 128 1024 120 overview target/vegetation/128
use glam::{Mat4, Quat, Vec2, Vec3};
use solstice_asset::{Instance, Material, SceneAsset};
use solstice_builder::{build_virtual_mesh, paginate, BuildOptions, Mesh, Vertex};
use solstice_procgen::foliage::{grass::Meadow, tree::Species, Rng};
use solstice_render::{profiler::SECTIONS, Camera, Environment, Gpu, Renderer, RendererSettings};
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::Path;
use std::time::Instant;

const WIDTH: u32 = 1920;
const HEIGHT: u32 = 1080;
const WARMUP: u32 = 48;

fn mesh(scene: &mut SceneAsset, source: &Mesh) -> u32 {
    let index = scene.meshes.len() as u32;
    scene
        .meshes
        .push(build_virtual_mesh(source, &BuildOptions::default()));
    index
}

fn scene(side: u32, trees: u32) -> SceneAsset {
    let mut scene = SceneAsset {
        texture_size: 512,
        ..Default::default()
    };
    let grass = Meadow::default();
    let grass_mesh = mesh(&mut scene, &grass.mesh());
    scene.textures.push(grass.atlas(0).bake(512));
    scene.materials.push(Material {
        base_color: [1.0; 4],
        texture_layer: 0,
        roughness: 1.0,
        normal_strength: 1.0,
        flags: Material::TWO_SIDED | Material::FOLIAGE | Material::SWAYS,
        ..Default::default()
    });
    let extent = side as f32 * grass.size;
    let mut rng = Rng::new(917, 11);
    for z in 0..side {
        for x in 0..side {
            scene.instances.push(Instance {
                mesh: grass_mesh,
                material: 0,
                transform: Mat4::from_rotation_translation(
                    Quat::from_rotation_y(rng.index(4) as f32 * std::f32::consts::FRAC_PI_2),
                    Vec3::new(
                        (x as f32 + 0.5) * grass.size - extent * 0.5,
                        0.0,
                        (z as f32 + 0.5) * grass.size - extent * 0.5,
                    ),
                ),
            });
        }
    }
    let mut plants = Vec::new();
    for (seed, species) in [Species::oak(), Species::birch(), Species::fir()]
        .iter()
        .enumerate()
    {
        let plant = species.grow(seed as u32);
        let wood = mesh(&mut scene, &plant.wood);
        let leaves = mesh(&mut scene, &plant.leaves);
        let bark_layer = scene.textures.len() as i32;
        scene
            .textures
            .push(species.bark.surface(seed as u32).bake(512));
        scene
            .textures
            .push(species.leaves.kind.atlas(seed as u32).bake(512));
        let bark = scene.materials.len() as u32;
        scene.materials.push(Material {
            base_color: [1.0; 4],
            texture_layer: bark_layer,
            uv_scale: 1.0 / solstice_procgen::foliage::bark::BARK_TILE_METRES,
            roughness: 1.0,
            normal_strength: 1.0,
            flags: Material::SWAYS | Material::TREE,
            ..Default::default()
        });
        scene.materials.push(Material {
            base_color: [1.0; 4],
            texture_layer: bark_layer + 1,
            roughness: 1.0,
            normal_strength: 1.0,
            flags: Material::TWO_SIDED | Material::FOLIAGE | Material::SWAYS | Material::TREE,
            ..Default::default()
        });
        plants.push((wood, leaves, bark));
    }
    for _ in 0..trees {
        let (wood, leaves, bark) = plants[rng.index(plants.len())];
        let position = Vec3::new(
            rng.signed() * extent * 0.48,
            0.0,
            rng.signed() * extent * 0.48,
        );
        let transform = Mat4::from_scale_rotation_translation(
            Vec3::splat(rng.range(0.8, 1.2)),
            Quat::from_rotation_y(rng.range(0.0, std::f32::consts::TAU)),
            position,
        );
        for (mesh, material) in [(wood, bark), (leaves, bark + 1)] {
            scene.instances.push(Instance {
                mesh,
                material,
                transform,
            });
        }
    }
    let ground = Mesh {
        vertices: [(-1.0, -1.0), (-1.0, 1.0), (1.0, 1.0), (1.0, -1.0)]
            .map(|(x, z)| Vertex {
                position: Vec3::new(x * extent, -0.04, z * extent),
                normal: Vec3::Y,
                uv: Vec2::new(x, z),
            })
            .to_vec(),
        indices: vec![0, 1, 2, 0, 2, 3],
    };
    let ground_mesh = mesh(&mut scene, &ground);
    let material = scene.materials.len() as u32;
    scene.materials.push(Material {
        base_color: [0.035, 0.055, 0.018, 1.0],
        roughness: 1.0,
        ..Default::default()
    });
    scene.instances.push(Instance {
        mesh: ground_mesh,
        material,
        transform: Mat4::IDENTITY,
    });
    scene
}

#[cfg(windows)]
fn process_bytes() -> (usize, usize) {
    #[repr(C)]
    #[derive(Default)]
    struct Counters {
        cb: u32,
        faults: u32,
        peak_working: usize,
        working: usize,
        peak_paged: usize,
        paged: usize,
        peak_nonpaged: usize,
        nonpaged: usize,
        pagefile: usize,
        peak_pagefile: usize,
        private: usize,
    }
    #[link(name = "psapi")]
    unsafe extern "system" {
        fn GetProcessMemoryInfo(process: isize, counters: *mut Counters, size: u32) -> i32;
    }
    let mut c = Counters {
        cb: std::mem::size_of::<Counters>() as u32,
        ..Default::default()
    };
    // SAFETY: -1 is the current-process pseudo handle; c points to a valid structure of cb bytes.
    let ok = unsafe { GetProcessMemoryInfo(-1, &mut c, std::mem::size_of::<Counters>() as u32) };
    if ok == 0 {
        (0, 0)
    } else {
        (c.working, c.private)
    }
}
#[cfg(not(windows))]
fn process_bytes() -> (usize, usize) {
    (0, 0)
}

fn screenshot(gpu: &Gpu, target: &wgpu::Texture, path: &Path) {
    let row = (WIDTH * 4).next_multiple_of(256);
    let staging = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("screenshot"),
        size: u64::from(row * HEIGHT),
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        target.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &staging,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row),
                rows_per_image: Some(HEIGHT),
            },
        },
        target.size(),
    );
    gpu.queue.submit([encoder.finish()]);
    staging
        .slice(..)
        .map_async(wgpu::MapMode::Read, |r| r.unwrap());
    gpu.device
        .poll(wgpu::PollType::wait_indefinitely())
        .unwrap();
    let data = staging.slice(..).get_mapped_range().unwrap();
    let pixels: Vec<u8> = data
        .chunks_exact(row as usize)
        .flat_map(|r| r[..(WIDTH * 4) as usize].iter().copied())
        .collect();
    let mut png = png::Encoder::new(BufWriter::new(File::create(path).unwrap()), WIDTH, HEIGHT);
    png.set_color(png::ColorType::Rgba);
    png.set_depth(png::BitDepth::Eight);
    png.write_header()
        .unwrap()
        .write_image_data(&pixels)
        .unwrap();
}

fn percentile(values: &[f64], fraction: f64) -> f64 {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    sorted[((sorted.len() - 1) as f64 * fraction).round() as usize]
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let number = |i: usize, fallback| {
        args.get(i)
            .map(|s| s.parse::<u32>().unwrap())
            .unwrap_or(fallback)
    };
    let side = number(1, 64);
    let trees = number(2, 256);
    let frames = number(3, 120).max(1);
    let mode = args.get(4).map_or("overview", String::as_str);
    let output = Path::new(args.get(5).map_or("target/vegetation", String::as_str));
    fs::create_dir_all(output).unwrap();
    let start = Instant::now();
    let scene = scene(side, trees);
    let loaded = paginate(&scene);
    eprintln!("baked in {:.2}s: {} grass patches, {} trees, {} instances, {} unique triangles, {} instanced source triangles, {} pages",
        start.elapsed().as_secs_f64(), side * side, trees, scene.instances.len(), scene.unique_source_triangles(), scene.total_source_triangles(), loaded.scene.pages.len());
    drop(scene);
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::PRIMARY,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let gpu = pollster::block_on(Gpu::request(
        &instance,
        None,
        wgpu::PowerPreference::HighPerformance,
    ))
    .unwrap();
    eprintln!("adapter: {:?}", gpu.adapter_info);
    let settings = RendererSettings::default();
    eprintln!("fixed settings: {settings:?}");
    let mut renderer = Renderer::new(&gpu, &loaded, wgpu::TextureFormat::Rgba8Unorm, settings);
    drop(loaded);
    let target = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("vegetation output"),
        size: wgpu::Extent3d {
            width: WIDTH,
            height: HEIGHT,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&Default::default());
    let extent = side as f32 * Meadow::default().size;
    let mut camera = if matches!(mode, "ground" | "walk" | "orbit") {
        Camera {
            position: Vec3::new(0.0, 1.7, extent * 0.3),
            pitch: -0.08,
            fov_y: 65.0f32.to_radians(),
            ..Default::default()
        }
    } else {
        Camera {
            position: Vec3::new(0.0, extent * 0.55, extent * 0.7),
            pitch: -(0.55f32 / 0.7).atan(),
            fov_y: 65.0f32.to_radians(),
            ..Default::default()
        }
    };
    let environment = Environment::default();
    let mut csv = BufWriter::new(File::create(output.join("frames.csv")).unwrap());
    writeln!(csv,"frame,cpu_ms,wall_ms,clusters,triangles,dropped,shadow_dropped,working_bytes,private_bytes,{}",SECTIONS.join(",")).unwrap();
    let mut cpu = Vec::new();
    let mut wall = Vec::new();
    let mut gpu_times = Vec::new();
    let mut sections = vec![Vec::new(); SECTIONS.len()];
    let mut dropped = 0;
    let mut shadow_dropped = 0;
    let mut max_private = 0;
    let mut max_working = 0;
    for frame in 0..WARMUP + frames {
        if mode == "walk" {
            camera.position.x = (frame as f32 * 0.005).sin() * 8.0;
            camera.yaw = (frame as f32 * 0.004).sin() * 0.25;
        } else if mode == "orbit" {
            camera.yaw = frame.saturating_sub(WARMUP) as f32 * std::f32::consts::TAU / 600.0;
        }
        let started = Instant::now();
        renderer.render(
            &gpu,
            &camera,
            &environment,
            &view,
            (WIDTH, HEIGHT),
            1.0 / 60.0,
        );
        let cpu_ms = started.elapsed().as_secs_f64() * 1000.0;
        gpu.device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
        let wall_ms = started.elapsed().as_secs_f64() * 1000.0;
        // Startup is not timed, but missing geometry there is still a quality failure.
        let stats = renderer.stats();
        dropped = dropped.max(stats.dropped);
        shadow_dropped = shadow_dropped.max(stats.shadow_dropped);
        if frame >= WARMUP {
            let times = renderer
                .section_times()
                .expect("this benchmark needs native GPU timestamps");
            let (working, private) = process_bytes();
            max_private = max_private.max(private);
            max_working = max_working.max(working);
            cpu.push(cpu_ms);
            wall.push(wall_ms);
            gpu_times.push(times.iter().map(|&x| f64::from(x)).sum());
            write!(
                csv,
                "{frame},{cpu_ms:.6},{wall_ms:.6},{},{},{},{},{working},{private}",
                stats.clusters, stats.triangles, stats.dropped, stats.shadow_dropped
            )
            .unwrap();
            for (i, time) in times.iter().enumerate() {
                sections[i].push(f64::from(*time));
                write!(csv, ",{time:.6}").unwrap();
            }
            writeln!(csv).unwrap();
            if mode == "orbit" && (frame + 1 - WARMUP).is_multiple_of(600) {
                screenshot(
                    &gpu,
                    &target,
                    &output.join(format!("orbit-{:04}.png", frame + 1 - WARMUP)),
                );
            }
        }
    }
    csv.flush().unwrap();
    screenshot(&gpu, &target, &output.join("final.png"));
    let memory = renderer.scene_bytes();
    let stats = renderer.stats();
    println!("side,trees,mode,instances,source_triangles,clusters,triangles,cpu_median_ms,cpu_p95_ms,gpu_median_ms,gpu_p95_ms,wall_median_ms,scene_gpu_bytes,peak_working_bytes,peak_private_bytes,dropped,shadow_dropped");
    println!("{side},{trees},{mode},{},{},{},{},{:.6},{:.6},{:.6},{:.6},{:.6},{},{max_working},{max_private},{dropped},{shadow_dropped}",
        stats.instances,stats.source_triangles,stats.clusters,stats.triangles, percentile(&cpu,0.5),percentile(&cpu,0.95),
        percentile(&gpu_times,0.5),percentile(&gpu_times,0.95),percentile(&wall,0.5),memory.total());
    for (i, name) in SECTIONS.iter().enumerate() {
        eprintln!(
            "{name}: median {:.3}ms, p95 {:.3}ms",
            percentile(&sections[i], 0.5),
            percentile(&sections[i], 0.95)
        );
    }
    assert_eq!(
        (dropped, shadow_dropped),
        (0, 0),
        "capacity overflow means this run did not preserve image quality"
    );
}
