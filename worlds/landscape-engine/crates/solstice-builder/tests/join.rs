//! Two scenes joined are one sound scene: every index in it lands inside its table, pages
//! still follow what they need, and nothing of either is lost.
use glam::{Mat4, Vec2, Vec3};
use solstice_asset::{Instance, Material, SceneAsset};
use solstice_builder::{build_virtual_mesh, paginate, BuildOptions, Mesh, Vertex};

/// A bumpy sheet `across` squares on a side: enough triangles for several rounds of detail.
fn sheet(across: u32, lift: f32) -> Mesh {
    let mut mesh = Mesh::default();
    for row in 0..=across {
        for column in 0..=across {
            let (x, z) = (column as f32 * 0.2, row as f32 * 0.2);
            mesh.vertices.push(Vertex {
                position: Vec3::new(x, lift * (x * 3.1).sin() * (z * 2.3).cos(), z),
                normal: Vec3::Y,
                uv: Vec2::new(x, z),
            });
        }
    }
    let wide = across + 1;
    for row in 0..across {
        for column in 0..across {
            let at = row * wide + column;
            mesh.indices
                .extend([at, at + wide, at + 1, at + 1, at + wide, at + wide + 1]);
        }
    }
    mesh
}

fn scene(sheets: &[(u32, f32)], copies: usize) -> SceneAsset {
    let mut scene = SceneAsset::default();
    for (index, &(across, lift)) in sheets.iter().enumerate() {
        scene.meshes.push(build_virtual_mesh(
            &sheet(across, lift),
            &BuildOptions::default(),
        ));
        scene.materials.push(Material::default());
        for copy in 0..copies {
            scene.instances.push(Instance {
                mesh: index as u32,
                material: index as u32,
                transform: Mat4::from_translation(Vec3::new(
                    copy as f32 * 30.0,
                    0.0,
                    index as f32 * 30.0,
                )),
            });
        }
    }
    scene
}

#[test]
fn two_scenes_joined_are_one_sound_scene() {
    let map = scene(&[(90, 0.4), (40, 0.1)], 3);
    let shared = scene(&[(70, 0.3)], 2);
    let (mut joined, second) = (paginate(&map), paginate(&shared));
    let (first_alone, second_alone) = (joined.scene.clone(), second.scene.clone());
    let added = joined.join(second).expect("they join");
    let both = &joined.scene;

    // Nothing is lost, and the first scene's own numbers are as they were.
    assert_eq!(
        both.clusters.len(),
        first_alone.clusters.len() + second_alone.clusters.len()
    );
    assert_eq!(
        both.nodes.len(),
        first_alone.nodes.len() + second_alone.nodes.len()
    );
    assert_eq!(
        both.sets.len(),
        first_alone.sets.len() + second_alone.sets.len()
    );
    assert_eq!(
        both.pages.len(),
        first_alone.pages.len() + second_alone.pages.len()
    );
    assert_eq!(
        both.pinned_pages,
        first_alone.pinned_pages + second_alone.pinned_pages
    );
    assert_eq!(added.instances, 6);
    assert_eq!((added.meshes, added.materials), (2, 2));
    for (now, was) in both.instances.iter().zip(&first_alone.instances) {
        assert_eq!(
            (now.mesh, now.material, now.transform),
            (was.mesh, was.material, was.transform)
        );
    }
    assert_eq!(both.instances[6].mesh, 2);
    assert_eq!(
        both.source_triangles,
        first_alone.source_triangles + second_alone.source_triangles
    );
    assert_eq!(
        joined.pages.len(),
        both.pages.len() - both.pinned_pages as usize
    );

    // Every index lands in its table.
    for instance in &both.instances {
        assert!((instance.mesh as usize) < both.meshes.len());
        assert!((instance.material as usize) < both.materials.len());
    }
    for mesh in &both.meshes {
        assert!(mesh
            .root_node
            .is_some_and(|node| (node as usize) < both.nodes.len()));
    }
    for node in &both.nodes {
        let table = if node.is_leaf != 0 {
            both.clusters.len()
        } else {
            both.nodes.len()
        };
        assert!((node.first + node.count) as usize <= table);
    }
    assert!(both
        .generated
        .iter()
        .all(|&cluster| (cluster as usize) < both.clusters.len()));
    // Sets are in the order of their pages, and each page's sets are the ones it says.
    assert!(both
        .sets
        .windows(2)
        .all(|pair| pair[0].page <= pair[1].page));
    for (page, info) in both.pages.iter().enumerate() {
        let sets = &both.sets[info.first_set as usize..(info.first_set + info.set_count) as usize];
        assert!(
            sets.iter().all(|set| set.page == page as u32),
            "page {page}"
        );
    }
    for set in &both.sets {
        assert!((set.first_cluster + set.cluster_count) as usize <= both.clusters.len());
        assert!((set.first_generated + set.generated_count) as usize <= both.generated.len());
        assert!((set.leaf_node as usize) < both.nodes.len());
        // The leaf says which page its clusters are in: a view asks for that page by it.
        let leaf = both.nodes[set.leaf_node as usize].is_leaf;
        assert_ne!(leaf & solstice_asset::gpu::NODE_LEAF, 0);
        assert_eq!(leaf & solstice_asset::gpu::NODE_PAGE_MASK, set.page);
    }
    // A page needs only pages before it, as it did before the two were joined.
    for (page, needs) in both.page_dependencies().iter().enumerate() {
        assert!(
            needs.iter().all(|&needed| (needed as usize) < page),
            "page {page} needs {needs:?}"
        );
    }
    // The pinned pages' contents are where their sizes say, one after another.
    let pinned: usize = both.pages[..both.pinned_pages as usize]
        .iter()
        .map(|page| page.raw_bytes())
        .sum();
    assert_eq!(both.pinned_data.len(), pinned);
    // And each page that is not pinned is the size its entry says.
    for (index, page) in joined.pages.iter().enumerate() {
        assert_eq!(
            page.len(),
            both.pages[both.pinned_pages as usize + index].raw_bytes()
        );
    }
}
