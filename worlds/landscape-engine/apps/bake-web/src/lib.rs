//! The bake, for the browser (`docs/editor.md`, "One bake"): the editor's Web Worker hands it a
//! project's scene and gets back the scene file the viewer plays, the same bytes the native
//! `bake --project` writes for it.
//!
//! Surfaces take minutes to make, so the worker first hands over ready-made ones
//! (`bake --surfaces`) with [`provide_layer`]. Files a scene names, such as models and actors,
//! can't be read here yet: a scene that needs them is refused with what it asked for.

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

/// Bakes a project's scene, given as its file's text. `name` names it in errors.
pub fn bake(scene: &str, name: &str) -> Result<Vec<u8>, String> {
    let scene = solstice_project::Scene::parse(scene).map_err(|e| format!("{name}: {e}"))?;
    let (built, _warnings) =
        solstice_bake::project::build_scene(&scene, name, std::path::Path::new("."), None)?;
    Ok(solstice_bake::project::encode(built))
}

/// Hands over a ready-made texture layer, from a file `bake --surfaces` wrote, under `key`.
/// Returns its edge length.
pub fn provide(key: &str, file: &[u8]) -> Result<u32, String> {
    let (layer, size) = solstice_bake::textures::decode_layer(file)?;
    solstice_bake::textures::provide(key, size, layer);
    Ok(size)
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen(start)]
pub fn start() {
    console_error_panic_hook::set_once();
}

/// See [`bake`].
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn bake_scene(scene: &str, name: &str) -> Result<Vec<u8>, JsValue> {
    bake(scene, name).map_err(|e| JsValue::from_str(&e))
}

/// See [`provide`].
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn provide_layer(key: &str, file: &[u8]) -> Result<u32, JsValue> {
    provide(key, file).map_err(|e| JsValue::from_str(&e))
}

/// The key a built-in surface's layer is provided under.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn surface_key(name: &str) -> String {
    solstice_bake::textures::surface_key(name, 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A blank scene's terrain bakes to the bytes `bake --project` gives, with no files read.
    #[test]
    fn a_blank_scene_bakes_as_the_native_bake_does() {
        let text = r#"{
  "format": 1,
  "camera": { "position": [0.0, 20.0, 60.0], "yaw_radians": 0.0, "pitch_radians": -0.2 },
  "terrain": { "size": 64.0, "cells": 32, "height": 0.3, "shore_x": 20.0 },
  "sea": { "level": -0.6, "ocean": "coastal" }
}"#;
        let scene = solstice_project::Scene::parse(text).unwrap();
        let (native, _) = solstice_bake::project::build_scene(
            &scene,
            "t.scene",
            std::path::Path::new("."),
            Some(32),
        )
        .unwrap();
        let mut small = text.to_owned();
        small = small.replace("\"format\": 1,", "\"format\": 1,\n  \"texture_size\": 32,");
        let baked = bake(&small, "t.scene").unwrap();
        assert_eq!(baked, solstice_bake::project::encode(native));
        assert!(bake("{", "t.scene").unwrap_err().contains("t.scene"));
    }
}
