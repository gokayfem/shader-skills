//! What the scene builders that plant things share: the materials of bark, leaves and grass
//! blades, each flagged to sway in the wind, and a wood's species grown into a scene.

use glam::Mat4;
use solstice_asset::{Instance, Material, SceneAsset};
use solstice_builder::{build_virtual_mesh, BuildOptions};
use solstice_procgen::foliage::bark::BARK_TILE_METRES;
use solstice_procgen::foliage::tree::Species;

/// A tree's wood: its trunk leans and sways, its branches bob.
pub fn bark(texture_layer: i32) -> Material {
    Material {
        base_color: [1.0; 4],
        roughness: 1.0,
        uv_scale: 1.0 / BARK_TILE_METRES,
        normal_strength: 1.0,
        texture_layer,
        flags: Material::SWAYS | Material::TREE,
        ..Default::default()
    }
}

/// A tree's leaves: they sway with the wood they hang from, and flutter.
pub fn leaves(texture_layer: i32) -> Material {
    Material {
        base_color: [1.0; 4],
        roughness: 1.0,
        normal_strength: 1.0,
        texture_layer,
        flags: Material::TWO_SIDED | Material::FOLIAGE | Material::SWAYS | Material::TREE,
        ..Default::default()
    }
}

/// Grass blades, which sway as grass.
pub fn blades(texture_layer: i32) -> Material {
    Material {
        base_color: [1.0; 4],
        roughness: 1.0,
        normal_strength: 1.0,
        texture_layer,
        flags: Material::TWO_SIDED | Material::FOLIAGE | Material::SWAYS,
        ..Default::default()
    }
}

/// A species grown into a scene: copies from different seeds, each two meshes, wood then
/// leaves, and its two materials.
#[derive(Clone, Copy, Debug)]
pub struct Grown {
    pub first_mesh: u32,
    pub variants: u32,
    pub bark: u32,
    pub leaves: u32,
}

impl Grown {
    /// Places copy `copy` (taken modulo the copies there are) where `transform` puts it.
    pub fn place(&self, scene: &mut SceneAsset, copy: u32, transform: Mat4) {
        let mesh = self.first_mesh + (copy % self.variants) * 2;
        for (mesh, material) in [(mesh, self.bark), (mesh + 1, self.leaves)] {
            scene.instances.push(Instance {
                mesh,
                material,
                transform,
            });
        }
    }
}

/// Grows each built-in species named, that many copies of each, into `scene`: their meshes,
/// their bark and leaf textures, and their materials.
pub fn grow(scene: &mut SceneAsset, plants: &[(&str, u32)]) -> Vec<Grown> {
    plants
        .iter()
        .map(|&(name, variants)| {
            let species = Species::by_name(name).expect("a built-in species");
            let first_mesh = scene.meshes.len() as u32;
            for seed in 0..variants {
                let plant = species.grow(seed);
                let options = BuildOptions::default();
                scene.meshes.push(build_virtual_mesh(&plant.wood, &options));
                scene
                    .meshes
                    .push(build_virtual_mesh(&plant.leaves, &options));
            }
            let bark_layer = scene.textures.len() as i32;
            scene
                .textures
                .push(species.bark.surface(0).bake(scene.texture_size));
            let leaf_layer = scene.textures.len() as i32;
            scene
                .textures
                .push(species.leaves.kind.atlas(0).bake(scene.texture_size));
            scene.materials.push(bark(bark_layer));
            scene.materials.push(leaves(leaf_layer));
            let leaves = scene.materials.len() as u32 - 1;
            Grown {
                first_mesh,
                variants,
                bark: leaves - 1,
                leaves,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_copy_is_placed_as_its_wood_then_its_leaves() {
        let grown = Grown {
            first_mesh: 10,
            variants: 2,
            bark: 4,
            leaves: 5,
        };
        let mut scene = SceneAsset::default();
        // Copy 3 of two is copy 1: meshes 12 and 13.
        grown.place(&mut scene, 3, Mat4::IDENTITY);
        let placed: Vec<(u32, u32)> = scene
            .instances
            .iter()
            .map(|i| (i.mesh, i.material))
            .collect();
        assert_eq!(placed, [(12, 4), (13, 5)]);
    }

    #[test]
    fn every_plant_material_sways_and_only_trees_sway_as_trees() {
        for material in [bark(0), leaves(0), blades(0)] {
            assert_ne!(material.flags & Material::SWAYS, 0);
        }
        assert_ne!(bark(0).flags & Material::TREE, 0);
        assert_ne!(leaves(0).flags & Material::TREE, 0);
        assert_eq!(blades(0).flags & Material::TREE, 0);
    }
}
