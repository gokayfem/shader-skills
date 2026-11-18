//! Generates the harbour scene's kit as glTF, because no CC0 scan exists of what it holds.
//!
//! ```text
//! townkit [--output FOLDER] [--without walls|roofs|details]...
//! ```
//!
//! `ground.gltf` is the land and seabed, `stone.gltf` the dressed stone and `houses.gltf` the
//! houses, all placed whole. The kit goes in target/town by default. `--without` leaves part of the
//! houses out, which is how each part's cost is measured.
//! `town.gltf` holds named parts, placed by `scenes/harbour.txt`. `docs/harbour.md` is the design.
//! The textures come first: `python scripts/fetch-scans.py scenes/harbour.surfaces 2k`.

mod geometry;
mod gltf;
mod ground;
mod houses;
mod stone;
mod town;

use gltf::{File, Plain};
use ground::{Surface, PATCHES, SURFACES};
use houses::Parts;
use std::path::{Path, PathBuf};

const fn plain(name: &'static str, color: [f32; 3], roughness: f32) -> Plain {
    Plain {
        name,
        color,
        roughness,
    }
}

fn ground_material(surface: Surface) -> Plain {
    match surface {
        Surface::Sand => plain("sand", [0.42, 0.36, 0.26], 0.95),
        Surface::Grass => plain("grass", [0.11, 0.16, 0.06], 0.95),
        Surface::Dirt => plain("dirt", [0.2, 0.16, 0.11], 0.95),
        Surface::Rock => plain("rock", [0.17, 0.16, 0.15], 0.9),
    }
}

/// The ground, each patch split by surface.
fn ground_file() -> File {
    let mut file = File::default();
    let materials = SURFACES.map(|surface| file.material(&ground_material(surface)));
    for row in 0..PATCHES {
        for column in 0..PATCHES {
            let geometry = ground::patch(column, row);
            let pieces: Vec<_> = geometry.iter().zip(materials).collect();
            file.part(&format!("ground_{column}_{row}"), &pieces);
        }
    }
    file
}

/// The solid stonework and the massing. `stonework` adds the material they are drawn in.
fn town_file(stonework: impl FnOnce(&mut File) -> usize) -> File {
    let mut file = File::default();
    let granite = stonework(&mut file);
    let limewash = file.material(&plain("limewash", [0.72, 0.7, 0.64], 0.9));
    let terracotta = file.material(&plain("terracotta", [0.42, 0.18, 0.1], 0.8));
    let timber = file.material(&plain("timber", [0.14, 0.09, 0.06], 0.8));

    file.part("quay", &[(&town::quay(), granite)]);
    file.part("mole", &[(&town::mole(), granite)]);
    let massing = town::massing();
    file.part(
        "massing",
        &[
            (&massing.walls, limewash),
            (&massing.roofs, terracotta),
            (&massing.stone, granite),
            (&massing.timber, timber),
        ],
    );
    file
}

fn parse_args() -> Result<(PathBuf, Parts), String> {
    let mut args = std::env::args().skip(1);
    let mut folder = PathBuf::from("target/town");
    let mut parts = Parts {
        walls: true,
        roofs: true,
        details: true,
    };
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--output" => folder = args.next().ok_or("--output needs a folder")?.into(),
            "--without" => match args.next().ok_or("--without needs a part")?.as_str() {
                "walls" => parts.walls = false,
                "roofs" => parts.roofs = false,
                "details" => parts.details = false,
                other => return Err(format!("no part of a house is called '{other}'")),
            },
            other => return Err(format!("unknown option '{other}'")),
        }
    }
    if !(parts.walls || parts.roofs || parts.details) {
        return Err("leaving out every part of the houses would write an empty file".into());
    }
    Ok((folder, parts))
}

fn write_kit(folder: &Path, parts: Parts) -> std::io::Result<()> {
    ground_file().write(folder, "ground")?;
    town_file(|file| file.textured(&stone::FLAGS)).write(folder, "town")?;
    stone::file(&folder.join("../scan"))?.write(folder, "stone")?;
    houses::file(&folder.join("../scan"), parts)?.write(folder, "houses")
}

fn main() -> Result<(), String> {
    let (folder, parts) = parse_args()?;
    write_kit(&folder, parts).map_err(|e| format!("{}: {e}", folder.display()))?;
    println!("wrote the kit to {}", folder.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use solstice_import::{import_gltf, ImportOptions};

    #[test]
    fn the_importer_places_a_node_where_it_was_put() {
        use glam::{Quat, Vec3};
        let mut g = geometry::Geometry::default();
        g.cuboid(Vec3::ZERO, Vec3::ONE);
        let mut file = File::default();
        let material = file.material(&plain("test", [0.5; 3], 0.5));
        let mesh = file.mesh(&[(&g, material)]).unwrap();
        let (at, turn) = (Vec3::new(3.0, 4.0, 5.0), Quat::from_rotation_y(0.7));
        file.node(mesh, at, turn, 2.0);
        file.node(mesh, -at, Quat::IDENTITY, 1.0);

        let folder = std::env::temp_dir().join(format!("townkit-node-{}", std::process::id()));
        file.write(&folder, "nodes").unwrap();
        let imported = import_gltf(folder.join("nodes.gltf"), &ImportOptions::default());
        std::fs::remove_dir_all(&folder).unwrap();
        let scene = imported.unwrap().scene;

        assert_eq!(scene.instances.len(), 2);
        assert_eq!(scene.instances[0].mesh, scene.instances[1].mesh, "shared");
        let moved = scene.instances[0].transform.transform_point3(Vec3::X);
        assert!(
            moved.abs_diff_eq(at + turn * Vec3::X * 2.0, 1.0e-4),
            "{moved}"
        );
    }

    #[test]
    fn the_importer_reads_the_town_and_finds_every_part_by_name() {
        let folder = std::env::temp_dir().join(format!("townkit-{}", std::process::id()));
        // In a plain material, since a test has no fetched textures to read.
        town_file(|file| file.material(&plain("granite", [0.3, 0.29, 0.27], 0.85)))
            .write(&folder, "town")
            .unwrap();
        let imported = import_gltf(folder.join("town.gltf"), &ImportOptions::default());
        std::fs::remove_dir_all(&folder).unwrap();
        let imported = imported.unwrap();
        assert!(imported.warnings.is_empty(), "{:?}", imported.warnings);
        for part in ["quay", "mole", "massing"] {
            assert!(imported.parts.contains_key(part), "{part} is missing");
        }
        assert_eq!(imported.parts["massing"].len(), 4, "one per material");
    }
}
