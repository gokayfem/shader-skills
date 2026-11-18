//! Plants on open ground, for judging each species and breed of grass: a row of every built-in
//! species, then any species files given, each followed by a second plant grown from another
//! seed; and in front of them a square of every built-in breed of grass, then any grass files.

use glam::{Mat4, Vec3};
use solstice_asset::scene::Viewpoint;
use solstice_asset::{Instance, Material, SceneAsset, TextureLayer};
use solstice_bake::meshes;
use solstice_builder::{build_virtual_mesh, BuildOptions};
use solstice_procgen::foliage::bark::BARK_TILE_METRES;
use solstice_procgen::foliage::grass::Meadow;
use solstice_procgen::foliage::tree::Species;
use solstice_procgen::surface::TILE_METRES;
use solstice_procgen::surfaces;
use std::path::PathBuf;

/// Space between plants along the row, in metres.
const SPACING: f32 = 16.0;
/// Plants of each species, grown from different seeds.
const COPIES: u32 = 2;
/// Edge of each square of grass, in metres, and how far in front of the trees the squares lie.
const GRASS_SQUARE: f32 = 6.0;
const GRASS_ROW: f32 = 14.0;
/// Different patches of each breed, laid out at random so no repeat shows.
const PATCH_SEEDS: u32 = 2;

/// Reads each file with `parse`, naming the file in any error.
fn read_all<T>(
    files: &[PathBuf],
    parse: fn(&str) -> Result<T, String>,
) -> Result<Vec<(String, T)>, String> {
    files
        .iter()
        .map(|path| {
            let text =
                std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
            let parsed = parse(&text).map_err(|e| format!("{}: {e}", path.display()))?;
            Ok((path.display().to_string(), parsed))
        })
        .collect()
}

pub fn build(
    texture_size: u32,
    species_files: &[PathBuf],
    grass_files: &[PathBuf],
    only: bool,
) -> Result<SceneAsset, String> {
    // With `only`, just the files given, for looking at one plant while it is being written.
    let mut species: Vec<(String, Species)> = Species::names()
        .filter(|_| !only)
        .map(|name| (name.to_string(), Species::by_name(name).unwrap()))
        .collect();
    species.extend(read_all(species_files, Species::parse)?);
    let mut breeds: Vec<(String, Meadow)> = Meadow::names()
        .filter(|_| !only)
        .map(|name| (name.to_string(), Meadow::by_name(name).unwrap()))
        .collect();
    breeds.extend(read_all(grass_files, Meadow::parse)?);

    let options = BuildOptions::default();
    let length = SPACING * (species.len() as u32 * COPIES) as f32;
    let mut scene = SceneAsset {
        texture_size,
        meshes: vec![build_virtual_mesh(
            &meshes::terrain(length + 40.0, 64, |_, _| 0.0),
            &options,
        )],
        viewpoint: Some(Viewpoint {
            position: [0.0, 4.0, 26.0],
            yaw: 0.0,
            pitch: 0.05,
        }),
        ..Default::default()
    };
    let mut layers: Vec<(String, TextureLayer)> = Vec::new();
    let mut layer = |name: &str, make: &dyn Fn() -> TextureLayer| -> i32 {
        if let Some(i) = layers.iter().position(|(n, _)| n == name) {
            return i as i32;
        }
        println!("  {name}");
        layers.push((name.to_string(), make()));
        layers.len() as i32 - 1
    };
    let ground = layer("grass", &|| surfaces::grass(0).bake(texture_size));
    scene.materials.push(Material {
        base_color: [0.8, 0.8, 0.8, 1.0],
        roughness: 1.0,
        uv_scale: 1.0 / TILE_METRES,
        texture_layer: ground,
        ..Default::default()
    });
    scene.instances.push(Instance {
        mesh: 0,
        material: 0,
        transform: Mat4::IDENTITY,
    });

    let start = -length * 0.5 + SPACING * 0.5;
    for (index, (name, species)) in species.iter().enumerate() {
        let bark = species.bark;
        let leaf = species.leaves.kind;
        let bark_layer = layer(bark.name(), &|| bark.surface(0).bake(texture_size));
        let leaf_layer = layer(leaf.name(), &|| leaf.atlas(0).bake(texture_size));
        scene.materials.push(Material {
            base_color: [1.0; 4],
            roughness: 1.0,
            uv_scale: 1.0 / BARK_TILE_METRES,
            normal_strength: 1.0,
            texture_layer: bark_layer,
            ..Default::default()
        });
        scene.materials.push(Material {
            base_color: [1.0; 4],
            roughness: 1.0,
            normal_strength: 1.0,
            texture_layer: leaf_layer,
            flags: Material::TWO_SIDED | Material::FOLIAGE,
            ..Default::default()
        });
        let materials = scene.materials.len() as u32;
        for seed in 0..COPIES {
            let plant = species.grow(seed);
            println!(
                "{name} {seed}: {} wood and {} leaf triangles",
                plant.wood.triangle_count(),
                plant.leaves.triangle_count()
            );
            let x = start + (index as u32 * COPIES + seed) as f32 * SPACING;
            let transform = Mat4::from_translation(Vec3::new(x, -0.05, 0.0));
            for (mesh, material) in [(&plant.wood, materials - 2), (&plant.leaves, materials - 1)] {
                scene.instances.push(Instance {
                    mesh: scene.meshes.len() as u32,
                    material,
                    transform,
                });
                scene.meshes.push(build_virtual_mesh(mesh, &options));
            }
            // Young shoots of another colour than the bark, where a species has them.
            if let (Some((_, [r, g, b])), false) = (species.twigs, plant.twigs.indices.is_empty()) {
                println!("  and {} twig triangles", plant.twigs.triangle_count());
                scene.materials.push(Material {
                    base_color: [r, g, b, 1.0],
                    roughness: 0.6,
                    ..Default::default()
                });
                scene.instances.push(Instance {
                    mesh: scene.meshes.len() as u32,
                    material: scene.materials.len() as u32 - 1,
                    transform,
                });
                scene
                    .meshes
                    .push(build_virtual_mesh(&plant.twigs, &options));
            }
        }
    }

    // A square of each breed, made of copies of a few patches turned at random.
    let row_start = -(breeds.len() as f32 - 1.0) * (GRASS_SQUARE + 2.0) * 0.5;
    for (index, (name, breed)) in breeds.iter().enumerate() {
        let atlas = layer(&format!("{name} blades"), &|| {
            breed.atlas(0).bake(texture_size)
        });
        scene.materials.push(Material {
            base_color: [1.0; 4],
            roughness: 1.0,
            normal_strength: 1.0,
            texture_layer: atlas,
            flags: Material::TWO_SIDED | Material::FOLIAGE,
            ..Default::default()
        });
        let material = scene.materials.len() as u32 - 1;
        let first_mesh = scene.meshes.len() as u32;
        for seed in 0..PATCH_SEEDS {
            let mesh = Meadow {
                seed,
                ..breed.clone()
            }
            .mesh();
            println!("{name} grass {seed}: {} triangles", mesh.triangle_count());
            scene.meshes.push(build_virtual_mesh(&mesh, &options));
        }
        let patches = (GRASS_SQUARE / breed.size).ceil() as u32;
        let corner = Vec3::new(
            row_start + index as f32 * (GRASS_SQUARE + 2.0) - GRASS_SQUARE * 0.5,
            0.0,
            GRASS_ROW - GRASS_SQUARE * 0.5,
        );
        for j in 0..patches {
            for i in 0..patches {
                let pick = (i * 7 + j * 13 + index as u32) % (PATCH_SEEDS * 4);
                let half = breed.size * 0.5;
                let centre = corner
                    + Vec3::new(
                        (i as f32 + 0.5) * breed.size,
                        0.0,
                        (j as f32 + 0.5) * breed.size,
                    );
                let transform = Mat4::from_translation(centre)
                    * Mat4::from_rotation_y(
                        (pick / PATCH_SEEDS) as f32 * std::f32::consts::FRAC_PI_2,
                    )
                    * Mat4::from_translation(Vec3::new(-half, 0.0, -half));
                scene.instances.push(Instance {
                    mesh: first_mesh + pick % PATCH_SEEDS,
                    material,
                    transform,
                });
            }
        }
    }
    scene.textures = layers.into_iter().map(|(_, layer)| layer).collect();
    Ok(scene)
}
