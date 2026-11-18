//! Checks a rigged character's glTF before the engine takes it (step 6 of docs/gameplay.md).
//!
//! The export tool (`tools/blender/solstice_export.py`) prepares a character in Blender and has
//! Blender write it. This reads the file back, as the importer will, and lists every way it falls
//! short of what skinning needs:
//! - one skin, which every mesh uses, of at most [`MAX_JOINTS`] bones in one tree, each with an
//!   inverse bind matrix that undoes where the bone stands, so the file is in its rest pose;
//! - triangles, with texture coordinates;
//! - at most four weights a vertex, none negative, summing to one, each naming a bone of the
//!   skin;
//! - no morph targets, which the engine doesn't draw;
//! - every material with its base colour, normal, and metallic-roughness textures, all on the
//!   first texture coordinates, opaque or cut out, never blended;
//! - every bone the rig's role file names, and no leaf bone that deforms nothing unless the role
//!   file names it.
//!
//! What can't be seen in the file, such as geometry a mask hid in Blender, the export tool
//! checks before it writes it.

use glam::Mat4;
use gltf::mesh::Semantic;
use std::collections::BTreeSet;
use std::fmt::Write;
use std::path::Path;

/// How far a vertex's weights may sum from one: glTF's own tolerance for normalised weights is
/// of this order, and Blender writes them as floats.
pub const WEIGHT_SUM_TOLERANCE: f32 = 2.0e-3;

/// The most bones a skeleton may have: the engine's bone indices are 8 bits (docs/gameplay.md,
/// Skinning in virtual geometry).
pub const MAX_JOINTS: usize = 256;

/// How far an inverse bind matrix times its bone's place may be from the identity, in each of
/// its entries: a tenth of a millimetre at a metre.
pub const BIND_TOLERANCE: f32 = 1.0e-4;

/// A rig's role file: which bones play each part, for retargeting animation onto it.
///
/// ```text
/// # ROLE   BONE...           a chain lists its bones from the body outwards
/// pelvis   pelvis
/// spine    spine_01 spine_02 spine_03
/// arm_l    clavicle_l upperarm_l lowerarm_l hand_l
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Roles {
    pub roles: Vec<(String, Vec<String>)>,
}

impl Roles {
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut roles: Vec<(String, Vec<String>)> = Vec::new();
        for (index, line) in text.lines().enumerate() {
            let words: Vec<&str> = line
                .split('#')
                .next()
                .unwrap_or("")
                .split_whitespace()
                .collect();
            let Some((&role, bones)) = words.split_first() else {
                continue;
            };
            let at = |message: String| format!("line {}: {message}", index + 1);
            if bones.is_empty() {
                return Err(at(format!("role `{role}` names no bones")));
            }
            if roles.iter().any(|(r, _)| r == role) {
                return Err(at(format!("role `{role}` is given twice")));
            }
            if let Some(bone) = bones
                .iter()
                .find(|b| roles.iter().any(|(_, bs)| bs.iter().any(|x| x == **b)))
            {
                return Err(at(format!("bone `{bone}` already plays another role")));
            }
            roles.push((
                role.to_owned(),
                bones.iter().map(|b| (*b).to_owned()).collect(),
            ));
        }
        if roles.is_empty() {
            return Err("a role file needs at least one role".to_owned());
        }
        Ok(Self { roles })
    }

    /// Every bone named, in order.
    pub fn bones(&self) -> impl Iterator<Item = &str> {
        self.roles
            .iter()
            .flat_map(|(_, bones)| bones.iter().map(String::as_str))
    }
}

/// What a character file holds, and everything wrong with it.
#[derive(Clone, Debug, Default)]
pub struct CharacterReport {
    pub joints: Vec<String>,
    pub meshes: usize,
    pub vertices: usize,
    pub triangles: usize,
    pub materials: usize,
    /// Every way the file falls short, each a sentence. Empty for a good character.
    pub problems: Vec<String>,
}

impl CharacterReport {
    pub fn summary(&self) -> String {
        let mut text = format!(
            "{} bones, {} meshes, {} vertices, {} triangles, {} materials",
            self.joints.len(),
            self.meshes,
            self.vertices,
            self.triangles,
            self.materials
        );
        if self.problems.is_empty() {
            text.push_str("; no problems");
        } else {
            let _ = write!(text, "; {} problems:", self.problems.len());
            for problem in &self.problems {
                let _ = write!(text, "\n  - {problem}");
            }
        }
        text
    }
}

/// Reads a character's glTF and checks it, against `roles` if given.
pub fn check_character(path: &Path, roles: Option<&Roles>) -> Result<CharacterReport, String> {
    let (document, buffers, _) =
        gltf::import(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut report = CharacterReport::default();
    let mut problems = Vec::new();

    let skins: Vec<_> = document.skins().collect();
    if skins.len() != 1 {
        problems.push(format!("a character has one skin, not {}", skins.len()));
    }
    let skin = skins.first();
    report.joints = skin
        .map(|s| {
            s.joints()
                .map(|j| j.name().unwrap_or("").to_owned())
                .collect()
        })
        .unwrap_or_default();
    let mut used = vec![false; report.joints.len()];
    let mut used_materials = BTreeSet::new();
    if let Some(skin) = skin {
        check_skin(&document, &buffers, skin, &mut problems);
    }

    for node in document.nodes() {
        let Some(mesh) = node.mesh() else {
            continue;
        };
        report.meshes += 1;
        let name = node.name().unwrap_or("(unnamed)");
        if node.skin().map(|s| s.index()) != skin.map(|s| s.index()) {
            problems.push(format!("mesh `{name}` isn't bound to the character's skin"));
        }
        for primitive in mesh.primitives() {
            if primitive.mode() != gltf::mesh::Mode::Triangles {
                problems.push(format!(
                    "mesh `{name}` has a part drawn as {:?}, not triangles: remove loose edges and points",
                    primitive.mode()
                ));
            }
            if primitive.get(&Semantic::TexCoords(0)).is_none() {
                problems.push(format!(
                    "mesh `{name}` has a part with no texture coordinates"
                ));
            }
            if let Some(material) = primitive.material().index() {
                used_materials.insert(material);
            } else {
                problems.push(format!("mesh `{name}` has a part with no material"));
            }
            if primitive.morph_targets().next().is_some() {
                problems.push(format!(
                    "mesh `{name}` has morph targets, which the engine doesn't draw: apply them"
                ));
            }
            if primitive.get(&Semantic::Joints(1)).is_some()
                || primitive.get(&Semantic::Weights(1)).is_some()
            {
                problems.push(format!("mesh `{name}` has more than four weights a vertex"));
            }
            let reader = primitive.reader(|b| buffers.get(b.index()).map(|d| &d.0[..]));
            let vertices = reader.read_positions().map_or(0, |p| p.len());
            report.vertices += vertices;
            report.triangles += reader
                .read_indices()
                .map_or(vertices, |i| i.into_u32().len())
                / 3;
            let (Some(joints), Some(weights)) = (reader.read_joints(0), reader.read_weights(0))
            else {
                problems.push(format!("mesh `{name}` has no bone weights"));
                continue;
            };
            let (mut off, mut stray, mut empty, mut bad) = (0, 0, 0, 0);
            for (joints, weights) in joints.into_u16().zip(weights.into_f32()) {
                if weights.iter().any(|w| !w.is_finite() || *w < 0.0) {
                    bad += 1;
                    continue;
                }
                let sum: f32 = weights.iter().sum();
                if sum < 1.0e-6 {
                    empty += 1;
                } else if (sum - 1.0).abs() > WEIGHT_SUM_TOLERANCE {
                    off += 1;
                }
                for (joint, weight) in joints.iter().zip(weights) {
                    if weight > 0.0 {
                        match used.get_mut(usize::from(*joint)) {
                            Some(slot) => *slot = true,
                            None => stray += 1,
                        }
                    }
                }
            }
            if bad > 0 {
                problems.push(format!(
                    "mesh `{name}`: {bad} vertices have a negative weight, or one that isn't a number"
                ));
            }
            if empty > 0 {
                problems.push(format!("mesh `{name}`: {empty} vertices follow no bone"));
            }
            if off > 0 {
                problems.push(format!(
                    "mesh `{name}`: {off} vertices have weights that don't sum to one"
                ));
            }
            if stray > 0 {
                problems.push(format!(
                    "mesh `{name}`: {stray} weights name a bone the skin doesn't have"
                ));
            }
        }
    }

    for material in document.materials() {
        let Some(index) = material.index() else {
            continue;
        };
        if !used_materials.contains(&index) {
            continue;
        }
        report.materials += 1;
        let name = material.name().unwrap_or("(unnamed)");
        let pbr = material.pbr_metallic_roughness();
        let mut missing = Vec::new();
        if pbr.base_color_texture().is_none() {
            missing.push("base colour");
        }
        if material.normal_texture().is_none() {
            missing.push("normal");
        }
        if pbr.metallic_roughness_texture().is_none() {
            missing.push("metallic-roughness");
        }
        if !missing.is_empty() {
            problems.push(format!(
                "material `{name}` has no {} texture",
                missing.join(" or ")
            ));
        }
        let sets = [
            pbr.base_color_texture().map(|t| t.tex_coord()),
            material.normal_texture().map(|t| t.tex_coord()),
            pbr.metallic_roughness_texture().map(|t| t.tex_coord()),
        ];
        if sets.iter().flatten().any(|&set| set != 0) {
            problems.push(format!(
                "material `{name}` reads a texture through other texture coordinates than the first"
            ));
        }
        if material.alpha_mode() == gltf::material::AlphaMode::Blend {
            problems.push(format!(
                "material `{name}` is blended: make it opaque, or cut out with a mask"
            ));
        }
    }

    if let Some(roles) = roles {
        for bone in roles.bones() {
            if !report.joints.iter().any(|j| j == bone) {
                problems.push(format!("the role file's bone `{bone}` isn't in the skin"));
            }
        }
    }
    // A bone that deforms nothing is dead weight, unless animation needs it by name, or other
    // bones hang from it.
    let joint_nodes: Vec<gltf::Node> = skin.map(|s| s.joints().collect()).unwrap_or_default();
    let in_skin: BTreeSet<usize> = joint_nodes.iter().map(|n| n.index()).collect();
    for ((joint, used), node) in report.joints.iter().zip(&used).zip(&joint_nodes) {
        let named = roles.is_some_and(|r| r.bones().any(|b| b == joint));
        let leaf = !node.children().any(|c| in_skin.contains(&c.index()));
        if !used && !named && leaf {
            problems.push(format!(
                "bone `{joint}` deforms nothing and plays no role: remove it"
            ));
        }
    }
    // A mesh in several parts would say the same thing once a part.
    let mut seen = BTreeSet::new();
    problems.retain(|p| seen.insert(p.clone()));
    report.problems = problems;
    Ok(report)
}

/// Checks the skeleton: its size, that it is one tree, and that each inverse bind matrix undoes
/// where its bone stands, so the file is in the pose the skin was bound in.
fn check_skin(
    document: &gltf::Document,
    buffers: &[gltf::buffer::Data],
    skin: &gltf::Skin,
    problems: &mut Vec<String>,
) {
    let joints: Vec<gltf::Node> = skin.joints().collect();
    if joints.len() > MAX_JOINTS {
        problems.push(format!(
            "the skin has {} bones, more than the {MAX_JOINTS} the engine takes",
            joints.len()
        ));
    }
    // Each node's parent, and its place in the world.
    let mut parent = vec![None; document.nodes().len()];
    for node in document.nodes() {
        for child in node.children() {
            parent[child.index()] = Some(node.index());
        }
    }
    let local: Vec<Mat4> = document
        .nodes()
        .map(|n| Mat4::from_cols_array_2d(&n.transform().matrix()))
        .collect();
    let world = |mut index: usize| {
        let mut matrix = local[index];
        while let Some(up) = parent[index] {
            matrix = local[up] * matrix;
            index = up;
        }
        matrix
    };
    let in_skin: BTreeSet<usize> = joints.iter().map(|j| j.index()).collect();
    let roots = joints
        .iter()
        .filter(|j| !parent[j.index()].is_some_and(|p| in_skin.contains(&p)))
        .count();
    if roots != 1 {
        problems.push(format!("the skin's bones form {roots} trees, not one"));
    }
    let reader = skin.reader(|b| buffers.get(b.index()).map(|d| &d.0[..]));
    let Some(inverse_binds) = reader.read_inverse_bind_matrices() else {
        problems.push("the skin has no inverse bind matrices".to_owned());
        return;
    };
    let inverse_binds: Vec<Mat4> = inverse_binds
        .map(|m| Mat4::from_cols_array_2d(&m))
        .collect();
    if inverse_binds.len() != joints.len() {
        problems.push(format!(
            "the skin has {} inverse bind matrices for {} bones",
            inverse_binds.len(),
            joints.len()
        ));
        return;
    }
    let posed: Vec<&str> = joints
        .iter()
        .zip(&inverse_binds)
        .filter(|(joint, inverse)| {
            !(**inverse * world(joint.index())).abs_diff_eq(Mat4::IDENTITY, BIND_TOLERANCE)
        })
        .map(|(joint, _)| joint.name().unwrap_or("(unnamed)"))
        .collect();
    if !posed.is_empty() {
        problems.push(format!(
            "{} bones aren't where the skin was bound, so the file isn't in its rest pose: {}",
            posed.len(),
            posed.iter().take(5).copied().collect::<Vec<_>>().join(", ")
        ));
    }
}

#[cfg(test)]
#[path = "character_tests.rs"]
mod tests;
