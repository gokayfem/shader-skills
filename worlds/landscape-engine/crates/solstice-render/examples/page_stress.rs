//! Lossless page-packing measurements on connected and disconnected geometry.
//! Run: cargo run --release -p solstice-render --example page_stress
use glam::{Mat4, Vec2, Vec3};
use solstice_asset::{Instance, Material, SceneAsset};
use solstice_builder::{build_virtual_mesh, paginate, BuildOptions, Mesh, Vertex};
use std::time::Instant;

fn grid(side: u32) -> Mesh {
    let mut mesh = Mesh::default();
    for z in 0..=side {
        for x in 0..=side {
            let (x, z) = (x as f32, z as f32);
            mesh.vertices.push(Vertex {
                position: Vec3::new(x, (x * 0.07).sin() * (z * 0.05).cos(), z),
                normal: Vec3::Y,
                uv: Vec2::new(x, z) / side as f32,
            });
        }
    }
    for z in 0..side {
        for x in 0..side {
            let a = z * (side + 1) + x;
            let b = a + side + 1;
            mesh.indices
                .extend_from_slice(&[a, b, a + 1, a + 1, b, b + 1]);
        }
    }
    mesh
}

fn blades(count: u32) -> Mesh {
    let mut mesh = Mesh::default();
    for i in 0..count {
        let x = (i % 128) as f32 * 0.15;
        let z = (i / 128) as f32 * 0.15;
        let start = mesh.vertices.len() as u32;
        for (p, uv) in [
            (Vec3::new(x, 0.0, z), Vec2::ZERO),
            (Vec3::new(x + 0.05, 0.0, z), Vec2::X),
            (Vec3::new(x + 0.03, 0.5 + (i % 7) as f32 * 0.01, z), Vec2::Y),
        ] {
            mesh.vertices.push(Vertex {
                position: p,
                normal: Vec3::Z,
                uv,
            });
        }
        mesh.indices
            .extend_from_slice(&[start, start + 1, start + 2]);
    }
    mesh
}

fn main() {
    println!("case,source_triangles,clusters,pages,raw_bytes,deflated_bytes,pool_bytes,packing_ms");
    for (name, mesh) in [("terrain", grid(192)), ("foliage", blades(32_768))] {
        let scene = SceneAsset {
            meshes: vec![build_virtual_mesh(&mesh, &BuildOptions::default())],
            materials: vec![Material::default()],
            instances: vec![Instance {
                mesh: 0,
                material: 0,
                transform: Mat4::IDENTITY,
            }],
            ..Default::default()
        };
        let start = Instant::now();
        let loaded = paginate(&scene);
        let packing_ms = start.elapsed().as_secs_f64() * 1_000.0;
        let mut raw = 0;
        let mut deflated = 0;
        for page in 0..loaded.scene.pages.len() as u32 {
            let bytes = loaded.page(page);
            raw += bytes.len();
            let info = &loaded.scene.pages[page as usize];
            deflated += solstice_asset::file::deflate_page(bytes, info).len();
        }
        // Every cluster must still decode to exactly the original packed values, including
        // triangle ordering. This checks the thing the GPU consumes, not just byte counts.
        for set in &loaded.scene.sets {
            let info = &loaded.scene.pages[set.page as usize];
            let bytes = loaded.page(set.page);
            for c in set.first_cluster..set.first_cluster + set.cluster_count {
                let before = &scene.meshes[0].clusters[c as usize];
                let after = &loaded.scene.clusters[c as usize];
                let vs = before.vertex_count() as usize;
                let ts = before.triangle_count() as usize;
                // Offsets are in words: a vertex is three, a triangle three bytes.
                let vertex_at = after.vertex_offset as usize * 4;
                let triangle_at = info.vertex_bytes() + after.triangle_offset as usize * 4;
                let source_vertices =
                    &scene.meshes[0].vertices[before.vertex_offset as usize..][..vs];
                let source_triangles =
                    &scene.meshes[0].triangles[before.triangle_offset as usize..][..ts];
                assert_eq!(
                    &bytes[vertex_at..][..vs * 12],
                    bytemuck::cast_slice(source_vertices)
                );
                let mut packed = Vec::new();
                solstice_asset::gpu::pack_triangle_words(source_triangles, &mut packed);
                assert_eq!(
                    &bytes[triangle_at..][..packed.len() * 4],
                    bytemuck::cast_slice(&packed)
                );
            }
        }
        println!(
            "{name},{},{},{},{raw},{deflated},{},{packing_ms:.3}",
            mesh.indices.len() / 3,
            loaded.scene.clusters.len(),
            loaded.scene.pages.len(),
            loaded.scene.pages.len() * solstice_asset::paged::PAGE_BYTES
        );
    }
}
