//! Packs a truck that is built of pieces: `bake --kit TRUCK.actor --output TRUCK.truck`.
//!
//! The truck's model has every piece of it, each a named object: the bare truck, and every
//! thing that can be fitted to it. Baked into a scene, all
//! of it would be read whenever the truck was. Packed, each piece is a small scene of its
//! own in one file (`solstice_asset::pack`), beside the texts that say what the truck is:
//! its actor file, its numbers and its kit. The game reads the texts, and then only the
//! pieces the truck is wearing (`solstice_asset::pack::read`).

use solstice_asset::{pack, SceneAsset};
use solstice_game::{resolve, ActorFile};
use solstice_import::{import_gltf, ImportOptions};
use std::collections::HashMap;
use std::path::Path;

pub fn bake_kit(actor_path: &Path, output: &Path) -> Result<(), String> {
    let read = |path: &Path| {
        std::fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))
    };
    let file_name = |path: &Path| {
        let name = path.file_name().ok_or("the actor file has no name")?;
        Ok::<String, String>(name.to_string_lossy().into_owned())
    };
    let actor_name = file_name(actor_path)?;
    let actor_text = read(actor_path)?;
    let actor = ActorFile::parse(&actor_text).map_err(|error| format!("{actor_name}: {error}"))?;
    let folder = actor_path.parent().unwrap_or(Path::new("."));
    let mesh = actor.mesh.as_ref().ok_or("the actor has no mesh")?;
    let (Some(vehicle), Some(kit)) = (&actor.vehicle, &actor.kit) else {
        return Err(
            "the actor is not a truck with a kit: it needs `vehicle` and `kit` lines".into(),
        );
    };
    // Its texts, by the names its actor file knows them by.
    let mut texts = vec![(actor_name.clone(), actor_text.clone())];
    for named in [&vehicle.path, kit] {
        texts.push((resolve(&actor_name, named), read(&folder.join(named))?));
    }

    let model = folder.join(&mesh.path);
    let imported = import_gltf(&model, &ImportOptions::default())
        .map_err(|error| format!("{}: {error}", model.display()))?;
    let whole = imported.scene;
    // Each piece is the instances of one named object, with the meshes and the materials
    // they use and no others.
    let mut order: Vec<&str> = Vec::new();
    for name in imported.names.iter().filter(|name| !name.is_empty()) {
        if !order.contains(&name.as_str()) {
            order.push(name);
        }
    }
    let mut pieces = Vec::new();
    let (mut triangles, mut unpinned) = (0, 0);
    for name in order {
        let mut piece = SceneAsset::default();
        let (mut meshes, mut materials) = (HashMap::new(), HashMap::new());
        for (instance, _) in whole
            .instances
            .iter()
            .zip(&imported.names)
            .filter(|(_, of)| of.as_str() == name)
        {
            let mesh = *meshes.entry(instance.mesh).or_insert_with(|| {
                piece
                    .meshes
                    .push(whole.meshes[instance.mesh as usize].clone());
                piece.meshes.len() as u32 - 1
            });
            let material = *materials.entry(instance.material).or_insert_with(|| {
                piece
                    .materials
                    .push(whole.materials[instance.material as usize]);
                piece.materials.len() as u32 - 1
            });
            piece.instances.push(solstice_asset::Instance {
                mesh,
                material,
                transform: instance.transform,
            });
        }
        triangles += piece.total_source_triangles();
        let mut loaded = solstice_builder::paginate(&piece);
        // A truck bumps into things with its own box, not with its pieces.
        loaded.collision = None;
        unpinned += usize::from(!loaded.pages.is_empty());
        pieces.push((name, solstice_asset::file::encode(&loaded)));
    }

    let mut things: Vec<(&str, &str, &[u8])> = Vec::new();
    for (name, text) in &texts {
        things.push(("text", name, text.as_bytes()));
    }
    for (name, bytes) in &pieces {
        things.push(("piece", name, bytes));
    }
    let file = pack::write(&things);
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("{}: {error}", parent.display()))?;
    }
    std::fs::write(output, &file).map_err(|error| format!("{}: {error}", output.display()))?;
    println!(
        "wrote {}: {} pieces, {triangles} triangles, {:.1} MB{}",
        output.display(),
        pieces.len(),
        file.len() as f64 / 1e6,
        if unpinned > 0 {
            format!(" ({unpinned} pieces have pages that are not always loaded)")
        } else {
            String::new()
        }
    );
    Ok(())
}
