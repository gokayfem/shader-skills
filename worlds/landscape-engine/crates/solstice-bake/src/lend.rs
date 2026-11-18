//! What a scene lends to others, and what one leaves out because another lends it.
//!
//! A game's maps are made of much the same things: the same trees, grass and stone, the same
//! textures under them. Baked into each, they are in every map's file. Instead one scene, the
//! one whose file every map is read with, *lends* them: it carries a list of every mesh and
//! texture it has, each with a key, which is a hash of what it is ([`lends`]). A map baked
//! beside that file leaves out whatever of its own has a key on the list ([`borrow`]): the
//! mesh becomes an empty place, the texture is dropped, and the map carries a list of what it
//! left. Read together, the two are joined and the places are filled
//! (`solstice_asset::join::fill`).
//!
//! Nothing has to be told which things are shared. Whatever the two scenes both have, bit for
//! bit, is left out, and whatever differs by anything is kept: a map can never be given the
//! wrong tree. So what is to be shared has to be made the same way in both, wherever it is
//! used.
//!
//! Keys are of a scene as it is built, before its textures are compressed: both the lender
//! and the borrower are hashed at that point.
use sha2::{Digest, Sha256};
use solstice_asset::join::Lent;
use solstice_asset::{SceneAsset, TextureLayer, VirtualMesh};
use std::collections::HashMap;
use std::fmt::Write as _;

/// A key: the start of a SHA-256, in hex. Sixteen bytes of it are plenty to tell apart the
/// few hundred things a game has.
fn key(hash: Sha256) -> String {
    hash.finalize()[..16]
        .iter()
        .fold(String::new(), |mut text, byte| {
            let _ = write!(text, "{byte:02x}");
            text
        })
}

/// A mesh's key: everything of it that is drawn, and how it is read.
fn mesh_key(mesh: &VirtualMesh) -> String {
    let mut hash = Sha256::new();
    let mut part = |bytes: &[u8]| {
        hash.update((bytes.len() as u64).to_le_bytes());
        hash.update(bytes);
    };
    part(bytemuck::cast_slice(&mesh.clusters));
    part(bytemuck::cast_slice(&mesh.nodes));
    part(bytemuck::cast_slice(&mesh.vertices));
    part(bytemuck::cast_slice(&mesh.triangles));
    part(bytemuck::cast_slice(&mesh.sets));
    part(bytemuck::cast_slice(&mesh.generated));
    part(bytemuck::cast_slice(&mesh.cluster_bones));
    part(bytemuck::cast_slice(&[
        mesh.root_node,
        mesh.hierarchy_depth,
    ]));
    part(bytemuck::cast_slice(&mesh.quant_origin.to_array()));
    part(bytemuck::cast_slice(&[mesh.quant_step]));
    part(bytemuck::cast_slice(&mesh.uv_origin));
    part(bytemuck::cast_slice(&mesh.uv_step));
    key(hash)
}

/// A texture layer's key: every level of its three maps, at the size it is.
fn texture_key(layer: &TextureLayer, size: u32) -> String {
    let mut hash = Sha256::new();
    hash.update(size.to_le_bytes());
    for maps in [&layer.color_mips, &layer.normal_mips, &layer.material_mips] {
        hash.update((maps.len() as u64).to_le_bytes());
        for level in maps {
            hash.update((level.len() as u64).to_le_bytes());
            hash.update(level);
        }
    }
    key(hash)
}

/// The list of what a scene lends, as the text of its `solstice_asset::join::LENDS` file:
/// every mesh that has anything in it, and every texture layer.
pub fn lends(scene: &SceneAsset) -> String {
    let mut text = String::from(
        "# What this scene lends to those read with it: what, its number here, its key.\n",
    );
    for (index, mesh) in scene.meshes.iter().enumerate() {
        if !mesh.clusters.is_empty() {
            let _ = writeln!(text, "{} {index} {}", Lent::Mesh.word(), mesh_key(mesh));
        }
    }
    for (layer, texture) in scene.textures.iter().enumerate() {
        let _ = writeln!(
            text,
            "{} {layer} {}",
            Lent::Texture.word(),
            texture_key(texture, scene.texture_size)
        );
    }
    text
}

/// What a scene left out.
pub struct Borrowed {
    /// The list of it, as the text of the scene's `solstice_asset::join::BORROWS` file.
    pub text: String,
    pub meshes: usize,
    pub textures: usize,
}

/// Leaves out of a scene whatever a lender's list has: `lends` is the text of that list.
/// Each such mesh is left an empty place, which keeps its number, and each such texture is
/// dropped, the layers after it moving down. Returns the list of what was left, for whoever
/// joins the two to fill.
pub fn borrow(scene: &mut SceneAsset, lends: &str) -> Borrowed {
    let there: HashMap<(Lent, &str), u32> = solstice_asset::join::lent(lends)
        .map(|(what, index, key)| ((what, key), index))
        .collect();
    let mut text = String::from(
        "# What this scene leaves to the file it is read with: a mesh of its own that is an empty\n# place, or a material of its own that wants a texture, and the key of what goes there.\n",
    );
    let mut meshes = 0;
    for (index, mesh) in scene.meshes.iter_mut().enumerate() {
        if mesh.clusters.is_empty() {
            continue;
        }
        let key = mesh_key(mesh);
        if there.contains_key(&(Lent::Mesh, key.as_str())) {
            let _ = writeln!(text, "{} {index} {key}", Lent::Mesh.word());
            // What is said of the whole of it stays, for whoever counts or measures.
            *mesh = VirtualMesh {
                quant_origin: mesh.quant_origin,
                quant_step: mesh.quant_step,
                uv_origin: mesh.uv_origin,
                uv_step: mesh.uv_step,
                bounds_center: mesh.bounds_center,
                bounds_radius: mesh.bounds_radius,
                source_triangles: mesh.source_triangles,
                ..Default::default()
            };
            meshes += 1;
        }
    }
    // Each layer's key if it is lent, and where it is among those that are kept if it is not.
    let mut kept = Vec::new();
    let mut now: Vec<Result<i32, String>> = Vec::new();
    for texture in std::mem::take(&mut scene.textures) {
        let key = texture_key(&texture, scene.texture_size);
        if there.contains_key(&(Lent::Texture, key.as_str())) {
            now.push(Err(key));
        } else {
            now.push(Ok(kept.len() as i32));
            kept.push(texture);
        }
    }
    scene.textures = kept;
    let textures = now.iter().filter(|layer| layer.is_err()).count();
    for (index, material) in scene.materials.iter_mut().enumerate() {
        let Some(layer) = usize::try_from(material.texture_layer)
            .ok()
            .and_then(|layer| now.get(layer))
        else {
            continue;
        };
        match layer {
            Ok(layer) => material.texture_layer = *layer,
            Err(key) => {
                let _ = writeln!(text, "{} {index} {key}", Lent::Texture.word());
                material.texture_layer = -1;
            }
        }
    }
    Borrowed {
        text,
        meshes,
        textures,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::{Mat4, Vec2, Vec3};
    use solstice_asset::join::{BORROWS, LENDS};
    use solstice_asset::{GameFile, Instance, Material};
    use solstice_builder::{build_virtual_mesh, paginate, BuildOptions, Mesh, Vertex};

    /// A bumpy sheet, which `lift` makes one thing or another.
    fn sheet(across: u32, lift: f32) -> VirtualMesh {
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
        build_virtual_mesh(&mesh, &BuildOptions::default())
    }

    /// A texture of one shade, with all its levels.
    fn texture(size: u32, shade: u8) -> TextureLayer {
        let levels = |bytes: usize| -> Vec<Vec<u8>> {
            (0..=size.trailing_zeros())
                .map(|level| vec![shade; ((size >> level) as usize).pow(2) * bytes])
                .collect()
        };
        TextureLayer {
            color_mips: levels(4),
            normal_mips: levels(2),
            material_mips: levels(2),
        }
    }

    #[test]
    fn a_map_leaves_out_what_its_lender_has_and_is_whole_again_joined() {
        const SIZE: u32 = 128;
        let (tree, rock, land) = (sheet(60, 0.4), sheet(40, 0.2), sheet(80, 0.1));
        // The lender: a tree and a rock, and their textures, placed nowhere; and one thing
        // of its own that is.
        let mut lender = SceneAsset {
            texture_size: SIZE,
            meshes: vec![sheet(30, 0.7), tree.clone(), rock.clone()],
            textures: vec![texture(SIZE, 10), texture(SIZE, 20), texture(SIZE, 30)],
            materials: vec![Material {
                texture_layer: 0,
                ..Default::default()
            }],
            instances: vec![Instance {
                mesh: 0,
                material: 0,
                transform: Mat4::IDENTITY,
            }],
            ..Default::default()
        };
        lender.gameplay.push(GameFile {
            name: LENDS.to_owned(),
            text: lends(&lender),
        });

        // The map: its own land under its own texture, and the tree and the rock, each
        // under a texture the lender has.
        let whole = SceneAsset {
            texture_size: SIZE,
            meshes: vec![land, rock, tree],
            textures: vec![texture(SIZE, 30), texture(SIZE, 77), texture(SIZE, 20)],
            materials: (0..3)
                .map(|layer| Material {
                    texture_layer: layer,
                    roughness: 0.1 * layer as f32,
                    ..Default::default()
                })
                .collect(),
            instances: (0..3)
                .map(|index| Instance {
                    mesh: index,
                    material: index,
                    transform: Mat4::from_translation(Vec3::X * 40.0 * index as f32),
                })
                .collect(),
            ..Default::default()
        };
        let mut map = whole.clone();
        let left = borrow(&mut map, &lender.gameplay[0].text);
        assert_eq!((left.meshes, left.textures), (2, 2));
        assert!(!map.meshes[0].clusters.is_empty(), "its own land is kept");
        assert!(map.meshes[1].clusters.is_empty() && map.meshes[2].clusters.is_empty());
        assert_eq!(map.textures.len(), 1);
        assert_eq!(
            map.materials
                .iter()
                .map(|material| material.texture_layer)
                .collect::<Vec<_>>(),
            [-1, 0, -1]
        );
        map.gameplay.push(GameFile {
            name: BORROWS.to_owned(),
            text: left.text.clone(),
        });

        // Joined and filled, it draws what the whole map drew.
        let mut joined = paginate(&map);
        let lender = paginate(&lender);
        let lent = lender.scene.gameplay[0].text.clone();
        let added = joined.join(lender).expect("they join");
        solstice_asset::join::fill(&mut joined.scene, &left.text, &lent, added)
            .expect("it is filled");
        let alone = paginate(&whole);
        for (index, (now, was)) in joined
            .scene
            .instances
            .iter()
            .zip(&alone.scene.instances)
            .enumerate()
        {
            assert_eq!((now.mesh, now.material), (was.mesh, was.material));
            let (mine, theirs) = (
                joined.scene.meshes[now.mesh as usize],
                alone.scene.meshes[was.mesh as usize],
            );
            assert_eq!(
                mine.root_node.is_some(),
                theirs.root_node.is_some(),
                "instance {index}"
            );
            assert_eq!(
                (mine.quant_origin, mine.quant_step),
                (theirs.quant_origin, theirs.quant_step)
            );
            // The same tree under each: as many clusters at its root's leaf, of the same size.
            let root = |scene: &solstice_asset::PagedScene,
                        mesh: &solstice_asset::paged::MeshInfo| {
                let node = scene.nodes[mesh.root_node.expect("it has geometry") as usize];
                (node.count, node.center, node.radius)
            };
            assert_eq!(
                root(&joined.scene, &mine),
                root(&alone.scene, &theirs),
                "instance {index}"
            );
            // And the same texture: its coarsest level is the shade it was.
            let shade = |scene: &solstice_asset::PagedScene, material: u32| {
                let layer = scene.materials[material as usize].texture_layer;
                scene.base_textures[layer as usize]
                    .color_mips
                    .last()
                    .expect("levels")[0]
            };
            assert_eq!(
                shade(&joined.scene, now.material),
                shade(&alone.scene, was.material),
                "instance {index}"
            );
        }
        assert_eq!(
            joined.scene.materials[2].roughness,
            alone.scene.materials[2].roughness
        );

        // A lender that has changed since is noticed, not drawn wrongly.
        let other = lent.replace(lent.lines().nth(2).expect("a line"), "mesh 1 0000");
        let mut again = paginate(&map);
        assert!(solstice_asset::join::fill(&mut again.scene, &left.text, &other, added).is_err());
    }
}
