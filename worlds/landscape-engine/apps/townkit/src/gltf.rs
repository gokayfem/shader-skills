//! Writes meshes as a glTF 2.0 file and its binary buffer.

use crate::geometry::Geometry;
use glam::{Quat, Vec3};
use std::collections::HashMap;
use std::fmt::Write as _;
use std::io;
use std::path::Path;

/// A material with no textures.
pub struct Plain {
    pub name: &'static str,
    /// Linear base colour.
    pub color: [f32; 3],
    pub roughness: f32,
}

/// A material whose maps come from a Poly Haven texture fetched by `scripts/fetch-scans.py`.
pub struct Textured {
    /// The texture's name, which is also its folder under `target/scan`.
    pub surface: &'static str,
    /// How many metres one repeat of the texture covers.
    pub metres: f32,
    /// Whether it has an occlusion-roughness-metallic map, rather than only a roughness map.
    pub arm: bool,
    /// Multiplies the texture's colour.
    pub tint: [f32; 3],
}

/// One glTF file being put together. A part is a node at the origin with its vertices in world
/// space, so that the scene places it at 0 0 0; a node instances a shared mesh.
#[derive(Default)]
pub struct File {
    bin: Vec<u8>,
    materials: Vec<String>,
    accessors: Vec<String>,
    views: Vec<String>,
    meshes: Vec<String>,
    nodes: Vec<String>,
    images: Vec<String>,
    textures: Vec<String>,
    /// Each surface's colour, normal and roughness textures, so that tints of it share them.
    surfaces: HashMap<(&'static str, bool), (usize, usize, usize)>,
}

impl File {
    pub fn material(&mut self, material: &Plain) -> usize {
        let [r, g, b] = material.color;
        self.materials.push(format!(
            r#"{{"name":"{}","pbrMetallicRoughness":{{"baseColorFactor":[{r},{g},{b},1],"metallicFactor":0,"roughnessFactor":{}}}}}"#,
            material.name, material.roughness
        ));
        self.materials.len() - 1
    }

    fn texture(&mut self, uri: String) -> usize {
        self.images.push(format!(r#"{{"uri":"{uri}"}}"#));
        self.textures
            .push(format!(r#"{{"source":{}}}"#, self.images.len() - 1));
        self.textures.len() - 1
    }

    /// Adds a material for a fetched texture. UVs are in metres, as [`Geometry`] makes them, and
    /// the texture's own transform scales them to its repeat.
    pub fn textured(&mut self, material: &Textured) -> usize {
        let name = material.surface;
        let (color, normal, roughness) = match self.surfaces.get(&(name, material.arm)) {
            Some(&maps) => maps,
            None => {
                let mut map = |kind: &str| {
                    self.texture(format!("../scan/{name}/textures/{name}_{kind}_2k.jpg"))
                };
                let maps = (
                    map("diff"),
                    map("nor_gl"),
                    map(if material.arm { "arm" } else { "rough" }),
                );
                self.surfaces.insert((name, material.arm), maps);
                maps
            }
        };
        let scale = 1.0 / material.metres;
        let [red, green, blue] = material.tint;
        let transform =
            format!(r#""extensions":{{"KHR_texture_transform":{{"scale":[{scale},{scale}]}}}}"#);
        let occlusion = if material.arm {
            format!(r#","occlusionTexture":{{"index":{roughness},{transform}}}"#)
        } else {
            String::new()
        };
        self.materials.push(format!(
            r#"{{"name":"{name}","pbrMetallicRoughness":{{"baseColorFactor":[{red},{green},{blue},1],"baseColorTexture":{{"index":{color},{transform}}},"metallicRoughnessTexture":{{"index":{roughness},{transform}}},"metallicFactor":0,"roughnessFactor":1}},"normalTexture":{{"index":{normal},{transform}}}{occlusion}}}"#
        ));
        self.materials.len() - 1
    }

    /// Adds raw little-endian values and an accessor for them, returning the accessor's number.
    fn accessor(
        &mut self,
        bytes: &[u8],
        component: u32,
        count: usize,
        kind: &str,
        bounds: Option<String>,
    ) -> usize {
        let offset = self.bin.len();
        self.bin.extend_from_slice(bytes);
        self.views.push(format!(
            r#"{{"buffer":0,"byteOffset":{offset},"byteLength":{}}}"#,
            bytes.len()
        ));
        let mut accessor = format!(
            r#"{{"bufferView":{},"componentType":{component},"count":{count},"type":"{kind}""#,
            self.views.len() - 1
        );
        if let Some(bounds) = bounds {
            accessor.push_str(&bounds);
        }
        accessor.push('}');
        self.accessors.push(accessor);
        self.accessors.len() - 1
    }

    fn floats(
        &mut self,
        values: &[f32],
        count: usize,
        kind: &str,
        bounds: Option<String>,
    ) -> usize {
        let bytes: Vec<u8> = values.iter().flat_map(|v| v.to_le_bytes()).collect();
        self.accessor(&bytes, 5126, count, kind, bounds)
    }

    /// Adds a named part at the origin, made of pieces, each in its own material.
    pub fn part(&mut self, name: &str, pieces: &[(&Geometry, usize)]) {
        if let Some(mesh) = self.mesh(pieces) {
            self.nodes
                .push(format!(r#"{{"name":"{name}","mesh":{mesh}}}"#));
        }
    }

    /// Places a shared mesh, with its own origin at `at`.
    pub fn node(&mut self, mesh: usize, at: Vec3, rotation: Quat, scale: f32) {
        let (r, t) = (rotation.to_array(), at.to_array());
        self.nodes.push(format!(
            r#"{{"mesh":{mesh},"translation":[{},{},{}],"rotation":[{},{},{},{}],"scale":[{scale},{scale},{scale}]}}"#,
            t[0], t[1], t[2], r[0], r[1], r[2], r[3]
        ));
    }

    /// Adds a mesh made of pieces, each in its own material, returning its number. Pieces with no
    /// triangles are left out; if none are left, there is no mesh.
    pub fn mesh(&mut self, pieces: &[(&Geometry, usize)]) -> Option<usize> {
        let mut primitives = String::new();
        for (geometry, material) in pieces.iter().filter(|(g, _)| !g.indices.is_empty()) {
            let (mut min, mut max) = (glam::Vec3::splat(f32::MAX), glam::Vec3::splat(f32::MIN));
            for &p in &geometry.positions {
                min = min.min(p);
                max = max.max(p);
            }
            let bounds = format!(
                r#","min":[{},{},{}],"max":[{},{},{}]"#,
                min.x, min.y, min.z, max.x, max.y, max.z
            );
            let count = geometry.positions.len();
            let positions: Vec<f32> = geometry
                .positions
                .iter()
                .flat_map(|p| p.to_array())
                .collect();
            let normals: Vec<f32> = geometry.normals.iter().flat_map(|n| n.to_array()).collect();
            let uvs: Vec<f32> = geometry.uvs.iter().flat_map(|uv| uv.to_array()).collect();
            let position = self.floats(&positions, count, "VEC3", Some(bounds));
            let normal = self.floats(&normals, count, "VEC3", None);
            let uv = self.floats(&uvs, count, "VEC2", None);
            let index_bytes: Vec<u8> = geometry
                .indices
                .iter()
                .flat_map(|i| i.to_le_bytes())
                .collect();
            let indices = self.accessor(&index_bytes, 5125, geometry.indices.len(), "SCALAR", None);
            if !primitives.is_empty() {
                primitives.push(',');
            }
            let _ = write!(
                primitives,
                r#"{{"attributes":{{"POSITION":{position},"NORMAL":{normal},"TEXCOORD_0":{uv}}},"indices":{indices},"material":{material}}}"#
            );
        }
        if primitives.is_empty() {
            return None;
        }
        self.meshes
            .push(format!(r#"{{"primitives":[{primitives}]}}"#));
        Some(self.meshes.len() - 1)
    }

    /// Writes `NAME.gltf` and `NAME.bin` into `folder`.
    pub fn write(&self, folder: &Path, name: &str) -> io::Result<()> {
        let nodes: Vec<String> = (0..self.nodes.len()).map(|i| i.to_string()).collect();
        let textures = if self.textures.is_empty() {
            String::new()
        } else {
            format!(
                r#""extensionsUsed":["KHR_texture_transform"],"images":[{}],"textures":[{}],"#,
                self.images.join(","),
                self.textures.join(",")
            )
        };
        let json = format!(
            r#"{{"asset":{{"version":"2.0","generator":"townkit"}},{textures}"scene":0,"scenes":[{{"nodes":[{}]}}],"nodes":[{}],"meshes":[{}],"materials":[{}],"accessors":[{}],"bufferViews":[{}],"buffers":[{{"uri":"{name}.bin","byteLength":{}}}]}}"#,
            nodes.join(","),
            self.nodes.join(","),
            self.meshes.join(","),
            self.materials.join(","),
            self.accessors.join(","),
            self.views.join(","),
            self.bin.len(),
        );
        std::fs::create_dir_all(folder)?;
        std::fs::write(folder.join(format!("{name}.gltf")), json)?;
        std::fs::write(folder.join(format!("{name}.bin")), &self.bin)
    }
}
