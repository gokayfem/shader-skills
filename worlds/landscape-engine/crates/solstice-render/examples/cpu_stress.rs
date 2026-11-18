//! Repeatable CPU scaling measurements, with no quality settings or GPU involved.
//! Run: cargo run --release -p solstice-render --example cpu_stress

use bytemuck::Zeroable;
use glam::{Mat4, Vec3};
use solstice_render::{motion::InstanceTable, scene::GpuInstance, streaming::residency::Residency};
use std::{hint::black_box, time::Instant};

fn table(count: u32) -> InstanceTable {
    let mut table = InstanceTable::new(
        vec![GpuInstance::zeroed(); count as usize],
        (0..count).map(Some).collect(),
        vec![(Vec3::ZERO, 1.0); count as usize],
    );
    // Initialise the transforms through the same public API used by the renderer.
    for i in 0..count {
        table.set_transform(i, Mat4::IDENTITY, true);
    }
    table.advance();
    table
}

fn report(case: &str, count: u32, mut samples: Vec<f64>, checksum: u64) {
    samples.sort_by(f64::total_cmp);
    println!(
        "{case},{count},{:.6},{:.6},{checksum}",
        samples[samples.len() / 2],
        samples[samples.len() - 1]
    );
}

fn hash(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325, |h, &b| {
        (h ^ u64::from(b)).wrapping_mul(0x100000001b3)
    })
}

fn main() {
    println!("case,count,median_ms,max_ms,checksum");
    for count in [1_024, 8_192, 32_768] {
        let mut table = table(count);
        let mut reads = Vec::new();
        let mut updates = Vec::new();
        for round in 0..7 {
            let start = Instant::now();
            for i in 0..count {
                // A common game loop: read an object's current transform, then move it.
                let mut transform = table.transform(black_box(i)).unwrap();
                transform.w_axis.x += 1.0;
                table.set_transform(i, transform, false);
            }
            if round > 0 {
                reads.push(start.elapsed().as_secs_f64() * 1_000.0);
            }
            let start = Instant::now();
            black_box(table.advance());
            if round > 0 {
                updates.push(start.elapsed().as_secs_f64() * 1_000.0);
            }
        }
        for i in 0..count {
            assert_eq!(table.transform(i).unwrap().w_axis.x, 7.0);
        }
        let checksum = hash(bytemuck::cast_slice(table.gpu_data(0..count as usize)));
        report("read_move", count, reads, checksum);
        report("advance", count, updates, checksum);
    }
    for count in [1_024, 8_192, 32_768] {
        let mut table = table(count);
        let mut samples = Vec::new();
        for round in 0..7 {
            let start = Instant::now();
            // Multiple simulation ticks before drawing, submitted in reverse instance order.
            for tick in 0..8 {
                for i in (0..count).rev() {
                    table.set_transform(
                        i,
                        Mat4::from_translation(Vec3::new(i as f32, tick as f32, 0.0)),
                        tick == 7,
                    );
                }
            }
            black_box(table.advance());
            if round > 0 {
                samples.push(start.elapsed().as_secs_f64() * 1_000.0);
            }
        }
        let checksum = hash(bytemuck::cast_slice(table.gpu_data(0..count as usize)));
        report("eight_updates", count, samples, checksum);
    }
    for count in [4_096u32, 65_536, 1_048_576] {
        let mut dependencies = vec![vec![0]; count as usize];
        dependencies[0].clear();
        let mut residency = Residency::new(dependencies, 1, 512);
        let requests: Vec<u32> = (0..count)
            .map(|i| {
                if i % 8 == 0 {
                    2 + i.wrapping_mul(2654435761) % 1024
                } else {
                    0
                }
            })
            .collect();
        let mut samples = Vec::new();
        let mut checksum = 0;
        for frame in 0..21 {
            let start = Instant::now();
            let fetched = residency.plan(frame, black_box(&requests), 32);
            if frame > 0 {
                samples.push(start.elapsed().as_secs_f64() * 1_000.0);
            }
            checksum ^= hash(bytemuck::cast_slice(&fetched));
        }
        report("stream_plan", count, samples, checksum);
    }
}
