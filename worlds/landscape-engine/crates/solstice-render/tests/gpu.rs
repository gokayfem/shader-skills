//! End-to-end tests on a real GPU. They pass trivially, with a note, if none is available.

use glam::{Mat4, Quat, Vec2, Vec3};
use solstice_asset::PackedSkin;
use solstice_asset::{
    Card, CardImage, CardVertex, Cards, GlowMaps, Instance, Light, LoadedScene, Material,
    SceneAsset, TextureLayer,
};
use solstice_builder::{
    build_virtual_mesh, compress_textures, masked_mip_chain, mip_chain, paginate, BuildOptions,
    Mesh, Vertex,
};
use solstice_render::{
    Camera, DebugView, Effect, Environment, Gpu, Ocean, Place, Pond, Quality, Renderer,
    RendererSettings, StreamRequest, StreamingBudget, TimeOfDay, Weather,
};
use std::sync::{Arc, Mutex};

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

fn request_gpu() -> Option<Gpu> {
    request_gpu_with(None)
}

/// A GPU asking for no more than `limits`, as a small one is given, or `None` if there is none.
fn request_gpu_within(limits: &wgpu::Limits) -> Option<Gpu> {
    request_gpu_with(Some(limits))
}

/// Devices are made one at a time: made at once from many test threads, the Vulkan loader,
/// validation layer and driver on Windows can crash or deadlock.
fn request_gpu_with(limits: Option<&wgpu::Limits>) -> Option<Gpu> {
    static MAKING: Mutex<()> = Mutex::new(());
    let _one_at_a_time = MAKING
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::PRIMARY,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    pollster::block_on(Gpu::request_within(
        &instance,
        None,
        wgpu::PowerPreference::HighPerformance,
        limits,
    ))
    .map_err(|error| eprintln!("skipping GPU test: {error}"))
    .ok()
}

/// A unit icosphere: closed, so every pixel inside its silhouette must be covered.
fn sphere(subdivisions: u32) -> Mesh {
    let t = (1.0 + 5.0_f32.sqrt()) / 2.0;
    let mut positions: Vec<Vec3> = [
        [-1.0, t, 0.0],
        [1.0, t, 0.0],
        [-1.0, -t, 0.0],
        [1.0, -t, 0.0],
        [0.0, -1.0, t],
        [0.0, 1.0, t],
        [0.0, -1.0, -t],
        [0.0, 1.0, -t],
        [t, 0.0, -1.0],
        [t, 0.0, 1.0],
        [-t, 0.0, -1.0],
        [-t, 0.0, 1.0],
    ]
    .map(|p| Vec3::from_array(p).normalize())
    .to_vec();
    let mut faces: Vec<[u32; 3]> = vec![
        [0, 11, 5],
        [0, 5, 1],
        [0, 1, 7],
        [0, 7, 10],
        [0, 10, 11],
        [1, 5, 9],
        [5, 11, 4],
        [11, 10, 2],
        [10, 7, 6],
        [7, 1, 8],
        [3, 9, 4],
        [3, 4, 2],
        [3, 2, 6],
        [3, 6, 8],
        [3, 8, 9],
        [4, 9, 5],
        [2, 4, 11],
        [6, 2, 10],
        [8, 6, 7],
        [9, 8, 1],
    ];
    for _ in 0..subdivisions {
        let mut midpoints = std::collections::HashMap::new();
        let mut next = Vec::with_capacity(faces.len() * 4);
        for [a, b, c] in faces {
            let mut midpoint = |i: u32, j: u32| {
                *midpoints.entry((i.min(j), i.max(j))).or_insert_with(|| {
                    positions.push((positions[i as usize] + positions[j as usize]).normalize());
                    positions.len() as u32 - 1
                })
            };
            let (ab, bc, ca) = (midpoint(a, b), midpoint(b, c), midpoint(c, a));
            next.extend([[a, ab, ca], [b, bc, ab], [c, ca, bc], [ab, bc, ca]]);
        }
        faces = next;
    }
    Mesh {
        vertices: positions
            .iter()
            .map(|&p| Vertex {
                position: p,
                normal: p,
                uv: Vec2::new(p.x, p.y),
            })
            .collect(),
        indices: faces.into_iter().flatten().collect(),
    }
}

fn white_material() -> Material {
    Material {
        base_color: [1.0; 4],
        ..Default::default()
    }
}

fn instance(mesh: u32, transform: Mat4) -> Instance {
    Instance {
        mesh,
        material: 0,
        transform,
    }
}

/// No sun and no sky: the background is black, whatever the debug view shows is pure.
fn darkness() -> Environment {
    Environment {
        sun_illuminance: 0.0,
        sky_intensity: 0.0,
        ..Default::default()
    }
}

struct Run {
    pixels: Vec<u8>,
    size: u32,
    triangles: u32,
    clusters: u32,
    software_clusters: u32,
    /// Clusters the last shadow cascade redrawn sent to the software rasterizer.
    shadow_software_clusters: u32,
    shadow_clusters: u32,
    /// Clusters it drew as specks.
    shadow_specks: u32,
}

/// Renders a few frames in darkness, fails on any validation error, and reads back the result.
fn render(
    gpu: &Gpu,
    scene: &SceneAsset,
    camera: &Camera,
    settings: RendererSettings,
    size: u32,
) -> Run {
    render_in(gpu, scene, camera, settings, size, &darkness())
}

/// Renders a few frames under `environment`, fails on any validation error, and reads back
/// the result.
fn render_in(
    gpu: &Gpu,
    scene: &SceneAsset,
    camera: &Camera,
    settings: RendererSettings,
    size: u32,
    environment: &Environment,
) -> Run {
    let errors: Arc<Mutex<Vec<String>>> = Default::default();
    let sink = errors.clone();
    gpu.device
        .on_uncaptured_error(Arc::new(move |error: wgpu::Error| {
            sink.lock().unwrap().push(error.to_string());
        }));

    let mut renderer = Renderer::new(gpu, &paginate(scene), FORMAT, settings);
    let target = test_target(gpu, size);
    let view = target.create_view(&Default::default());

    // Enough frames for the depth history to exist and the stats readback to cycle.
    for _ in 0..8 {
        renderer.render(gpu, camera, environment, &view, (size, size), 1.0 / 60.0);
        gpu.device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
    }

    let pixels = read_pixels(gpu, &target, size);
    let errors = errors.lock().unwrap();
    assert!(errors.is_empty(), "GPU errors: {}", errors.join("\n"));
    let stats = renderer.stats();
    Run {
        pixels,
        size,
        triangles: stats.triangles,
        clusters: stats.clusters,
        software_clusters: stats.software_clusters,
        shadow_software_clusters: stats.shadow_software_clusters,
        shadow_clusters: stats.shadow_clusters,
        shadow_specks: stats.shadow_specks,
    }
}

fn test_target(gpu: &Gpu, size: u32) -> wgpu::Texture {
    gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("test target"),
        size: wgpu::Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    })
}

fn read_pixels(gpu: &Gpu, target: &wgpu::Texture, size: u32) -> Vec<u8> {
    let row = (size * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
    let staging = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("test readback"),
        size: u64::from(row) * u64::from(size),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
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
        .map_async(wgpu::MapMode::Read, |result| result.unwrap());
    gpu.device
        .poll(wgpu::PollType::wait_indefinitely())
        .unwrap();

    let data = staging.slice(..).get_mapped_range().unwrap();
    let mut pixels = Vec::new();
    for line in data.chunks_exact(row as usize) {
        pixels.extend_from_slice(&line[..(size * 4) as usize]);
    }
    pixels
}

/// Uncovered pixels inside the silhouette of a unit sphere `distance` away, in a white-on-black
/// image of a sphere centred on the view.
fn holes_in_sphere(pixels: &[u8], size: u32, distance: f32, fov_y: f32) -> u32 {
    holes_within(pixels, size, distance, fov_y, 0.95)
}

/// The same, only as far out as `fraction` of the sphere's radius on screen.
fn holes_within(pixels: &[u8], size: u32, distance: f32, fov_y: f32, fraction: f32) -> u32 {
    let angular = (1.0 / distance).asin();
    let half = size as f32 * 0.5;
    let radius = angular.tan() / (fov_y * 0.5).tan() * half;
    let inner = (radius * fraction).min(half - 2.0);
    let mut holes = 0;
    for y in 0..size {
        for x in 0..size {
            let (dx, dy) = (x as f32 + 0.5 - half, y as f32 + 0.5 - half);
            if dx * dx + dy * dy < inner * inner && pixels[((y * size + x) * 4) as usize] < 200 {
                holes += 1;
            }
        }
    }
    holes
}

/// The sphere fills most of the view, mixing LOD levels, and both rasterizers draw parts of
/// it. A crack in the DAG or a seam between rasterizers would show the black background.
#[test]
fn closed_surface_has_no_holes() {
    let Some(gpu) = request_gpu() else { return };
    let mesh = build_virtual_mesh(&sphere(7), &BuildOptions::default());
    let scene = SceneAsset {
        meshes: vec![mesh],
        materials: vec![white_material()],
        instances: vec![instance(0, Mat4::IDENTITY)],
        ..Default::default()
    };
    const SIZE: u32 = 512;

    // From far to close, so that both rasterizers and many LOD levels are exercised.
    let mut used_software = false;
    let mut used_hardware = false;
    for distance in [12.0_f32, 5.0, 2.5, 1.6] {
        let camera = Camera {
            position: Vec3::new(0.0, 0.0, distance),
            fov_y: 50.0_f32.to_radians(),
            near: 0.05,
            ..Default::default()
        };
        // No temporal accumulation: jittered history would smooth pinholes out of sight.
        let settings = RendererSettings {
            debug_view: DebugView::Albedo,
            temporal_accumulation: false,
            ..Default::default()
        };
        let run = render(&gpu, &scene, &camera, settings, SIZE);
        assert!(run.clusters > 0, "nothing was drawn at distance {distance}");
        used_software |= run.software_clusters > 0;
        used_hardware |= run.clusters > run.software_clusters;

        let holes = holes_in_sphere(&run.pixels, SIZE, distance, camera.fov_y);
        assert_eq!(holes, 0, "{holes} uncovered pixels at distance {distance}");
    }
    assert!(
        used_software,
        "this test should have exercised the software rasterizer"
    );
    assert!(
        used_hardware,
        "this test should have exercised the hardware rasterizer"
    );
}

fn generous_budget() -> StreamingBudget {
    StreamingBudget {
        geometry_bytes: 1 << 40,
        texture_bytes: 1 << 40,
    }
}

/// One step of a streaming host: hands over a few pages and starts what's wanted, like a slow
/// network with pages arriving out of order.
fn pump(
    renderer: &mut Renderer,
    loaded: &LoadedScene,
    queue: &mut Vec<StreamRequest>,
    per_frame: usize,
) {
    queue.extend(renderer.stream_requests(16));
    for _ in 0..per_frame.min(queue.len()) {
        // Popping the newest first delivers children before their parents.
        let request = queue.pop().unwrap();
        let data = requested_bytes(loaded, &request);
        match request.clusters {
            Some((start, end)) => {
                let records = ordered_clusters(loaded)[start as usize..end as usize].to_vec();
                renderer.deliver_page(request.id, data, records)
            }
            None => renderer.deliver(request.id, data),
        };
    }
}

/// A scene's cluster records as the browser's file has them: each page's sets in turn.
fn ordered_clusters(loaded: &LoadedScene) -> Vec<u8> {
    let scene = &loaded.scene;
    let ordered: Vec<solstice_asset::PackedCluster> = scene
        .sets
        .iter()
        .flat_map(|set| {
            let range =
                set.first_cluster as usize..(set.first_cluster + set.cluster_count) as usize;
            scene.clusters[range].iter().copied()
        })
        .collect();
    solstice_asset::file::cluster_bytes(&ordered).to_vec()
}

/// A streaming renderer for a scene as a browser loads one too large for its memory: its cluster
/// records and pinned pages arrive apart from its manifest, and records only with their pages.
fn split_renderer(
    gpu: &Gpu,
    loaded: &LoadedScene,
    settings: RendererSettings,
    budget: StreamingBudget,
) -> Renderer {
    let mut small = loaded.scene.clone();
    small.external = Some(solstice_asset::paged::ExternalTables {
        clusters: small.clusters.len() as u64,
        pinned_bytes: small.pinned_data.len() as u64,
    });
    small.clusters.clear();
    small.pinned_data.clear();
    let mut renderer = Renderer::streaming(gpu, &small, FORMAT, settings, budget);
    let records = ordered_clusters(loaded);
    let mut at = 0;
    for page in 0..loaded.scene.pinned_pages {
        let bytes = loaded.scene.page_cluster_count(page) as usize * 48;
        let data = loaded.scene.pinned_page(page);
        renderer.install_pinned(gpu, page, data, &records[at..at + bytes]);
        at += bytes;
    }
    renderer.finish_external_tables(gpu);
    renderer
}

/// What a request's bytes come to once unpacked, from a scene in memory.
fn requested_bytes(loaded: &LoadedScene, request: &StreamRequest) -> Vec<u8> {
    if request.deflated {
        return loaded.page(request.id).to_vec();
    }
    let layer = ((request.id & 0x7fff_ffff) >> 8) as usize;
    let mip = (request.id & 0xff) as usize;
    let upper = &loaded.upper_textures[layer];
    [
        upper.color_mips[mip].as_slice(),
        upper.normal_mips[mip].as_slice(),
        upper.material_mips[mip].as_slice(),
    ]
    .concat()
}

/// A scene whose big tables arrive in parts, as a browser loads one too large for its memory,
/// must draw exactly what the same scene does with them in its manifest.
#[test]
fn a_scene_whose_tables_arrive_separately_draws_the_same() {
    let Some(gpu) = request_gpu() else { return };
    let scene = SceneAsset {
        meshes: vec![build_virtual_mesh(&sphere(7), &BuildOptions::default())],
        materials: vec![Material::default()],
        instances: vec![instance(0, Mat4::IDENTITY)],
        ..Default::default()
    };
    let loaded = paginate(&scene);
    assert!(loaded.scene.pinned_pages > 0);
    const SIZE: u32 = 128;
    let camera = Camera {
        position: Vec3::new(0.0, 0.0, 1.8),
        near: 0.05,
        ..Default::default()
    };
    let settings = RendererSettings {
        debug_view: DebugView::Albedo,
        temporal_accumulation: false,
        ..Default::default()
    };
    let draw = |renderer: &mut Renderer| {
        let target = test_target(&gpu, SIZE);
        let view = target.create_view(&Default::default());
        let mut queue = Vec::new();
        for _ in 0..80 {
            pump(renderer, &loaded, &mut queue, 8);
            renderer.render(&gpu, &camera, &darkness(), &view, (SIZE, SIZE), 1.0 / 60.0);
            gpu.device
                .poll(wgpu::PollType::wait_indefinitely())
                .unwrap();
        }
        read_pixels(&gpu, &target, SIZE)
    };

    let mut whole = Renderer::streaming(&gpu, &loaded.scene, FORMAT, settings, generous_budget());
    let expected = draw(&mut whole);

    let errors: Arc<Mutex<Vec<String>>> = Default::default();
    let sink = errors.clone();
    gpu.device
        .on_uncaptured_error(Arc::new(move |error: wgpu::Error| {
            sink.lock().unwrap().push(error.to_string());
        }));
    let mut split = split_renderer(&gpu, &loaded, settings, generous_budget());
    let actual = draw(&mut split);
    assert!(
        errors.lock().unwrap().is_empty(),
        "{:?}",
        errors.lock().unwrap()
    );
    assert_eq!(actual, expected);
}

/// With prefetch on, pages load for what is near the camera but behind it, down to a coarser
/// level than for what it sees.
#[test]
fn prefetch_loads_what_is_near_but_out_of_view() {
    let Some(gpu) = request_gpu() else { return };
    // Two meshes, so that each has pages of its own: one in view, one behind the camera.
    let scene = SceneAsset {
        meshes: vec![
            build_virtual_mesh(&sphere(7), &BuildOptions::default()),
            build_virtual_mesh(&sphere(7), &BuildOptions::default()),
        ],
        materials: vec![Material::default()],
        instances: vec![
            instance(0, Mat4::IDENTITY),
            instance(1, Mat4::from_translation(Vec3::new(0.0, 0.0, 3.2))),
        ],
        ..Default::default()
    };
    let loaded = paginate(&scene);
    const SIZE: u32 = 256;
    let camera = Camera {
        position: Vec3::new(0.0, 0.0, 1.6),
        fov_y: 50.0_f32.to_radians(),
        near: 0.05,
        ..Default::default()
    };
    let installed = |prefetch_radius: f32| {
        let settings = RendererSettings {
            debug_view: DebugView::Albedo,
            temporal_accumulation: false,
            prefetch_radius,
            ..Default::default()
        };
        let mut renderer =
            Renderer::streaming(&gpu, &loaded.scene, FORMAT, settings, generous_budget());
        let target = test_target(&gpu, SIZE);
        let view = target.create_view(&Default::default());
        let mut queue = Vec::new();
        for _ in 0..160 {
            pump(&mut renderer, &loaded, &mut queue, 8);
            renderer.render(&gpu, &camera, &darkness(), &view, (SIZE, SIZE), 1.0 / 60.0);
            gpu.device
                .poll(wgpu::PollType::wait_indefinitely())
                .unwrap();
        }
        renderer.stats().streaming.installed
    };
    let pinned = loaded.scene.pinned_pages;
    let (facing, around, far) = (installed(0.0), installed(10.0), installed(0.05));
    let seen = facing - pinned;
    assert!(seen > 2, "the sphere in view streams in: {seen} pages");
    let behind = around - facing;
    assert!(behind > 0, "what is behind the camera loads too");
    assert!(behind < seen, "but coarser: {behind} pages against {seen}");
    assert_eq!(far, facing, "and not from beyond the radius");
}

/// The scene starts with only its coarsest pages and fills in slowly. Every frame along the way
/// must cover the whole sphere, with detail only ever being added.
#[test]
fn a_loading_scene_never_shows_a_hole() {
    let Some(gpu) = request_gpu() else { return };
    let mesh = build_virtual_mesh(&sphere(7), &BuildOptions::default());
    let scene = SceneAsset {
        meshes: vec![mesh],
        materials: vec![white_material()],
        instances: vec![instance(0, Mat4::IDENTITY)],
        ..Default::default()
    };
    let loaded = paginate(&scene);
    assert!(loaded.scene.pages.len() > 8);
    const SIZE: u32 = 512;
    const DISTANCE: f32 = 1.6;
    let camera = Camera {
        position: Vec3::new(0.0, 0.0, DISTANCE),
        fov_y: 50.0_f32.to_radians(),
        near: 0.05,
        ..Default::default()
    };
    let settings = RendererSettings {
        debug_view: DebugView::Albedo,
        temporal_accumulation: false,
        ..Default::default()
    };
    let mut renderer =
        Renderer::streaming(&gpu, &loaded.scene, FORMAT, settings, generous_budget());
    let target = test_target(&gpu, SIZE);
    let view = target.create_view(&Default::default());

    let mut queue = Vec::new();
    let mut first_triangles = None;
    let mut last_stats = renderer.stats();
    for frame in 0..240 {
        pump(&mut renderer, &loaded, &mut queue, 1);
        renderer.render(&gpu, &camera, &darkness(), &view, (SIZE, SIZE), 1.0 / 60.0);
        gpu.device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
        last_stats = renderer.stats();
        if frame % 4 == 0 {
            let pixels = read_pixels(&gpu, &target, SIZE);
            let holes = holes_in_sphere(&pixels, SIZE, DISTANCE, camera.fov_y);
            assert_eq!(
                holes, 0,
                "{holes} uncovered pixels at frame {frame}, with {} pages in",
                last_stats.streaming.installed
            );
        }
        if frame == 6 {
            first_triangles = Some(last_stats.triangles);
        }
    }
    let streaming = last_stats.streaming;
    assert!(streaming.installed > loaded.scene.pinned_pages);
    assert!(
        (streaming.installed as usize) < loaded.scene.pages.len(),
        "a view of one side shouldn't need every page"
    );
    assert!(last_stats.triangles > first_triangles.unwrap());
}

/// A pool too small for the scene: visiting one sphere then another must evict the first, stay
/// within the pool and leave both whole.
#[test]
fn a_small_pool_evicts_what_is_out_of_view_and_shows_no_holes() {
    let Some(gpu) = request_gpu() else { return };
    let mesh = build_virtual_mesh(&sphere(7), &BuildOptions::default());
    const APART: f32 = 100.0;
    let scene = SceneAsset {
        meshes: vec![mesh.clone(), mesh],
        materials: vec![white_material()],
        instances: vec![
            instance(0, Mat4::IDENTITY),
            instance(1, Mat4::from_translation(Vec3::X * APART)),
        ],
        ..Default::default()
    };
    let loaded = paginate(&scene);
    const SIZE: u32 = 256;
    const DISTANCE: f32 = 1.6;
    let settings = RendererSettings {
        debug_view: DebugView::Albedo,
        temporal_accumulation: false,
        ..Default::default()
    };

    // Room for the coarsest pages and a part of what one close view needs.
    let pinned = loaded.scene.pinned_pages;
    let slots = pinned + 10;
    assert!(slots < loaded.scene.pages.len() as u32);
    let budget = StreamingBudget {
        geometry_bytes: u64::from(slots) * solstice_asset::paged::PAGE_BYTES as u64,
        ..generous_budget()
    };
    let mut renderer = Renderer::streaming(&gpu, &loaded.scene, FORMAT, settings, budget);
    let target = test_target(&gpu, SIZE);
    let view = target.create_view(&Default::default());

    let mut queue = Vec::new();
    for (visit, x) in [0.0, APART, 0.0, APART].into_iter().enumerate() {
        let camera = Camera {
            position: Vec3::new(x, 0.0, DISTANCE),
            fov_y: 50.0_f32.to_radians(),
            near: 0.05,
            ..Default::default()
        };
        for frame in 0..90 {
            pump(&mut renderer, &loaded, &mut queue, 4);
            renderer.render(&gpu, &camera, &darkness(), &view, (SIZE, SIZE), 1.0 / 60.0);
            gpu.device
                .poll(wgpu::PollType::wait_indefinitely())
                .unwrap();
            let streaming = renderer.stats().streaming;
            assert!(streaming.installed <= slots, "the pool overflowed");
            if frame % 8 == 0 {
                let pixels = read_pixels(&gpu, &target, SIZE);
                let holes = holes_in_sphere(&pixels, SIZE, DISTANCE, camera.fov_y);
                assert_eq!(holes, 0, "visit {visit}, frame {frame}");
            }
        }
    }
    // Forced regions divide this small pool between three bands, none of which then overflows.
    // How bands evict is covered by the residency tests.
    if std::env::var_os("SOLSTICE_FORCE_REGIONS").is_none() {
        assert!(
            renderer.stats().streaming.evictions > 0,
            "the second sphere should have pushed out the first"
        );
    }
}

/// The same, for a scene whose cluster records come and go with its pages.
#[test]
fn a_small_pool_of_a_split_scene_evicts_and_shows_no_holes() {
    let Some(gpu) = request_gpu() else { return };
    let mesh = build_virtual_mesh(&sphere(7), &BuildOptions::default());
    const APART: f32 = 100.0;
    let scene = SceneAsset {
        meshes: vec![mesh.clone(), mesh],
        materials: vec![white_material()],
        instances: vec![
            instance(0, Mat4::IDENTITY),
            instance(1, Mat4::from_translation(Vec3::X * APART)),
        ],
        ..Default::default()
    };
    let loaded = paginate(&scene);
    const SIZE: u32 = 256;
    const DISTANCE: f32 = 1.6;
    let settings = RendererSettings {
        debug_view: DebugView::Albedo,
        temporal_accumulation: false,
        ..Default::default()
    };

    // Room for the coarsest pages and a part of what one close view needs.
    let pinned = loaded.scene.pinned_pages;
    let slots = pinned + 10;
    assert!(slots < loaded.scene.pages.len() as u32);
    let budget = StreamingBudget {
        geometry_bytes: u64::from(slots) * solstice_asset::paged::PAGE_BYTES as u64,
        ..generous_budget()
    };
    let mut renderer = split_renderer(&gpu, &loaded, settings, budget);
    let target = test_target(&gpu, SIZE);
    let view = target.create_view(&Default::default());

    let mut queue = Vec::new();
    for (visit, x) in [0.0, APART, 0.0, APART].into_iter().enumerate() {
        let camera = Camera {
            position: Vec3::new(x, 0.0, DISTANCE),
            fov_y: 50.0_f32.to_radians(),
            near: 0.05,
            ..Default::default()
        };
        for frame in 0..90 {
            pump(&mut renderer, &loaded, &mut queue, 4);
            renderer.render(&gpu, &camera, &darkness(), &view, (SIZE, SIZE), 1.0 / 60.0);
            gpu.device
                .poll(wgpu::PollType::wait_indefinitely())
                .unwrap();
            let streaming = renderer.stats().streaming;
            assert!(streaming.installed <= slots, "the pool overflowed");
            if frame % 8 == 0 {
                let pixels = read_pixels(&gpu, &target, SIZE);
                let holes = holes_in_sphere(&pixels, SIZE, DISTANCE, camera.fov_y);
                assert_eq!(holes, 0, "visit {visit}, frame {frame}");
            }
        }
    }
    // Forced regions divide this small pool between three bands, none of which then overflows.
    // How bands evict is covered by the residency tests.
    if std::env::var_os("SOLSTICE_FORCE_REGIONS").is_none() {
        assert!(
            renderer.stats().streaming.evictions > 0,
            "the second sphere should have pushed out the first"
        );
    }
}

/// A close view of a texture whose largest level alone is green. It must start out using the
/// red levels in the manifest, then fetch the green one, by itself.
#[test]
fn a_texture_sharpens_once_a_view_needs_it() {
    let Some(gpu) = request_gpu() else { return };
    const EDGE: u32 = 1024;
    let level = |edge: u32, texel: [u8; 4]| -> Vec<u8> {
        texel
            .iter()
            .copied()
            .cycle()
            .take((edge * edge) as usize * 4)
            .collect()
    };
    let flat = |edge: u32, value: u8| vec![value; (edge * edge) as usize * 2];
    let chain = (0..=EDGE.ilog2()).map(|m| EDGE >> m);
    let layer = TextureLayer {
        color_mips: chain
            .clone()
            .map(|edge| {
                let texel = if edge == EDGE {
                    [0, 255, 0, 255]
                } else {
                    [255, 0, 0, 255]
                };
                level(edge, texel)
            })
            .collect(),
        normal_mips: chain.clone().map(|edge| flat(edge, 128)).collect(),
        material_mips: chain.map(|edge| flat(edge, 128)).collect(),
    };
    let scene = SceneAsset {
        meshes: vec![build_virtual_mesh(&sphere(5), &BuildOptions::default())],
        materials: vec![Material {
            texture_layer: 0,
            uv_scale: 8.0,
            ..white_material()
        }],
        instances: vec![instance(0, Mat4::IDENTITY)],
        texture_size: EDGE,
        textures: vec![layer],
        ..Default::default()
    };
    let loaded = paginate(&scene);
    const SIZE: u32 = 256;
    let camera = Camera {
        position: Vec3::new(0.0, 0.0, 1.05),
        fov_y: 50.0_f32.to_radians(),
        near: 0.01,
        ..Default::default()
    };
    let settings = RendererSettings {
        debug_view: DebugView::Albedo,
        temporal_accumulation: false,
        ..Default::default()
    };
    let mut renderer =
        Renderer::streaming(&gpu, &loaded.scene, FORMAT, settings, generous_budget());
    let target = test_target(&gpu, SIZE);
    let view = target.create_view(&Default::default());

    let center = |pixels: &[u8]| {
        let at = ((SIZE / 2 * SIZE + SIZE / 2) * 4) as usize;
        [pixels[at], pixels[at + 1]]
    };
    renderer.render(&gpu, &camera, &darkness(), &view, (SIZE, SIZE), 1.0 / 60.0);
    gpu.device
        .poll(wgpu::PollType::wait_indefinitely())
        .unwrap();
    let [red, green] = center(&read_pixels(&gpu, &target, SIZE));
    assert!(red > 200 && green < 50, "starts red, not {red}, {green}");

    let mut queue = Vec::new();
    for _ in 0..60 {
        pump(&mut renderer, &loaded, &mut queue, 8);
        renderer.render(&gpu, &camera, &darkness(), &view, (SIZE, SIZE), 1.0 / 60.0);
        gpu.device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
    }
    let stats = renderer.stats().streaming;
    assert_eq!(stats.middle_layers, 1);
    assert_eq!(stats.top_layers, 1);
    let [red, green] = center(&read_pixels(&gpu, &target, SIZE));
    assert!(green > 200 && red < 50, "ends green, not {red}, {green}");
}

/// A wall hides a crowd of spheres. Occlusion culling must skip them, leaving far fewer
/// triangles than the same crowd with the wall out of the way.
#[test]
fn occlusion_culling_skips_hidden_geometry() {
    let Some(gpu) = request_gpu() else { return };
    let ball = build_virtual_mesh(&sphere(5), &BuildOptions::default());
    let wall = build_virtual_mesh(&sphere(3), &BuildOptions::default());

    let mut instances = vec![];
    for x in -5..=5 {
        for y in -5..=5 {
            let position = Vec3::new(x as f32 * 2.2, y as f32 * 2.2, -30.0);
            instances.push(instance(0, Mat4::from_translation(position)));
        }
    }
    let crowd_only = SceneAsset {
        meshes: vec![ball.clone(), wall.clone()],
        materials: vec![white_material()],
        instances: instances.clone(),
        ..Default::default()
    };
    // A huge flattened sphere right in front of the camera covers the whole view.
    instances.push(instance(
        1,
        Mat4::from_scale_rotation_translation(
            Vec3::new(200.0, 200.0, 1.0),
            Default::default(),
            Vec3::new(0.0, 0.0, 4.0),
        ),
    ));
    let with_wall = SceneAsset {
        meshes: vec![ball, wall],
        materials: vec![white_material()],
        instances,
        ..Default::default()
    };

    let camera = Camera {
        position: Vec3::new(0.0, 0.0, 10.0),
        ..Default::default()
    };
    let settings = RendererSettings {
        debug_view: DebugView::Albedo,
        ..Default::default()
    };
    let open = render(&gpu, &crowd_only, &camera, settings, 256);
    let blocked = render(&gpu, &with_wall, &camera, settings, 256);
    assert!(
        open.triangles > 1000,
        "crowd drew only {} triangles",
        open.triangles
    );
    assert!(
        blocked.triangles * 5 < open.triangles,
        "hidden crowd still cost {} triangles, against {} when visible",
        blocked.triangles,
        open.triangles
    );
}

/// Collapsed layouts and hidden frames give canvases of a pixel or two. Nothing useful can be
/// drawn, but every pass must still be valid, or the errors never stop.
#[test]
fn tiny_outputs_render_without_errors() {
    let Some(gpu) = request_gpu() else { return };
    let mesh = build_virtual_mesh(&sphere(2), &BuildOptions::default());
    let scene = SceneAsset {
        meshes: vec![mesh],
        materials: vec![white_material()],
        instances: vec![instance(0, Mat4::IDENTITY)],
        ..Default::default()
    };
    let camera = Camera {
        position: Vec3::new(0.0, 0.0, 5.0),
        ..Default::default()
    };
    // `render` itself fails the test on any validation error.
    for size in [1, 2, 3, 17, 33] {
        render(&gpu, &scene, &camera, RendererSettings::default(), size);
    }
}

/// Every probe ray count has its own pipelines, and switching quality at run time picks
/// another, with targets of another size. Each must render, alone and after a switch.
#[test]
fn every_probe_ray_count_renders_without_errors() {
    let Some(gpu) = request_gpu() else { return };
    let mesh = build_virtual_mesh(&sphere(2), &BuildOptions::default());
    let scene = SceneAsset {
        meshes: vec![mesh],
        materials: vec![white_material()],
        instances: vec![instance(0, Mat4::IDENTITY)],
        ..Default::default()
    };
    let camera = Camera {
        position: Vec3::new(0.0, 0.0, 5.0),
        ..Default::default()
    };
    for quality in [Quality::Low, Quality::Medium, Quality::High] {
        let mut settings = RendererSettings::default();
        settings.apply_quality(quality);
        let run = render(&gpu, &scene, &camera, settings, 64);
        assert!(run.clusters > 0, "nothing was drawn at {quality:?}");
    }

    let errors: Arc<Mutex<Vec<String>>> = Default::default();
    let sink = errors.clone();
    gpu.device
        .on_uncaptured_error(Arc::new(move |error: wgpu::Error| {
            sink.lock().unwrap().push(error.to_string());
        }));
    let mut renderer = Renderer::new(&gpu, &paginate(&scene), FORMAT, RendererSettings::default());
    let target = test_target(&gpu, 64);
    let view = target.create_view(&Default::default());
    for rays in [8, 6, 8] {
        renderer.settings_mut().probe_rays = rays;
        for _ in 0..3 {
            renderer.render(&gpu, &camera, &darkness(), &view, (64, 64), 1.0 / 60.0);
        }
    }
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
}

/// The same flat texture must look the same stored raw, as blocks the GPU samples directly,
/// and as blocks unpacked for a GPU without BC. 1024 is large enough that unpacking also
/// drops the largest mip.
#[test]
fn every_texture_path_renders_the_same_colour() {
    let Some(mut gpu) = request_gpu() else { return };
    const EDGE: u32 = 1024;
    let n = EDGE as usize * EDGE as usize;
    let layer = TextureLayer {
        color_mips: mip_chain([200, 100, 50, 255].repeat(n), EDGE, 4),
        normal_mips: mip_chain([128, 128].repeat(n), EDGE, 2),
        material_mips: mip_chain([128, 255].repeat(n), EDGE, 2),
    };
    let mesh = build_virtual_mesh(&sphere(4), &BuildOptions::default());
    let mut scene = SceneAsset {
        meshes: vec![mesh],
        materials: vec![Material {
            base_color: [1.0; 4],
            texture_layer: 0,
            ..Default::default()
        }],
        instances: vec![instance(0, Mat4::IDENTITY)],
        texture_size: EDGE,
        textures: vec![layer],
        ..Default::default()
    };
    let camera = Camera {
        position: Vec3::new(0.0, 0.0, 3.0),
        ..Default::default()
    };
    let settings = RendererSettings {
        debug_view: DebugView::Albedo,
        temporal_accumulation: false,
        ..Default::default()
    };
    let centre = |run: &Run| {
        let at = ((run.size / 2 * run.size + run.size / 2) * 4) as usize;
        run.pixels[at..at + 3].to_vec()
    };

    let raw = centre(&render(&gpu, &scene, &camera, settings, 64));
    assert!(
        raw[0] > raw[1] && raw[1] > raw[2],
        "texture missing: {raw:?}"
    );

    compress_textures(&mut scene);
    let paths = [("blocks", true), ("unpacked", false)];
    let supported = gpu.block_compressed_textures;
    for (name, keep_blocks) in paths {
        if keep_blocks && !supported {
            continue;
        }
        gpu.block_compressed_textures = keep_blocks;
        let result = centre(&render(&gpu, &scene, &camera, settings, 64));
        for (channel, (a, b)) in raw.iter().zip(&result).enumerate() {
            assert!(
                a.abs_diff(*b) <= 2,
                "{name}: channel {channel} is {b}, raw is {a}"
            );
        }
    }
}

/// The centre pixel of a square whose triangles face away from the camera, `distance` away, in
/// the normals view, and whether software rasterization drew any of it.
fn facing_away(gpu: &Gpu, flags: u32, distance: f32) -> ([u8; 3], bool) {
    const CELLS: u32 = 8;
    let mut mesh = Mesh::default();
    for j in 0..=CELLS {
        for i in 0..=CELLS {
            let (x, y) = (i as f32 / CELLS as f32 - 0.5, j as f32 / CELLS as f32 - 0.5);
            mesh.vertices.push(Vertex {
                position: Vec3::new(x, y, 0.0),
                normal: Vec3::NEG_Z,
                uv: Vec2::new(x, y),
            });
        }
    }
    for j in 0..CELLS {
        for i in 0..CELLS {
            let a = j * (CELLS + 1) + i;
            let (b, c, d) = (a + 1, a + CELLS + 1, a + CELLS + 2);
            // Clockwise as seen from +Z, where the camera is.
            mesh.indices.extend_from_slice(&[a, c, b, b, c, d]);
        }
    }
    let scene = SceneAsset {
        meshes: vec![build_virtual_mesh(&mesh, &BuildOptions::default())],
        materials: vec![Material {
            flags,
            ..white_material()
        }],
        instances: vec![instance(0, Mat4::IDENTITY)],
        ..Default::default()
    };
    let camera = Camera {
        position: Vec3::new(0.0, 0.0, distance),
        ..Default::default()
    };
    let settings = RendererSettings {
        debug_view: DebugView::Normals,
        temporal_accumulation: false,
        ..Default::default()
    };
    let run = render(gpu, &scene, &camera, settings, 128);
    let at = ((run.size / 2 * run.size + run.size / 2) * 4) as usize;
    (
        [run.pixels[at], run.pixels[at + 1], run.pixels[at + 2]],
        run.software_clusters > 0,
    )
}

/// Back faces are culled, unless the material is two-sided, when they are drawn with their normal
/// turned to the viewer. Checked close up, where hardware rasterizes, and far away, where software
/// does.
#[test]
fn two_sided_materials_draw_their_back_faces() {
    let Some(gpu) = request_gpu() else { return };
    for (distance, software) in [(1.0, false), (20.0, true)] {
        let (one_sided, used_software) = facing_away(&gpu, 0, distance);
        assert_eq!(
            used_software, software,
            "at {distance} m the wrong rasterizer ran"
        );
        assert_eq!(
            one_sided,
            [0, 0, 0],
            "a back face was drawn at {distance} m"
        );

        let (two_sided, _) = facing_away(&gpu, Material::TWO_SIDED, distance);
        // Encoded as n * 0.5 + 0.5 and then sRGB, so a normal towards the camera, +Z, is
        // (187, 187, 255).
        assert!(
            two_sided[2] > 240
                && two_sided[0].abs_diff(187) < 20
                && two_sided[1].abs_diff(187) < 20,
            "at {distance} m the back face shows {two_sided:?}, not a normal facing the camera"
        );
    }
}

/// The world normal at the centre of a flat, camera-facing tile, encoded as red and green.
fn tile_normal(gpu: &Gpu, uv: impl Fn(f32, f32) -> Vec2, stored: [u8; 2]) -> (i32, i32) {
    const CELLS: u32 = 8;
    const EDGE: u32 = 16;
    let n = (EDGE * EDGE) as usize;
    let mut mesh = Mesh::default();
    for j in 0..=CELLS {
        for i in 0..=CELLS {
            let (x, y) = (
                i as f32 / CELLS as f32 * 4.0 - 2.0,
                j as f32 / CELLS as f32 * 4.0 - 2.0,
            );
            mesh.vertices.push(Vertex {
                position: Vec3::new(x, y, 0.0),
                normal: Vec3::Z,
                uv: uv(x, y),
            });
        }
    }
    for j in 0..CELLS {
        for i in 0..CELLS {
            let a = j * (CELLS + 1) + i;
            let (b, c, d) = (a + 1, a + CELLS + 1, a + CELLS + 2);
            mesh.indices.extend_from_slice(&[a, b, c, b, d, c]);
        }
    }
    let scene = SceneAsset {
        meshes: vec![build_virtual_mesh(&mesh, &BuildOptions::default())],
        materials: vec![Material {
            base_color: [1.0; 4],
            texture_layer: 0,
            ..Default::default()
        }],
        instances: vec![instance(0, Mat4::IDENTITY)],
        texture_size: EDGE,
        textures: vec![TextureLayer {
            color_mips: mip_chain([200, 200, 200, 255].repeat(n), EDGE, 4),
            normal_mips: mip_chain(stored.repeat(n), EDGE, 2),
            material_mips: mip_chain([128, 255].repeat(n), EDGE, 2),
        }],
        ..Default::default()
    };
    let camera = Camera {
        position: Vec3::new(0.0, 0.0, 3.0),
        ..Default::default()
    };
    let settings = RendererSettings {
        debug_view: DebugView::Normals,
        temporal_accumulation: false,
        ..Default::default()
    };
    let run = render(gpu, &scene, &camera, settings, 64);
    let at = ((run.size / 2 * run.size + run.size / 2) * 4) as usize;
    (i32::from(run.pixels[at]), i32::from(run.pixels[at + 1]))
}

/// A stored normal leaning towards increasing U must lean that way in the world, or every bump in
/// every texture lights as a dent. Tested with UVs in both handednesses, which models use freely.
#[test]
fn normal_maps_lean_the_way_their_texture_says() {
    let Some(gpu) = request_gpu() else { return };
    const TOWARDS_U: [u8; 2] = [220, 128];
    const TOWARDS_V: [u8; 2] = [128, 220];
    const FLAT: [u8; 2] = [128, 128];

    let plain = |x: f32, y: f32| Vec2::new(x, y);
    let (flat_x, flat_y) = tile_normal(&gpu, plain, FLAT);
    let (x, y) = tile_normal(&gpu, plain, TOWARDS_U);
    assert!(
        x > flat_x + 20,
        "+U should lean +X: {x} against flat {flat_x}"
    );
    assert!(
        y.abs_diff(flat_y) < 20,
        "+U should not lean in Y: {y} against {flat_y}"
    );
    let (x, y) = tile_normal(&gpu, plain, TOWARDS_V);
    assert!(
        y > flat_y + 20,
        "+V should lean +Y: {y} against flat {flat_y}"
    );
    assert!(
        x.abs_diff(flat_x) < 20,
        "+V should not lean in X: {x} against {flat_x}"
    );

    // Mirrored, as a model's far side often is: increasing U now runs towards -X.
    let mirrored = |x: f32, y: f32| Vec2::new(-x, y);
    let (x, _) = tile_normal(&gpu, mirrored, TOWARDS_U);
    assert!(
        x < flat_x - 20,
        "mirrored +U should lean -X: {x} against flat {flat_x}"
    );
    let (_, y) = tile_normal(&gpu, mirrored, TOWARDS_V);
    assert!(
        y > flat_y + 20,
        "mirrored +V should still lean +Y: {y} against flat {flat_y}"
    );
}

/// Baking compresses every scene, including those with no textures to compress.
#[test]
fn compressed_scene_without_textures_renders() {
    let Some(gpu) = request_gpu() else { return };
    let mut scene = SceneAsset {
        meshes: vec![build_virtual_mesh(&sphere(3), &BuildOptions::default())],
        materials: vec![white_material()],
        instances: vec![instance(0, Mat4::IDENTITY)],
        texture_size: 1024,
        ..Default::default()
    };
    compress_textures(&mut scene);
    let camera = Camera {
        position: Vec3::new(0.0, 0.0, 3.0),
        ..Default::default()
    };
    let run = render(&gpu, &scene, &camera, RendererSettings::default(), 64);
    assert!(run.clusters > 0);
}

/// Lights reach only the tiles of the screen their range meets, and the lighting loops over
/// those alone. A lamp over the ball lights it; six hundred lamps far off to the side change
/// nothing; and without the lamp the ball stays dark, as there is no sun or sky here.
#[test]
fn tiled_lights_reach_what_is_in_their_range_and_nothing_else() {
    let Some(gpu) = request_gpu() else {
        return;
    };
    let camera = Camera {
        position: Vec3::new(0.0, 2.0, 8.0),
        yaw: 0.0,
        pitch: -0.1,
        ..Default::default()
    };
    // No sun, no sky and no indirect light, with the exposure held to daylight, so only the
    // lamps' direct light shows: the GI volume lights the ball from the lamps too, by its own
    // loop over them, and would hide a tile that left one out.
    let night = Environment {
        sun_illuminance: 0.0,
        sky_intensity: 0.0,
        gi_intensity: 0.0,
        daylight_illuminance: 10.0,
        ..Default::default()
    };
    let lamp = |x: f32, z: f32| Light {
        position: [x, 3.0, z],
        range: 6.0,
        intensity: [40.0; 3],
        _pad: 0,
    };
    let near = lamp(1.5, 1.5);
    // A field of lamps 200 m off to the side, in range of nothing on screen.
    let far: Vec<Light> = (0..600)
        .map(|i| {
            lamp(
                200.0 + (i % 30) as f32 * 5.0,
                -200.0 + (i / 30) as f32 * 5.0,
            )
        })
        .collect();
    // Not a multiple of the tile size, so the tiles along two edges reach past the viewport.
    let size = 120;
    let render_lights = |lights: Vec<Light>| {
        let mut scene = ball_on_floor();
        scene.lights = lights;
        render_in(
            &gpu,
            &scene,
            &camera,
            RendererSettings::default(),
            size,
            &night,
        )
        .pixels
    };
    let dark = render_lights(Vec::new());
    let lit = render_lights(vec![near]);
    // The near lamp comes last, so a tile that listed the wrong index would pick a far one.
    let crowded = render_lights([far, vec![near]].concat());

    let ball = |pixels: &[u8]| brightness(patch(pixels, size, size / 2, size / 2, 6));
    assert!(
        ball(&lit) > ball(&dark) + 20.0,
        "the lamp lights the ball: {} lit, {} dark",
        ball(&lit),
        ball(&dark)
    );
    let difference: f32 = lit
        .iter()
        .zip(&crowded)
        .map(|(&a, &b)| (f32::from(a) - f32::from(b)).abs())
        .sum::<f32>()
        / lit.len() as f32;
    assert!(
        difference < 1.0,
        "far lamps changed the picture by {difference} per channel on average"
    );
}

/// Small casters are drawn into the shadow maps by the software rasterizer: a ball 20 cm
/// across is one cluster, under sixteen texels wide in the nearest cascade, so it takes that
/// path, and if it drew nothing the ball would cast no shadow. Under a low sun the shadow
/// falls on the floor half a metre from the ball, where it must be darker than the same floor
/// with no ball; and the cascade must report that every cluster but the floor's one, which is
/// too large for software, went that way, so that the shadow cannot be the hardware's.
#[test]
fn small_casters_shadow_through_the_software_rasterizer() {
    let Some(gpu) = request_gpu() else {
        return;
    };
    let camera = Camera {
        position: Vec3::new(0.0, 2.0, 2.5),
        yaw: 0.0,
        pitch: -0.7,
        ..Default::default()
    };
    // Low enough that the shadow lands well clear of the ball, beyond the reach of the
    // screen-space contact shadows, which would darken the floor next to it on their own.
    let sun_direction = Vec3::new(5.0, 1.0, 0.0).normalize();
    let sun = Environment {
        sun_direction,
        sky_intensity: 0.0,
        gi_intensity: 0.0,
        ..Default::default()
    };
    let size = 256;
    let settings = RendererSettings {
        temporal_accumulation: false,
        ..Default::default()
    };
    let (radius, centre) = (0.1, Vec3::new(0.0, 0.1, 0.0));
    let mut scene = ball_on_floor();
    scene.instances[0].transform =
        Mat4::from_scale_rotation_translation(Vec3::splat(radius), Quat::IDENTITY, centre);
    let run = render_in(&gpu, &scene, &camera, settings, size, &sun);
    assert!(
        run.shadow_software_clusters > 0
            && run.shadow_software_clusters + 1 == run.shadow_clusters,
        "{} of the cascade's {} clusters went to the software rasterizer; all but the floor's should",
        run.shadow_software_clusters,
        run.shadow_clusters
    );
    let with_ball = run.pixels;
    scene.instances.remove(0);
    let floor = render_in(&gpu, &scene, &camera, settings, size, &sun).pixels;
    // Where the ball's centre casts to on the floor, on screen.
    let on_floor = centre - sun_direction * (centre.y / sun_direction.y);
    let clip = camera.projection(1.0) * camera.view() * on_floor.extend(1.0);
    let ndc = clip.truncate() / clip.w;
    let (px, py) = (
        ((ndc.x * 0.5 + 0.5) * size as f32) as u32,
        ((0.5 - ndc.y * 0.5) * size as f32) as u32,
    );
    let shadowed = brightness(patch(&with_ball, size, px, py, 2));
    let lit = brightness(patch(&floor, size, px, py, 2));
    assert!(
        shadowed + 30.0 < lit,
        "the floor at ({px}, {py}) is {shadowed} with the ball and {lit} without: no shadow"
    );
}

/// The floor of [`ball_on_floor`] with, in place of the ball, a square sheet of `side` by `side`
/// balls of `radius`, touching, level at `height` over the origin. Each ball is one cluster.
fn sheet_of_balls(side: u32, radius: f32, height: f32) -> SceneAsset {
    let mut scene = ball_on_floor();
    scene.meshes[0] = build_virtual_mesh(&sphere(0), &BuildOptions::default());
    let floor = scene.instances.pop().expect("the floor");
    scene.instances.clear();
    let half = side as f32 * radius;
    for z in 0..side {
        for x in 0..side {
            let at = Vec3::new(
                (2 * x + 1) as f32 * radius - half,
                height,
                (2 * z + 1) as f32 * radius - half,
            );
            scene.instances.push(instance(
                0,
                Mat4::from_scale_rotation_translation(Vec3::splat(radius), Quat::IDENTITY, at),
            ));
        }
    }
    scene.instances.push(floor);
    scene
}

/// Clusters under a texel of a shadow map across are drawn into it as specks: a point each, in
/// place of their triangles. A sheet of touching balls four millimetres across, half a texel of
/// even the nearest cascade, must cast a shadow, and the cascade must report that the balls
/// went as specks, so that the shadow cannot be the rasterizers'. A speck marks every texel its
/// bounds touch, where a rasterizer marks only the texels whose centres a triangle covers, and
/// misses most of them at this size: so specks never let through more light than triangles do.
#[test]
fn clusters_under_a_texel_shadow_as_specks() {
    let Some(gpu) = request_gpu() else {
        return;
    };
    let camera = Camera {
        position: Vec3::new(0.0, 2.0, 2.5),
        yaw: 0.0,
        pitch: -0.7,
        ..Default::default()
    };
    let sun_direction = Vec3::new(1.0, 1.0, 0.0).normalize();
    let sun = Environment {
        sun_direction,
        sky_intensity: 0.0,
        gi_intensity: 0.0,
        ..Default::default()
    };
    let size = 256;
    let settings = |speck_texels: f32| RendererSettings {
        temporal_accumulation: false,
        speck_texels,
        ..Default::default()
    };
    let (side, height) = (150, 0.5);
    let scene = sheet_of_balls(side, 0.002, height);
    // On the floor in the sheet's shadow, clear of the sheet as the camera sees it.
    let in_shadow = Vec3::new(-0.5, 0.0, 0.1);
    let (px, py) = pixel_of(&camera, size, in_shadow);

    let specks = render_in(&gpu, &scene, &camera, settings(1.0), size, &sun);
    assert!(
        specks.shadow_specks >= side * side / 2,
        "{} of the sheet's {} balls were drawn as specks",
        specks.shadow_specks,
        side * side
    );
    let triangles = render_in(&gpu, &scene, &camera, settings(0.0), size, &sun);
    assert_eq!(triangles.shadow_specks, 0, "specks are off");

    let mut bare = scene.clone();
    bare.instances.drain(..(side * side) as usize);
    let lit = brightness(patch(
        &render_in(&gpu, &bare, &camera, settings(1.0), size, &sun).pixels,
        size,
        px,
        py,
        2,
    ));
    let as_specks = brightness(patch(&specks.pixels, size, px, py, 2));
    let as_triangles = brightness(patch(&triangles.pixels, size, px, py, 2));
    assert!(
        as_specks + 30.0 < lit,
        "the floor at ({px}, {py}) is {as_specks} under the sheet and {lit} without it: no shadow"
    );
    assert!(
        as_specks < as_triangles + 12.0,
        "the shadow is {as_specks} drawn as specks, lighter than the {as_triangles} of triangles"
    );
}

/// The GI's voxels take clusters under a voxel across as specks too: a clump of small red balls
/// shows red in the voxels, where only the specks' own pass can have put it.
#[test]
fn clusters_under_a_voxel_are_voxelized_as_specks() {
    let Some(gpu) = request_gpu() else { return };
    let size = 96;
    let camera = over_the_floor();
    let mut scene = red_ball_on_floor();
    scene.meshes[0] = build_virtual_mesh(&sphere(0), &BuildOptions::default());
    let floor = scene.instances.pop().expect("the floor");
    scene.instances.clear();
    // A block of balls a twentieth of a metre across, in voxels a fifth of a metre across.
    let centre = Vec3::new(0.0, 1.0, 0.0);
    for index in 0..1000 {
        let cell = Vec3::new(
            (index % 10) as f32,
            (index / 10 % 10) as f32,
            (index / 100) as f32,
        );
        let mut ball = instance(
            0,
            Mat4::from_scale_rotation_translation(
                Vec3::splat(0.025),
                Quat::IDENTITY,
                centre + (cell - 4.5) * 0.1,
            ),
        );
        ball.material = 1;
        scene.instances.push(ball);
    }
    scene.instances.push(floor);
    let (bx, by) = pixel_of(&camera, size, centre);
    let redness = |speck_texels: f32| {
        let settings = RendererSettings {
            debug_view: DebugView::Voxels,
            speck_texels,
            ..Default::default()
        };
        let mut stage = Stage::new(&gpu, &scene, settings, size);
        stage.frames(&gpu, &camera, &low_sun(), 12);
        let [r, g, _] = patch(&stage.pixels(&gpu), size, bx, by, 1);
        r / g.max(1.0)
    };
    let as_specks = redness(1.0);
    assert!(
        as_specks > 2.0,
        "the balls drawn as specks aren't in the voxels: {as_specks}"
    );
    let as_triangles = redness(0.0);
    assert!(
        as_triangles > 2.0,
        "the balls drawn as triangles aren't in the voxels: {as_triangles}"
    );
}

/// A white ball on a wide floor, lit by the sun from +X, for judging how clouds light a scene.
fn ball_on_floor() -> SceneAsset {
    let mut floor = Mesh::default();
    for (x, z) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
        floor.vertices.push(Vertex {
            position: Vec3::new(x * 400.0, 0.0, z * 400.0),
            normal: Vec3::Y,
            uv: Vec2::new(x, z),
        });
    }
    floor.indices = vec![0, 2, 1, 0, 3, 2];
    let options = BuildOptions::default();
    SceneAsset {
        meshes: vec![
            build_virtual_mesh(&sphere(4), &options),
            build_virtual_mesh(&floor, &options),
        ],
        materials: vec![white_material()],
        instances: vec![
            instance(0, Mat4::from_translation(Vec3::new(0.0, 1.0, 0.0))),
            instance(1, Mat4::IDENTITY),
        ],
        ..Default::default()
    }
}

/// Renders the ball on its floor under `weather`, after `seconds` of it, lit by a sun from +X,
/// and reads back the image.
fn render_weather(
    gpu: &Gpu,
    camera: &Camera,
    weather: Weather,
    seconds: f32,
    size: u32,
) -> Vec<u8> {
    let environment = Environment {
        sun_direction: Vec3::new(1.0, 0.7, 0.2).normalize(),
        ..Default::default()
    };
    render_lit(gpu, camera, weather, seconds, &environment, size, 60)
}

/// Renders the ball on its floor under `weather` and `environment` for `frames` frames, and
/// reads back the last.
fn render_lit(
    gpu: &Gpu,
    camera: &Camera,
    weather: Weather,
    seconds: f32,
    environment: &Environment,
    size: u32,
    frames: u32,
) -> Vec<u8> {
    let errors: Arc<Mutex<Vec<String>>> = Default::default();
    let sink = errors.clone();
    gpu.device
        .on_uncaptured_error(Arc::new(move |error: wgpu::Error| {
            sink.lock().unwrap().push(error.to_string());
        }));
    // SOLSTICE_TEST_DEBUG=NAME shows a debug view instead, to see where a fault comes from.
    let mut settings = RendererSettings::default();
    if let Ok(name) = std::env::var("SOLSTICE_TEST_DEBUG") {
        settings.debug_view = DebugView::from_name(&name).expect("a debug view");
    }
    if std::env::var("SOLSTICE_TEST_NO_TAA").is_ok() {
        settings.temporal_accumulation = false;
    }
    let mut renderer = Renderer::new(gpu, &paginate(&ball_on_floor()), FORMAT, settings);
    renderer.set_weather(weather, 0.0);
    renderer.advance_weather(seconds);
    let target = test_target(gpu, size);
    let view = target.create_view(&Default::default());
    // Enough frames for the clouds' history and the exposure to settle.
    for _ in 0..frames {
        renderer.render(gpu, camera, environment, &view, (size, size), 1.0 / 60.0);
        gpu.device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
    }
    let pixels = read_pixels(gpu, &target, size);
    let errors = errors.lock().unwrap();
    assert!(errors.is_empty(), "GPU errors: {}", errors.join("\n"));
    // SOLSTICE_TEST_IMAGES=DIR keeps each image, to see what a failing test saw.
    if let Ok(dir) = std::env::var("SOLSTICE_TEST_IMAGES") {
        let name = format!(
            "{dir}/weather_{:.0}_{:.0}_{:.0}_{:.0}.ppm",
            camera.pitch * 100.0,
            camera.position.z,
            weather.coverage * 100.0,
            seconds
        );
        let mut ppm = format!("P6\n{size} {size}\n255\n").into_bytes();
        ppm.extend(pixels.chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]]));
        std::fs::write(name, ppm).unwrap();
    }
    pixels
}

/// Mean colour of a square of pixels centred at (x, y), `half` pixels either side.
fn patch(pixels: &[u8], size: u32, x: u32, y: u32, half: u32) -> [f32; 3] {
    let mut sum = [0.0; 3];
    let mut count = 0.0;
    for py in y - half..=y + half {
        for px in x - half..=x + half {
            let i = ((py * size + px) * 4) as usize;
            for c in 0..3 {
                sum[c] += pixels[i + c] as f32;
            }
            count += 1.0;
        }
    }
    sum.map(|s| s / count)
}

fn brightness(c: [f32; 3]) -> f32 {
    (c[0] + c[1] + c[2]) / 3.0
}

#[test]
fn clouds_fill_the_sky_and_grey_it() {
    let Some(gpu) = request_gpu() else { return };
    let camera = Camera {
        position: Vec3::new(0.0, 2.0, 0.0),
        pitch: 1.2,
        ..Default::default()
    };
    let sky = |name: &str| {
        let pixels = render_weather(&gpu, &camera, Weather::by_name(name).unwrap(), 0.0, 96);
        patch(&pixels, 96, 48, 30, 20)
    };
    let clear = sky("clear");
    let overcast = sky("overcast");
    // A clear sky is blue; an overcast one is grey.
    assert!(clear[2] > clear[0] * 1.3, "clear sky {clear:?}");
    assert!(
        overcast[2] < overcast[0] * 1.15,
        "overcast sky {overcast:?}"
    );
}

#[test]
fn overcast_light_is_soft_and_direct_sun_is_not() {
    let Some(gpu) = request_gpu() else { return };
    // Looking at the ball from the side, with the sun on its right: its shadow falls to the
    // left across the floor. Exposure adapts to each image, so compare the floor in the ball's
    // shadow with open floor, not either alone.
    let camera = Camera {
        position: Vec3::new(0.0, 1.0, 5.0),
        ..Default::default()
    };
    let contrast = |name: &str| {
        let pixels = render_weather(&gpu, &camera, Weather::by_name(name).unwrap(), 0.0, 128);
        let open = brightness(patch(&pixels, 128, 104, 87, 2));
        let shadow = brightness(patch(&pixels, 128, 22, 87, 2));
        open / shadow.max(1.0)
    };
    let clear = contrast("clear");
    let overcast = contrast("overcast");
    assert!(
        clear > 1.6,
        "open floor against the ball's shadow in clear weather: {clear}"
    );
    assert!(
        overcast < 1.25,
        "under overcast the shadow should all but vanish: {overcast}"
    );
}

#[test]
fn the_wind_carries_the_clouds_along() {
    let Some(gpu) = request_gpu() else { return };
    let camera = Camera {
        position: Vec3::new(0.0, 2.0, 0.0),
        pitch: 1.0,
        ..Default::default()
    };
    let weather = Weather::by_name("broken").unwrap();
    let before = render_weather(&gpu, &camera, weather, 0.0, 96);
    let after = render_weather(&gpu, &camera, weather, 300.0, 96);
    let changed = before
        .chunks_exact(4)
        .zip(after.chunks_exact(4))
        .filter(|(a, b)| (0..3).any(|c| a[c].abs_diff(b[c]) > 12))
        .count();
    let share = changed as f32 / (96.0 * 96.0);
    assert!(
        share > 0.1,
        "only {share} of the sky changed in five minutes of wind"
    );
}

/// A camera at the ball's floor looking along `direction`, `fov_degrees` high.
fn looking_at(direction: Vec3, fov_degrees: f32) -> Camera {
    Camera {
        position: Vec3::new(0.0, 2.0, 0.0),
        yaw: (-direction.x).atan2(-direction.z),
        pitch: direction.y.asin(),
        fov_y: fov_degrees.to_radians(),
        ..Default::default()
    }
}

fn mean_brightness(pixels: &[u8]) -> f32 {
    let sum: f32 = pixels
        .chunks_exact(4)
        .map(|p| (p[0] as f32 + p[1] as f32 + p[2] as f32) / 3.0)
        .sum();
    sum / (pixels.len() / 4) as f32
}

#[test]
fn twilight_fades_steadily_into_night() {
    let Some(gpu) = request_gpu() else { return };
    // Towards the sunset over a summer evening, from just before sunset into full night. The
    // scene may only ever grow darker: once it dipped below night in civil twilight.
    let place = Place::default();
    let daylight = Environment::default();
    let start = TimeOfDay {
        day: 172.0,
        hours: 19.0,
    };
    let camera = looking_at(
        start.sky(&place).sun * Vec3::new(1.0, 0.0, 1.0) + Vec3::Y * 0.1,
        60.0,
    );
    let clear = Weather::by_name("clear").unwrap();
    let mut previous = f32::MAX;
    for step in 0..12 {
        let time = TimeOfDay {
            hours: 19.0 + step as f32 * 0.25,
            ..start
        };
        let environment = time.environment(&place, &daylight);
        let pixels = render_lit(&gpu, &camera, clear, 0.0, &environment, 64, 240);
        let mean = mean_brightness(&pixels);
        assert!(
            mean <= previous * 1.04 + 1.0,
            "brightened from {previous} to {mean} at {:.2} h",
            time.hours
        );
        previous = mean;
    }
}

/// Something dim fills the view: a room lit through its windows, as far as exposure knows. By day
/// the eye adapts to it and sees it plainly. By night the same dimness is the night, and stays
/// dark: exposure reaching as far by night would turn every night grey.
#[test]
fn a_dim_room_is_seen_plainly_by_day_and_night_stays_dark() {
    let Some(gpu) = request_gpu() else { return };
    let size = 64;
    // A black wall glowing faintly, ten stops and more below sunlit ground and three below where
    // exposure stops by night, with the sun behind it and no sky to light or mirror in it.
    let mut scene = wall(0.0);
    scene.materials[0].emissive = [0.0006; 3];
    let day = Environment {
        sun_direction: Vec3::new(0.0, 0.7, -0.7).normalize(),
        sky_intensity: 0.0,
        ..Default::default()
    };
    // The moon stands where the sun did, at a 2500th of its light.
    let night = Environment {
        sun_illuminance: day.sun_illuminance / 2500.0,
        daylight_illuminance: day.sun_illuminance,
        ..day
    };
    let seen = |environment: &Environment| {
        let mut stage = Stage::new(&gpu, &scene, RendererSettings::default(), size);
        stage.frames(&gpu, &Camera::default(), environment, 30);
        mean_brightness(&stage.pixels(&gpu))
    };
    let (by_day, by_night) = (seen(&day), seen(&night));
    assert!(by_day > 90.0, "the room by day is {by_day}");
    assert!(
        by_night < by_day * 0.6,
        "the night is brightened to {by_night}, against {by_day} by day"
    );
}

#[test]
fn the_moon_is_drawn_without_a_dark_halo() {
    let Some(gpu) = request_gpu() else { return };
    // A waxing gibbous moon at midnight in June, filling the middle of a narrow view.
    let place = Place::default();
    let time = TimeOfDay {
        day: 172.0,
        hours: 0.0,
    };
    let camera = looking_at(time.sky(&place).moon, 10.0);
    let environment = time.environment(&place, &Environment::default());
    let size = 192;
    let pixels = render_lit(
        &gpu,
        &camera,
        Weather::by_name("clear").unwrap(),
        0.0,
        &environment,
        size,
        120,
    );
    let at = |x: f32, y: f32| brightness(patch(&pixels, size, x as u32, y as u32, 0));
    let centre = size as f32 / 2.0;
    let moon = brightness(patch(&pixels, size, size / 2, size / 2, 2));
    // Pixels just outside the disc, which is about 8 pixels across its radius, and open sky
    // further out, all the way round.
    let (mut rim, mut sky) = (f32::MAX, 0.0_f32);
    for i in 0..32 {
        let angle = i as f32 / 32.0 * std::f32::consts::TAU;
        let (s, c) = angle.sin_cos();
        rim = rim.min(at(centre + c * 10.0, centre + s * 10.0));
        sky = sky.max(at(centre + c * 20.0, centre + s * 20.0));
    }
    assert!(
        moon > sky * 2.0,
        "the moon {moon} hardly stands out from the sky {sky}"
    );
    assert!(
        rim >= sky * 0.85 - 2.0,
        "a dark ring round the moon: {rim} against sky {sky}"
    );
}

#[test]
fn a_moonless_night_is_full_of_stars() {
    let Some(gpu) = request_gpu() else { return };
    // Early June, after the crescent moon has set: looking high into the southern sky.
    let place = Place::default();
    let time = TimeOfDay {
        day: 157.0,
        hours: 1.0,
    };
    let camera = looking_at(Vec3::new(0.0, 0.8, 0.6).normalize(), 55.0);
    let environment = time.environment(&place, &Environment::default());
    let size = 256;
    let pixels = render_lit(
        &gpu,
        &camera,
        Weather::by_name("clear").unwrap(),
        0.0,
        &environment,
        size,
        120,
    );
    let mut levels: Vec<f32> = pixels
        .chunks_exact(4)
        .map(|p| (p[0] as f32 + p[1] as f32 + p[2] as f32) / 3.0)
        .collect();
    let stars = levels.iter().filter(|&&v| v > 0.0).count();
    levels.sort_by(f32::total_cmp);
    let background = levels[levels.len() / 2];
    let bright = levels.iter().filter(|&&v| v > background + 30.0).count();
    assert!(stars > 0);
    assert!(
        bright > 40,
        "only {bright} star pixels stand out from a sky of {background}"
    );
    assert!(
        background < 60.0,
        "the night sky is too bright for stars to show: {background}"
    );
}

/// The ball on its floor with an open sea at `sea_level`, seen through `debug` (the sea's
/// normals for coverage, or the final image), after enough frames for its waves to run.
fn render_sea(
    gpu: &Gpu,
    camera: &Camera,
    ocean: &str,
    sea_level: f32,
    debug: DebugView,
    quality: Quality,
    size: u32,
) -> Vec<u8> {
    let environment = Environment {
        sun_direction: Vec3::new(1.0, 0.7, 0.2).normalize(),
        ..Default::default()
    };
    render_sea_under(
        gpu,
        camera,
        (ocean, sea_level),
        debug,
        quality,
        size,
        &environment,
    )
}

/// `render_sea` for a `sea`, its ocean's name and its level, under `environment`.
fn render_sea_under(
    gpu: &Gpu,
    camera: &Camera,
    (ocean, sea_level): (&str, f32),
    debug: DebugView,
    quality: Quality,
    size: u32,
    environment: &Environment,
) -> Vec<u8> {
    let errors: Arc<Mutex<Vec<String>>> = Default::default();
    let sink = errors.clone();
    gpu.device
        .on_uncaptured_error(Arc::new(move |error: wgpu::Error| {
            sink.lock().unwrap().push(error.to_string());
        }));
    let mut settings = RendererSettings::default();
    settings.apply_quality(quality);
    settings.debug_view = debug;
    // Temporal accumulation would average thin cracks away, hiding what these tests look for.
    settings.temporal_accumulation = false;
    let mut renderer = Renderer::new(gpu, &paginate(&ball_on_floor()), FORMAT, settings);
    renderer.set_ocean(Some(Ocean::by_name(ocean).unwrap()), 0.0);
    renderer.set_sea_level(sea_level);
    let target = test_target(gpu, size);
    let view = target.create_view(&Default::default());
    for _ in 0..30 {
        renderer.render(gpu, camera, environment, &view, (size, size), 1.0 / 60.0);
        gpu.device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
    }
    let pixels = read_pixels(gpu, &target, size);
    let errors = errors.lock().unwrap();
    assert!(errors.is_empty(), "GPU errors: {}", errors.join("\n"));
    // SOLSTICE_TEST_IMAGES=DIR keeps each image, to see what a failing test saw.
    if let Ok(dir) = std::env::var("SOLSTICE_TEST_IMAGES") {
        let name = format!(
            "{dir}/sea_{:.0}_{:.0}_{:.0}_{debug:?}.ppm",
            camera.position.x, camera.position.y, camera.position.z
        );
        let mut ppm = format!(
            "P6
{size} {size}
255
"
        )
        .into_bytes();
        ppm.extend(pixels.chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]]));
        std::fs::write(name, ppm).unwrap();
    }
    pixels
}

/// Whether a pixel of the ocean debug view shows the sea: a normal leaning up, so green
/// high, and red and blue within a tilt of 0.75 of the middle, as steep wave faces have. The
/// sky and the white ball clip to white, which this is not.
fn shows_sea(pixel: &[u8]) -> bool {
    let near_middle = |c: u8| (32..=223).contains(&c);
    pixel[1] > 180 && near_middle(pixel[0]) && near_middle(pixel[2])
}

#[test]
fn the_sea_covers_everything_below_the_horizon() {
    let Some(gpu) = request_gpu() else { return };
    // The sea stands 5 m up. From just above the water, from a cliff, from far out at sea and from high above: every
    // pixel below the horizon shows sea, so the surface has no cracks or holes, whatever the
    // patches' sizes and however far they reach. The ball stands near the origin, so these
    // look away from it.
    for (size, quality, position, pitch) in [
        (128, Quality::High, Vec3::new(30.0, 6.5, 0.0), -0.15),
        (128, Quality::High, Vec3::new(40.0, 30.0, 3.0), -0.5),
        (128, Quality::Low, Vec3::new(5_000.0, 11.0, -3_000.0), -0.1),
        (128, Quality::Medium, Vec3::new(60.0, 400.0, 0.0), -1.2),
        // Finely, looking along the water where many levels meet within the view.
        (384, Quality::High, Vec3::new(12.0, 8.0, -5.0), -0.08),
    ] {
        let camera = Camera {
            position,
            yaw: -1.57,
            pitch,
            ..Default::default()
        };
        // The open ocean's swell, well above the floor, which its troughs must not reach.
        let pixels = render_sea(&gpu, &camera, "ocean", 5.0, DebugView::Ocean, quality, size);
        // Rows below the horizon, which sits at the screen row of the direction straight
        // out, ignoring the few rows the waves themselves raise and lower.
        let tan_half = (camera.fov_y * 0.5).tan();
        // Pitched down, the horizon is above the middle of the view.
        let horizon = ((0.5 - 0.5 * (-pitch).tan() / tan_half) * size as f32).max(0.0) as u32;
        let first = (horizon + 6).min(size);
        let mut missing = 0;
        for y in first..size {
            for x in 0..size {
                let i = ((y * size + x) * 4) as usize;
                if !shows_sea(&pixels[i..i + 4]) {
                    missing += 1;
                }
            }
        }
        let checked = (size - first) * size;
        assert!(
            checked > 1000,
            "{position}: only {checked} pixels below the horizon"
        );
        assert_eq!(
            missing, 0,
            "{position}, {quality:?}: {missing} of {checked} pixels show no sea"
        );
    }
}

#[test]
fn what_stands_out_of_the_sea_is_not_covered_by_it() {
    let Some(gpu) = request_gpu() else { return };
    // The ball's top stands out of calm water half way up it, a harbour's, without swell: its
    // upper half shows the ball, not water.
    let camera = Camera {
        position: Vec3::new(0.0, 1.6, 4.0),
        ..Default::default()
    };
    let size = 128;
    let pixels = render_sea(
        &gpu,
        &camera,
        "murky",
        1.0,
        DebugView::Ocean,
        Quality::High,
        size,
    );
    let at = |x: u32, y: u32| {
        let i = ((y * size + x) * 4) as usize;
        pixels[i..i + 4].to_vec()
    };
    // The ball's upper half, above the water, and the sea in front of it, low in the view.
    assert!(
        !shows_sea(&at(64, 52)),
        "the ball is drawn as water: {:?}",
        at(64, 52)
    );
    assert!(
        shows_sea(&at(64, 120)),
        "the sea in front is missing: {:?}",
        at(64, 120)
    );
}

/// The ball on its floor with these ponds, and a calm sea at `sea` if any, as finally drawn.
fn render_ponds(gpu: &Gpu, camera: &Camera, ponds: &[Pond], sea: Option<f32>) -> Vec<u8> {
    render_ponds_in(gpu, camera, ponds, sea, Weather::default())
}

/// The same, in `weather`.
fn render_ponds_in(
    gpu: &Gpu,
    camera: &Camera,
    ponds: &[Pond],
    sea: Option<f32>,
    weather: Weather,
) -> Vec<u8> {
    let errors: Arc<Mutex<Vec<String>>> = Default::default();
    let sink = errors.clone();
    gpu.device
        .on_uncaptured_error(Arc::new(move |error: wgpu::Error| {
            sink.lock().unwrap().push(error.to_string());
        }));
    let mut settings = RendererSettings::default();
    settings.apply_quality(Quality::High);
    // Each pixel is then this frame's alone, so one outside a pond can be held to what it was.
    settings.temporal_accumulation = false;
    let mut renderer = Renderer::new(gpu, &paginate(&ball_on_floor()), FORMAT, settings);
    if let Some(level) = sea {
        renderer.set_ocean(Some(Ocean::by_name("murky").unwrap()), 0.0);
        renderer.set_sea_level(level);
    }
    renderer.set_ponds(ponds);
    renderer.set_weather(weather, 0.0);
    // Daylight well above what the views measure keeps exposure at its limit, the same for all.
    let environment = Environment {
        sun_direction: Vec3::new(1.0, 0.7, 0.2).normalize(),
        daylight_illuminance: 4000.0,
        ..Default::default()
    };
    let target = test_target(gpu, POND_SIZE);
    let view = target.create_view(&Default::default());
    for _ in 0..30 {
        renderer.render(
            gpu,
            camera,
            &environment,
            &view,
            (POND_SIZE, POND_SIZE),
            1.0 / 60.0,
        );
        gpu.device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
    }
    let pixels = read_pixels(gpu, &target, POND_SIZE);
    let errors = errors.lock().unwrap();
    assert!(
        errors.is_empty(),
        "GPU errors: {}",
        errors.join(
            "
"
        )
    );
    pixels
}

const POND_SIZE: u32 = 128;

/// How far apart two colours are: the most any channel differs.
fn apart(a: [f32; 3], b: [f32; 3]) -> f32 {
    (0..3).map(|c| (a[c] - b[c]).abs()).fold(0.0, f32::max)
}

#[test]
fn a_pond_is_water_within_its_rectangle_and_nothing_outside_it() {
    let Some(gpu) = request_gpu() else { return };
    // Looking down at the floor with the ball in the middle. A pond 2.5 m deep covers the floor
    // to the ball's left, and stops short of the ball.
    let camera = Camera {
        position: Vec3::new(0.0, 8.0, 10.0),
        pitch: -0.7,
        ..Default::default()
    };
    let pond = Pond::parse("-60,-60,-1.5,60,2.5").unwrap();
    let dry = render_ponds(&gpu, &camera, &[], None);
    let wet = render_ponds(&gpu, &camera, &[pond], None);
    let (left, right, row) = (POND_SIZE / 4, POND_SIZE * 3 / 4, POND_SIZE / 2);
    let changed = apart(
        patch(&dry, POND_SIZE, left, row, 6),
        patch(&wet, POND_SIZE, left, row, 6),
    );
    let kept = apart(
        patch(&dry, POND_SIZE, right, row, 6),
        patch(&wet, POND_SIZE, right, row, 6),
    );
    assert!(kept < 3.0, "the floor outside the pond changed by {kept}");
    assert!(
        changed > 20.0,
        "the floor under the pond changed by only {changed}"
    );

    // A scene with a sea draws the sea, and its ponds not at all.
    let sea = render_ponds(&gpu, &camera, &[], Some(1.0));
    let both = render_ponds(&gpu, &camera, &[pond], Some(1.0));
    for x in [left, right] {
        let differs = apart(
            patch(&sea, POND_SIZE, x, row, 6),
            patch(&both, POND_SIZE, x, row, 6),
        );
        assert!(differs < 3.0, "a pond changed the sea by {differs} at {x}");
    }
}

#[test]
fn the_bed_shows_through_shallow_water_and_not_through_deep() {
    let Some(gpu) = request_gpu() else { return };
    // Looking down into a clear sea over the white floor, away from the ball: under 30 cm of
    // water the floor shows bright; under 30 m the water absorbs what it reflects, and the
    // sea is dark. Water that hid its bed at any depth, or showed it at every depth, fails.
    let size = 64;
    // Daylight is set well above what either view measures, so exposure stays at its darkest
    // limit for both. Left to follow each picture it brings the deep sea up to middle grey, and
    // the two then differ by where exposure stops, not by what the water hides.
    let environment = Environment {
        sun_direction: Vec3::new(1.0, 0.7, 0.2).normalize(),
        daylight_illuminance: 4000.0,
        ..Default::default()
    };
    let brightness = |depth: f32| {
        let camera = Camera {
            position: Vec3::new(40.0, depth + 4.0, 0.0),
            yaw: -1.57,
            pitch: -1.3,
            ..Default::default()
        };
        let pixels = render_sea_under(
            &gpu,
            &camera,
            ("ocean", depth),
            DebugView::Lit,
            Quality::High,
            size,
            &environment,
        );
        // The middle of the view, clear of the sun's highlight at the edges.
        let mut sum = 0.0;
        let mut count = 0.0;
        for y in size / 4..size * 3 / 4 {
            for x in size / 4..size * 3 / 4 {
                let i = ((y * size + x) * 4) as usize;
                sum += pixels[i..i + 3].iter().map(|&c| c as f32).sum::<f32>() / 3.0;
                count += 1.0;
            }
        }
        sum / count
    };
    let shallow = brightness(0.3);
    let deep = brightness(30.0);
    assert!(
        shallow > deep * 1.5 + 10.0,
        "the bed under 30 cm of water ({shallow}) is not brighter than 30 m of sea ({deep})"
    );
}

#[test]
fn waves_do_not_rise_through_low_land() {
    let Some(gpu) = request_gpu() else { return };
    // The floor stands just 20 cm above the open ocean, whose swell rises far higher. Waves die
    // as the water shoals, and over dry land there are none, so no crest ever shows through the
    // floor, however low it lies. Seen from high above, the view spans more than a whole
    // wavelength of the swell, so crests are always in it, and all of it is floor.
    let size = 128;
    let camera = Camera {
        position: Vec3::new(20.0, 250.0, 10.0),
        yaw: -1.57,
        pitch: -1.55,
        ..Default::default()
    };
    let pixels = render_sea(
        &gpu,
        &camera,
        "ocean",
        -0.2,
        DebugView::Ocean,
        Quality::High,
        size,
    );
    let wet = pixels.chunks_exact(4).filter(|p| shows_sea(p)).count();
    assert_eq!(wet, 0, "{wet} pixels of sea show through the floor");
}

/// The ball on its floor, both of `material`, with a calm sea at `sea_level` or none, seen
/// through the base-colour debug view: the albedo the G-buffer holds, wetness included.
fn render_albedo(gpu: &Gpu, material: Material, sea_level: Option<f32>) -> Vec<u8> {
    render_wet(
        gpu,
        material,
        sea_level,
        DebugView::Albedo,
        Vec3::new(0.0, 1.6, 4.0),
        0.0,
    )
}

/// The ball on its floor, both of `material`, with a calm sea at `sea_level` or none, seen
/// through `debug` from `position` pitched by `pitch`, under a low sun straight ahead.
fn render_wet(
    gpu: &Gpu,
    material: Material,
    sea_level: Option<f32>,
    debug: DebugView,
    position: Vec3,
    pitch: f32,
) -> Vec<u8> {
    let size = 128;
    let mut scene = ball_on_floor();
    scene.materials = vec![material];
    let settings = RendererSettings {
        debug_view: debug,
        temporal_accumulation: false,
        ..Default::default()
    };
    let mut renderer = Renderer::new(gpu, &paginate(&scene), FORMAT, settings);
    if let Some(level) = sea_level {
        renderer.set_ocean(Some(Ocean::by_name("murky").unwrap()), 0.0);
        renderer.set_sea_level(level);
    }
    let camera = Camera {
        position,
        pitch,
        ..Default::default()
    };
    let environment = Environment {
        sun_direction: Vec3::new(0.0, 0.35, -1.0).normalize(),
        ..Default::default()
    };
    let target = test_target(gpu, size);
    let view = target.create_view(&Default::default());
    for _ in 0..4 {
        renderer.render(gpu, &camera, &environment, &view, (size, size), 1.0 / 60.0);
        gpu.device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
    }
    read_pixels(gpu, &target, size)
}

fn brightness_at(pixels: &[u8], x: u32, y: u32) -> f32 {
    let i = ((y * 128 + x) * 4) as usize;
    pixels[i..i + 3].iter().map(|&c| c as f32).sum::<f32>() / 3.0
}

#[test]
fn water_darkens_porous_surfaces_but_not_metal_or_what_stands_above_it() {
    let Some(gpu) = request_gpu() else { return };
    // The sea lies 5 cm below the floor, so the floor is wet with it. The ball rises well
    // above the water, out of its reach.
    let (floor, ball) = ((64, 120), (64, 52));
    let sand = Material {
        base_color: [0.45, 0.38, 0.28, 1.0],
        roughness: 0.95,
        ..Default::default()
    };
    let dry = render_albedo(&gpu, sand, None);
    let wet = render_albedo(&gpu, sand, Some(-0.05));
    let (dry_floor, wet_floor) = (
        brightness_at(&dry, floor.0, floor.1),
        brightness_at(&wet, floor.0, floor.1),
    );
    // Wet sand reflects about half what it does dry; in the debug view's sRGB, darker by
    // well over a tenth.
    assert!(
        wet_floor < dry_floor * 0.85,
        "wet sand ({wet_floor}) is not darker than dry ({dry_floor})"
    );
    let (dry_ball, wet_ball) = (
        brightness_at(&dry, ball.0, ball.1),
        brightness_at(&wet, ball.0, ball.1),
    );
    assert!(
        (dry_ball - wet_ball).abs() < 2.0,
        "the ball's upper half, far above the water, changed: {dry_ball} dry, {wet_ball} wet"
    );

    // Metal has no pores to fill: water only films it, which leaves its colour alone. Its
    // fringe is thin, so the sea comes up level with the floor to wet it.
    let metal = Material {
        base_color: [0.6, 0.55, 0.5, 1.0],
        roughness: 0.6,
        metallic: 1.0,
        ..Default::default()
    };
    let dry = brightness_at(&render_albedo(&gpu, metal, None), floor.0, floor.1);
    let wet = brightness_at(&render_albedo(&gpu, metal, Some(0.0)), floor.0, floor.1);
    assert!(
        (dry - wet).abs() < 2.0,
        "wet metal changed colour: {dry} dry, {wet} wet"
    );
}

#[test]
fn wet_ground_shines_where_dry_ground_does_not() {
    let Some(gpu) = request_gpu() else { return };
    // A low sun straight ahead, mirrored by the floor a few metres in front of the camera, to
    // one side of the ball. Dry sand is too rough to show it as more than a faint sheen; wet,
    // a film of water turns it into a bright glint.
    let sand = Material {
        base_color: [0.45, 0.38, 0.28, 1.0],
        roughness: 0.95,
        ..Default::default()
    };
    let position = Vec3::new(10.0, 1.6, 4.0);
    let peak = |pixels: &[u8]| {
        (70..128)
            .flat_map(|y| (0..128).map(move |x| (x, y)))
            .map(|(x, y)| brightness_at(pixels, x, y))
            .fold(0.0, f32::max)
    };
    let dry = peak(&render_wet(
        &gpu,
        sand,
        None,
        DebugView::Lit,
        position,
        -0.25,
    ));
    let wet = peak(&render_wet(
        &gpu,
        sand,
        Some(-0.05),
        DebugView::Lit,
        position,
        -0.25,
    ));
    assert!(
        wet > dry * 1.3 + 10.0,
        "the wet floor's glint ({wet}) is no brighter than the dry floor's sheen ({dry})"
    );
}

#[test]
fn a_falling_sea_leaves_a_wet_line_that_dries() {
    let Some(gpu) = request_gpu() else { return };
    // The ball stands in a calm harbour. With the sea at 1.2 m its side at 0.9 m is under
    // water; the sea then falls to 0.6 m, as a tide going out, and that side stands 30 cm
    // clear of it, above any fringe. It stays wet from where the water was, and only after
    // some minutes has it dried to match a ball that the sea never reached so high.
    let size = 128;
    let sand = Material {
        base_color: [0.45, 0.38, 0.28, 1.0],
        roughness: 0.95,
        ..Default::default()
    };
    let mut scene = ball_on_floor();
    scene.materials = vec![sand];
    let scene = paginate(&scene);
    let camera = Camera {
        position: Vec3::new(0.0, 0.9, 4.0),
        ..Default::default()
    };
    let target = test_target(&gpu, size);
    let view = target.create_view(&Default::default());
    let renderer_at = |level: f32| {
        let settings = RendererSettings {
            debug_view: DebugView::Albedo,
            temporal_accumulation: false,
            ..Default::default()
        };
        let mut renderer = Renderer::new(&gpu, &scene, FORMAT, settings);
        renderer.set_ocean(Some(Ocean::by_name("murky").unwrap()), 0.0);
        renderer.set_sea_level(level);
        renderer
    };
    let run_from = |renderer: &mut Renderer, camera: &Camera, frames: u32, seconds: f32| {
        for _ in 0..frames {
            renderer.render(
                &gpu,
                camera,
                &Environment::default(),
                &view,
                (size, size),
                seconds,
            );
            gpu.device
                .poll(wgpu::PollType::wait_indefinitely())
                .unwrap();
        }
        brightness_at(&read_pixels(&gpu, &target, size), 64, 64)
    };
    let run = |renderer: &mut Renderer, frames: u32, seconds: f32| {
        run_from(renderer, &camera, frames, seconds)
    };

    let mut ebbing = renderer_at(1.2);
    run(&mut ebbing, 30, 1.0 / 60.0);
    ebbing.set_sea_level(0.6);
    let just_left = run(&mut ebbing, 4, 1.0 / 60.0);
    let never_reached = run(&mut renderer_at(0.6), 34, 1.0 / 60.0);
    assert!(
        just_left < never_reached * 0.85,
        "where the sea just was ({just_left}) is not wetter than where it never reached ({never_reached})"
    );
    // The map is fixed in the world: walking 40 m off, its window sliding with the camera, and
    // back again, the line is where it was.
    let away = Camera {
        position: camera.position + Vec3::new(40.3, 0.0, -17.6),
        ..camera
    };
    run_from(&mut ebbing, &away, 4, 1.0 / 60.0);
    let back = run(&mut ebbing, 2, 1.0 / 60.0);
    assert!(
        (back - just_left).abs() < 3.0,
        "the line moved with the camera: {back} on coming back, {just_left} before"
    );
    // Going further than the map reaches, it forgets the line: coming back, the spot starts
    // afresh from where the water is now, well below it.
    let far = Camera {
        position: camera.position + Vec3::new(300.0, 0.0, 0.0),
        ..camera
    };
    let mut wandering = renderer_at(1.2);
    run(&mut wandering, 30, 1.0 / 60.0);
    wandering.set_sea_level(0.6);
    run_from(&mut wandering, &far, 4, 1.0 / 60.0);
    let returned = run(&mut wandering, 2, 1.0 / 60.0);
    assert!(
        (returned - never_reached).abs() < 3.0,
        "a spot the map left behind kept its line: {returned}, against {never_reached} never wetted"
    );
    // Ten minutes on, twenty half-lives of drying.
    let dried = run(&mut ebbing, 2, 300.0);
    assert!(
        (dried - never_reached).abs() < 3.0,
        "after ten minutes the line is still wet: {dried}, against {never_reached} never wetted"
    );
    // A rising sea wets the ball in the very frame it rises: resolve reads the map written
    // that frame, not the one before.
    let mut rising = renderer_at(0.6);
    run(&mut rising, 30, 1.0 / 60.0);
    rising.set_sea_level(1.2);
    let risen = run(&mut rising, 1, 1.0 / 60.0);
    assert!(
        risen < never_reached * 0.85,
        "the frame the sea rose, its new water was not yet wet: {risen}, against {never_reached} dry"
    );
}

#[test]
fn waves_wet_the_faces_they_break_against() {
    let Some(gpu) = request_gpu() else { return };
    // The ball stands in a metre of water under the open ocean's swell, which the shallows
    // hold to crests about 38 cm high. Its side 30 cm above the calm sea gets wet from the
    // crests that break against it, though the ground under that side, the ball itself, stands
    // above the sea. Fifteen seconds take in more than a whole swell period.
    let size = 128;
    let sand = Material {
        base_color: [0.45, 0.38, 0.28, 1.0],
        roughness: 0.95,
        ..Default::default()
    };
    let mut scene = ball_on_floor();
    scene.materials = vec![sand];
    let scene = paginate(&scene);
    let camera = Camera {
        position: Vec3::new(0.0, 1.3, 4.0),
        ..Default::default()
    };
    let target = test_target(&gpu, size);
    let view = target.create_view(&Default::default());
    let side_at = |sea_level: f32| {
        let settings = RendererSettings {
            debug_view: DebugView::Albedo,
            temporal_accumulation: false,
            ..Default::default()
        };
        let mut renderer = Renderer::new(&gpu, &scene, FORMAT, settings);
        renderer.set_ocean(Some(Ocean::by_name("ocean").unwrap()), 0.0);
        renderer.set_sea_level(sea_level);
        for _ in 0..60 {
            renderer.render(
                &gpu,
                &camera,
                &Environment::default(),
                &view,
                (size, size),
                0.25,
            );
            gpu.device
                .poll(wgpu::PollType::wait_indefinitely())
                .unwrap();
        }
        brightness_at(&read_pixels(&gpu, &target, size), 64, 64)
    };
    let dry = side_at(-3.0);
    let washed = side_at(1.0);
    assert!(
        washed < dry * 0.85,
        "the side the waves break against is dry: {washed}, against {dry} with no sea near"
    );
}

/// A red ball, instance 0, standing on a white floor, instance 1.
fn red_ball_on_floor() -> SceneAsset {
    let mut scene = ball_on_floor();
    scene.materials.push(Material {
        base_color: [0.9, 0.08, 0.06, 1.0],
        ..Default::default()
    });
    scene.instances[0].material = 1;
    scene
}

/// A renderer drawing into a test target, frame by frame, between which a test can change the
/// scene. It fails on any validation error.
struct Stage {
    renderer: Renderer,
    target: wgpu::Texture,
    view: wgpu::TextureView,
    size: u32,
    errors: Arc<Mutex<Vec<String>>>,
}

impl Stage {
    fn new(gpu: &Gpu, scene: &SceneAsset, settings: RendererSettings, size: u32) -> Self {
        let errors: Arc<Mutex<Vec<String>>> = Default::default();
        let sink = errors.clone();
        gpu.device
            .on_uncaptured_error(Arc::new(move |error: wgpu::Error| {
                sink.lock().unwrap().push(error.to_string());
            }));
        let target = test_target(gpu, size);
        let view = target.create_view(&Default::default());
        Self {
            renderer: Renderer::new(gpu, &paginate(scene), FORMAT, settings),
            target,
            view,
            size,
            errors,
        }
    }

    fn frames(&mut self, gpu: &Gpu, camera: &Camera, environment: &Environment, count: u32) {
        for _ in 0..count {
            self.renderer.render(
                gpu,
                camera,
                environment,
                &self.view,
                (self.size, self.size),
                1.0 / 60.0,
            );
            gpu.device
                .poll(wgpu::PollType::wait_indefinitely())
                .unwrap();
        }
    }

    fn pixels(&self, gpu: &Gpu) -> Vec<u8> {
        let pixels = read_pixels(gpu, &self.target, self.size);
        let errors = self.errors.lock().unwrap();
        assert!(errors.is_empty(), "GPU errors: {}", errors.join("\n"));
        pixels
    }
}

/// The pixel a point in the world lands on, seen by `camera` in a square image.
fn pixel_of(camera: &Camera, size: u32, world: Vec3) -> (u32, u32) {
    let ndc = (camera.projection(1.0) * camera.view()).project_point3(world);
    let x = ((ndc.x * 0.5 + 0.5) * size as f32) as u32;
    let y = ((0.5 - ndc.y * 0.5) * size as f32) as u32;
    (x.min(size - 1), y.min(size - 1))
}

/// Looking down at the floor around the origin from above and in front.
fn over_the_floor() -> Camera {
    Camera {
        position: Vec3::new(1.5, 11.0, 11.0),
        pitch: -std::f32::consts::FRAC_PI_4,
        ..Default::default()
    }
}

/// A sun 45 degrees up in the +X direction, so a ball's shadow falls on the floor to its -X
/// side: centred 1 m from its foot, from 2.4 m to 0.6 m beyond it, as seen from above.
fn low_sun() -> Environment {
    Environment {
        sun_direction: Vec3::new(1.0, 1.0, 0.0).normalize(),
        ..Default::default()
    }
}

/// The ball at `x` along the floor, `scale` times its size, standing on the floor.
fn place_scaled(x: f32, scale: f32) -> Mat4 {
    Mat4::from_scale_rotation_translation(
        Vec3::splat(scale),
        glam::Quat::IDENTITY,
        Vec3::new(x, scale, 0.0),
    )
}

fn place(x: f32) -> Mat4 {
    place_scaled(x, 1.0)
}

/// Settings for tests that look at a frame or two after a change: no temporal accumulation,
/// which would blend old frames in and hide a late change.
fn untemporal() -> RendererSettings {
    RendererSettings {
        temporal_accumulation: false,
        ..Default::default()
    }
}

/// The ball and the light on the floor its shadow falls on, seen by a camera `scale` times as
/// far away as [`over_the_floor`], with the ball `scale` times as large.
struct ShadowProbe {
    camera: Camera,
    size: u32,
    scale: f32,
}

impl ShadowProbe {
    fn new(scale: f32, size: u32) -> Self {
        let mut camera = over_the_floor();
        camera.position *= scale;
        Self {
            camera,
            size,
            scale,
        }
    }

    /// Light on the floor where the shadow of a ball at `x` falls, against open floor.
    fn shade(&self, pixels: &[u8], x: f32) -> f32 {
        let at = |p: Vec3| {
            let (px, py) = pixel_of(&self.camera, self.size, p * self.scale);
            brightness(patch(pixels, self.size, px, py, 1))
        };
        at(Vec3::new(x - 1.6, 0.0, 0.0)) / at(Vec3::new(-4.0, 0.0, 3.0))
    }

    /// Whether the ball is drawn at `x`.
    fn red(&self, pixels: &[u8], x: f32) -> bool {
        let (px, py) = pixel_of(&self.camera, self.size, Vec3::new(x, 1.0, 0.0) * self.scale);
        let [r, g, _] = patch(pixels, self.size, px, py, 1);
        r > 2.0 * g
    }
}

/// Moves a ball twice, `scale` times as large and seen from `scale` times as far, and checks
/// that it is drawn where it went with its shadow, and that no shadow is left behind.
fn check_moving_shadows(scale: f32) {
    let Some(gpu) = request_gpu() else { return };
    let probe = ShadowProbe::new(scale, 192);
    let sun = low_sun();
    let mut scene = red_ball_on_floor();
    scene.instances[0].transform = place_scaled(0.0, scale);
    let mut stage = Stage::new(&gpu, &scene, untemporal(), probe.size);

    stage.frames(&gpu, &probe.camera, &sun, 8);
    let pixels = stage.pixels(&gpu);
    assert!(probe.red(&pixels, 0.0), "the ball isn't where it stands");
    assert!(
        probe.shade(&pixels, 0.0) < 0.6,
        "the standing ball casts no shadow"
    );

    for (from, to) in [(0.0, 3.0), (3.0, 6.0)] {
        assert!(stage
            .renderer
            .move_instance(0, place_scaled(to * scale, scale)));
        // Two frames: the first draws the move, and by the second every cached cascade must
        // have caught up, not just the near one.
        stage.frames(&gpu, &probe.camera, &sun, 2);
        let pixels = stage.pixels(&gpu);
        assert!(probe.red(&pixels, to), "the ball isn't drawn at {to}");
        assert!(
            !probe.red(&pixels, from),
            "the ball is still drawn at {from}"
        );
        let (new, old) = (probe.shade(&pixels, to), probe.shade(&pixels, from));
        assert!(
            new < 0.6,
            "the ball moved to {to} casts no shadow there: {new}"
        );
        assert!(old > 0.85, "a shadow is left behind at {from}: {old}");
    }

    // Static again where it stands: its shadow comes back from the cached cascades, at once.
    assert!(stage.renderer.set_instance_movable(0, false));
    stage.frames(&gpu, &probe.camera, &sun, 2);
    let shade = probe.shade(&stage.pixels(&gpu), 6.0);
    assert!(
        shade < 0.6,
        "the ball made static again lost its shadow: {shade}"
    );
}

/// The near cascades, about 15 m away.
#[test]
fn a_moved_ball_is_drawn_where_it_went_with_its_shadow_and_none_is_left_behind() {
    check_moving_shadows(1.0);
}

/// The same, six times as large and far: about 90 m away, in the far cascades, which have the
/// lower row of the movable layer.
#[test]
fn a_distant_moved_ball_casts_its_shadow_from_the_far_cascades() {
    check_moving_shadows(6.0);
}

#[test]
fn moving_things_show_their_motion_and_stop_showing_it_when_they_stop() {
    let Some(gpu) = request_gpu() else { return };
    let size = 128;
    let camera = over_the_floor();
    let settings = RendererSettings {
        debug_view: DebugView::Motion,
        ..untemporal()
    };
    let mut stage = Stage::new(&gpu, &red_ball_on_floor(), settings, size);
    let environment = low_sun();

    // Red over green, at the ball's centre and on the floor beside it.
    let colours = |pixels: &[u8], x: f32| {
        let (bx, by) = pixel_of(&camera, size, Vec3::new(x, 1.0, 0.0));
        let (fx, fy) = pixel_of(&camera, size, Vec3::new(-4.0, 0.0, 3.0));
        (
            patch(pixels, size, bx, by, 1),
            patch(pixels, size, fx, fy, 1),
        )
    };

    stage.frames(&gpu, &camera, &environment, 4);
    let mut x = 0.0;
    for _ in 0..6 {
        x += 0.1;
        stage.renderer.move_instance(0, place(x));
        stage.frames(&gpu, &camera, &environment, 1);
    }
    let (ball, floor) = colours(&stage.pixels(&gpu), x);
    assert!(
        ball[0] > floor[0] + 30.0,
        "the ball moving right shows no motion to the right: {ball:?} against {floor:?}"
    );
    assert!(
        (ball[1] - floor[1]).abs() < 15.0,
        "the ball moving across the screen shows motion up or down: {ball:?} against {floor:?}"
    );

    // Still from here on: the very next frame must show no motion.
    stage.frames(&gpu, &camera, &environment, 1);
    let (ball, floor) = colours(&stage.pixels(&gpu), x);
    assert!(
        (ball[0] - floor[0]).abs() < 10.0,
        "the ball still shows motion after it stopped: {ball:?} against {floor:?}"
    );

    // A jump shows no motion at all, even a long one.
    x += 2.0;
    stage.renderer.place_instance(0, place(x));
    stage.frames(&gpu, &camera, &environment, 1);
    let (ball, floor) = colours(&stage.pixels(&gpu), x);
    assert!(
        (ball[0] - floor[0]).abs() < 10.0,
        "the ball that jumped shows motion: {ball:?} against {floor:?}"
    );
}

#[test]
fn a_hidden_ball_is_not_drawn_and_casts_no_shadow() {
    let Some(gpu) = request_gpu() else { return };
    let probe = ShadowProbe::new(1.0, 128);
    let sun = low_sun();
    let mut stage = Stage::new(&gpu, &red_ball_on_floor(), untemporal(), probe.size);
    stage.frames(&gpu, &probe.camera, &sun, 6);
    // Hidden while static, then while movable, after the movable layer has drawn it.
    for movable in [false, true] {
        assert!(stage.renderer.set_instance_movable(0, movable));
        stage.frames(&gpu, &probe.camera, &sun, 3);
        assert!(
            probe.shade(&stage.pixels(&gpu), 0.0) < 0.6,
            "no shadow, movable {movable}"
        );
        assert!(stage.renderer.set_instance_hidden(0, true));
        stage.frames(&gpu, &probe.camera, &sun, 2);
        let pixels = stage.pixels(&gpu);
        assert!(
            !probe.red(&pixels, 0.0),
            "the hidden ball is drawn, movable {movable}"
        );
        let shade = probe.shade(&pixels, 0.0);
        assert!(
            shade > 0.85,
            "the hidden ball casts a shadow, movable {movable}: {shade}"
        );
        assert!(stage.renderer.set_instance_hidden(0, false));
    }
}

#[test]
fn a_lamp_put_out_stops_glowing() {
    let Some(gpu) = request_gpu() else { return };
    let size = 96;
    let mut scene = red_ball_on_floor();
    // Far brighter than the sunlit floor.
    scene.materials[1].emissive = [60.0, 45.0, 15.0];
    let camera = over_the_floor();
    let sun = low_sun();
    let mut stage = Stage::new(&gpu, &scene, RendererSettings::default(), size);
    let (bx, by) = pixel_of(&camera, size, Vec3::new(0.0, 1.0, 0.0));
    let green_at = |pixels: &[u8]| patch(pixels, size, bx, by, 1)[1];
    stage.frames(&gpu, &camera, &sun, 20);
    let lit = green_at(&stage.pixels(&gpu));
    assert!(stage.renderer.set_instance_glow(0, 0.0));
    stage.frames(&gpu, &camera, &sun, 20);
    let out = green_at(&stage.pixels(&gpu));
    // Unlit, the red ball has little green; glowing, the lamp's yellow light has plenty.
    assert!(lit > 150.0, "the lamp doesn't glow: {lit}");
    assert!(
        out < lit * 0.5,
        "the lamp put out still glows: {out} against {lit}"
    );
}

#[test]
fn instances_without_geometry_are_refused() {
    let Some(gpu) = request_gpu() else { return };
    let mut scene = red_ball_on_floor();
    // A third instance of a mesh with nothing in it.
    scene.meshes.push(build_virtual_mesh(
        &Mesh::default(),
        &BuildOptions::default(),
    ));
    scene.instances.push(instance(2, Mat4::IDENTITY));
    let renderer = &mut Stage::new(&gpu, &scene, RendererSettings::default(), 16).renderer;
    assert_eq!(renderer.instance_count(), 3);
    assert!(!renderer.move_instance(2, Mat4::IDENTITY));
    assert!(!renderer.move_instance(3, Mat4::IDENTITY));
    assert!(renderer.move_instance(1, Mat4::IDENTITY));
    // The ball's mesh is bounded by its clusters' spheres, so its bounds are near the unit
    // sphere around (0, 1, 0), and hold it.
    let (center, radius) = renderer.instance_bounds(0).unwrap();
    assert!(center.distance(Vec3::Y) < 0.3 && radius >= 1.0 - center.distance(Vec3::Y));
}

#[test]
fn a_movable_ball_is_in_the_gi_voxels_only_if_it_stays_in_them() {
    let Some(gpu) = request_gpu() else { return };
    let size = 96;
    let camera = over_the_floor();
    let settings = RendererSettings {
        debug_view: DebugView::Voxels,
        ..Default::default()
    };
    let (bx, by) = pixel_of(&camera, size, Vec3::new(0.0, 1.0, 0.0));
    // How red the voxels look where the ball stands.
    let redness = |stage: &Stage| {
        let [r, g, _] = patch(&stage.pixels(&gpu), size, bx, by, 1);
        r / g.max(1.0)
    };
    let mut stage = Stage::new(&gpu, &red_ball_on_floor(), settings, size);
    stage.renderer.set_instance_movable(0, true);
    stage.renderer.set_instance_stays_in_gi(0, true);
    stage.frames(&gpu, &camera, &low_sun(), 12);
    let kept = redness(&stage);
    assert!(
        kept > 2.0,
        "the ball that stays in GI isn't in the voxels: {kept}"
    );

    // Hidden, it must leave the voxels too, once they have been rebuilt.
    stage.renderer.set_instance_hidden(0, true);
    stage.frames(&gpu, &camera, &low_sun(), 40);
    let hidden = redness(&stage);
    assert!(
        hidden < 1.3,
        "the hidden ball is still in the voxels: {hidden}"
    );

    // Shown again, but left out of GI.
    stage.renderer.set_instance_hidden(0, false);
    stage.renderer.set_instance_stays_in_gi(0, false);
    stage.frames(&gpu, &camera, &low_sun(), 40);
    let left_out = redness(&stage);
    assert!(
        left_out < 1.3,
        "the movable ball is in the voxels: {left_out}"
    );
}

/// A wall of grey `albedo`, glowing as brightly, facing the camera 10 m in front of it along -Z.
fn wall(albedo: f32) -> SceneAsset {
    let mut wall = Mesh::default();
    for (x, y) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
        wall.vertices.push(Vertex {
            position: Vec3::new(x * 60.0, y * 60.0, -10.0),
            normal: Vec3::Z,
            uv: Vec2::new(x, y),
        });
    }
    wall.indices = vec![0, 1, 2, 0, 2, 3];
    SceneAsset {
        meshes: vec![build_virtual_mesh(&wall, &BuildOptions::default())],
        materials: vec![Material {
            base_color: [albedo, albedo, albedo, 1.0],
            // It glows as bright as it is pale, so views that light it show it in the dark.
            emissive: [albedo; 3],
            flags: Material::TWO_SIDED,
            ..Default::default()
        }],
        instances: vec![instance(0, Mat4::IDENTITY)],
        ..Default::default()
    }
}

/// A BC7 block of one colour: mode 6, with both ends the same.
fn solid_bc7([r, g, b, a]: [u8; 4]) -> [u8; 16] {
    let mut bits = 1u128 << 6;
    let mut at = 7;
    for channel in [r, r, g, g, b, b, a, a] {
        bits |= u128::from(channel >> 1) << at;
        at += 7;
    }
    bits |= 3 << at;
    bits.to_le_bytes()
}

/// A whole mip chain of a BC7 texture of one colour.
fn solid_chain(width: u32, height: u32, color: [u8; 4]) -> Vec<Vec<u8>> {
    (0..=width.max(height).ilog2())
        .map(|level| {
            let side = |edge: u32| (edge >> level).max(1).div_ceil(4) as usize;
            solid_bc7(color).repeat(side(width) * side(height))
        })
        .collect()
}

/// Renders a scene as [`render_in`] does, from what is to be drawn as it would be on disk, in the
/// dark, without the temporal filter. Fails on any validation error.
fn render_loaded(gpu: &Gpu, loaded: &LoadedScene, camera: &Camera, size: u32) -> Vec<u8> {
    let errors: Arc<Mutex<Vec<String>>> = Default::default();
    let sink = errors.clone();
    gpu.device
        .on_uncaptured_error(Arc::new(move |error: wgpu::Error| {
            sink.lock().unwrap().push(error.to_string());
        }));
    let mut renderer = Renderer::new(gpu, loaded, FORMAT, untemporal());
    let target = test_target(gpu, size);
    let view = target.create_view(&Default::default());
    for _ in 0..8 {
        renderer.render(
            gpu,
            camera,
            &Environment::default(),
            &view,
            (size, size),
            1.0 / 60.0,
        );
        gpu.device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
    }
    let pixels = read_pixels(gpu, &target, size);
    let errors = errors.lock().unwrap();
    assert!(errors.is_empty(), "GPU errors: {}", errors.join("\n"));
    pixels
}

fn has_block_textures(gpu: &Gpu) -> bool {
    gpu.device
        .features()
        .contains(wgpu::Features::TEXTURE_COMPRESSION_BC)
}

/// A square of `half` metres around (`x`, `y`, `z`), facing the camera, as six corners.
fn card_square(x: f32, y: f32, z: f32, half: f32) -> [CardVertex; 6] {
    let corner = |sx: f32, sy: f32| CardVertex {
        position: [x + sx * half, y + sy * half, z],
        uv: [sx * 0.5 + 0.5, 0.5 - sy * 0.5],
    };
    [
        corner(-1.0, -1.0),
        corner(1.0, -1.0),
        corner(1.0, 1.0),
        corner(-1.0, -1.0),
        corner(1.0, 1.0),
        corner(-1.0, 1.0),
    ]
}

/// Cards, one per entry: where it is, and which of the images it shows.
fn cards_of(places: &[(f32, f32, f32, u32)], images: Vec<CardImage>) -> Cards {
    let mut cards = Cards {
        images,
        ..Default::default()
    };
    for &(x, y, z, image) in places {
        let first_vertex = cards.vertices.len() as u32;
        cards.vertices.extend(card_square(x, y, z, 1.0));
        cards.cards.push(Card {
            first_vertex,
            vertex_count: 6,
            image,
            opacity: 1.0,
            emissive: [200.0; 3],
        });
    }
    cards
}

fn image_of(alpha: u8, glow: u8) -> CardImage {
    CardImage {
        width: 8,
        height: 64,
        color_mips: solid_chain(8, 64, [255, 255, 255, alpha]),
        glow_mips: solid_chain(8, 64, [glow, glow, glow, 255]),
    }
}

/// A card adds its glow, is hidden by what is nearer, and covers what is behind it by its alpha.
#[test]
fn cards_glow_cover_and_go_behind_what_is_nearer() {
    let Some(gpu) = request_gpu() else {
        return;
    };
    if !has_block_textures(&gpu) {
        return;
    }
    let size = 64;
    let camera = Camera {
        position: Vec3::ZERO,
        ..Default::default()
    };
    // The wall, 10 m off, glows enough to see it covered.
    let mut scene = wall(0.0);
    scene.materials[0].emissive = [5.0; 3];
    let mut loaded = paginate(&scene);
    let at = |x: f32, z: f32| pixel_of(&camera, size, Vec3::new(x, 0.0, z));
    let glowing = (-2.5, -5.0, 1);
    let behind = (2.5, -20.0, 1);
    let covering = (0.0, -5.0, 2);
    loaded.scene.cards = cards_of(
        &[
            (glowing.0, 0.0, glowing.1, 0),
            (behind.0 * 4.0, 0.0, behind.1, 0),
            (covering.0, 0.0, covering.1, 1),
        ],
        vec![image_of(0, 255), image_of(255, 0)],
    );
    let pixels = render_loaded(&gpu, &loaded, &camera, size);
    let at_pixels = |(x, y): (u32, u32)| brightness(patch(&pixels, size, x, y, 0));
    // Exposure follows the whole picture, so each is compared with the wall beside it.
    let wall_beside = at_pixels(pixel_of(&camera, size, Vec3::new(0.0, -5.0, -10.0)));
    assert!(wall_beside > 10.0, "the wall shows: {wall_beside}");

    let glow = at_pixels(at(glowing.0, glowing.1));
    assert!(
        glow > wall_beside + 20.0,
        "the glow {glow} is added to {wall_beside}"
    );
    let hidden = at_pixels(at(behind.0 * 4.0, behind.1));
    assert!(
        (hidden - wall_beside).abs() < 5.0,
        "the wall hides what is behind it: {hidden} against {wall_beside}"
    );
    let covered = at_pixels(at(covering.0, covering.1));
    assert!(
        covered < wall_beside * 0.5,
        "a solid card with no glow hides the wall: {covered} against {wall_beside}"
    );
}

/// An emissive map decides where a material glows, and two materials can have different ones.
#[test]
fn emissive_maps_decide_where_a_material_glows() {
    let Some(gpu) = request_gpu() else {
        return;
    };
    if !has_block_textures(&gpu) {
        return;
    }
    let size = 64;
    let camera = Camera {
        position: Vec3::ZERO,
        ..Default::default()
    };
    let mut scene = wall(0.0);
    let glowing = |layer: u32| Material {
        base_color: [0.0, 0.0, 0.0, 1.0],
        emissive: [Material::GLOW_MAP_AVERAGE * 100.0; 3],
        flags: Material::TWO_SIDED | (layer + 1) << Material::GLOW_SHIFT,
        ..Default::default()
    };
    scene.materials = vec![glowing(0), glowing(1)];
    scene.instances = vec![
        Instance {
            mesh: 0,
            material: 0,
            transform: Mat4::from_translation(Vec3::new(-60.0, 0.0, 0.0)),
        },
        Instance {
            mesh: 0,
            material: 1,
            transform: Mat4::from_translation(Vec3::new(60.0, 0.0, 0.0)),
        },
    ];
    let mut loaded = paginate(&scene);
    loaded.scene.glow = GlowMaps {
        edge: 16,
        layers: vec![
            solid_chain(16, 16, [255, 255, 255, 255]),
            solid_chain(16, 16, [0, 0, 0, 255]),
        ],
    };
    let pixels = render_loaded(&gpu, &loaded, &camera, size);
    let left = brightness(patch(&pixels, size, size / 4, size / 2, 1));
    let right = brightness(patch(&pixels, size, size * 3 / 4, size / 2, 1));
    // Exposure follows the whole picture, so what counts is the difference.
    assert!(
        left > right + 40.0,
        "the white map {left} glows more than the black {right}"
    );
}

/// `count` still white particles of radius 3 m in one place, fading over `soft` metres: large
/// enough on screen that the resolve filter does not soften their middles.
fn still_particles(count: u32, soft: f32) -> Effect {
    Effect::parse(&format!(
        "emitter e\n budget {count}\n spawn burst={count}\n lifetime 1000\n size 3\n color 1 1 1 1\n render sprite soft={soft} shading=unlit"
    ))
    .unwrap()
}

/// Renders a wall of `albedo` with the particles of `effect`, if any, at `at`, in a debug view
/// that shows the scene's colour as it is, with no exposure or history, and reads back the image.
#[allow(clippy::too_many_arguments)]
fn render_particles(
    gpu: &Gpu,
    albedo: f32,
    effect: Option<&Effect>,
    at: Vec3,
    debug: DebugView,
    sea_level: Option<f32>,
    size: u32,
) -> Vec<u8> {
    let errors: Arc<Mutex<Vec<String>>> = Default::default();
    let sink = errors.clone();
    gpu.device
        .on_uncaptured_error(Arc::new(move |error: wgpu::Error| {
            sink.lock().unwrap().push(error.to_string());
        }));
    let settings = RendererSettings {
        debug_view: debug,
        temporal_accumulation: false,
        ..Default::default()
    };
    let mut renderer = Renderer::new(gpu, &paginate(&wall(albedo)), FORMAT, settings);
    if let Some(level) = sea_level {
        renderer.set_ocean(Some(Ocean::by_name("coastal").unwrap()), 0.0);
        renderer.set_sea_level(level);
    }
    if let Some(effect) = effect {
        renderer
            .particles()
            .spawn(effect, Mat4::from_translation(at))
            .expect("room for the effect");
    }
    let camera = Camera {
        position: Vec3::ZERO,
        ..Default::default()
    };
    let target = test_target(gpu, size);
    let view = target.create_view(&Default::default());
    for _ in 0..4 {
        renderer.render(gpu, &camera, &darkness(), &view, (size, size), 1.0 / 60.0);
        gpu.device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
    }
    let pixels = read_pixels(gpu, &target, size);
    let errors = errors.lock().unwrap();
    assert!(errors.is_empty(), "GPU errors: {}", errors.join("\n"));
    pixels
}

fn srgb_to_linear(byte: f32) -> f32 {
    let c = byte / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// A particle fades by how far it stands in front of the surface behind it, over its soft
/// distance: nearly gone just in front of the wall, a quarter at a quarter of the distance, whole
/// beyond it, and hidden behind it.
#[test]
fn particles_fade_softly_into_the_surface_behind_them() {
    let Some(gpu) = request_gpu() else {
        return;
    };
    let size = 64;
    let centre = |z: f32, soft: f32| {
        let pixels = render_particles(
            &gpu,
            0.0,
            Some(&still_particles(1, soft)),
            Vec3::new(0.0, 0.0, z),
            DebugView::Albedo,
            None,
            size,
        );
        srgb_to_linear(patch(&pixels, size, size / 2, size / 2, 0)[0])
    };
    let (touching, quarter, clear) = (centre(-9.97, 1.0), centre(-9.75, 1.0), centre(-8.0, 1.0));
    assert!(touching < 0.06, "touching the wall: {touching}");
    assert!(
        (quarter - 0.25).abs() < 0.05,
        "a quarter of the way out: {quarter}"
    );
    assert!(clear > 0.95, "well clear of it: {clear}");
    // Without a fade, just in front of the wall it shows whole, and behind it the depth test
    // alone hides it.
    assert!(centre(-9.97, 0.0) > 0.95);
    assert_eq!(centre(-10.5, 0.0), 0.0, "behind the wall");
}

/// The overdraw view counts layers of particles: more layers, hotter.
#[test]
fn the_overdraw_view_counts_layers() {
    let Some(gpu) = request_gpu() else {
        return;
    };
    let size = 64;
    let red = |count| {
        let pixels = render_particles(
            &gpu,
            0.0,
            Some(&still_particles(count, 0.0)),
            Vec3::new(0.0, 0.0, -9.0),
            DebugView::Particles,
            None,
            size,
        );
        // Away from the sprite there is nothing at all.
        assert_eq!(brightness(patch(&pixels, size, 2, 2, 1)), 0.0);
        srgb_to_linear(patch(&pixels, size, size / 2, size / 2, 0)[0])
    };
    let (one, two) = (red(1), red(2));
    assert!((one - 0.25).abs() < 0.02, "{one}");
    assert!((two - 0.5).abs() < 0.03, "{two}");
}

/// Over the sea, particles are drawn into the colour post reads, the water's, so they show.
#[test]
fn particles_show_over_the_sea() {
    let Some(gpu) = request_gpu() else {
        return;
    };
    let size = 64;
    let pixels = render_particles(
        &gpu,
        0.0,
        Some(&still_particles(1, 0.0)),
        Vec3::new(0.0, 0.0, -5.0),
        DebugView::Particles,
        Some(-1.0),
        size,
    );
    let one = srgb_to_linear(patch(&pixels, size, size / 2, size / 2, 0)[0]);
    assert!((one - 0.25).abs() < 0.02, "{one}");
}

/// The fountain runs at every quality, in the lit view, with no GPU errors, and its particles
/// are counted, fewer on lower qualities.
#[test]
fn the_fountain_runs_at_every_quality() {
    let Some(gpu) = request_gpu() else {
        return;
    };
    let mut alive = Vec::new();
    for quality in [Quality::Low, Quality::Medium, Quality::High] {
        let errors: Arc<Mutex<Vec<String>>> = Default::default();
        let sink = errors.clone();
        gpu.device
            .on_uncaptured_error(Arc::new(move |error: wgpu::Error| {
                sink.lock().unwrap().push(error.to_string());
            }));
        let mut settings = RendererSettings::default();
        settings.apply_quality(quality);
        let mut renderer = Renderer::new(&gpu, &paginate(&ball_on_floor()), FORMAT, settings);
        let fountain = renderer
            .particles()
            .spawn(
                &Effect::by_name("fountain").unwrap(),
                Mat4::from_translation(Vec3::new(0.0, 0.0, -4.0)),
            )
            .unwrap();
        assert!(renderer.particles().set(fountain, "strength", 0.5));
        let target = test_target(&gpu, 64);
        let view = target.create_view(&Default::default());
        let camera = Camera {
            position: Vec3::new(0.0, 1.5, 4.0),
            ..Default::default()
        };
        for _ in 0..40 {
            renderer.render(
                &gpu,
                &camera,
                &Environment::default(),
                &view,
                (64, 64),
                1.0 / 60.0,
            );
            gpu.device
                .poll(wgpu::PollType::wait_indefinitely())
                .unwrap();
        }
        let stats = renderer.stats().particles;
        assert_eq!(stats.emitters, 2);
        assert!(stats.alive > 50, "{quality:?}: {stats:?}");
        assert_eq!(stats.dropped, 0);
        alive.push(stats.alive);
        let errors = errors.lock().unwrap();
        assert!(errors.is_empty(), "{quality:?}: {}", errors.join("\n"));
    }
    assert!(alive[0] < alive[1] && alive[1] < alive[2], "{alive:?}");
}

/// Additive particles add their light and hide nothing; alpha ones cover what is behind.
#[test]
fn additive_particles_add_light_and_alpha_particles_cover() {
    let Some(gpu) = request_gpu() else {
        return;
    };
    let size = 64;
    let centre = |blend: &str, z: f32| {
        let effect = Effect::parse(&format!(
            "emitter e
 budget 1
 spawn burst=1
 lifetime 1000
 size 3
 color 0.25 0.25 0.25 1
 render sprite soft=0 blend={blend} shading=unlit"
        ))
        .unwrap();
        let pixels = render_particles(
            &gpu,
            0.5,
            Some(&effect),
            Vec3::new(0.0, 0.0, z),
            DebugView::Albedo,
            None,
            size,
        );
        srgb_to_linear(patch(&pixels, size, size / 2, size / 2, 0)[0])
    };
    // Behind the wall, the wall alone.
    let wall = centre("alpha", -11.0);
    let covered = centre("alpha", -8.0);
    let added = centre("add", -8.0);
    assert!(wall > 0.3, "{wall}");
    assert!((covered - 0.25).abs() < 0.03, "covered: {covered}");
    assert!(
        (added - (wall + 0.25)).abs() < 0.04,
        "added: {added} over {wall}"
    );
}

/// With nothing running, the overdraw view is black rather than the scene.
#[test]
fn the_overdraw_view_is_black_with_no_particles() {
    let Some(gpu) = request_gpu() else {
        return;
    };
    let size = 64;
    let pixels = render_particles(
        &gpu,
        0.8,
        None,
        Vec3::ZERO,
        DebugView::Particles,
        None,
        size,
    );
    assert_eq!(brightness(patch(&pixels, size, size / 2, size / 2, 4)), 0.0);
}

/// One still particle of radius `size`, lit, scattering by `phase`, from a single burst.
fn lit_particle(size: f32, phase: f32) -> Effect {
    Effect::parse(&format!(
        "emitter e\n budget 1\n spawn burst=1\n lifetime 1000\n size {size}\n color 1 1 1 1\n render sprite soft=0 phase={phase}"
    ))
    .unwrap()
}

/// A stage whose settings show what tests of light need: no history, and a fixed view.
fn light_stage(gpu: &Gpu, scene: &SceneAsset, debug: DebugView, size: u32) -> Stage {
    let settings = RendererSettings {
        debug_view: debug,
        temporal_accumulation: false,
        ..Default::default()
    };
    Stage::new(gpu, scene, settings, size)
}

/// Linear brightness of the pixel where `world` lands.
fn linear_at(pixels: &[u8], camera: &Camera, size: u32, world: Vec3) -> [f32; 3] {
    let (x, y) = pixel_of(camera, size, world);
    patch(pixels, size, x, y, 0).map(srgb_to_linear)
}

/// A particle in the ball's shadow takes less light than one in the sun, by about the sun's
/// share.
#[test]
fn a_particle_in_shadow_is_darker_than_one_in_the_sun() {
    let Some(gpu) = request_gpu() else {
        return;
    };
    let size = 128;
    let camera = over_the_floor();
    let effect = lit_particle(0.35, 0.0);
    // The sun is low over +X, so the ball shades the air on its -X side.
    let shaded = Vec3::new(-1.2, 0.2, 0.0);
    let sunlit = Vec3::new(2.5, 0.2, 0.0);
    // The light alone, and the final image, where the same light lights the particle's colour.
    for (view, most) in [(DebugView::ParticleLight, 0.5), (DebugView::Lit, 0.7)] {
        let mut stage = light_stage(&gpu, &ball_on_floor(), view, size);
        for at in [shaded, sunlit] {
            stage
                .renderer
                .particles()
                .spawn(&effect, Mat4::from_translation(at))
                .unwrap();
        }
        // A dim sun keeps the light view's values below 1, where it would clip.
        let environment = Environment {
            sun_illuminance: 1.0,
            ..low_sun()
        };
        stage.frames(&gpu, &camera, &environment, 12);
        let pixels = stage.pixels(&gpu);
        let dark = brightness(linear_at(&pixels, &camera, size, shaded));
        let lit = brightness(linear_at(&pixels, &camera, size, sunlit));
        assert!(lit > 0.1, "{view:?}: the sunlit particle: {lit}");
        assert!(
            dark < lit * most,
            "{view:?}: shaded {dark} against sunlit {lit}"
        );
        assert!(
            dark > 0.0,
            "{view:?}: the shaded particle still has the sky: {dark}"
        );
    }

    // With GI turned off, as surfaces lose their indirect light, the shaded particle loses
    // all it had: in shadow, nothing is left.
    let mut stage = light_stage(&gpu, &ball_on_floor(), DebugView::ParticleLight, size);
    stage
        .renderer
        .particles()
        .spawn(&effect, Mat4::from_translation(shaded))
        .unwrap();
    let no_gi = Environment {
        sun_illuminance: 1.0,
        gi_intensity: 0.0,
        ..low_sun()
    };
    stage.frames(&gpu, &camera, &no_gi, 12);
    let left = brightness(linear_at(&stage.pixels(&gpu), &camera, size, shaded));
    assert!(left < 0.005, "without GI, the shaded particle keeps {left}");
}

/// When the sun moves, a particle's light follows within a few frames at every quality, however
/// seldom each particle's light is worked out afresh there.
#[test]
fn particle_light_follows_a_moving_sun_at_every_quality() {
    let Some(gpu) = request_gpu() else {
        return;
    };
    let size = 128;
    let camera = over_the_floor();
    let at = Vec3::new(-1.2, 0.2, 0.0);
    for quality in [Quality::Low, Quality::Medium, Quality::High] {
        let mut settings = RendererSettings {
            debug_view: DebugView::ParticleLight,
            temporal_accumulation: false,
            ..Default::default()
        };
        settings.apply_quality(quality);
        settings.render_scale = 1.0;
        let mut stage = Stage::new(&gpu, &ball_on_floor(), settings, size);
        stage
            .renderer
            .particles()
            .spawn(&lit_particle(0.35, 0.0), Mat4::from_translation(at))
            .unwrap();
        // First the ball shades it from a sun over +X; then the sun swings round to -X.
        let shaded = Environment {
            sun_illuminance: 1.0,
            ..low_sun()
        };
        let open = Environment {
            sun_direction: Vec3::new(-1.0, 1.0, 0.0).normalize(),
            ..shaded
        };
        stage.frames(&gpu, &camera, &shaded, 12);
        let before = brightness(linear_at(&stage.pixels(&gpu), &camera, size, at));
        stage.frames(&gpu, &camera, &open, 8);
        let after = brightness(linear_at(&stage.pixels(&gpu), &camera, size, at));
        assert!(after > before * 2.0, "{quality:?}: {before} then {after}");
    }
}

/// With the sun behind it, a particle that scatters onward glows far brighter than with the sun
/// behind the eye.
#[test]
fn onward_scattering_particles_glow_against_the_sun() {
    let Some(gpu) = request_gpu() else {
        return;
    };
    let size = 64;
    let camera = Camera {
        position: Vec3::new(0.0, 2.0, 0.0),
        ..Default::default()
    };
    let at = Vec3::new(0.0, 2.0, -6.0);
    let seen = |sun: Vec3| {
        let mut stage = light_stage(&gpu, &ball_on_floor(), DebugView::ParticleLight, size);
        stage
            .renderer
            .particles()
            .spawn(&lit_particle(0.8, 0.8), Mat4::from_translation(at))
            .unwrap();
        let environment = Environment {
            sun_direction: sun.normalize(),
            sun_illuminance: 0.02,
            ..Default::default()
        };
        stage.frames(&gpu, &camera, &environment, 6);
        brightness(linear_at(&stage.pixels(&gpu), &camera, size, at))
    };
    // The camera looks along -Z: a sun low to -Z is ahead, beyond the particle.
    let ahead = seen(Vec3::new(0.0, 0.3, -1.0));
    let behind = seen(Vec3::new(0.0, 0.3, 1.0));
    assert!(ahead > behind * 4.0, "ahead {ahead}, behind {behind}");
}

/// Far off, a particle fades into the haze exactly as a surface beside it does.
#[test]
fn distant_particles_fade_into_the_haze_as_surfaces_do() {
    let Some(gpu) = request_gpu() else {
        return;
    };
    let size = 64;
    // A glowing wall 600 m off, and a particle 5 m in front of it, glowing the same.
    let mut wall = Mesh::default();
    for (x, y) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
        wall.vertices.push(Vertex {
            position: Vec3::new(x * 800.0, y * 800.0, -600.0),
            normal: Vec3::Z,
            uv: Vec2::new(x, y),
        });
    }
    wall.indices = vec![0, 1, 2, 0, 2, 3];
    let scene = SceneAsset {
        meshes: vec![build_virtual_mesh(&wall, &BuildOptions::default())],
        materials: vec![Material {
            base_color: [0.0, 0.0, 0.0, 1.0],
            emissive: [2.0; 3],
            flags: Material::TWO_SIDED,
            ..Default::default()
        }],
        instances: vec![instance(0, Mat4::IDENTITY)],
        ..Default::default()
    };
    let camera = Camera {
        position: Vec3::ZERO,
        ..Default::default()
    };
    let mut stage = light_stage(&gpu, &scene, DebugView::Lit, size);
    let effect = Effect::parse(
        "emitter e\n budget 1\n spawn burst=1\n lifetime 1000\n size 60\n color 2 2 2 1\n render sprite soft=0 shading=unlit",
    )
    .unwrap();
    let at = Vec3::new(0.0, 0.0, -595.0);
    stage
        .renderer
        .particles()
        .spawn(&effect, Mat4::from_translation(at))
        .unwrap();
    // To its right, a black additive particle: it adds no light, and so must add no haze
    // either, as the haze over the wall behind it is already there.
    let black = Effect::parse(
        "emitter e\n budget 1\n spawn burst=1\n lifetime 1000\n size 60\n color 0 0 0 1\n render sprite soft=0 shading=unlit blend=add",
    )
    .unwrap();
    let right = Vec3::new(200.0, 0.0, -595.0);
    stage
        .renderer
        .particles()
        .spawn(&black, Mat4::from_translation(right))
        .unwrap();
    stage.frames(&gpu, &camera, &Environment::default(), 8);
    let pixels = stage.pixels(&gpu);
    let particle = brightness(patch(&pixels, size, size / 2, size / 2, 0));
    let surface = brightness(patch(&pixels, size, 6, size / 2, 0));
    let (rx, ry) = pixel_of(&camera, size, right);
    let added = brightness(patch(&pixels, size, rx, ry, 0));
    assert!(surface > 10.0, "the wall shows: {surface}");
    // Within a few levels: the two are seen in slightly different directions through the air.
    assert!(
        (particle - surface).abs() <= 5.0,
        "particle {particle} against the wall beside it {surface}"
    );
    assert!(
        (added - surface).abs() <= 5.0,
        "a black additive particle {added} against the wall {surface}"
    );
}

/// An effect started in the slice of one that has just ended starts with no light of its own:
/// none of the old effect's glow shows on its particles, even at Low, where particles are lit
/// only every few frames.
#[test]
fn a_reused_slice_keeps_no_light_from_its_last_effect() {
    let Some(gpu) = request_gpu() else {
        return;
    };
    let size = 64;
    let camera = Camera {
        position: Vec3::ZERO,
        ..Default::default()
    };
    let mut settings = RendererSettings {
        temporal_accumulation: false,
        ..Default::default()
    };
    settings.apply_quality(Quality::Low);
    settings.render_scale = 1.0;
    let mut stage = Stage::new(&gpu, &wall(0.0), settings, size);
    let cloud = |temperature: f32| {
        Effect::parse(&format!(
            "emitter e\n budget 64\n spawn burst=64\n lifetime 1000\n shape sphere 0.3\n size 0.5\n color 0 0 0 1\n temperature {temperature}\n render sprite soft=0 shading=unlit"
        ))
        .unwrap()
    };
    let at = Mat4::from_translation(Vec3::new(0.0, 0.0, -6.0));
    let night = Environment {
        sun_illuminance: 0.0,
        sky_intensity: 0.0,
        daylight_illuminance: 10.0,
        ..Default::default()
    };
    let hot = stage
        .renderer
        .particles()
        .spawn(&cloud(2400.0), at)
        .unwrap();
    stage.frames(&gpu, &camera, &night, 10);
    let glowing = brightness(patch(&stage.pixels(&gpu), size, size / 2, size / 2, 1));
    assert!(glowing > 100.0, "the hot cloud glows: {glowing}");
    stage.renderer.particles().kill(hot);
    stage.renderer.particles().spawn(&cloud(0.0), at).unwrap();
    for frame in 0..4 {
        stage.frames(&gpu, &camera, &night, 1);
        let seen = brightness(patch(&stage.pixels(&gpu), size, size / 2, size / 2, 1));
        assert!(seen < 3.0, "frame {frame} of the cold cloud glows: {seen}");
    }
}

/// While an emitter keeps spawning, and so its particles' places in its list keep moving, the
/// Medium and Low qualities, which light each particle only every few frames, still bring all
/// of a cloud's particles into a newly risen sun within a few frames, as High does.
#[test]
fn particles_relit_in_turns_are_never_left_out() {
    let Some(gpu) = request_gpu() else {
        return;
    };
    let size = 128;
    let camera = over_the_floor();
    // A burst in the ball's shadow, then a steady stream of 64 a frame, which shifts every
    // particle's place in the list by a workgroup each frame.
    let effect = Effect::parse(
        "emitter e\n budget 6000\n spawn burst=1500 rate=3840\n lifetime 1000\n shape box 0.6 0.6 0.6\n offset -1.4 0.4 0\n size 0.05\n color 1 1 1 1\n render sprite soft=0 phase=0",
    )
    .unwrap();
    let mut seen = Vec::new();
    for quality in [Quality::High, Quality::Medium, Quality::Low] {
        let mut settings = RendererSettings {
            debug_view: DebugView::ParticleLight,
            temporal_accumulation: false,
            ..Default::default()
        };
        settings.apply_quality(quality);
        settings.render_scale = 1.0;
        settings.particle_detail = 1.0;
        let mut stage = Stage::new(&gpu, &ball_on_floor(), settings, size);
        stage
            .renderer
            .particles()
            .spawn(&effect, Mat4::IDENTITY)
            .unwrap();
        let shaded = Environment {
            sun_illuminance: 1.0,
            ..low_sun()
        };
        let open = Environment {
            sun_direction: Vec3::new(-1.0, 1.0, 0.0).normalize(),
            ..shaded
        };
        stage.frames(&gpu, &camera, &shaded, 6);
        // Six frames: every particle is due by then at Low, where one in four is lit a frame;
        // left to chance, about a sixth would not be.
        stage.frames(&gpu, &camera, &open, 6);
        let (x, y) = pixel_of(&camera, size, Vec3::new(-1.4, 0.4, 0.0));
        seen.push(brightness(
            patch(&stage.pixels(&gpu), size, x, y, 3).map(srgb_to_linear),
        ));
    }
    let [high, medium, low] = seen[..] else {
        unreachable!()
    };
    assert!(high > 0.05, "{seen:?}");
    assert!(medium > high * 0.98, "Medium against High: {seen:?}");
    assert!(low > high * 0.98, "Low against High: {seen:?}");
}

/// Beside a sunlit red ball, a particle's light turns red, from light the GI volume has bounced
/// off the ball.
#[test]
fn particles_take_the_colour_of_light_bounced_off_the_scene() {
    let Some(gpu) = request_gpu() else {
        return;
    };
    let size = 128;
    let camera = over_the_floor();
    // Just off the ball's sunlit side.
    let at = Vec3::new(1.7, 1.0, 0.0);
    let redness = |scene: &SceneAsset| {
        let mut stage = light_stage(&gpu, scene, DebugView::ParticleLight, size);
        stage
            .renderer
            .particles()
            .spawn(&lit_particle(0.6, 0.0), Mat4::from_translation(at))
            .unwrap();
        let environment = Environment {
            sun_illuminance: 0.5,
            ..low_sun()
        };
        stage.frames(&gpu, &camera, &environment, 40);
        let [r, g, _] = linear_at(&stage.pixels(&gpu), &camera, size, at);
        r / g.max(1.0e-4)
    };
    let white = redness(&ball_on_floor());
    let red = redness(&red_ball_on_floor());
    // The ball fills about a quarter of the particle's surroundings, and the sun lights the
    // particle directly too, so the bounce adds a few percent of red, not more.
    assert!(red > white * 1.03, "red ball {red}, white ball {white}");
}

/// A hot particle glows by its temperature, orange as a black body at 2,400 K is, and its
/// light spreads into bloom around it.
#[test]
fn hot_particles_glow_as_black_bodies_and_bloom() {
    let Some(gpu) = request_gpu() else {
        return;
    };
    let size = 96;
    let camera = Camera {
        position: Vec3::ZERO,
        ..Default::default()
    };
    let glow = |temperature: f32| {
        let mut stage = light_stage(&gpu, &wall(0.0), DebugView::Lit, size);
        let effect = Effect::parse(&format!(
            "emitter e\n budget 1\n spawn burst=1\n lifetime 1000\n size 0.6\n color 0 0 0 1\n temperature {temperature}\n render sprite soft=0 shading=unlit"
        ))
        .unwrap();
        stage
            .renderer
            .particles()
            .spawn(&effect, Mat4::from_translation(Vec3::new(0.0, 0.0, -8.0)))
            .unwrap();
        // Night: no sun, but exposure still knows what daylight is.
        let environment = Environment {
            sun_illuminance: 0.0,
            sky_intensity: 0.0,
            daylight_illuminance: 10.0,
            ..Default::default()
        };
        stage.frames(&gpu, &camera, &environment, 30);
        stage.pixels(&gpu)
    };
    let hot = glow(2400.0);
    let cold = glow(0.0);
    // Its core clips to white, as fire does on camera; its halo keeps its colour.
    assert!(brightness(patch(&hot, size, size / 2, size / 2, 1)) > 200.0);
    let [r, g, b] = patch(&hot, size, size / 2 + 8, size / 2, 0);
    assert!(
        r > g && g > b,
        "a black body at 2,400 K is orange: {r} {g} {b}"
    );
    // Beyond the sprite, bloom; with no glow, nothing.
    let around = brightness(patch(&hot, size, size / 2 + 14, size / 2, 1));
    let dark = brightness(patch(&cold, size, size / 2 + 14, size / 2, 1));
    assert!(around > dark + 2.0, "bloom {around} against {dark}");
    // Without a temperature, nothing but the film grain.
    assert!(brightness(patch(&cold, size, size / 2, size / 2, 1)) < 3.0);
}

/// Measures what local lights add to particle lighting: a cloud of lit particles among `n`
/// lights, for n of 0, 16, 64 and 256, at High. Ignored by default; run with
/// `cargo test --release -p solstice-render --test gpu -- --ignored particle_light_cost
/// --nocapture`, and SOLSTICE_LOW_POWER=1 for the integrated GPU.
#[test]
#[ignore]
fn particle_light_cost_with_many_lights() {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::PRIMARY,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let power = if std::env::var("SOLSTICE_LOW_POWER").is_ok() {
        wgpu::PowerPreference::LowPower
    } else {
        wgpu::PowerPreference::HighPerformance
    };
    let Ok(gpu) = pollster::block_on(Gpu::request(&instance, None, power)) else {
        return;
    };
    eprintln!("{}", gpu.adapter_info.name);
    let effect = Effect::parse(
        "emitter cloud\n budget 40000\n spawn burst=40000\n lifetime 1000\n shape box 24 6 24\n offset 0 3 0\n size 0.05\n render sprite soft=0",
    )
    .unwrap();
    let camera = over_the_floor();
    for n in [0usize, 16, 64, 256] {
        let mut scene = ball_on_floor();
        scene.lights = (0..n)
            .map(|i| {
                let a = i as f32 * 2.399963;
                let r = 2.0 + 10.0 * ((i as f32 + 0.5) / n.max(1) as f32).sqrt();
                solstice_asset::Light {
                    position: [r * a.cos(), 1.5, r * a.sin()],
                    range: 8.0,
                    intensity: [2.0, 1.4, 0.8],
                    _pad: 0,
                }
            })
            .collect();
        let settings = RendererSettings {
            temporal_accumulation: false,
            ..Default::default()
        };
        let mut stage = Stage::new(&gpu, &scene, settings, 512);
        stage
            .renderer
            .particles()
            .spawn(&effect, Mat4::IDENTITY)
            .unwrap();
        let mut samples = Vec::new();
        for frame in 0..60 {
            stage.frames(&gpu, &camera, &low_sun(), 1);
            if frame >= 20 {
                if let Some(times) = stage.renderer.section_times() {
                    samples.push(times[solstice_render::profiler::section::PARTICLES]);
                }
            }
        }
        samples.sort_by(f32::total_cmp);
        let median = samples.get(samples.len() / 2).copied().unwrap_or(f32::NAN);
        eprintln!("{n:4} lights: particles section {median:.3} ms (40,000 lit particles)");
    }
}

/// Local lights light particles in their range; the ball stops one from lighting a particle
/// behind it; and lights beyond the strongest few, which are not traced, still count.
#[test]
fn local_lights_light_particles_and_the_scene_stops_them() {
    let Some(gpu) = request_gpu() else {
        return;
    };
    let size = 128;
    let camera = over_the_floor();
    let lamp = |x: f32, y: f32, z: f32, candela: f32| solstice_asset::Light {
        position: [x, y, z],
        range: 9.0,
        intensity: [candela; 3],
        _pad: 0,
    };
    let lit = |lights: Vec<solstice_asset::Light>, at: &[Vec3]| {
        let mut scene = ball_on_floor();
        scene.lights = lights;
        let mut stage = light_stage(&gpu, &scene, DebugView::ParticleLight, size);
        for &p in at {
            stage
                .renderer
                .particles()
                .spawn(&lit_particle(0.35, 0.0), Mat4::from_translation(p))
                .unwrap();
        }
        stage.frames(&gpu, &camera, &darkness(), 30);
        let pixels = stage.pixels(&gpu);
        at.iter()
            .map(|&p| brightness(linear_at(&pixels, &camera, size, p)))
            .collect::<Vec<_>>()
    };
    // A lamp at +X: one particle behind the ball from it, one as far off in the open.
    let behind = Vec3::new(-1.8, 1.0, 0.0);
    let open = Vec3::new(3.0, 1.0, -4.8);
    let seen = lit(vec![lamp(3.0, 1.0, 0.0, 6.0)], &[behind, open]);
    assert!(
        seen[1] > 0.02,
        "the lamp lights the open particle: {seen:?}"
    );
    assert!(seen[0] < seen[1] * 0.3, "the ball stops the lamp: {seen:?}");

    // Three lamps the same distance from a particle give half as much again as two: the
    // third, not traced, still counts.
    let at = Vec3::new(4.0, 1.0, 3.0);
    let around = |a: f32| lamp(at.x + 2.0 * a.cos(), 1.0, at.z + 2.0 * a.sin(), 1.5);
    let two = lit(vec![around(0.0), around(2.1)], &[at])[0];
    let three = lit(vec![around(0.0), around(2.1), around(4.2)], &[at])[0];
    assert!(two > 0.02, "{two}");
    assert!((three / two - 1.5).abs() < 0.15, "two {two}, three {three}");
}

/// Emitters are drawn far to near, whatever order they were started in: a nearer emitter's
/// smoke lies over a farther one's.
#[test]
fn nearer_emitters_draw_over_farther_ones() {
    let Some(gpu) = request_gpu() else {
        return;
    };
    let size = 64;
    let puff = |r: f32, g: f32, b: f32| {
        Effect::parse(&format!(
            "emitter e\n budget 1\n spawn burst=1\n lifetime 1000\n size 2\n color {r} {g} {b} 0.9\n render sprite soft=0 shading=unlit"
        ))
        .unwrap()
    };
    let camera = Camera {
        position: Vec3::ZERO,
        ..Default::default()
    };
    let centre = |first_near: bool| {
        let mut stage = light_stage(&gpu, &wall(0.0), DebugView::Albedo, size);
        let near = (puff(0.0, 0.0, 1.0), Vec3::new(0.0, 0.0, -4.0));
        let far = (puff(1.0, 0.0, 0.0), Vec3::new(0.0, 0.0, -7.0));
        let order = if first_near { [near, far] } else { [far, near] };
        for (effect, at) in &order {
            stage
                .renderer
                .particles()
                .spawn(effect, Mat4::from_translation(*at))
                .unwrap();
        }
        stage.frames(&gpu, &camera, &darkness(), 4);
        patch(&stage.pixels(&gpu), size, size / 2, size / 2, 0).map(srgb_to_linear)
    };
    for first_near in [true, false] {
        let [r, _, b] = centre(first_near);
        assert!(
            b > r * 4.0,
            "the near blue puff is on top: red {r}, blue {b}"
        );
    }
}

/// A small, faint particle moving fast across the view keeps its contrast under the temporal
/// filter, as it has without it. Its neighbourhood looks much like the wall behind it, so the
/// filter's colour clipping alone would keep the wall's history there and the particle would
/// fade into a smear; where particles cover, the filter trusts its history less.
#[test]
fn moving_translucent_particles_stay_bright_under_the_temporal_filter() {
    let Some(gpu) = request_gpu() else {
        return;
    };
    let size = 128;
    let camera = Camera {
        position: Vec3::ZERO,
        ..Default::default()
    };
    let effect = Effect::parse(
        "emitter e
 budget 1
 spawn burst=1
 lifetime 1000
 size 0.06
 velocity 4 0 0
 color 1 1 1 0.35
 render sprite soft=0 shading=unlit",
    )
    .unwrap();
    let frames = 12;
    let seconds = 1.0 / 60.0;
    let start = Vec3::new(-1.5, 0.0, -5.0);
    let end = start + Vec3::new(4.0 * seconds * (frames as f32 - 0.5), 0.0, 0.0);
    let contrast = |taa: bool| {
        let settings = RendererSettings {
            debug_view: DebugView::Lit,
            temporal_accumulation: taa,
            ..Default::default()
        };
        let mut stage = Stage::new(&gpu, &wall(0.2), settings, size);
        stage
            .renderer
            .particles()
            .spawn(&effect, Mat4::from_translation(start))
            .unwrap();
        stage.frames(&gpu, &camera, &Environment::default(), frames);
        let pixels = stage.pixels(&gpu);
        let (x, y) = pixel_of(&camera, size, end);
        let peak = (0..=2)
            .flat_map(|dy| (0..=2).map(move |dx| (x + dx - 1, y + dy - 1)))
            .map(|(x, y)| brightness(patch(&pixels, size, x, y, 0)))
            .fold(0.0, f32::max);
        peak - brightness(patch(&pixels, size, x, y + 12, 1))
    };
    let still = contrast(false);
    let filtered = contrast(true);
    eprintln!("contrast without the filter {still}, with it {filtered}");
    assert!(still > 8.0, "{still}");
    assert!(
        filtered > still * 0.7,
        "with the filter {filtered}, without {still}"
    );
}

/// A solid particle hides a translucent one behind it, as any surface would.
#[test]
fn solid_particles_hide_what_is_behind_them() {
    let Some(gpu) = request_gpu() else {
        return;
    };
    let size = 64;
    let camera = Camera {
        position: Vec3::ZERO,
        ..Default::default()
    };
    let mut stage = light_stage(&gpu, &wall(0.0), DebugView::Albedo, size);
    let solid = Effect::parse(
        "emitter e\n budget 1\n spawn burst=1\n lifetime 1000\n size 2\n color 1 0 0 1\n render sprite solid shading=unlit",
    )
    .unwrap();
    let behind = Effect::parse(
        "emitter e\n budget 1\n spawn burst=1\n lifetime 1000\n size 3\n color 0 0 1 1\n render sprite soft=0 shading=unlit",
    )
    .unwrap();
    // The translucent one is started first and is farther; either way the solid one is in front.
    for (effect, z) in [(&behind, -8.0), (&solid, -5.0)] {
        stage
            .renderer
            .particles()
            .spawn(effect, Mat4::from_translation(Vec3::new(0.0, 0.0, z)))
            .unwrap();
    }
    stage.frames(&gpu, &camera, &darkness(), 4);
    let [r, _, b] = patch(&stage.pixels(&gpu), size, size / 2, size / 2, 0).map(srgb_to_linear);
    assert!(r > 0.9 && b < 0.02, "red {r}, blue {b}");
}

/// Local lights are shadowed through their own maps: a lamp over the floor with a ball hung
/// between them leaves the floor under the ball dark and the floor beside it lit, and taking
/// the ball away lights the floor under it. A map never traced would shadow the floor beside
/// the ball too; a map never consulted would shadow nothing; a second lamp, out of the ball's
/// way and first in the list, would light the floor under the ball if the lamps' maps were
/// mixed up; and a low lamp lights the floor at a steep slant, where a map read with too
/// little margin, or compared at the wrong depth, would shadow the floor with itself.
#[test]
fn a_lamp_is_shadowed_through_its_map() {
    let Some(gpu) = request_gpu() else {
        return;
    };
    let camera = Camera {
        position: Vec3::new(0.0, 6.0, 9.0),
        yaw: 0.0,
        pitch: -0.6,
        ..Default::default()
    };
    // No sun, no sky and no indirect light, with the exposure held to daylight, so only the
    // lamp's direct light shows.
    let night = Environment {
        sun_illuminance: 0.0,
        sky_intensity: 0.0,
        gi_intensity: 0.0,
        daylight_illuminance: 10.0,
        ..Default::default()
    };
    let lamp = |position: [f32; 3]| Light {
        position,
        range: 9.0,
        intensity: [60.0; 3],
        _pad: 0,
    };
    let size = 128;
    // The volume resolves the ball at 0.2 m voxels and stops its traces most of a voxel short
    // of a surface, so the shadow is some 2.5 m in radius on the floor; the lit spot stands
    // well outside it, 42° from the lamp's vertical against the shadow's 26°. The slanted spot,
    // nearer the camera, is lit by the low lamp at 69° from its vertical, four times as much
    // as by the lamp over the ball.
    let under = Vec3::ZERO;
    let beside = Vec3::new(4.5, 0.0, 0.0);
    let slanted = Vec3::new(-2.5, 0.0, 4.5);
    let low_lamp = lamp([-6.5, 1.5, 4.5]);
    let floor_at = |pixels: &[u8], world: Vec3| {
        let (x, y) = pixel_of(&camera, size, world);
        brightness(patch(pixels, size, x, y, 2))
    };
    let mut scene = ball_on_floor();
    // The first lamp is far enough along +X that nothing it lights is measured; the low lamp
    // reaches the spot under the ball only faintly, at the edge of its range.
    scene.lights = vec![lamp([16.0, 5.0, 0.0]), lamp([0.0, 5.0, 0.0]), low_lamp];
    // The ball hangs between the lamp and the floor, a metre above the floor and two below
    // the lamp.
    scene.instances[0].transform = Mat4::from_translation(Vec3::new(0.0, 2.0, 0.0));
    let with_ball = render_in(
        &gpu,
        &scene,
        &camera,
        RendererSettings::default(),
        size,
        &night,
    )
    .pixels;
    scene.instances.remove(0);
    let without = render_in(
        &gpu,
        &scene,
        &camera,
        RendererSettings::default(),
        size,
        &night,
    )
    .pixels;

    let slant = floor_at(&without, slanted);
    scene.lights.pop();
    let no_low_lamp = render_in(
        &gpu,
        &scene,
        &camera,
        RendererSettings::default(),
        size,
        &night,
    )
    .pixels;
    let slant_unlit = floor_at(&no_low_lamp, slanted);

    let (shadowed, lit) = (floor_at(&with_ball, under), floor_at(&with_ball, beside));
    let (open, lit_alone) = (floor_at(&without, under), floor_at(&without, beside));
    assert!(
        shadowed + 20.0 < lit,
        "under the ball the floor is {shadowed}, beside it {lit}: no shadow"
    );
    assert!(
        shadowed + 20.0 < open,
        "under the ball the floor is {shadowed}, and {open} with the ball gone: no shadow"
    );
    // The probes' indirect specular is not scaled by `gi_intensity`, so the lit ball's glow
    // reaches the floor beside it a little; a map never traced would leave it near black.
    assert!(
        (lit - lit_alone).abs() < 30.0,
        "beside the ball the floor is {lit} with it and {lit_alone} without: the ball shadows what it should not"
    );
    assert!(
        slant > slant_unlit + 20.0,
        "the floor at a steep slant to the low lamp is {slant} with it and {slant_unlit} without: it shadows itself"
    );
}

/// A night with only a lamp's direct light on the bare floor, seen from 8 m above the origin: the
/// camera of the tests below, which read how the floor's brightness runs along X.
fn lamp_on_bare_floor(position: [f32; 3]) -> (SceneAsset, Camera, Environment) {
    let mut scene = ball_on_floor();
    scene.instances.remove(0);
    scene.lights = vec![Light {
        position,
        range: 30.0,
        intensity: [600.0; 3],
        _pad: 0,
    }];
    let camera = Camera {
        position: Vec3::new(0.0, 8.0, 0.0),
        pitch: -1.5,
        ..Default::default()
    };
    let night = Environment {
        sun_illuminance: 0.0,
        sky_intensity: 0.0,
        gi_intensity: 0.0,
        daylight_illuminance: 10.0,
        ..Default::default()
    };
    (scene, camera, night)
}

/// The floor's brightness in `pixels`, a 256-texel render, at (x, 0, z).
fn floor_brightness(pixels: &[u8], camera: &Camera, x: f32, z: f32) -> f32 {
    let (px, py) = pixel_of(camera, 256, Vec3::new(x, 0.0, z));
    brightness(patch(pixels, 256, px, py, 1))
}

/// A low lamp's light fades smoothly along the floor. Comparing a ray that skims the floor at a
/// fixed distance rather than where it meets the plane, or falling back to the point's distance
/// well before the ray runs along the floor, breaks the curve into a cliff.
#[test]
fn a_low_lamp_fades_smoothly_along_the_floor() {
    let Some(gpu) = request_gpu() else { return };
    // A metre up, the map's slanted rays reach the floor well past what the camera sees; 0.6 m
    // up, they reach it in view, where a fallback too eager to give up on the plane shows.
    for height in [1.0, 0.6] {
        let (scene, camera, night) = lamp_on_bare_floor([-8.0, height, 0.0]);
        let settings = RendererSettings::default();
        let pixels = render_in(&gpu, &scene, &camera, settings, 256, &night).pixels;
        let run: Vec<f32> = (-16..16)
            .map(|i| floor_brightness(&pixels, &camera, i as f32 * 0.25, 0.0))
            .collect();
        let worst = run
            .windows(3)
            .map(|w| (w[1] - (w[0] + w[2]) * 0.5).abs())
            .fold(0.0, f32::max);
        assert!(
            worst < 8.0,
            "a step of {worst} in the falloff from {height} m up: {run:?}"
        );
        assert!(
            run[0] > 40.0,
            "the floor nearest the lamp {height} m up is {}: nothing to fade",
            run[0]
        );
    }
}

/// A lamp outside the volume's finest level still lights the floor near the camera. Its rays
/// must trace that floor at the finest level's voxels, or its coarse level's threshold stops
/// them short of the floor, which then goes black.
#[test]
fn a_lamp_in_a_coarser_level_lights_the_floor_near_the_camera() {
    let Some(gpu) = request_gpu() else { return };
    let (scene, camera, night) = lamp_on_bare_floor([-11.0, 1.2, -3.0]);
    let settings = RendererSettings::default();
    let pixels = render_in(&gpu, &scene, &camera, settings, 256, &night).pixels;
    let under = floor_brightness(&pixels, &camera, 0.0, 0.0);
    assert!(under > 100.0, "the floor under the camera is {under}");
}

/// A wall just beyond a point of the floor, as seen from a low lamp, does not shadow the point:
/// the map's rays stop at the wall, and a ray skimming the floor would meet the floor's plane
/// past it. The floor 0.5 m before the wall stays near the same floor with the wall gone. The
/// wall is black, so that it sends no light of its own to the floor, and the exposure is held
/// so that the wall's filling part of the image does not change it.
#[test]
fn a_wall_beyond_the_floor_does_not_shadow_it() {
    let Some(gpu) = request_gpu() else { return };
    let (mut scene, camera, mut night) = lamp_on_bare_floor([-8.0, 1.0, 0.0]);
    // Well above what the image measures, so the exposure stays at its darkest limit.
    night.daylight_illuminance = 1500.0;
    let settings = RendererSettings::default;
    let open = render_in(&gpu, &scene, &camera, settings(), 256, &night).pixels;
    // A metre and a half thick, for the volume's 0.2 m voxels; the camera sees over it to the
    // floor in front.
    let block = build_virtual_mesh(
        &solid_box(Vec3::new(0.75, 1.5, 6.0)),
        &BuildOptions::default(),
    );
    scene.meshes.push(block);
    scene.materials.push(Material {
        base_color: [0.0, 0.0, 0.0, 1.0],
        ..Default::default()
    });
    scene.instances.push(Instance {
        mesh: scene.meshes.len() as u32 - 1,
        material: 1,
        transform: Mat4::from_translation(Vec3::new(2.25, 1.5, 0.0)),
    });
    let walled = render_in(&gpu, &scene, &camera, settings(), 256, &night).pixels;
    for z in [-1.0, 0.0, 1.0] {
        let (near, far) = (
            floor_brightness(&walled, &camera, 1.0, z),
            floor_brightness(&open, &camera, 1.0, z),
        );
        assert!(
            (near - far).abs() < 5.0,
            "the floor 0.5 m before the wall, at z {z}, is {near} with it and {far} without"
        );
    }
}

/// A closed box of the given half extents around the origin, with flat faces.
fn solid_box(half: Vec3) -> Mesh {
    let mut mesh = Mesh::default();
    for (normal, u, v) in [
        (Vec3::X, Vec3::Y, Vec3::Z),
        (Vec3::NEG_X, Vec3::Z, Vec3::Y),
        (Vec3::Y, Vec3::Z, Vec3::X),
        (Vec3::NEG_Y, Vec3::X, Vec3::Z),
        (Vec3::Z, Vec3::X, Vec3::Y),
        (Vec3::NEG_Z, Vec3::Y, Vec3::X),
    ] {
        let base = mesh.vertices.len() as u32;
        for (a, b) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
            mesh.vertices.push(Vertex {
                position: (normal + u * a + v * b) * half,
                normal,
                uv: Vec2::new(a, b),
            });
        }
        mesh.indices
            .extend([0, 1, 2, 0, 2, 3].map(|i: u32| base + i));
    }
    mesh
}

/// Edge of the cut-out test textures.
const CARD_EDGE: u32 = 64;

/// A unit square in the XY plane around the origin, its UVs running over it, facing +Z or away,
/// `cells` quads across.
fn card_mesh(facing_camera: bool, cells: u32) -> Mesh {
    let mut mesh = Mesh::default();
    for j in 0..=cells {
        for i in 0..=cells {
            let (u, v) = (i as f32 / cells as f32, j as f32 / cells as f32);
            mesh.vertices.push(Vertex {
                position: Vec3::new(u - 0.5, v - 0.5, 0.0),
                normal: if facing_camera { Vec3::Z } else { Vec3::NEG_Z },
                uv: Vec2::new(u, v),
            });
        }
    }
    for j in 0..cells {
        for i in 0..cells {
            let a = j * (cells + 1) + i;
            let (b, c, d) = (a + 1, a + cells + 1, a + cells + 2);
            if facing_camera {
                mesh.indices.extend_from_slice(&[a, b, c, b, d, c]);
            } else {
                mesh.indices.extend_from_slice(&[a, c, b, b, c, d]);
            }
        }
    }
    mesh
}

/// The texture layer of a card: pale, with its alpha given per texel.
fn card_layer(alpha: impl Fn(u32, u32) -> u8) -> TextureLayer {
    let n = (CARD_EDGE * CARD_EDGE) as usize;
    let mut color = Vec::with_capacity(n * 4);
    for y in 0..CARD_EDGE {
        for x in 0..CARD_EDGE {
            color.extend_from_slice(&[220, 220, 220, alpha(x, y)]);
        }
    }
    TextureLayer {
        color_mips: masked_mip_chain(color, CARD_EDGE),
        normal_mips: mip_chain([128, 128].repeat(n), CARD_EDGE, 2),
        material_mips: mip_chain([128, 255].repeat(n), CARD_EDGE, 2),
    }
}

/// The left half of the texture is a hole, the right half solid.
fn half_cut(x: u32, _: u32) -> u8 {
    if x < CARD_EDGE / 2 {
        0
    } else {
        255
    }
}

fn card_material(flags: u32) -> Material {
    Material {
        base_color: [1.0; 4],
        texture_layer: 0,
        flags: Material::MASKED | flags,
        ..Default::default()
    }
}

/// One card, as the only thing in the scene.
fn card_scene(flags: u32, facing_camera: bool, alpha: impl Fn(u32, u32) -> u8) -> SceneAsset {
    SceneAsset {
        meshes: vec![build_virtual_mesh(
            &card_mesh(facing_camera, 8),
            &BuildOptions::default(),
        )],
        materials: vec![card_material(flags)],
        instances: vec![instance(0, Mat4::IDENTITY)],
        texture_size: CARD_EDGE,
        textures: vec![card_layer(alpha)],
        ..Default::default()
    }
}

/// The albedo of a scene from `distance` along +Z, and whether software drew any of it.
fn card_albedo(gpu: &Gpu, scene: &SceneAsset, distance: f32) -> (Vec<u8>, Camera, bool) {
    let camera = Camera {
        position: Vec3::new(0.0, 0.0, distance),
        ..Default::default()
    };
    let settings = RendererSettings {
        debug_view: DebugView::Albedo,
        temporal_accumulation: false,
        ..Default::default()
    };
    let run = render(gpu, scene, &camera, settings, 256);
    (run.pixels, camera, run.software_clusters > 0)
}

/// A hole in a cut-out shows what is behind it and the solid part does not, on both rasterizers.
#[test]
fn cut_outs_have_holes_on_both_rasterizers() {
    let Some(gpu) = request_gpu() else { return };
    let scene = card_scene(0, true, half_cut);
    for (distance, software) in [(1.0, false), (14.0, true)] {
        let (pixels, camera, used_software) = card_albedo(&gpu, &scene, distance);
        assert_eq!(used_software, software, "at {distance} m");
        let at = |x: f32| {
            let (px, py) = pixel_of(&camera, 256, Vec3::new(x, 0.0, 0.0));
            brightness(patch(&pixels, 256, px, py, 0))
        };
        let (hole, solid) = (at(-0.25), at(0.25));
        assert!(hole < 5.0, "at {distance} m the hole shows {hole}");
        assert!(
            solid > 100.0,
            "at {distance} m the solid part shows {solid}"
        );
    }
}

/// Only a two-sided card shows from behind, and it keeps its holes there.
#[test]
fn cut_outs_are_two_sided_only_where_the_material_says() {
    let Some(gpu) = request_gpu() else { return };
    for (distance, software) in [(1.0, false), (14.0, true)] {
        for two_sided in [false, true] {
            let flags = if two_sided { Material::TWO_SIDED } else { 0 };
            let mut scene = card_scene(flags, false, half_cut);
            // One quad, so that a back face's texture has to be the right way round across all
            // of it, and with points off its diagonal, which a swap would move.
            scene.meshes[0] = build_virtual_mesh(&card_mesh(false, 1), &BuildOptions::default());
            let (pixels, camera, used_software) = card_albedo(&gpu, &scene, distance);
            assert!(!two_sided || used_software == software, "at {distance} m");
            let at = |x: f32, y: f32| {
                let (px, py) = pixel_of(&camera, 256, Vec3::new(x, y, 0.0));
                brightness(patch(&pixels, 256, px, py, 0))
            };
            let (hole, solid) = (at(-0.25, 0.2), at(0.25, -0.2));
            assert!(hole < 5.0, "the hole at {distance} m shows {hole}");
            if two_sided {
                assert!(solid > 100.0, "two-sided at {distance} m: {solid}");
            } else {
                assert!(solid < 5.0, "one-sided at {distance} m: {solid}");
            }
        }
    }
}

/// Holes are not occluders: what stands behind one is still drawn, after occlusion culling has
/// had frames to act.
#[test]
fn what_stands_behind_a_hole_is_not_occluded() {
    let Some(gpu) = request_gpu() else { return };
    let mut scene = card_scene(0, true, half_cut);
    scene.instances[0].transform = Mat4::from_scale(Vec3::new(3.0, 3.0, 1.0));
    scene
        .meshes
        .push(build_virtual_mesh(&sphere(4), &BuildOptions::default()));
    // Red, as the card is grey: bright alone would pass with the card drawn over the ball.
    scene.materials.push(Material {
        base_color: [1.0, 0.0, 0.0, 1.0],
        ..Default::default()
    });
    let ball = Vec3::new(-0.75, 0.0, -3.0);
    scene.instances.push(Instance {
        mesh: 1,
        material: 1,
        transform: Mat4::from_scale_rotation_translation(Vec3::splat(0.4), Quat::IDENTITY, ball),
    });
    let (pixels, camera, _) = card_albedo(&gpu, &scene, 5.0);
    let (px, py) = pixel_of(&camera, 256, ball);
    let [red, green, _] = patch(&pixels, 256, px, py, 1);
    assert!(
        red > green + 100.0,
        "the ball behind the hole shows red {red} and green {green}"
    );
}

/// A two-sided cut-out card placed by `card` above a wide floor at y = 0.
fn card_over_floor(mesh: &Mesh, card: Mat4) -> SceneAsset {
    let mut floor = Mesh::default();
    for (x, z) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
        floor.vertices.push(Vertex {
            position: Vec3::new(x * 400.0, 0.0, z * 400.0),
            normal: Vec3::Y,
            uv: Vec2::new(x, z),
        });
    }
    floor.indices = vec![0, 2, 1, 0, 3, 2];
    let mut scene = card_scene(Material::TWO_SIDED, true, half_cut);
    scene.meshes[0] = build_virtual_mesh(mesh, &BuildOptions::default());
    scene.instances[0].transform = card;
    scene
        .meshes
        .push(build_virtual_mesh(&floor, &BuildOptions::default()));
    scene.materials.push(white_material());
    scene.instances.push(Instance {
        mesh: 1,
        material: 1,
        transform: Mat4::IDENTITY,
    });
    scene
}

/// A card placed by `card`, its hole on its -X side, casts the shadow of its solid half alone, on
/// the rasterizer its size sends it to.
#[allow(clippy::too_many_arguments)]
fn check_card_shadow(
    gpu: &Gpu,
    mesh: &Mesh,
    card: Mat4,
    camera: Camera,
    sun_slope: f32,
    offset: f32,
    software: bool,
) {
    let mut scene = card_over_floor(mesh, card);
    let sun_direction = Vec3::new(sun_slope, 1.0, 0.0).normalize();
    let sun = Environment {
        sun_direction,
        sky_intensity: 0.0,
        gi_intensity: 0.0,
        ..Default::default()
    };
    let settings = RendererSettings {
        temporal_accumulation: false,
        ..Default::default()
    };
    let run = render_in(gpu, &scene, &camera, settings, 512, &sun);
    // The statistic is of the last cascade drawn, where even a large card is small, so it can
    // only show that software was used, not that it was not.
    assert!(!software || run.shadow_software_clusters > 0, "no software");
    let with_card = run.pixels;
    scene.instances.remove(0);
    let bare = render_in(gpu, &scene, &camera, settings, 512, &sun).pixels;

    // Where a point of the card's casts to on the floor.
    let floor = |pixels: &[u8], local_x: f32| {
        let point = card.transform_point3(Vec3::new(local_x, 0.0, 0.0));
        let spot = point - sun_direction * (point.y / sun_direction.y);
        let (px, py) = pixel_of(&camera, 512, spot);
        brightness(patch(pixels, 512, px, py, 1))
    };
    let (solid, hole) = (floor(&with_card, offset), floor(&with_card, -offset));
    let (bare_solid, bare_hole) = (floor(&bare, offset), floor(&bare, -offset));
    // A small card's shadow is mostly penumbra, so these are modest margins.
    assert!(
        solid + 8.0 < bare_solid,
        "behind the solid half the floor is {solid} against {bare_solid} bare"
    );
    assert!(
        solid + 8.0 < hole,
        "the floor is {solid} behind the solid half and {hole} through the hole"
    );
    assert!(
        (hole - bare_hole).abs() < 10.0,
        "through the hole the floor is {hole} against {bare_hole} bare"
    );
}

/// A card that may move casts a shadow with holes too, drawn afresh over the cached cascades:
/// with one cull for every cascade, and with one each, as a great many moving things get.
#[test]
fn moving_cut_outs_have_holes_in_their_shadows() {
    let Some(gpu) = request_gpu() else { return };
    check_moving_card_shadow(&gpu, 0);
    check_moving_card_shadow(&gpu, 2100);
}

/// Checks a moving card's shadow, with `others` more tiny moving cards well away from it.
fn check_moving_card_shadow(gpu: &Gpu, others: u32) {
    let card = Mat4::from_translation(Vec3::new(0.0, 1.0, 0.0))
        * Mat4::from_rotation_x(-std::f32::consts::FRAC_PI_2)
        * Mat4::from_scale(Vec3::new(2.0, 2.0, 1.0));
    let mut scene = card_over_floor(&card_mesh(true, 8), card);
    for i in 0..others {
        let place = Vec3::new(
            -12.0 - (i % 50) as f32 * 0.1,
            0.5 + (i / 50) as f32 * 0.1,
            -4.0,
        );
        scene.instances.push(Instance {
            mesh: 0,
            material: 0,
            transform: Mat4::from_translation(place) * Mat4::from_scale(Vec3::splat(0.02)),
        });
    }
    let sun_direction = Vec3::new(0.5, 1.0, 0.0).normalize();
    let sun = Environment {
        sun_direction,
        sky_intensity: 0.0,
        gi_intensity: 0.0,
        ..Default::default()
    };
    let camera = Camera {
        position: Vec3::new(-0.5, 2.0, 2.5),
        pitch: -0.7,
        ..Default::default()
    };
    let settings = RendererSettings {
        temporal_accumulation: false,
        ..Default::default()
    };
    let mut stage = Stage::new(gpu, &scene, settings, 512);
    assert!(stage.renderer.set_instance_movable(0, true));
    for i in 0..others {
        assert!(stage.renderer.set_instance_movable(2 + i, true));
    }
    stage.frames(gpu, &camera, &sun, 8);
    let pixels = stage.pixels(gpu);
    let floor = |local_x: f32| {
        let point = card.transform_point3(Vec3::new(local_x, 0.0, 0.0));
        let spot = point - sun_direction * (point.y / sun_direction.y);
        let (px, py) = pixel_of(&camera, 512, spot);
        brightness(patch(&pixels, 512, px, py, 1))
    };
    let (solid, hole) = (floor(0.25), floor(-0.25));
    assert!(
        solid + 30.0 < hole,
        "with {others} others the floor is {solid} behind the solid half and {hole} through the hole"
    );
}

#[test]
fn sun_shadows_have_the_cut_outs_holes_on_both_rasterizers() {
    let Some(gpu) = request_gpu() else { return };
    let flat = |height: f32, size: f32| {
        Mat4::from_translation(Vec3::new(0.0, height, 0.0))
            * Mat4::from_rotation_x(-std::f32::consts::FRAC_PI_2)
            * Mat4::from_scale(Vec3::new(size, size, 1.0))
    };
    // Close up, a cascade's texels are small and the card takes hardware.
    let near = Camera {
        position: Vec3::new(-0.5, 2.0, 2.5),
        pitch: -0.7,
        ..Default::default()
    };
    check_card_shadow(
        &gpu,
        &card_mesh(true, 8),
        flat(1.0, 2.0),
        near,
        0.5,
        0.25,
        false,
    );
    // A flat card simplifies to clusters too large for software. A ball's surface keeps its
    // small clusters, so with half of it cut away the shadow can only come out right through
    // the software path, under a sun high enough that a ray meets just one half.
    let mut ball = sphere(5);
    for vertex in &mut ball.vertices {
        vertex.uv = Vec2::new((vertex.position.x + 1.0) * 0.5, 0.5);
    }
    let placed = Mat4::from_scale_rotation_translation(
        Vec3::splat(0.05),
        Quat::IDENTITY,
        Vec3::new(0.0, 0.3, 0.0),
    );
    check_card_shadow(&gpu, &ball, placed, near, 0.05, 0.5, true);
}

/// Local light goes through the voxels, which must have the wall's holes too: a lamp behind a
/// wall reaches the floor beyond through the open half and not past the solid half.
#[test]
fn light_passes_through_a_cut_outs_holes_in_the_voxels() {
    let Some(gpu) = request_gpu() else { return };
    // Turned so the card's -X side, the hole, is towards +Z, and standing on the floor.
    let wall = Mat4::from_translation(Vec3::new(0.0, 2.5, 0.0))
        * Mat4::from_rotation_y(std::f32::consts::FRAC_PI_2)
        * Mat4::from_scale(Vec3::new(10.0, 5.0, 1.0));
    let mut scene = card_over_floor(&card_mesh(true, 8), wall);
    scene.lights = vec![Light {
        position: [-4.0, 1.5, 0.0],
        range: 30.0,
        intensity: [600.0; 3],
        _pad: 0,
    }];
    let camera = Camera {
        position: Vec3::new(3.0, 8.0, 0.0),
        pitch: -1.5,
        ..Default::default()
    };
    let night = Environment {
        sun_illuminance: 0.0,
        sky_intensity: 0.0,
        gi_intensity: 0.0,
        daylight_illuminance: 10.0,
        ..Default::default()
    };
    let settings = RendererSettings::default();
    let pixels = render_in(&gpu, &scene, &camera, settings, 256, &night).pixels;
    let at = |z: f32| {
        let (px, py) = pixel_of(&camera, 256, Vec3::new(3.0, 0.0, z));
        brightness(patch(&pixels, 256, px, py, 1))
    };
    let (open, shut) = (at(2.0), at(-2.0));
    assert!(
        open > shut + 20.0,
        "beyond the wall the floor is {open} past the hole and {shut} past the solid half"
    );
}

/// A cut-out's alpha is coverage: what passes is lit the same however far above the test it is.
#[test]
fn coverage_is_not_read_as_cavity() {
    let Some(gpu) = request_gpu() else { return };
    let lit = |alpha: u8| {
        let scene = card_scene(0, true, |_, _| alpha);
        let camera = Camera {
            position: Vec3::new(0.0, 0.0, 1.0),
            ..Default::default()
        };
        let settings = RendererSettings {
            temporal_accumulation: false,
            ..Default::default()
        };
        let environment = Environment::default();
        let pixels = render_in(&gpu, &scene, &camera, settings, 64, &environment).pixels;
        brightness(patch(&pixels, 64, 32, 32, 2))
    };
    let (barely, fully) = (lit(130), lit(255));
    assert!(fully > 10.0, "the card shows: {fully}");
    assert!((barely - fully).abs() < 2.0, "{barely} against {fully}");
}

/// Nothing for cut-outs exists in a scene without any.
#[test]
fn only_scenes_with_cut_outs_have_their_passes() {
    let Some(gpu) = request_gpu() else { return };
    let scene = card_scene(0, true, half_cut);
    let masked = Stage::new(&gpu, &scene, Default::default(), 16);
    assert!(masked.renderer.draws_masked());
    let mut opaque = scene;
    opaque.materials[0].flags = 0;
    let plain = Stage::new(&gpu, &opaque, Default::default(), 16);
    assert!(!plain.renderer.draws_masked());
}

/// A fine pattern far away covers about what it covers near, and holds still under TAA.
#[test]
fn distant_cut_outs_keep_their_coverage_and_hold_still() {
    let Some(gpu) = request_gpu() else { return };
    let scatter = |x: u32, y: u32| {
        let h = ((y * CARD_EDGE + x) ^ 0x9e37_79b9).wrapping_mul(0x85eb_ca6b);
        if (h ^ (h >> 13)).wrapping_mul(0xc2b2_ae35) >> 30 == 0 {
            255
        } else {
            0
        }
    };
    let scene = card_scene(0, true, scatter);
    let settings = RendererSettings {
        debug_view: DebugView::Albedo,
        temporal_accumulation: true,
        ..Default::default()
    };
    let size = 512;
    let camera_at = |distance: f32| Camera {
        position: Vec3::new(0.0, 0.0, distance),
        ..Default::default()
    };
    // Pixels over the middle of the card, in reading order.
    let over_card = |pixels: &[u8], camera: &Camera| -> Vec<f32> {
        let (x0, y1) = pixel_of(camera, size, Vec3::new(-0.45, -0.45, 0.0));
        let (x1, y0) = pixel_of(camera, size, Vec3::new(0.45, 0.45, 0.0));
        let mut values = Vec::new();
        for y in y0..=y1 {
            for x in x0..=x1 {
                values.push(brightness(patch(pixels, size, x, y, 0)));
            }
        }
        values
    };
    let mean = |values: &[f32]| values.iter().sum::<f32>() / values.len() as f32;
    // Without temporal blending each pixel is solid or not, so they can be counted.
    let sharp = RendererSettings {
        temporal_accumulation: false,
        ..settings
    };
    let solid_share = |values: &[f32]| {
        let middle = values.iter().cloned().fold(0.0, f32::max) / 2.0;
        values.iter().filter(|&&v| v > middle).count() as f32 / values.len() as f32
    };

    let near_camera = camera_at(1.2);
    let mut stage = Stage::new(&gpu, &scene, sharp, size);
    stage.frames(&gpu, &near_camera, &darkness(), 4);
    let near = solid_share(&over_card(&stage.pixels(&gpu), &near_camera));

    let far_camera = camera_at(16.0);
    let mut stage = Stage::new(&gpu, &scene, sharp, size);
    stage.frames(&gpu, &far_camera, &darkness(), 4);
    let far = solid_share(&over_card(&stage.pixels(&gpu), &far_camera));

    let mut stage = Stage::new(&gpu, &scene, settings, size);
    stage.frames(&gpu, &far_camera, &darkness(), 24);
    let first = over_card(&stage.pixels(&gpu), &far_camera);
    stage.frames(&gpu, &far_camera, &darkness(), 1);
    let second = over_card(&stage.pixels(&gpu), &far_camera);

    assert!(near > 0.1, "near the card shows: {near}");
    assert!(
        (far / near - 1.0).abs() < 0.45,
        "{far} of the card is solid at a distance, {near} near"
    );
    let changes: Vec<f32> = first
        .iter()
        .zip(&second)
        .map(|(a, b)| (a - b).abs())
        .collect();
    let shimmer = mean(&changes);
    assert!(
        shimmer < 8.0,
        "pixels of the distant card change by {shimmer} a frame"
    );
}

/// A sphere with one skeleton of two bones stacked up its height, the lower carrying the bottom
/// half and the upper carrying the top, blended across the middle band, on a floor.
fn skinned_ball_on_floor() -> SceneAsset {
    use solstice_asset::{Bone, Skeleton};
    let mut scene = red_ball_on_floor();
    let source = sphere(4);
    let skins: Vec<PackedSkin> = source
        .vertices
        .iter()
        .map(|vertex| {
            let blend = ((vertex.position.y + 0.3) / 0.6).clamp(0.0, 1.0);
            PackedSkin::new([0, 1, 0, 0], [1.0 - blend, blend, 0.0, 0.0])
        })
        .collect();
    scene.meshes[0] =
        solstice_builder::build_skinned_mesh(&source, &skins, &BuildOptions::default());
    let bone = |name: &str, parent, y: f32| Bone {
        name: name.to_owned(),
        parent,
        translation: Vec3::new(0.0, y, 0.0),
        rotation: glam::Quat::IDENTITY,
        // The bones' places in the mesh, which is where the file binds them.
        inverse_bind: Mat4::from_translation(Vec3::new(0.0, -y, 0.0)),
        center: Vec3::ZERO,
        radius: 1.1,
    };
    scene.skeletons = vec![Skeleton {
        bones: vec![bone("root", None, 0.0), bone("top", Some(0), 0.0)],
    }];
    scene.poses = vec![0];
    scene.instance_poses = vec![0];
    scene
}

/// Poses the top bone of [`skinned_ball_on_floor`], in the mesh's own space.
fn pose_top(stage: &mut Stage, top: Mat4, jump: bool) {
    let bones = [Mat4::IDENTITY, top];
    let taken = if jump {
        stage.renderer.place_pose(0, &bones)
    } else {
        stage.renderer.set_pose(0, &bones)
    };
    assert!(taken);
}

#[test]
fn in_its_bind_pose_a_skinned_mesh_looks_as_a_rigid_one_does() {
    let Some(gpu) = request_gpu() else { return };
    let (camera, sun) = (over_the_floor(), low_sun());
    let size = 192;
    let settings = RendererSettings {
        debug_view: DebugView::Normals,
        ..untemporal()
    };
    let mut rigid = Stage::new(&gpu, &red_ball_on_floor(), settings, size);
    rigid.frames(&gpu, &camera, &sun, 3);
    let mut skinned = Stage::new(&gpu, &skinned_ball_on_floor(), settings, size);
    skinned.frames(&gpu, &camera, &sun, 3);
    // Two builds of one surface, whose clusters differ, so close rather than equal.
    let (rigid, skinned) = (rigid.pixels(&gpu), skinned.pixels(&gpu));
    let differing = rigid
        .chunks_exact(4)
        .zip(skinned.chunks_exact(4))
        .filter(|(a, b)| a.iter().zip(b.iter()).any(|(x, y)| x.abs_diff(*y) > 6))
        .count();
    assert!(differing < 100, "{differing} pixels differ");
}

#[test]
fn a_pose_moves_what_is_skinned_to_it_and_its_shadow() {
    let Some(gpu) = request_gpu() else { return };
    let probe = ShadowProbe::new(1.0, 192);
    let sun = low_sun();
    let mut stage = Stage::new(&gpu, &skinned_ball_on_floor(), untemporal(), probe.size);
    stage.frames(&gpu, &probe.camera, &sun, 8);
    let pixels = stage.pixels(&gpu);
    assert!(probe.red(&pixels, 0.0), "the ball isn't where it stands");
    assert!(probe.shade(&pixels, 0.0) < 0.6, "it casts no shadow");

    // The top bone takes all of what is above the band across by 3 m, the rest stays.
    pose_top(
        &mut stage,
        Mat4::from_translation(Vec3::new(3.0, 0.0, 0.0)),
        true,
    );
    stage.frames(&gpu, &probe.camera, &sun, 2);
    let pixels = stage.pixels(&gpu);
    // The top of the ball went with its bone.
    let (x, y) = pixel_of(&probe.camera, probe.size, Vec3::new(3.0, 1.75, 0.0));
    let [r, g, _] = patch(&pixels, probe.size, x, y, 1);
    assert!(r > 2.0 * g, "the top of the ball didn't go with its bone");
    // Its shadow went too: the sun is 45 degrees up, so what is above x = 3 at height 1.5 falls at
    // x = 1.5, where the floor was lit before.
    assert!(probe.shade(&pixels, 3.1) < 0.7, "its shadow stayed behind");
}

#[test]
fn a_cut_through_a_bending_mesh_has_no_holes_from_near_or_far() {
    let Some(gpu) = request_gpu() else { return };
    // Stays white where the ball is.
    let mut scene = SceneAsset {
        materials: vec![white_material()],
        instances: vec![instance(0, Mat4::IDENTITY)],
        ..Default::default()
    };
    let source = sphere(7);
    let skins: Vec<PackedSkin> = source
        .vertices
        .iter()
        .map(|vertex| {
            let blend = ((vertex.position.y + 0.4) / 0.8).clamp(0.0, 1.0);
            PackedSkin::new([0, 1, 0, 0], [1.0 - blend, blend, 0.0, 0.0])
        })
        .collect();
    scene.meshes.push(solstice_builder::build_skinned_mesh(
        &source,
        &skins,
        &BuildOptions::default(),
    ));
    let bone = |parent| solstice_asset::Bone {
        parent,
        inverse_bind: Mat4::IDENTITY,
        center: Vec3::ZERO,
        radius: 1.1,
        ..Default::default()
    };
    scene.skeletons = vec![solstice_asset::Skeleton {
        bones: vec![bone(None), bone(Some(0))],
    }];
    scene.poses = vec![0];
    scene.instance_poses = vec![0];

    const SIZE: u32 = 512;
    let settings = RendererSettings {
        debug_view: DebugView::Albedo,
        temporal_accumulation: false,
        ..Default::default()
    };
    let mut used_software = false;
    let mut used_hardware = false;
    for distance in [12.0_f32, 5.0, 2.5, 1.6] {
        let camera = Camera {
            position: Vec3::new(0.0, 0.0, distance),
            fov_y: 50.0_f32.to_radians(),
            near: 0.05,
            ..Default::default()
        };
        let mut stage = Stage::new(&gpu, &scene, settings, SIZE);
        // Twists the top half about the vertical and slides it a little, moving every vertex above
        // the band a different way from the ones below it. That narrows the waist and bares the
        // edge on one side, so only what stays covered in any case is looked at.
        for frame in 0..4 {
            let angle = 0.25 + frame as f32 * 0.3;
            pose_top(
                &mut stage,
                Mat4::from_translation(Vec3::X * 0.15) * Mat4::from_rotation_y(angle),
                false,
            );
            stage.frames(&gpu, &camera, &darkness(), 2);
            let pixels = stage.pixels(&gpu);
            let holes = holes_within(&pixels, SIZE, distance, 50.0_f32.to_radians(), 0.7);
            assert_eq!(
                holes, 0,
                "{holes} holes at distance {distance}, frame {frame}"
            );
            // A mesh that wasn't posed at all would have no holes either.
            if distance >= 5.0 {
                let beyond = covered_beyond(&pixels, SIZE, distance, 50.0_f32.to_radians(), 1.05);
                assert!(
                    beyond > 0,
                    "the top half didn't move, at distance {distance}"
                );
            }
        }
        let stats = stage.renderer.stats();
        assert!(stats.clusters > 0, "nothing was drawn");
        used_software |= stats.software_clusters > 0;
        used_hardware |= stats.software_clusters < stats.clusters;
    }
    assert!(
        used_software && used_hardware,
        "both rasterizers should draw it"
    );
}

#[test]
fn a_moving_pose_shows_its_motion_and_stops_showing_it_when_it_stops() {
    let Some(gpu) = request_gpu() else { return };
    let size = 128;
    let camera = over_the_floor();
    let settings = RendererSettings {
        debug_view: DebugView::Motion,
        ..untemporal()
    };
    let mut stage = Stage::new(&gpu, &skinned_ball_on_floor(), settings, size);
    let environment = low_sun();
    let colours = |pixels: &[u8], x: f32| {
        let (bx, by) = pixel_of(&camera, size, Vec3::new(x, 1.75, 0.0));
        let (fx, fy) = pixel_of(&camera, size, Vec3::new(-4.0, 0.0, 3.0));
        (
            patch(pixels, size, bx, by, 1),
            patch(pixels, size, fx, fy, 1),
        )
    };

    stage.frames(&gpu, &camera, &environment, 4);
    let mut x = 0.0;
    for _ in 0..6 {
        x += 0.1;
        pose_top(&mut stage, Mat4::from_translation(Vec3::X * x), false);
        stage.frames(&gpu, &camera, &environment, 1);
    }
    let (ball, floor) = colours(&stage.pixels(&gpu), x);
    assert!(
        ball[0] > floor[0] + 30.0,
        "the top of the ball moving right shows no motion: {ball:?} against {floor:?}"
    );

    stage.frames(&gpu, &camera, &environment, 1);
    let (ball, floor) = colours(&stage.pixels(&gpu), x);
    assert!(
        (ball[0] - floor[0]).abs() < 10.0,
        "it still shows motion after it stopped: {ball:?} against {floor:?}"
    );

    // A jump shows no motion, and neither does the first pose a character is given.
    x += 2.0;
    pose_top(&mut stage, Mat4::from_translation(Vec3::X * x), true);
    stage.frames(&gpu, &camera, &environment, 1);
    let (ball, floor) = colours(&stage.pixels(&gpu), x);
    assert!(
        (ball[0] - floor[0]).abs() < 10.0,
        "the pose that jumped shows motion: {ball:?} against {floor:?}"
    );

    // The first pose a character is given isn't a movement either.
    let mut stage = Stage::new(&gpu, &skinned_ball_on_floor(), settings, size);
    stage.frames(&gpu, &camera, &environment, 4);
    pose_top(&mut stage, Mat4::from_translation(Vec3::X), false);
    stage.frames(&gpu, &camera, &environment, 1);
    let (ball, floor) = colours(&stage.pixels(&gpu), 1.0);
    assert!(
        (ball[0] - floor[0]).abs() < 10.0,
        "the first pose shows motion: {ball:?} against {floor:?}"
    );
}

/// A cut-out card on one bone, for the bone to turn over.
fn skinned_card_scene() -> SceneAsset {
    let mut scene = card_scene(0, true, half_cut);
    let source = card_mesh(true, 8);
    let skins = vec![PackedSkin::new([0; 4], [1.0, 0.0, 0.0, 0.0]); source.vertices.len()];
    scene.meshes[0] =
        solstice_builder::build_skinned_mesh(&source, &skins, &BuildOptions::default());
    scene.skeletons = vec![solstice_asset::Skeleton {
        bones: vec![solstice_asset::Bone {
            inverse_bind: Mat4::IDENTITY,
            center: Vec3::ZERO,
            radius: 1.0,
            ..Default::default()
        }],
    }];
    scene.poses = vec![0];
    scene.instance_poses = vec![0];
    scene
}

/// A bone turning a cut-out half way round takes its hole with it, on both rasterizers.
#[test]
fn a_skinned_cut_out_keeps_its_holes_where_its_bones_take_it() {
    let Some(gpu) = request_gpu() else { return };
    let scene = skinned_card_scene();
    let settings = RendererSettings {
        debug_view: DebugView::Albedo,
        temporal_accumulation: false,
        ..Default::default()
    };
    for (distance, software) in [(1.0, false), (14.0, true)] {
        let camera = Camera {
            position: Vec3::new(0.0, 0.0, distance),
            ..Default::default()
        };
        let mut stage = Stage::new(&gpu, &scene, settings, 256);
        assert!(stage
            .renderer
            .place_pose(0, &[Mat4::from_rotation_z(std::f32::consts::PI)]));
        stage.frames(&gpu, &camera, &darkness(), 8);
        assert_eq!(
            stage.renderer.stats().software_clusters > 0,
            software,
            "at {distance} m"
        );
        let pixels = stage.pixels(&gpu);
        let at = |x: f32| {
            let (px, py) = pixel_of(&camera, 256, Vec3::new(x, 0.0, 0.0));
            brightness(patch(&pixels, 256, px, py, 0))
        };
        // The texture's left half is the hole, and its left half now lies on the right.
        let (solid, hole) = (at(-0.25), at(0.25));
        assert!(hole < 5.0, "at {distance} m the hole shows {hole}");
        assert!(
            solid > 100.0,
            "at {distance} m the solid part shows {solid}"
        );
    }
}

/// Covered pixels outside `fraction` of a unit sphere's radius on screen.
fn covered_beyond(pixels: &[u8], size: u32, distance: f32, fov_y: f32, fraction: f32) -> u32 {
    let angular = (1.0 / distance).asin();
    let half = size as f32 * 0.5;
    let outer = angular.tan() / (fov_y * 0.5).tan() * half * fraction;
    let mut covered = 0;
    for y in 0..size {
        for x in 0..size {
            let (dx, dy) = (x as f32 + 0.5 - half, y as f32 + 0.5 - half);
            if dx * dx + dy * dy > outer * outer && pixels[((y * size + x) * 4) as usize] >= 200 {
                covered += 1;
            }
        }
    }
    covered
}

/// A cut-out's shadow follows its bones too: a bone turns the card half way round, and the
/// shadow's hole goes with it.
#[test]
fn a_skinned_cut_outs_shadow_has_its_holes_where_its_bones_take_them() {
    let Some(gpu) = request_gpu() else { return };
    let card = Mat4::from_translation(Vec3::new(0.0, 1.0, 0.0))
        * Mat4::from_rotation_x(-std::f32::consts::FRAC_PI_2)
        * Mat4::from_scale(Vec3::new(2.0, 2.0, 1.0));
    let source = card_mesh(true, 8);
    let mut scene = card_over_floor(&source, card);
    let skins = vec![PackedSkin::new([0; 4], [1.0, 0.0, 0.0, 0.0]); source.vertices.len()];
    scene.meshes[0] =
        solstice_builder::build_skinned_mesh(&source, &skins, &BuildOptions::default());
    scene.skeletons = skinned_card_scene().skeletons;
    scene.poses = vec![0];
    scene.instance_poses = vec![0, solstice_asset::NO_POSE];
    let sun_direction = Vec3::new(0.5, 1.0, 0.0).normalize();
    let sun = Environment {
        sun_direction,
        sky_intensity: 0.0,
        gi_intensity: 0.0,
        ..Default::default()
    };
    let camera = Camera {
        position: Vec3::new(-0.5, 2.0, 2.5),
        pitch: -0.7,
        ..Default::default()
    };
    let settings = RendererSettings {
        temporal_accumulation: false,
        ..Default::default()
    };
    let mut stage = Stage::new(&gpu, &scene, settings, 512);
    assert!(stage
        .renderer
        .place_pose(0, &[Mat4::from_rotation_z(std::f32::consts::PI)]));
    stage.frames(&gpu, &camera, &sun, 8);
    let pixels = stage.pixels(&gpu);
    let floor = |local_x: f32| {
        let point = card.transform_point3(Vec3::new(local_x, 0.0, 0.0));
        let spot = point - sun_direction * (point.y / sun_direction.y);
        let (px, py) = pixel_of(&camera, 512, spot);
        brightness(patch(&pixels, 512, px, py, 1))
    };
    // Turned over, the solid half is on the -X side.
    let (solid, hole) = (floor(-0.25), floor(0.25));
    assert!(
        solid + 30.0 < hole,
        "the floor is {solid} behind the solid half and {hole} through the hole"
    );
}

/// Turning every bone turns the normals with them. A sphere turned is the same sphere, so its
/// image should be the unturned one's, which it wouldn't be if the normals stayed behind.
#[test]
fn turning_every_bone_turns_the_normals_with_it() {
    let Some(gpu) = request_gpu() else { return };
    let (camera, sun) = (over_the_floor(), low_sun());
    let settings = RendererSettings {
        debug_view: DebugView::Normals,
        ..untemporal()
    };
    let size = 192;
    let mut rigid = Stage::new(&gpu, &red_ball_on_floor(), settings, size);
    rigid.frames(&gpu, &camera, &sun, 3);
    let mut turned = Stage::new(&gpu, &skinned_ball_on_floor(), settings, size);
    let turn = Mat4::from_rotation_x(std::f32::consts::FRAC_PI_2);
    assert!(turned.renderer.place_pose(0, &[turn, turn]));
    turned.frames(&gpu, &camera, &sun, 3);
    let (rigid, turned) = (rigid.pixels(&gpu), turned.pixels(&gpu));
    let differing = rigid
        .chunks_exact(4)
        .zip(turned.chunks_exact(4))
        .filter(|(a, b)| a.iter().zip(b.iter()).any(|(x, y)| x.abs_diff(*y) > 12))
        .count();
    assert!(differing < 150, "{differing} pixels differ");
}

/// Everything about how a pose is found at once: a skinned mesh that isn't first in the scene,
/// so its bone lists are offset; a pose whose bone lies across a row of the bone texture; two
/// instances sharing a pose, one of them scaled; another pose left alone; and a pose wholly on
/// a later row, posed alone afterwards, which uploads that row and no other.
#[test]
fn each_instance_finds_its_own_pose() {
    use solstice_asset::{Bone, Skeleton, NO_POSE};
    let Some(gpu) = request_gpu() else { return };
    // Bone 141 of the second pose starts at texel 600 + 423, the last on the first row.
    const TOP: u8 = 141;
    let mut scene = red_ball_on_floor();
    let floor = scene.meshes.pop().unwrap();
    let source = sphere(4);
    let skins: Vec<PackedSkin> = source
        .vertices
        .iter()
        .map(|vertex| {
            let blend = ((vertex.position.y + 0.3) / 0.6).clamp(0.0, 1.0);
            PackedSkin::new([0, TOP, 0, 0], [1.0 - blend, blend, 0.0, 0.0])
        })
        .collect();
    let ball = solstice_builder::build_skinned_mesh(&source, &skins, &BuildOptions::default());
    scene.meshes = vec![floor, ball];
    let at = |x: f32, scale: f32| {
        Mat4::from_translation(Vec3::new(x, scale, 0.0)) * Mat4::from_scale(Vec3::splat(scale))
    };
    let placed = |mesh, material, transform| Instance {
        mesh,
        material,
        transform,
    };
    scene.instances = vec![
        placed(0, 0, Mat4::IDENTITY),
        placed(1, 1, at(-5.0, 1.0)),
        placed(1, 1, at(0.0, 1.0)),
        placed(1, 1, at(5.0, 2.0)),
        placed(1, 1, at(-2.5, 1.0)),
    ];
    let bone = Bone {
        name: String::new(),
        parent: None,
        translation: Vec3::ZERO,
        rotation: glam::Quat::IDENTITY,
        inverse_bind: Mat4::IDENTITY,
        center: Vec3::ZERO,
        radius: 1.1,
    };
    scene.skeletons = vec![Skeleton {
        bones: vec![bone; 200],
    }];
    scene.poses = vec![0, 0, 0];
    scene.instance_poses = vec![NO_POSE, 0, 1, 1, 2];

    let camera = over_the_floor();
    let mut stage = Stage::new(&gpu, &scene, untemporal(), 256);
    let mut bones = vec![Mat4::IDENTITY; 200];
    bones[usize::from(TOP)] = Mat4::from_translation(Vec3::Y * 1.5);
    assert!(stage.renderer.place_pose(1, &bones));
    stage.frames(&gpu, &camera, &low_sun(), 3);
    assert!(stage.renderer.place_pose(2, &bones));
    stage.frames(&gpu, &camera, &low_sun(), 3);
    let pixels = stage.pixels(&gpu);
    let red = |point: Vec3| {
        let (px, py) = pixel_of(&camera, 256, point);
        let [r, g, _] = patch(&pixels, 256, px, py, 1);
        r > 2.0 * g
    };
    assert!(red(Vec3::new(-5.0, 1.75, 0.0)), "the left ball isn't there");
    assert!(!red(Vec3::new(-5.0, 3.25, 0.0)), "the left ball moved");
    assert!(
        red(Vec3::new(0.0, 3.25, 0.0)),
        "the shared pose moved nothing"
    );
    assert!(
        red(Vec3::new(-2.5, 3.25, 0.0)),
        "the pose on the later row moved nothing"
    );
    assert!(
        red(Vec3::new(5.0, 6.5, 0.0)),
        "the scaled ball didn't go twice as far"
    );
}

/// A pose can carry a mesh right out of where it was built. Coarse clusters with too many bones
/// to list, and the hierarchy's nodes, are kept by the instance's reach.
#[test]
fn a_mesh_carried_far_from_its_bind_pose_is_still_drawn() {
    use solstice_asset::{Bone, Skeleton};
    let Some(gpu) = request_gpu() else { return };
    let source = sphere(6);
    const BONES: usize = 40;
    let skins: Vec<PackedSkin> = source
        .vertices
        .iter()
        .map(|vertex| {
            let turn = vertex.position.z.atan2(vertex.position.x) / std::f32::consts::TAU + 0.5;
            let bone = ((turn * BONES as f32) as usize).min(BONES - 1) as u8;
            PackedSkin::new([bone, 0, 0, 0], [1.0, 0.0, 0.0, 0.0])
        })
        .collect();
    let mesh = solstice_builder::build_skinned_mesh(&source, &skins, &BuildOptions::default());
    assert!(
        mesh.cluster_bones
            .iter()
            .any(|list| list[..2] == solstice_asset::gpu::BONES_OVERFLOW),
        "no cluster has too many bones to list"
    );
    let scene = SceneAsset {
        meshes: vec![mesh],
        materials: vec![white_material()],
        instances: vec![instance(0, Mat4::IDENTITY)],
        skeletons: vec![Skeleton {
            bones: vec![
                Bone {
                    name: String::new(),
                    parent: None,
                    translation: Vec3::ZERO,
                    rotation: glam::Quat::IDENTITY,
                    inverse_bind: Mat4::IDENTITY,
                    center: Vec3::ZERO,
                    radius: 1.1,
                };
                BONES
            ],
        }],
        poses: vec![0],
        instance_poses: vec![0],
        ..Default::default()
    };
    const SIZE: u32 = 512;
    let fov_y = 20.0_f32.to_radians();
    let settings = RendererSettings {
        debug_view: DebugView::Albedo,
        temporal_accumulation: false,
        ..Default::default()
    };
    // Far enough that the coarsest clusters are drawn, and near enough that fine ones are.
    for distance in [40.0_f32, 8.0] {
        let camera = Camera {
            position: Vec3::new(30.0, 0.0, distance),
            fov_y,
            near: 0.05,
            ..Default::default()
        };
        let mut stage = Stage::new(&gpu, &scene, settings, SIZE);
        let bones = vec![Mat4::from_translation(Vec3::X * 30.0); BONES];
        assert!(stage.renderer.place_pose(0, &bones));
        stage.frames(&gpu, &camera, &darkness(), 4);
        let pixels = stage.pixels(&gpu);
        let holes = holes_within(&pixels, SIZE, distance, fov_y, 0.9);
        assert_eq!(holes, 0, "{holes} holes at distance {distance}");
    }
}

/// Many phones, and Chrome's own software adapter, are given only the WebGPU defaults: four
/// storage textures and 16 sampled textures per stage, where the resolve and the ocean bind
/// more. With every fallback those limits choose, the sea, the ball and its floor draw without
/// an error, and as they draw on the same GPU with all it offers.
#[test]
fn the_webgpu_default_limits_draw_what_a_desktop_gpu_does() {
    let Some(full) = request_gpu() else { return };
    let Some(small) = request_gpu_within(&wgpu::Limits::defaults()) else {
        return;
    };
    assert!(
        small.fallbacks.separate_depth
            && small.fallbacks.ocean_depth_from_pyramid
            && small.fallbacks.pool_in_one_binding,
        "{:?}",
        small.fallbacks
    );
    // Over the water and the ball's shore at once.
    let camera = Camera {
        position: Vec3::new(8.0, 7.0, 8.0),
        yaw: 0.8,
        pitch: -0.4,
        ..Default::default()
    };
    for (debug, quality) in [
        (DebugView::Lit, Quality::High),
        (DebugView::Lit, Quality::Low),
        (DebugView::Ocean, Quality::High),
        (DebugView::Motion, Quality::High),
    ] {
        let size = 96;
        let expected = render_sea(&full, &camera, "ocean", 5.0, debug, quality, size);
        let got = render_sea(&small, &camera, "ocean", 5.0, debug, quality, size);
        let differing = expected
            .chunks_exact(4)
            .zip(got.chunks_exact(4))
            .filter(|(a, b)| a.iter().zip(*b).any(|(x, y)| x.abs_diff(*y) > 1))
            .count();
        assert_eq!(
            differing, 0,
            "{debug:?} at {quality:?}: {differing} pixels differ"
        );
        // Not a blank image either way.
        let distinct: std::collections::HashSet<&[u8]> = got.chunks_exact(4).collect();
        assert!(
            distinct.len() > 10,
            "{debug:?}: only {} colours",
            distinct.len()
        );
    }
}

/// A closed box from `min` to `max`, its faces flat-shaded.
fn box_mesh(min: Vec3, max: Vec3) -> Mesh {
    let mut mesh = Mesh::default();
    let faces = [
        (Vec3::X, Vec3::Z, Vec3::Y),
        (Vec3::NEG_X, Vec3::Y, Vec3::Z),
        (Vec3::Y, Vec3::X, Vec3::Z),
        (Vec3::NEG_Y, Vec3::Z, Vec3::X),
        (Vec3::Z, Vec3::Y, Vec3::X),
        (Vec3::NEG_Z, Vec3::X, Vec3::Y),
    ];
    let centre = (min + max) * 0.5;
    let half = (max - min) * 0.5;
    for (normal, u, v) in faces {
        let base = mesh.vertices.len() as u32;
        for (a, b) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
            mesh.vertices.push(Vertex {
                position: centre + (normal + u * a + v * b) * half,
                normal,
                uv: Vec2::new(a, b),
            });
        }
        // Counter-clockwise seen from outside.
        let outward = (u.cross(v)).dot(normal) > 0.0;
        let quad = if outward {
            [0, 1, 2, 0, 2, 3]
        } else {
            [0, 2, 1, 0, 3, 2]
        };
        mesh.indices.extend(quad.iter().map(|i| base + i));
    }
    mesh
}

/// A slender post standing on a grey floor, the post's material swaying in the wind if `sways`.
fn post_in_a_field(sways: bool) -> SceneAsset {
    let mut floor = Mesh::default();
    for (x, z) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
        floor.vertices.push(Vertex {
            position: Vec3::new(x * 50.0, 0.0, z * 50.0),
            normal: Vec3::Y,
            uv: Vec2::new(x, z),
        });
    }
    floor.indices = vec![0, 2, 1, 0, 3, 2];
    let options = BuildOptions::default();
    SceneAsset {
        meshes: vec![
            build_virtual_mesh(
                &box_mesh(Vec3::new(-0.04, 0.0, -0.04), Vec3::new(0.04, 0.8, 0.04)),
                &options,
            ),
            build_virtual_mesh(&floor, &options),
        ],
        materials: vec![
            Material {
                base_color: [1.0; 4],
                flags: if sways { Material::SWAYS } else { 0 },
                ..Default::default()
            },
            Material {
                base_color: [0.2, 0.2, 0.2, 1.0],
                ..Default::default()
            },
        ],
        instances: vec![
            instance(0, Mat4::IDENTITY),
            Instance {
                mesh: 1,
                material: 1,
                transform: Mat4::IDENTITY,
            },
        ],
        ..Default::default()
    }
}

/// Renders `scene` in `weather` seen from just south of the origin, looking north, with `debug`,
/// for `frames` frames `seconds` apart, and reads back the last.
fn render_in_wind(
    gpu: &Gpu,
    scene: &SceneAsset,
    weather: Weather,
    debug: DebugView,
    frames: u32,
    seconds: f32,
) -> Vec<u8> {
    let errors: Arc<Mutex<Vec<String>>> = Default::default();
    let sink = errors.clone();
    gpu.device
        .on_uncaptured_error(Arc::new(move |error: wgpu::Error| {
            sink.lock().unwrap().push(error.to_string());
        }));
    let settings = RendererSettings {
        debug_view: debug,
        temporal_accumulation: false,
        ..Default::default()
    };
    let mut renderer = Renderer::new(gpu, &paginate(scene), FORMAT, settings);
    renderer.set_weather(weather, 0.0);
    let camera = Camera {
        position: Vec3::new(0.0, 0.5, 2.5),
        fov_y: 40.0_f32.to_radians(),
        ..Default::default()
    };
    let target = test_target(gpu, 128);
    let view = target.create_view(&Default::default());
    for _ in 0..frames {
        renderer.render(gpu, &camera, &darkness(), &view, (128, 128), seconds);
        gpu.device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
    }
    let pixels = read_pixels(gpu, &target, 128);
    let errors = errors.lock().unwrap();
    assert!(errors.is_empty(), "GPU errors: {}", errors.join("\n"));
    pixels
}

/// A wind from the west, `speed` m/s at cloud height, as steady or gusty as asked.
fn westerly(speed: f32, gusts: f32) -> Weather {
    Weather {
        wind_speed: speed,
        wind_from: 270.0,
        gusts,
        turbulence: 0.0,
        ..Weather::by_name("clear").unwrap()
    }
}

/// The mean column of the post's white pixels in rows `rows`, from the albedo view.
fn post_column(pixels: &[u8], rows: std::ops::Range<u32>) -> f32 {
    let (mut sum, mut count) = (0.0, 0.0);
    for y in rows {
        for x in 0..128 {
            if pixels[((y * 128 + x) * 4) as usize] > 200 {
                sum += x as f32;
                count += 1.0;
            }
        }
    }
    assert!(count > 0.0, "no post in those rows");
    sum / count
}

#[test]
fn grass_that_sways_leans_downwind_and_nothing_else_does() {
    let Some(gpu) = request_gpu() else { return };
    let wind = westerly(14.0, 0.0);
    let swaying = render_in_wind(
        &gpu,
        &post_in_a_field(true),
        wind,
        DebugView::Albedo,
        6,
        1.0 / 60.0,
    );
    let standing = render_in_wind(
        &gpu,
        &post_in_a_field(false),
        wind,
        DebugView::Albedo,
        6,
        1.0 / 60.0,
    );
    // Its foot stays where it stands; its top leans towards +X, the right of the picture.
    // The post stands from row 99 up to row 43, the horizon at row 64.
    let (top, foot) = (44..56, 90..98);
    let lean = post_column(&swaying, top.clone()) - post_column(&standing, top.clone());
    let slip = post_column(&swaying, foot.clone()) - post_column(&standing, foot);
    assert!(lean > 3.0, "the top moved {lean} pixels");
    assert!(
        slip.abs() < lean * 0.3,
        "the foot moved {slip} pixels, the top {lean}"
    );
    // In still air it stands up straight.
    let still = render_in_wind(
        &gpu,
        &post_in_a_field(true),
        westerly(0.0, 0.0),
        DebugView::Albedo,
        6,
        1.0 / 60.0,
    );
    assert_eq!(still, standing);
    // Unflagged, a gale moves nothing.
    let gale = render_in_wind(
        &gpu,
        &post_in_a_field(false),
        westerly(40.0, 1.0),
        DebugView::Albedo,
        6,
        1.0 / 60.0,
    );
    assert_eq!(gale, standing);
}

#[test]
fn swaying_grass_says_how_it_moved_and_the_ground_does_not() {
    let Some(gpu) = request_gpu() else { return };
    // Gusty, and frames a tenth of a second apart, so it moves a good way from frame to frame.
    let wind = westerly(14.0, 1.0);
    let motion = render_in_wind(
        &gpu,
        &post_in_a_field(true),
        wind,
        DebugView::Motion,
        12,
        0.1,
    );
    let albedo = render_in_wind(
        &gpu,
        &post_in_a_field(true),
        wind,
        DebugView::Albedo,
        12,
        0.1,
    );
    // How far a pixel's motion reads from "no motion": the grey of the nearest floor, which
    // never moves.
    let still = ((127 * 128 + 64) * 4) as usize;
    let moved = |i: usize| -> u32 {
        u32::from(motion[i].abs_diff(motion[still]))
            + u32::from(motion[i + 1].abs_diff(motion[still + 1]))
    };
    let (mut post, mut posts, mut floor, mut floors) = (0, 0, 0, 0);
    for y in 44..128u32 {
        for x in 0..128u32 {
            let i = ((y * 128 + x) * 4) as usize;
            if albedo[i] > 240 {
                post += moved(i);
                posts += 1;
            } else if (100..150).contains(&albedo[i]) && !(40..100).contains(&x) {
                // Floor well clear of the post, whose edges blend with it, and of the far edge,
                // which blends with the sky.
                floor += moved(i);
                floors += 1;
            }
        }
    }
    assert!(
        posts > 20 && floors > 100,
        "{posts} post and {floors} floor pixels"
    );
    let post = post as f32 / posts as f32;
    let floor = floor as f32 / floors as f32;
    assert!(post > 3.0, "the post's motion reads {post}");
    assert!(floor < 2.0, "the floor's motion reads {floor}");
    assert!(
        post > floor * 10.0,
        "the post reads {post}, the floor {floor}"
    );
    // Standing still, the same post in the same wind reads no motion: below the horizon, where
    // it stands against the floor, it reads as the floor does.
    let standing = render_in_wind(
        &gpu,
        &post_in_a_field(false),
        wind,
        DebugView::Motion,
        12,
        0.1,
    );
    let mut most = 0;
    for i in (68 * 128 * 4..standing.len()).step_by(4) {
        most = most.max(
            u32::from(standing[i].abs_diff(standing[still]))
                + u32::from(standing[i + 1].abs_diff(standing[still + 1])),
        );
    }
    assert!(most <= 2, "motion {most} with nothing moving");
}

#[test]
fn grass_leaning_in_a_steady_wind_reads_only_how_it_moves_now() {
    let Some(gpu) = request_gpu() else { return };
    // A steady wind leans the post well over, but from one frame to the next only its shimmer
    // moves it. Motion that took the lean itself for movement would read many times more.
    let wind = westerly(14.0, 0.0);
    let motion = render_in_wind(
        &gpu,
        &post_in_a_field(true),
        wind,
        DebugView::Motion,
        12,
        1.0 / 60.0,
    );
    let albedo = render_in_wind(
        &gpu,
        &post_in_a_field(true),
        wind,
        DebugView::Albedo,
        12,
        1.0 / 60.0,
    );
    let still = ((127 * 128 + 64) * 4) as usize;
    let mut most = 0;
    let mut posts = 0;
    for y in 44..64u32 {
        for x in 0..128u32 {
            let i = ((y * 128 + x) * 4) as usize;
            if albedo[i] > 240 {
                posts += 1;
                most = most.max(
                    u32::from(motion[i].abs_diff(motion[still]))
                        + u32::from(motion[i + 1].abs_diff(motion[still + 1])),
                );
            }
        }
    }
    assert!(posts > 10, "only {posts} post pixels");
    assert!(most < 40, "the leaning post's motion reads {most}");
}

/// A floor with a roof standing over part of it, on four posts: a shelter in the rain.
fn shelter() -> SceneAsset {
    let mut floor = Mesh::default();
    for (x, z) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
        floor.vertices.push(Vertex {
            position: Vec3::new(x * 60.0, 0.0, z * 60.0),
            normal: Vec3::Y,
            uv: Vec2::new(x, z),
        });
    }
    floor.indices = vec![0, 2, 1, 0, 3, 2];
    let options = BuildOptions::default();
    let mut instances = vec![
        // The roof: 6 m square, 3 m up.
        instance(0, Mat4::IDENTITY),
        Instance {
            mesh: 1,
            material: 1,
            transform: Mat4::IDENTITY,
        },
    ];
    for (x, z) in [(-2.8, -2.8), (2.8, -2.8), (2.8, 2.8), (-2.8, 2.8)] {
        instances.push(instance(2, Mat4::from_translation(Vec3::new(x, 0.0, z))));
    }
    SceneAsset {
        meshes: vec![
            build_virtual_mesh(
                &box_mesh(Vec3::new(-3.0, 3.0, -3.0), Vec3::new(3.0, 3.2, 3.0)),
                &options,
            ),
            build_virtual_mesh(&floor, &options),
            build_virtual_mesh(
                &box_mesh(Vec3::new(-0.1, 0.0, -0.1), Vec3::new(0.1, 3.0, 0.1)),
                &options,
            ),
        ],
        materials: vec![
            white_material(),
            // Smooth, so a film shows at once.
            Material {
                base_color: [0.5, 0.5, 0.5, 1.0],
                roughness: 0.5,
                ..Default::default()
            },
        ],
        instances,
        ..Default::default()
    }
}

/// Renders the shelter after `seconds` of `weather` from `camera`, with `debug`, lit by a grey
/// day. SOLSTICE_TEST_IMAGES=DIR keeps the image.
fn render_shelter(
    gpu: &Gpu,
    weather: Weather,
    seconds: f32,
    debug: DebugView,
    camera: &Camera,
) -> Vec<u8> {
    let errors: Arc<Mutex<Vec<String>>> = Default::default();
    let sink = errors.clone();
    gpu.device
        .on_uncaptured_error(Arc::new(move |error: wgpu::Error| {
            sink.lock().unwrap().push(error.to_string());
        }));
    let settings = RendererSettings {
        debug_view: debug,
        temporal_accumulation: false,
        ..Default::default()
    };
    let mut renderer = Renderer::new(gpu, &paginate(&shelter()), FORMAT, settings);
    renderer.set_weather(weather, 0.0);
    renderer.advance_weather(seconds);
    let environment = Environment {
        sun_direction: Vec3::new(0.3, 0.8, 0.4).normalize(),
        ..Default::default()
    };
    let target = test_target(gpu, 128);
    let view = target.create_view(&Default::default());
    for _ in 0..10 {
        renderer.render(gpu, camera, &environment, &view, (128, 128), 1.0 / 60.0);
        gpu.device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
    }
    let pixels = read_pixels(gpu, &target, 128);
    let errors = errors.lock().unwrap();
    assert!(errors.is_empty(), "GPU errors: {}", errors.join(", "));
    if let Ok(dir) = std::env::var("SOLSTICE_TEST_IMAGES") {
        let name = format!(
            "{dir}/shelter_{debug:?}_{:.0}_{:.0}_{:.0}.ppm",
            weather.rain * 100.0,
            seconds * 10.0,
            camera.pitch * 100.0
        );
        let mut ppm = format!("P6\n{} {}\n255\n", 128, 128).into_bytes();
        ppm.extend(pixels.chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]]));
        std::fs::write(name, ppm).unwrap();
    }
    pixels
}

/// Beside the shelter, low, looking under its roof.
fn beside_the_shelter() -> Camera {
    Camera {
        position: Vec3::new(0.0, 1.2, 9.0),
        pitch: -0.12,
        fov_y: 50.0_f32.to_radians(),
        ..Default::default()
    }
}

/// How blue a patch of the wetness view is: blue for soaked, grey for dry.
fn wet_blue(pixels: &[u8], x: u32, y: u32) -> f32 {
    let [r, _, b] = patch(pixels, 128, x, y, 2);
    b - r
}

#[test]
fn rain_wets_the_open_ground_and_leaves_what_is_sheltered_dry() {
    let Some(gpu) = request_gpu() else { return };
    let storm = Weather::by_name("storm").unwrap();
    let camera = beside_the_shelter();
    let wet = render_shelter(&gpu, storm, 600.0, DebugView::Wetness, &camera);
    // The floor in the open, near the camera, and under the roof, beyond it.
    let open = wet_blue(&wet, 64, 110);
    let sheltered = wet_blue(&wet, 64, 67);
    assert!(open > 60.0, "the open floor reads {open}");
    // Under the roof it is only damp, even in a long storm.
    assert!(
        sheltered < open * 0.6,
        "under the roof reads {sheltered}, the open {open}"
    );
    // Without rain, nothing is wet.
    let clear = Weather::by_name("clear").unwrap();
    let dry = render_shelter(&gpu, clear, 600.0, DebugView::Wetness, &camera);
    assert!(wet_blue(&dry, 64, 110) < 5.0);
}

#[test]
fn rain_falls_in_the_open_and_not_under_a_roof() {
    let Some(gpu) = request_gpu() else { return };
    let storm = Weather::by_name("storm").unwrap();
    // Standing under the roof, looking out and up at its underside.
    let camera = Camera {
        position: Vec3::new(0.0, 1.0, 0.0),
        pitch: 0.35,
        fov_y: 70.0_f32.to_radians(),
        ..Default::default()
    };
    // The same storm a moment apart: its drops have moved, and little else has.
    let now = render_shelter(&gpu, storm, 30.0, DebugView::Lit, &camera);
    let later = render_shelter(&gpu, storm, 30.3, DebugView::Lit, &camera);
    let changed = |rows: std::ops::Range<u32>| {
        let mut count = 0;
        for y in rows.clone() {
            for x in 0..128u32 {
                let i = ((y * 128 + x) * 4) as usize;
                if (0..3).any(|c| now[i + c].abs_diff(later[i + c]) > 6) {
                    count += 1;
                }
            }
        }
        count as f32 / (rows.len() * 128) as f32
    };
    // The roof's underside fills the top of the picture; the open air beyond it, the bottom.
    let roof = changed(0..30);
    let open = changed(90..128);
    assert!(open > 0.01, "rain changed only {open} of the open air");
    assert!(
        roof < open * 0.2,
        "under the roof {roof}, in the open {open}"
    );
}

#[test]
fn the_wind_ruffles_a_pond() {
    let Some(gpu) = request_gpu() else { return };
    // Looking along a pond towards the sun, low, where ripples break up its glitter.
    let camera = Camera {
        position: Vec3::new(-55.0, 1.5, 0.0),
        yaw: (-1.0_f32).atan2(-0.2),
        pitch: -0.2,
        ..Default::default()
    };
    let pond = Pond::parse("-60,-60,-1.5,60,0.3").unwrap();
    let wind = |speed: f32| Weather {
        wind_speed: speed,
        wind_from: 200.0,
        gusts: 0.0,
        turbulence: 0.0,
        rain: 0.0,
        ..Weather::by_name("clear").unwrap()
    };
    // At cloud height: half that blows near the water, still air, a gentle breeze of 3 m/s and
    // a fresh one of 7, below the speed at which ripples stop growing.
    let still = render_ponds_in(&gpu, &camera, &[pond], None, wind(0.5));
    let breezy = render_ponds_in(&gpu, &camera, &[pond], None, wind(6.0));
    let gale = render_ponds_in(&gpu, &camera, &[pond], None, wind(14.0));
    if let Ok(dir) = std::env::var("SOLSTICE_TEST_IMAGES") {
        for (name, p) in [("still", &still), ("gale", &gale)] {
            let mut ppm = format!(
                "P6
{POND_SIZE} {POND_SIZE}
255
"
            )
            .into_bytes();
            ppm.extend(p.chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]]));
            std::fs::write(format!("{dir}/pond_{name}.ppm"), ppm).unwrap();
        }
    }
    // How far, on average over the water, an image is from still water's.
    let from_still = |image: &[u8]| {
        let mut sum = 0.0;
        let mut count = 0.0;
        for y in POND_SIZE / 2 + 8..POND_SIZE {
            for x in 0..POND_SIZE {
                let i = ((y * POND_SIZE + x) * 4) as usize;
                for c in 0..3 {
                    sum += f32::from(image[i + c].abs_diff(still[i + c]));
                }
                count += 3.0;
            }
        }
        sum / count
    };
    let (breeze, blow) = (from_still(&breezy), from_still(&gale));
    // The wind breaks the sun's glitter up, more the harder it blows. Still water is a mirror, so
    // the sun's image in it is small, and so is the share of the picture the wind can change.
    assert!(blow > 0.6, "a gale changed the water by only {blow}");
    assert!(
        breeze < blow,
        "a breeze changed it by {breeze}, a gale by {blow}"
    );
}

/// The shelter by night in a storm, a frame after lightning strikes ahead if `strike`.
fn render_flash(gpu: &Gpu, strike: bool) -> Vec<u8> {
    let settings = RendererSettings {
        temporal_accumulation: false,
        ..Default::default()
    };
    let mut renderer = Renderer::new(gpu, &paginate(&shelter()), FORMAT, settings);
    let mut storm = Weather::by_name("storm").unwrap();
    // No strikes but the one asked for.
    storm.thunder = 0.0;
    renderer.set_weather(storm, 0.0);
    let environment = Environment {
        sun_direction: Vec3::new(0.3, 0.6, 0.4).normalize(),
        sun_illuminance: 0.3,
        ..Default::default()
    };
    let camera = beside_the_shelter();
    let target = test_target(gpu, 128);
    let view = target.create_view(&Default::default());
    for frame in 0..40 {
        if strike && frame == 38 {
            renderer.strike_lightning();
        }
        renderer.render(gpu, &camera, &environment, &view, (128, 128), 1.0 / 60.0);
        gpu.device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
    }
    read_pixels(gpu, &target, 128)
}

#[test]
fn a_flash_of_lightning_lights_the_night() {
    let Some(gpu) = request_gpu() else { return };
    let dark = render_flash(&gpu, false);
    let lit = render_flash(&gpu, true);
    let bright = |p: &[u8], x: u32, y: u32| brightness(patch(p, 128, x, y, 3));
    // The open floor in front, and the sky above, light up.
    let (floor_dark, floor_lit) = (bright(&dark, 64, 112), bright(&lit, 64, 112));
    assert!(
        floor_lit > floor_dark * 1.5 + 5.0,
        "the floor goes from {floor_dark} to {floor_lit}"
    );
    let (sky_dark, sky_lit) = (bright(&dark, 64, 4), bright(&lit, 64, 4));
    assert!(
        sky_lit > sky_dark + 5.0,
        "the sky goes from {sky_dark} to {sky_lit}"
    );
}

/// Where a ball has just rolled past, the floor it uncovers takes the light the floor has, not
/// the light the ball had there. The diffuse GI is remembered from frame to frame, and once kept
/// the ball's light behind it for many frames: by day the sun outshines it, but at dusk and by
/// night that light is most of what is seen, and everything that moved left a trail.
#[test]
fn indirect_light_leaves_no_trail_behind_what_moves() {
    let Some(gpu) = request_gpu() else { return };
    let size = 128;
    let camera = over_the_floor();
    // Dusk: the sun low and a fiftieth as bright, the sky lighting most of the floor.
    let dusk = Environment {
        sun_illuminance: low_sun().sun_illuminance / 50.0,
        daylight_illuminance: low_sun().sun_illuminance,
        ..low_sun()
    };
    // The diffuse GI alone, without the temporal filter that would smooth it.
    let settings = RendererSettings {
        debug_view: DebugView::DiffuseGi,
        ..untemporal()
    };
    let (start, end) = (2.0, -2.0);
    let mut moving = Stage::new(&gpu, &red_ball_on_floor(), settings, size);
    moving.renderer.move_instance(0, place(start));
    moving.frames(&gpu, &camera, &dusk, 30);
    let steps = 20;
    for step in 1..=steps {
        let x = start + (end - start) * step as f32 / steps as f32;
        moving.renderer.move_instance(0, place(x));
        moving.frames(&gpu, &camera, &dusk, 1);
    }
    let moved = moving.pixels(&gpu);
    let mut still = Stage::new(&gpu, &red_ball_on_floor(), settings, size);
    still.renderer.move_instance(0, place(end));
    still.frames(&gpu, &camera, &dusk, 60);
    let settled = still.pixels(&gpu);

    // The floor the ball crossed and hid from view, from just behind where it stands to where it
    // set off. It kept a trail of 4.8 on a floor of 110 here, against 1.4 since.
    let mut trail = 0.0;
    let mut floor = 0.0;
    let mut count = 0.0;
    for step in 0..=8 {
        let x = end + 1.6 + (start - end - 1.6) * step as f32 / 8.0;
        for z in [-2.5, -2.0, -1.5, -1.0, -0.5, 0.0] {
            let (px, py) = pixel_of(&camera, size, Vec3::new(x, 0.0, z));
            let a = brightness(patch(&moved, size, px, py, 0));
            let b = brightness(patch(&settled, size, px, py, 0));
            trail += (a - b).abs();
            floor += b;
            count += 1.0;
        }
    }
    let (trail, floor) = (trail / count, floor / count);
    assert!(
        floor > 20.0,
        "the floor's indirect light is too dim to tell: {floor}"
    );
    assert!(
        trail < floor * 0.025,
        "the ball left a trail in the indirect light: {trail:.1} on a floor of {floor:.1}"
    );
}

/// The editor's pick finds the instance under a point of the screen, and nothing on the sky.
#[test]
fn a_pick_finds_the_instance_under_a_point() {
    let Some(gpu) = request_gpu() else { return };
    let ball = build_virtual_mesh(&sphere(4), &BuildOptions::default());
    // Three balls in a row across the view: instance 0 left, 1 in the middle, 2 right.
    let scene = SceneAsset {
        meshes: vec![ball],
        materials: vec![white_material()],
        instances: [-3.0, 0.0, 3.0]
            .iter()
            .map(|&x| instance(0, Mat4::from_translation(Vec3::new(x, 0.0, 0.0))))
            .collect(),
        ..Default::default()
    };
    let camera = Camera {
        position: Vec3::new(0.0, 0.0, 10.0),
        ..Default::default()
    };
    let size = 256;
    let mut renderer = Renderer::new(&gpu, &paginate(&scene), FORMAT, Default::default());
    let target = test_target(&gpu, size);
    let view = target.create_view(&Default::default());
    let mut pick = |x: f32, y: f32| {
        renderer.request_pick(x, y);
        for _ in 0..4 {
            renderer.render(&gpu, &camera, &darkness(), &view, (size, size), 1.0 / 60.0);
            gpu.device
                .poll(wgpu::PollType::wait_indefinitely())
                .unwrap();
            if let Some(picked) = renderer.take_pick() {
                return picked;
            }
        }
        panic!("the pick at {x}, {y} never came back");
    };
    // A frame first, so the depth history exists.
    pick(0.5, 0.5);
    let middle = pick(0.5, 0.5);
    assert_eq!(middle.instance, Some(1));
    // The ball's near face is 9 m away: reversed depth is near / distance.
    let distance = camera.near / middle.depth;
    assert!(
        (distance - 9.0).abs() < 0.1,
        "the middle ball is {distance} m away"
    );
    // 3 m left of centre at 10 m, with a 55 degree view, is about 0.2 of the screen across.
    assert_eq!(pick(0.21, 0.5).instance, Some(0));
    assert_eq!(pick(0.79, 0.5).instance, Some(2));
    assert_eq!(pick(0.5, 0.05).instance, None, "the sky above them");
}

/// A map of ruts with one square pressed a little everywhere.
struct OnePressedSquare;

impl solstice_render::RutSource for OnePressedSquare {
    fn cell(&self) -> f32 {
        0.04
    }
    fn is_empty(&self) -> bool {
        false
    }
    fn version(&self, tile: (i32, i32)) -> u32 {
        u32::from(tile == (0, 0))
    }
    fn levels(&self, tile: (i32, i32), layer: usize) -> Option<Vec<Vec<u8>>> {
        use solstice_render::ruts::{LEVELS, TILE};
        (tile == (0, 0)).then(|| {
            let cell: [u8; 4] = if layer == 0 {
                [127, 0, 128, 200]
            } else {
                [128, 128, 200, 255]
            };
            (0..LEVELS)
                .map(|level| cell.repeat(((TILE >> level) * (TILE >> level)) as usize))
                .collect()
        })
    }
}

/// The ruts' texture is a stand-in until the first press, and is made whole then: the frames
/// either side of that draw without errors, with the resolve pass bound to the new texture.
#[test]
fn ruts_are_drawn_from_the_first_press_without_errors() {
    let Some(gpu) = request_gpu() else { return };
    let mesh = build_virtual_mesh(&sphere(2), &BuildOptions::default());
    let scene = SceneAsset {
        meshes: vec![mesh],
        materials: vec![white_material()],
        instances: vec![instance(0, Mat4::IDENTITY)],
        ..Default::default()
    };
    let camera = Camera {
        position: Vec3::new(0.0, 0.0, 5.0),
        ..Default::default()
    };
    let errors: Arc<Mutex<Vec<String>>> = Default::default();
    let sink = errors.clone();
    gpu.device
        .on_uncaptured_error(Arc::new(move |error: wgpu::Error| {
            sink.lock().unwrap().push(error.to_string());
        }));
    let mut renderer = Renderer::new(&gpu, &paginate(&scene), FORMAT, RendererSettings::default());
    let target = test_target(&gpu, 64);
    let view = target.create_view(&Default::default());
    for _ in 0..2 {
        renderer.render(&gpu, &camera, &darkness(), &view, (64, 64), 1.0 / 60.0);
    }
    assert_eq!(renderer.sync_ruts(glam::Vec2::ZERO, &OnePressedSquare), 1);
    for _ in 0..2 {
        renderer.render(&gpu, &camera, &darkness(), &view, (64, 64), 1.0 / 60.0);
    }
    // Nothing changed since: nothing is copied again.
    assert_eq!(renderer.sync_ruts(glam::Vec2::ZERO, &OnePressedSquare), 0);
    renderer.render(&gpu, &camera, &darkness(), &view, (64, 64), 1.0 / 60.0);
    gpu.device
        .poll(wgpu::PollType::wait_indefinitely())
        .unwrap();
    let errors = errors.lock().unwrap();
    assert!(errors.is_empty(), "GPU errors: {}", errors.join("\n"));
}
