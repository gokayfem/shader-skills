//! Builds scenes (`docs/editor.md`, "One bake"): from an editor project or a scene's text, into a
//! [`solstice_asset::SceneAsset`] ready to page and write. The native `bake` command and the
//! editor's bake in the browser both use it, so the two give the same scene.

pub mod circuit;
pub mod foliage;
pub mod land;
pub mod lend;
pub mod meshes;
pub mod project;
pub mod scene_file;
pub mod textures;
