//! Ready-made texture layers. Procedural surfaces such as grass and sand are the same in every
//! scene and take minutes to generate, far too long for the editor's bake in the browser
//! (`docs/editor.md`). They are made once by `bake --surfaces`, as ordinary scene files holding
//! one layer each, and the browser hands them over with [`provide`] before it bakes. A bake asks
//! for each layer through [`layer`], which gives the one provided or makes it.
//!
//! Provided layers are block compressed, as `bake --surfaces` wrote them; the bake's compression
//! leaves them as they are (`solstice_builder::compress_textures`). A natively baked scene
//! provides none, so it is made exactly as before.

use solstice_asset::{file, SceneAsset, TextureEncoding, TextureLayer};
use solstice_builder::{compress_textures, paginate};
use std::sync::Mutex;

/// Layers provided, by key and edge length.
static PROVIDED: Mutex<Vec<(String, u32, TextureLayer)>> = Mutex::new(Vec::new());

/// The key of a built-in surface, as `solstice_procgen::surfaces::by_name` names it.
pub fn surface_key(name: &str, seed: u32) -> String {
    format!("surface-{name}-{seed}")
}

/// A built-in surface's layer at `size`, seed 0: the one provided, or else generated.
///
/// # Panics
/// If the procedural library has no surface called `name`; callers name built-in ones.
pub fn surface(name: &str, size: u32) -> TextureLayer {
    layer(&surface_key(name, 0), size, || {
        solstice_procgen::surfaces::by_name(name, 0)
            .unwrap_or_else(|| panic!("no built-in surface `{name}`"))
            .bake(size)
    })
}

/// Hands over a ready-made layer, for any bake from now on that asks for `key` at `size`.
pub fn provide(key: &str, size: u32, layer: TextureLayer) {
    let mut provided = PROVIDED.lock().unwrap_or_else(|e| e.into_inner());
    provided.retain(|(k, s, _)| !(k == key && *s == size));
    provided.push((key.to_owned(), size, layer));
}

/// Whether a layer for `key` at `size` has been provided.
pub fn provided(key: &str, size: u32) -> bool {
    let provided = PROVIDED.lock().unwrap_or_else(|e| e.into_inner());
    provided.iter().any(|(k, s, _)| k == key && *s == size)
}

/// The layer for `key` at `size`: the one provided, or else `make`'s.
pub fn layer(key: &str, size: u32, make: impl FnOnce() -> TextureLayer) -> TextureLayer {
    let found = {
        let provided = PROVIDED.lock().unwrap_or_else(|e| e.into_inner());
        provided
            .iter()
            .find(|(k, s, _)| k == key && *s == size)
            .map(|(_, _, layer)| layer.clone())
    };
    found.unwrap_or_else(make)
}

/// A scene file holding one layer, compressed, as `bake --surfaces` writes them.
pub fn encode_layer(layer: TextureLayer, size: u32) -> Vec<u8> {
    let mut scene = SceneAsset {
        texture_size: size,
        textures: vec![layer],
        ..Default::default()
    };
    compress_textures(&mut scene);
    file::encode(&paginate(&scene))
}

/// The layer in a scene file `encode_layer` wrote, whole, and its edge length.
pub fn decode_layer(bytes: &[u8]) -> Result<(TextureLayer, u32), String> {
    let loaded = file::decode(bytes).map_err(|e| e.to_string())?;
    let scene = &loaded.scene;
    if scene.base_textures.len() != 1 || loaded.upper_textures.len() != 1 {
        return Err("a surface's file holds one texture layer".into());
    }
    if scene.texture_encoding != TextureEncoding::Block {
        return Err("a surface's file is block compressed".into());
    }
    let (upper, base) = (&loaded.upper_textures[0], &scene.base_textures[0]);
    let join = |a: &[Vec<u8>], b: &[Vec<u8>]| a.iter().chain(b).cloned().collect();
    Ok((
        TextureLayer {
            color_mips: join(&upper.color_mips, &base.color_mips),
            normal_mips: join(&upper.normal_mips, &base.normal_mips),
            material_mips: join(&upper.material_mips, &base.material_mips),
        },
        scene.texture_size,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parts(layer: &TextureLayer) -> [&Vec<Vec<u8>>; 3] {
        [&layer.color_mips, &layer.normal_mips, &layer.material_mips]
    }

    fn raw_layer(size: u32) -> TextureLayer {
        let mips = |channels: usize| -> Vec<Vec<u8>> {
            (0..size.trailing_zeros() + 1)
                .map(|level| {
                    let edge = (size >> level) as usize;
                    (0..edge * edge * channels)
                        .map(|i| (i * 7 + level as usize) as u8)
                        .collect()
                })
                .collect()
        };
        TextureLayer {
            color_mips: mips(4),
            normal_mips: mips(2),
            material_mips: mips(2),
        }
    }

    #[test]
    fn a_layer_comes_back_as_it_was_compressed() {
        let size = 64;
        let bytes = encode_layer(raw_layer(size), size);
        let (layer, read_size) = decode_layer(&bytes).unwrap();
        assert_eq!(read_size, size);
        let mut expected = SceneAsset {
            texture_size: size,
            textures: vec![raw_layer(size)],
            ..Default::default()
        };
        compress_textures(&mut expected);
        assert_eq!(parts(&layer), parts(&expected.textures[0]));
    }

    #[test]
    fn a_provided_layer_is_used_and_others_are_made() {
        let size = 16;
        let made = || raw_layer(size);
        let key = "surface-test-provided-0";
        assert_eq!(parts(&layer(key, size, made)), parts(&raw_layer(size)));
        let mut other = raw_layer(size);
        other.color_mips[0][0] ^= 1;
        provide(key, size, other.clone());
        assert!(provided(key, size));
        assert_eq!(parts(&layer(key, size, made)), parts(&other));
        // Another size is made afresh.
        assert_eq!(parts(&layer(key, 8, || raw_layer(8))), parts(&raw_layer(8)));
    }
}
