//! Exercises the real traversal and control shaders beyond one dispatch dimension.
use super::*;
use crate::scene::GpuInstance;
use solstice_asset::{PackedCluster, PackedNode};

fn read_words(gpu: &Gpu, buffer: &wgpu::Buffer, bytes: u64) -> Vec<u32> {
    let staging = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("cull stress readback"),
        size: bytes,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    encoder.copy_buffer_to_buffer(buffer, 0, &staging, 0, bytes);
    gpu.queue.submit([encoder.finish()]);
    staging
        .slice(..)
        .map_async(wgpu::MapMode::Read, |r| r.unwrap());
    gpu.device
        .poll(wgpu::PollType::wait_indefinitely())
        .unwrap();
    bytemuck::cast_slice(&staging.slice(..).get_mapped_range().unwrap()).to_vec()
}

#[test]
fn traversal_visits_every_entry_beyond_one_dispatch_dimension() {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::PRIMARY,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let gpu = match pollster::block_on(Gpu::request(
        &instance,
        None,
        wgpu::PowerPreference::HighPerformance,
    )) {
        Ok(gpu) => gpu,
        Err(error) => {
            eprintln!("skipping GPU stress test: {error}");
            return;
        }
    };
    eprintln!("cull stress adapter: {:?}", gpu.adapter_info);
    let pipelines = CullPipelines::new(&gpu, shaders::Deform::default(), None);
    let state = gpu.storage_buffer(
        "stress state",
        STATE_SIZE,
        wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC,
    );
    let args = gpu.storage_buffer(
        "stress args",
        ARGS_SIZE,
        wgpu::BufferUsages::INDIRECT | wgpu::BufferUsages::COPY_SRC,
    );
    let control = gpu.bind_group(
        "stress control",
        &pipelines.control_layout,
        &[state.as_entire_binding(), args.as_entire_binding()],
    );

    let slots = ViewSlots::new(&gpu, 1);
    slots.write(
        &gpu,
        0,
        &PassView::orthographic(Mat4::IDENTITY, Vec3::ZERO, Vec2::splat(16.0), 1.0, 1.0),
    );
    let mut placed = GpuInstance::zeroed();
    placed.set_world(Mat4::IDENTITY);
    placed.set_bones(crate::skin::NO_BONES);
    let instances = gpu.storage_buffer_init("stress instance", &[placed]);
    // Each input appends exactly one entry. No rasterization or giant scene is needed to
    // prove that the actual process shader visits the entire input, including its tail.
    let nodes = gpu.storage_buffer_init(
        "stress node",
        &[PackedNode {
            radius: 1.0,
            max_parent_error: solstice_asset::gpu::ROOT_PARENT_ERROR,
            count: 1,
            ..Default::default()
        }],
    );
    let clusters = gpu.storage_buffer_init("stress cluster", &[PackedCluster::zeroed()]);
    let texture = |format| {
        gpu.device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("stress placeholder"),
                size: wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&Default::default())
    };
    let float_view = texture(wgpu::TextureFormat::Rgba32Float);
    let uint_view = texture(wgpu::TextureFormat::Rgba32Uint);
    let scene = gpu.bind_group(
        "stress scene",
        &pipelines.scene_layout,
        &[
            slots.binding(),
            instances.as_entire_binding(),
            nodes.as_entire_binding(),
            clusters.as_entire_binding(),
            wgpu::BindingResource::TextureView(&float_view),
            wgpu::BindingResource::TextureView(&float_view),
            wgpu::BindingResource::TextureView(&uint_view),
        ],
    );
    let queue = gpu.storage_buffer(
        "stress queue",
        ((1 << 22) * 2 + 64) * 8,
        wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
    );
    let scratch = gpu.storage_buffer("stress scratch", 16, wgpu::BufferUsages::empty());
    // Separate buffers avoid writable binding aliasing.
    let visible = gpu.storage_buffer("stress visible", 16, wgpu::BufferUsages::empty());
    let requests = gpu.storage_buffer("stress requests", 16, wgpu::BufferUsages::empty());
    let work = gpu.bind_group(
        "stress work",
        &pipelines.work_layout,
        &[
            state.as_entire_binding(),
            queue.as_entire_binding(),
            scratch.as_entire_binding(),
            visible.as_entire_binding(),
            requests.as_entire_binding(),
        ],
    );
    for count in [0, 1, 31, 32, 33, 65_535 * 32, 65_535 * 32 + 1, 1 << 22] {
        for begin in [0, 37] {
            let mut words = [0u32; STATE_SIZE as usize / 4];
            words[1] = begin;
            words[2] = begin + count;
            gpu.queue
                .write_buffer(&state, 0, bytemuck::cast_slice(&words));
            let mut encoder = gpu.device.create_command_encoder(&Default::default());
            {
                let mut pass = encoder.begin_compute_pass(&Default::default());
                pass.set_pipeline(&pipelines.advance);
                pass.set_bind_group(0, &control, &[]);
                pass.dispatch_workgroups(1, 1, 1);
            }
            gpu.queue.submit([encoder.finish()]);
            let dispatch = read_words(&gpu, &args, 12);
            assert!(
                dispatch.iter().all(|&n| n <= 65_535),
                "{count} queued entries produced an invalid WebGPU dispatch: {dispatch:?}"
            );
            let needed = count.div_ceil(32);
            assert!(dispatch[0] * dispatch[1] >= needed);
            assert!(dispatch[0] * dispatch[1] < needed + dispatch[1]);
            let mut encoder = gpu.device.create_command_encoder(&Default::default());
            {
                let mut pass = encoder.begin_compute_pass(&Default::default());
                pass.set_pipeline(&pipelines.process);
                pass.set_bind_group(0, &scene, &[0]);
                pass.set_bind_group(1, &work, &[]);
                pass.dispatch_workgroups_indirect(&args, 0);
            }
            gpu.queue.submit([encoder.finish()]);
            let result = read_words(&gpu, &state, STATE_SIZE);
            assert_eq!(result[2], begin + count * 2, "count={count}, begin={begin}");
            assert_eq!(result[9], 0, "no entry should be dropped");
        }
    }

    // Distant child subtrees must be rejected before they consume queue capacity.
    // Values exactly at the threshold match process's existing <= comparison.
    let errors = [
        0.0,
        0.5,
        1.0,
        1.0001,
        2.0,
        solstice_asset::gpu::ROOT_PARENT_ERROR,
    ];
    let mut children = vec![PackedNode {
        radius: 1.0,
        max_parent_error: solstice_asset::gpu::ROOT_PARENT_ERROR,
        first: 1,
        count: 120,
        ..Default::default()
    }];
    children.extend((0..120).map(|index| PackedNode {
        radius: 1.0,
        max_parent_error: errors[index % errors.len()],
        ..Default::default()
    }));
    let child_nodes = gpu.storage_buffer_init("child LOD nodes", &children);
    let child_scene = gpu.bind_group(
        "child LOD scene",
        &pipelines.scene_layout,
        &[
            slots.binding(),
            instances.as_entire_binding(),
            child_nodes.as_entire_binding(),
            clusters.as_entire_binding(),
            wgpu::BindingResource::TextureView(&float_view),
            wgpu::BindingResource::TextureView(&float_view),
            wgpu::BindingResource::TextureView(&uint_view),
        ],
    );
    let mut words = [0u32; STATE_SIZE as usize / 4];
    words[2] = 1;
    gpu.queue
        .write_buffer(&state, 0, bytemuck::cast_slice(&words));
    gpu.queue.write_buffer(&queue, 0, &[0u8; 8]);
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_compute_pass(&Default::default());
        pass.set_pipeline(&pipelines.advance);
        pass.set_bind_group(0, &control, &[]);
        pass.dispatch_workgroups(1, 1, 1);
        pass.set_pipeline(&pipelines.process);
        pass.set_bind_group(0, &child_scene, &[0]);
        pass.set_bind_group(1, &work, &[]);
        pass.dispatch_workgroups_indirect(&args, 0);
    }
    gpu.queue.submit([encoder.finish()]);
    let result = read_words(&gpu, &state, STATE_SIZE);
    let expected: Vec<u32> = (0..120)
        .filter(|&i| errors[i % errors.len()] > 1.0)
        .map(|i| i as u32 + 1)
        .collect();
    assert_eq!(result[2], expected.len() as u32 + 1);
    assert_eq!(result[9], 0);
    let entries = read_words(&gpu, &queue, u64::from(result[2]) * 8);
    let actual: Vec<u32> = entries[2..].chunks_exact(2).map(|entry| entry[1]).collect();
    assert_eq!(
        actual, expected,
        "only children that process would visit may be queued"
    );
}
