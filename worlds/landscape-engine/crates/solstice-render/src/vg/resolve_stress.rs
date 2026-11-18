//! Exercise the production interpolation on triangles crossing the eye plane.
use crate::gpu::{Gpu, LayoutBuilder};
use glam::{Vec3, Vec4};

#[test]
fn barycentrics_stay_finite_when_a_vertex_is_on_the_eye_plane() {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::PRIMARY,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let Ok(gpu) = pollster::block_on(Gpu::request(
        &instance,
        None,
        wgpu::PowerPreference::HighPerformance,
    )) else {
        eprintln!("skipping interpolation test: no GPU");
        return;
    };
    // Compile the exact function used by the G-buffer resolve, not a reimplementation.
    let source = include_str!("../../shaders/vg/resolve.wgsl");
    let start = source.find("fn barycentrics(").unwrap();
    let end = start + source[start..].find("\n}").unwrap() + 2;
    let source = format!(
        "{}\n{}\n{}",
        include_str!("../../shaders/common/math.wgsl"),
        &source[start..end],
        r#"
struct Case { clip: array<vec4f, 3>, ndc: vec4f };
@group(0) @binding(0) var<storage, read> cases: array<Case>;
@group(0) @binding(1) var<storage, read_write> results: array<vec4f>;
@compute @workgroup_size(1)
fn main(@builtin(global_invocation_id) id: vec3u) {
    results[id.x] = vec4f(barycentrics(cases[id.x].clip, cases[id.x].ndc.xy), 1.0);
}
"#
    );
    let module = gpu
        .device
        .create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("production barycentrics stress"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
    let layout = LayoutBuilder::new()
        .storage_read()
        .storage_write()
        .build(&gpu, "barycentric stress");
    let pipeline = gpu.compute_pipeline("barycentric stress", &module, "main", &[&layout]);
    let mut data = Vec::<Vec4>::new();
    let mut expected = Vec::new();
    for w in [1.0, 0.0, -0.25, 1.0e-20, 10.0] {
        let clip = [
            Vec4::new(-1.0, -1.0, 0.1, w),
            Vec4::new(1.0, -1.0, 0.1, 2.0),
            Vec4::new(0.0, 1.0, 0.1, 1.0),
        ];
        for b in [
            Vec3::new(0.2, 0.3, 0.5),
            Vec3::new(0.01, 0.01, 0.98),
            Vec3::new(-0.01, 0.5, 0.51),
        ] {
            let p = clip[0] * b.x + clip[1] * b.y + clip[2] * b.z;
            data.extend(clip);
            data.push(p / p.w);
            expected.push(b);
        }
    }
    // A zero-area triangle cannot supply interpolation, but must not poison later lighting.
    data.extend([Vec4::ONE; 4]);
    expected.push(Vec3::X);
    let input = gpu.storage_buffer_init("barycentric cases", &data);
    let size = expected.len() as u64 * 16;
    let output = gpu.storage_buffer("barycentric results", size, wgpu::BufferUsages::COPY_SRC);
    let group = gpu.bind_group(
        "barycentric stress",
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
        pass.dispatch_workgroups(expected.len() as u32, 1, 1);
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
    let results: &[Vec4] = bytemuck::cast_slice(&mapping);
    for (i, (result, wanted)) in results.iter().zip(expected).enumerate() {
        assert!(
            result.is_finite() && (result.truncate() - wanted).abs().max_element() < 2.0e-5,
            "case {i}: expected {wanted}, got {result}"
        );
    }
}
