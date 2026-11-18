//! A scene showing every surface in the procedural library, for judging how each one looks.

use glam::{Mat4, Vec3};
use solstice_asset::scene::Viewpoint;
use solstice_asset::{Instance, Material, SceneAsset};
use solstice_bake::meshes;
use solstice_builder::{build_virtual_mesh, BuildOptions};
use solstice_procgen::surfaces;

const COLUMNS: usize = 5;
const TILE: f32 = 8.0;
const SPACING: f32 = 9.0;

/// A flat tile and a ball for each surface, laid out in rows.
pub fn build(texture_size: u32) -> SceneAsset {
    let options = BuildOptions::default();
    let meshes = vec![
        build_virtual_mesh(&meshes::terrain(TILE, 8, |_, _| 0.0), &options),
        build_virtual_mesh(&meshes::sphere(5), &options),
    ];
    let mut scene = SceneAsset {
        meshes,
        texture_size,
        viewpoint: Some(Viewpoint {
            position: [0.0, 15.0, 24.0],
            yaw: 0.0,
            pitch: -0.62,
        }),
        ..Default::default()
    };
    let names = surfaces::NAMES;
    for (index, name) in names.iter().enumerate() {
        println!("generating {name}...");
        let surface = surfaces::by_name(name, 0).expect("listed surface exists");
        scene.textures.push(surface.bake(texture_size));
        let surface = &surface;
        scene.materials.push(Material {
            base_color: [1.0; 4],
            roughness: 1.0,
            uv_scale: 1.0 / surface.tile_metres(),
            normal_strength: 1.0,
            texture_layer: index as i32,
            ..Default::default()
        });
        let column = (index % COLUMNS) as f32 - (COLUMNS as f32 - 1.0) * 0.5;
        let row = (index / COLUMNS) as f32 - 1.0;
        let at = Vec3::new(column * SPACING, 0.0, row * SPACING);
        scene.instances.push(Instance {
            mesh: 0,
            material: index as u32,
            transform: Mat4::from_translation(at),
        });
        scene.instances.push(Instance {
            mesh: 1,
            material: index as u32,
            transform: Mat4::from_translation(at + Vec3::new(2.2, 1.0, -2.2)),
        });
    }
    scene
}
