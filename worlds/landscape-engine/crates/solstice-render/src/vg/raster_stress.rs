//! Compare production sample bounds with an exhaustive viewport scan on the GPU.
use crate::gpu::{test_gpu, LayoutBuilder};

#[test]
fn tiny_triangles_keep_every_covered_sample() {
    let Some(gpu) = test_gpu() else { return };
    let source = include_str!("../../shaders/vg/raster_sw_body.wgsl");
    let source = &source[source.find("fn edge_f(").unwrap()..];
    let first = source.find("    let first =").unwrap();
    let end = first + source[first..].find("    if first.x").unwrap();
    // The reference visits all 256 samples, even those outside the triangle's bounding
    // box. Only the unchanged edge/fill/depth rules decide which ones it writes.
    let reference = format!(
        "{}    let first = vec2i(0);\n    let last = vec2i(15);\n{}",
        &source[..first],
        &source[end..]
    );
    let header = r#"
const SUBPIXEL: f32 = 256.0;
const SUBPIXEL_I: i32 = 256;
const MAX_EXTENT_PX: f32 = 512.0;
struct View { viewport: vec2f }
const view = View(vec2f(16.0));
struct Triangle { a: vec4f, b: vec4f, c: vec4f }
@group(0) @binding(0) var<storage, read> triangles: array<Triangle>;
@group(0) @binding(1) var<storage, read_write> pixels: array<u32>;
fn depth_bias(a: vec3f, b: vec3f, c: vec3f) -> f32 { return 0.0; }
fn cover_triangle(x: u32, y: u32, z: u32, a: vec3f, b: vec3f, c: vec3f, instance: u32) {}
fn covered(a: f32, b: f32, c: f32, pixel: vec2f) -> bool { return true; }
fn write_visibility(pixel: u32, depth: f32, id: u32) {
    pixels[id * 256u + pixel] = bitcast<u32>(depth);
}
@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3u) {
    if id.x >= arrayLength(&triangles) { return; }
    let t = triangles[id.x];
    rasterize_triangle(t.a.xyz, t.b.xyz, t.c.xyz, id.x, t.a.w > 0.0, vec3u(0u, 1u, 2u), 0u);
}
"#;
    let mut seed = 917u32;
    let mut random = || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        seed as f32 / u32::MAX as f32
    };
    let mut cases: Vec<[[f32; 4]; 3]> = Vec::new();
    for n in 0..8192 {
        let x = random() * 20.0 - 2.0;
        let y = random() * 20.0 - 2.0;
        let extent = if n % 3 == 0 { 0.01 } else { 5.0 };
        cases.push([
            [x, y, 0.1, (n % 2) as f32],
            [x + random() * extent, y + random() * extent, 0.9, 0.0],
            [x + random() * extent, y - random() * extent, 0.5, 0.0],
        ]);
    }
    // Quantization ties, exact pixel centres, top/left shared edges and clipped edges.
    for step in -260..=260 {
        let x = 7.5 + step as f32 / 512.0;
        for y in [-0.5, 0.5, 7.5, 15.5, 16.5] {
            cases.push([
                [x, y, 0.2, 1.0],
                [x + 0.01, y + 2.0, 0.7, 0.0],
                [x + 2.0, y, 0.9, 0.0],
            ]);
            cases.push([
                [x, y, 0.2, 0.0],
                [x + 2.0, y, 0.7, 0.0],
                [x, y + 0.01, 0.9, 0.0],
            ]);
        }
    }
    let input = gpu.storage_buffer_init("sample bounds cases", &cases);
    let size = cases.len() as u64 * 256 * 4;
    let layout = LayoutBuilder::new()
        .storage_read()
        .storage_write()
        .build(&gpu, "sample bounds");
    let run = |body: &str| {
        let module = gpu
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("sample bounds"),
                source: wgpu::ShaderSource::Wgsl(format!("{header}\n{body}").into()),
            });
        let pipeline = gpu.compute_pipeline("sample bounds", &module, "main", &[&layout]);
        let output = gpu.storage_buffer("samples", size, wgpu::BufferUsages::COPY_SRC);
        let group = gpu.bind_group(
            "sample bounds",
            &layout,
            &[input.as_entire_binding(), output.as_entire_binding()],
        );
        let staging = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.dispatch_workgroups((cases.len() as u32).div_ceil(64), 1, 1);
        }
        encoder.copy_buffer_to_buffer(&output, 0, &staging, 0, size);
        gpu.queue.submit([encoder.finish()]);
        staging
            .slice(..)
            .map_async(wgpu::MapMode::Read, |r| r.unwrap());
        gpu.device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
        let mapping = staging.slice(..).get_mapped_range().unwrap();
        bytemuck::cast_slice::<u8, u32>(&mapping).to_vec()
    };
    let expected = run(&reference);
    let actual = run(source);
    assert!(expected.iter().filter(|&&v| v != 0).count() > 10_000);
    for (pixel, (actual, expected)) in actual.iter().zip(expected).enumerate() {
        assert_eq!(
            *actual,
            expected,
            "triangle {}, sample ({}, {})",
            pixel / 256,
            pixel % 16,
            pixel / 16 % 16
        );
    }
}
