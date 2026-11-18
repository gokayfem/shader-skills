//! Each test writes a small skinned glTF by hand, so every value checked is known exactly.

use super::*;
use std::path::PathBuf;

/// A scratch folder, with a one-pixel texture in it, removed when dropped.
struct Folder(PathBuf);

impl Folder {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let unique = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "solstice-character-{}-{unique}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let file = std::fs::File::create(dir.join("t.png")).unwrap();
        let mut png = png::Encoder::new(std::io::BufWriter::new(file), 1, 1);
        png.set_color(png::ColorType::Rgba);
        png.write_header()
            .unwrap()
            .write_image_data(&[200, 150, 120, 255])
            .unwrap();
        Self(dir)
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

const IDENTITY: [f32; 16] = [
    1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
];

/// A character to write: a quad of four vertices, skinned to a chain of bones.
struct Character {
    /// Each bone, the child of the one before.
    bones: Vec<&'static str>,
    /// Each vertex's four bones and their weights.
    weights: [([u16; 4], [f32; 4]); 4],
    morph: bool,
    second_weights: bool,
    skinned: bool,
    /// How many copies of the skin the file has.
    skins: usize,
    /// Base colour, normal, and metallic-roughness textures.
    textures: [bool; 3],
    alpha: &'static str,
    uvs: bool,
    /// The primitive's mode: 4 is triangles.
    mode: u32,
    /// Whether the skin has inverse bind matrices, and how many it has short.
    inverse_binds: bool,
    binds_short: usize,
    /// Moves the second bone a metre up, away from where the skin was bound.
    posed: bool,
    /// Leaves the third bone out of the chain, as a tree of its own.
    two_trees: bool,
    /// The texture coordinates the normal map reads through.
    normal_tex_coord: u32,
    material: bool,
    weighted: bool,
    /// Adds a material no mesh uses, without a single texture.
    unused_material: bool,
    /// Leaves out the index buffer, so each three vertices are a triangle.
    unindexed: bool,
    /// Lists the skin's joints last bone first, so children come before their parents.
    reversed_joints: bool,
}

impl Default for Character {
    fn default() -> Self {
        Self {
            bones: vec!["Root", "pelvis", "spine"],
            weights: [
                ([1, 0, 0, 0], [1.0, 0.0, 0.0, 0.0]),
                ([1, 2, 0, 0], [0.5, 0.5, 0.0, 0.0]),
                ([2, 0, 0, 0], [1.0, 0.0, 0.0, 0.0]),
                ([2, 1, 0, 0], [0.75, 0.25, 0.0, 0.0]),
            ],
            morph: false,
            second_weights: false,
            skinned: true,
            skins: 1,
            textures: [true; 3],
            alpha: "OPAQUE",
            uvs: true,
            mode: 4,
            inverse_binds: true,
            binds_short: 0,
            posed: false,
            two_trees: false,
            normal_tex_coord: 0,
            material: true,
            weighted: true,
            unused_material: false,
            unindexed: false,
            reversed_joints: false,
        }
    }
}

fn floats(values: &[f32]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_le_bytes()).collect()
}

impl Character {
    /// Writes it, and checks it against `roles`.
    fn check(&self, roles: Option<&str>) -> CharacterReport {
        let folder = Folder::new();
        let path = self.write(&folder);
        let roles = roles.map(|r| Roles::parse(r).unwrap());
        check_character(&path, roles.as_ref()).unwrap()
    }

    /// Writes it into `folder`, and says where.
    fn write(&self, folder: &Folder) -> PathBuf {
        let mut bin = Vec::new();
        let mut views = Vec::new();
        let mut add = |bytes: Vec<u8>| {
            views.push(format!(
                r#"{{"buffer": 0, "byteOffset": {}, "byteLength": {}}}"#,
                bin.len(),
                bytes.len()
            ));
            bin.extend(bytes);
            views.len() - 1
        };
        let p = add(floats(&[0., 0., 0., 1., 0., 0., 1., 1., 0., 0., 1., 0.]));
        let i = add([0u16, 1, 2, 0, 2, 3]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect());
        let j = add(self
            .weights
            .iter()
            .flat_map(|(j, _)| j.iter().flat_map(|v| v.to_le_bytes()))
            .collect());
        let w = add(floats(
            &self
                .weights
                .iter()
                .flat_map(|(_, w)| *w)
                .collect::<Vec<_>>(),
        ));
        let bones = self.bones.len();
        let binds = bones - self.binds_short;
        let ibm = add(floats(
            &(0..binds).flat_map(|_| IDENTITY).collect::<Vec<_>>(),
        ));
        let uv = add(floats(&[0., 0., 1., 0., 1., 1., 0., 1.]));
        let mut accessors = vec![
            format!(
                r#"{{"bufferView": {p}, "componentType": 5126, "count": 4, "type": "VEC3", "min": [0,0,0], "max": [1,1,0]}}"#
            ),
            format!(
                r#"{{"bufferView": {i}, "componentType": 5123, "count": 6, "type": "SCALAR"}}"#
            ),
            format!(r#"{{"bufferView": {j}, "componentType": 5123, "count": 4, "type": "VEC4"}}"#),
            format!(r#"{{"bufferView": {w}, "componentType": 5126, "count": 4, "type": "VEC4"}}"#),
            format!(
                r#"{{"bufferView": {ibm}, "componentType": 5126, "count": {binds}, "type": "MAT4"}}"#
            ),
            format!(r#"{{"bufferView": {uv}, "componentType": 5126, "count": 4, "type": "VEC2"}}"#),
        ];
        let mut attributes = r#""POSITION": 0"#.to_owned();
        if self.weighted {
            attributes += r#", "JOINTS_0": 2, "WEIGHTS_0": 3"#;
        }
        if self.uvs {
            attributes += r#", "TEXCOORD_0": 5"#;
        }
        if self.second_weights {
            attributes += r#", "JOINTS_1": 2, "WEIGHTS_1": 3"#;
        }
        let targets = if self.morph {
            accessors.push(accessors[0].clone());
            r#", "targets": [{"POSITION": 6}]"#
        } else {
            ""
        };
        let [base, normal, mr] = self.textures;
        let texture = |on: bool, key: &str, set: u32| {
            if on {
                format!(r#""{key}": {{"index": 0, "texCoord": {set}}},"#)
            } else {
                String::new()
            }
        };
        let mut materials = vec![format!(
            r#"{{"name": "skin", {} "pbrMetallicRoughness": {{ {} {} "roughnessFactor": 1}}, "alphaMode": "{}"}}"#,
            texture(normal, "normalTexture", self.normal_tex_coord),
            texture(base, "baseColorTexture", 0),
            texture(mr, "metallicRoughnessTexture", 0),
            self.alpha
        )];
        if self.unused_material {
            materials.push(r#"{"name": "spare", "alphaMode": "BLEND"}"#.to_owned());
        }
        let mut primitive = format!(r#""attributes": {{ {attributes} }}, "mode": {}"#, self.mode);
        if !self.unindexed {
            primitive += r#", "indices": 1"#;
        }
        if self.material {
            primitive += r#", "material": 0"#;
        }
        // Node 0 is the mesh; the bones follow, each the parent of the next.
        let mut nodes = vec![format!(
            r#"{{"name": "body", "mesh": 0 {}}}"#,
            if self.skinned { r#", "skin": 0"# } else { "" }
        )];
        let mut scene_roots = vec!["0".to_owned(), "1".to_owned()];
        for (index, bone) in self.bones.iter().enumerate() {
            let separate = self.two_trees && index == 1;
            let children = if index + 1 < bones && !separate {
                format!(r#", "children": [{}]"#, index + 2)
            } else {
                String::new()
            };
            if self.two_trees && index == 2 {
                scene_roots.push((index + 1).to_string());
            }
            let place = if self.posed && index == 1 {
                r#", "translation": [0, 1, 0]"#
            } else {
                ""
            };
            nodes.push(format!(r#"{{"name": "{bone}" {children} {place}}}"#));
        }
        let mut joints: Vec<String> = (1..=bones).map(|n| n.to_string()).collect();
        if self.reversed_joints {
            joints.reverse();
        }
        let binds = if self.inverse_binds {
            r#", "inverseBindMatrices": 4"#
        } else {
            ""
        };
        let skin = format!(r#"{{"joints": [{}] {binds}}}"#, joints.join(","));
        let json = format!(
            r#"{{
            "asset": {{"version": "2.0"}},
            "scene": 0, "scenes": [{{"nodes": [{}]}}],
            "nodes": [{}],
            "skins": [{}],
            "meshes": [{{"primitives": [{{ {primitive} {targets} }}]}}],
            "materials": [{}],
            "images": [{{"uri": "t.png"}}],
            "textures": [{{"source": 0}}],
            "buffers": [{{"uri": "c.bin", "byteLength": {}}}],
            "bufferViews": [{}],
            "accessors": [{}]
        }}"#,
            scene_roots.join(","),
            nodes.join(","),
            vec![skin; self.skins].join(","),
            materials.join(","),
            bin.len(),
            views.join(","),
            accessors.join(",")
        );
        std::fs::write(folder.0.join("c.bin"), &bin).unwrap();
        let path = folder.0.join("c.gltf");
        std::fs::write(&path, json).unwrap();
        path
    }
}

fn import_skinned(character: &Character) -> Result<crate::Imported, crate::ImportError> {
    let folder = Folder::new();
    let path = character.write(&folder);
    crate::import_gltf(
        path,
        &crate::ImportOptions {
            texture_size: 4,
            skinned: true,
            ..Default::default()
        },
    )
}

#[test]
fn importing_a_skin_keeps_its_bones_and_weights() {
    let imported = import_skinned(&Character::default()).unwrap();
    let scene = &imported.scene;
    assert_eq!(scene.skeletons.len(), 1);
    let bones = &scene.skeletons[0].bones;
    assert_eq!(
        bones.iter().map(|b| b.name.as_str()).collect::<Vec<_>>(),
        ["Root", "pelvis", "spine"]
    );
    assert_eq!(bones[0].parent, None);
    assert_eq!(bones[2].parent, Some(1));
    assert_eq!(scene.poses, [0]);
    assert_eq!(scene.instance_poses, [0]);

    let mesh = &scene.meshes[0];
    assert!(mesh.skinned());
    let cluster = mesh.clusters[0];
    assert_eq!(cluster.vertex_count(), 4);
    // The four vertices' skins follow them, end to end.
    let records = &mesh.vertices[cluster.vertex_offset as usize + 4..];
    let skins: Vec<solstice_asset::PackedSkin> = (0..4)
        .map(|vertex| solstice_asset::gpu::skin_at(records, vertex))
        .collect();
    // Vertices may be listed in any order, but each of the file's four skins must be there.
    for (joints, weights) in Character::default().weights {
        let wanted = solstice_asset::PackedSkin::new(joints.map(|j| j as u8), weights);
        assert!(skins.contains(&wanted), "{wanted:?} missing from {skins:?}");
    }
    assert_eq!(&mesh.cluster_bones[0][..3], &[1, 2, 2]);
    // No weight reaches the root, so it moves nothing.
    assert_eq!(bones[0].radius, 0.0);
    assert!(bones[2].radius > 0.5, "{}", bones[2].radius);
}

/// The file's joints run children first, but bones are numbered parents first, so weights given
/// by the file's numbers must follow their bones to the new numbers.
#[test]
fn weights_follow_their_bones_when_bones_are_renumbered() {
    let imported = import_skinned(&Character {
        reversed_joints: true,
        ..Default::default()
    })
    .unwrap();
    let scene = &imported.scene;
    let bones = &scene.skeletons[0].bones;
    assert_eq!(
        bones.iter().map(|b| b.name.as_str()).collect::<Vec<_>>(),
        ["Root", "pelvis", "spine"]
    );
    assert_eq!(bones[1].parent, Some(0));
    let mesh = &scene.meshes[0];
    let cluster = mesh.clusters[0];
    // The four vertices' skins follow them, end to end.
    let records = &mesh.vertices[cluster.vertex_offset as usize + 4..];
    let skins: Vec<solstice_asset::PackedSkin> = (0..4)
        .map(|vertex| solstice_asset::gpu::skin_at(records, vertex))
        .collect();
    // In this file joint 0 is the spine (bone 2), 1 the pelvis (bone 1) and 2 the root (bone 0).
    for (joints, weights) in Character::default().weights {
        let numbered = joints.map(|j| 2 - j as u8);
        let wanted = solstice_asset::PackedSkin::new(numbered, weights);
        assert!(skins.contains(&wanted), "{wanted:?} missing from {skins:?}");
    }
}

#[test]
fn without_being_asked_a_skin_imports_as_a_statue() {
    let folder = Folder::new();
    let path = Character::default().write(&folder);
    let imported = crate::import_gltf(
        path,
        &crate::ImportOptions {
            texture_size: 4,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(imported.scene.skeletons.is_empty());
    assert!(!imported.scene.meshes[0].skinned());
    assert!(imported
        .warnings
        .iter()
        .any(|w| w.contains("skinning is ignored")));
}

#[test]
fn a_skin_must_be_there_to_keep() {
    let none = Character {
        skins: 0,
        skinned: false,
        ..Default::default()
    };
    assert!(import_skinned(&none).is_err());
    let unweighted = Character {
        weighted: false,
        ..Default::default()
    };
    assert!(import_skinned(&unweighted).is_err());
}

#[test]
fn placing_a_skinned_model_twice_gives_two_poses() {
    let mut scene = solstice_asset::SceneAsset {
        texture_size: 4,
        ..Default::default()
    };
    let imported = import_skinned(&Character::default()).unwrap();
    let placements = crate::ModelPlacements {
        whole: vec![
            glam::Mat4::IDENTITY,
            glam::Mat4::from_translation(glam::Vec3::X),
        ],
        parts: Vec::new(),
    };
    crate::append_model(&mut scene, imported, &placements).unwrap();
    assert_eq!(scene.skeletons.len(), 1);
    assert_eq!(scene.poses, [0, 0]);
    assert_eq!(scene.instance_poses, [0, 1]);
}

const ROLES: &str = "root Root\npelvis pelvis\nspine spine\n";

fn has(report: &CharacterReport, part: &str) -> bool {
    report.problems.iter().any(|p| p.contains(part))
}

#[test]
fn a_good_character_has_no_problems() {
    let report = Character::default().check(Some(ROLES));
    assert!(report.problems.is_empty(), "{}", report.summary());
    assert_eq!(report.joints, ["Root", "pelvis", "spine"]);
    assert_eq!(
        (report.meshes, report.vertices, report.triangles),
        (1, 4, 2)
    );
    assert_eq!(report.materials, 1);
    assert!(report.summary().ends_with("no problems"));
}

#[test]
fn every_kind_of_mistake_is_found() {
    let cases: Vec<(Character, &str)> = vec![
        (
            Character {
                morph: true,
                ..Default::default()
            },
            "morph targets",
        ),
        (
            Character {
                second_weights: true,
                ..Default::default()
            },
            "more than four weights",
        ),
        (
            Character {
                skinned: false,
                ..Default::default()
            },
            "isn't bound to the character's skin",
        ),
        (
            Character {
                textures: [true, false, true],
                ..Default::default()
            },
            "has no normal texture",
        ),
        (
            Character {
                textures: [false, true, false],
                ..Default::default()
            },
            "has no base colour or metallic-roughness texture",
        ),
        (
            Character {
                alpha: "BLEND",
                ..Default::default()
            },
            "is blended",
        ),
    ];
    for (character, part) in cases {
        let report = character.check(Some(ROLES));
        assert!(has(&report, part), "{part}: {}", report.summary());
    }
    let mut odd = Character::default();
    odd.weights[1].1 = [0.5, 0.4, 0.0, 0.0];
    assert!(has(
        &odd.check(None),
        "1 vertices have weights that don't sum to one"
    ));
    let mut nothing = Character::default();
    nothing.weights[2].1 = [0.0; 4];
    assert!(has(&nothing.check(None), "1 vertices follow no bone"));
    let mut stray = Character::default();
    stray.weights[3].0 = [7, 1, 0, 0];
    assert!(has(
        &stray.check(None),
        "1 weights name a bone the skin doesn't have"
    ));
}

#[test]
fn what_skinning_needs_of_the_skeleton_is_checked() {
    let cases: Vec<(Character, &str)> = vec![
        (
            Character {
                inverse_binds: false,
                ..Default::default()
            },
            "the skin has no inverse bind matrices",
        ),
        (
            Character {
                binds_short: 1,
                ..Default::default()
            },
            "the skin has 2 inverse bind matrices for 3 bones",
        ),
        (
            Character {
                posed: true,
                ..Default::default()
            },
            "2 bones aren't where the skin was bound, so the file isn't in its rest pose: pelvis, spine",
        ),
        (
            Character {
                two_trees: true,
                ..Default::default()
            },
            "the skin's bones form 2 trees, not one",
        ),
    ];
    for (character, part) in cases {
        let report = character.check(Some(ROLES));
        assert!(has(&report, part), "{part}: {}", report.summary());
    }
    let many: Vec<&'static str> = (0..=MAX_JOINTS)
        .map(|i| &*Box::leak(format!("b{i}").into_boxed_str()))
        .collect();
    let report = Character {
        bones: many,
        ..Default::default()
    }
    .check(None);
    assert!(
        has(&report, "the skin has 257 bones, more than the 256"),
        "{}",
        report.summary()
    );
}

#[test]
fn what_skinning_needs_of_each_mesh_is_checked() {
    let cases: Vec<(Character, &str)> = vec![
        (
            Character {
                uvs: false,
                ..Default::default()
            },
            "has a part with no texture coordinates",
        ),
        (
            Character {
                mode: 1,
                ..Default::default()
            },
            "has a part drawn as Lines, not triangles",
        ),
        (
            Character {
                material: false,
                ..Default::default()
            },
            "has a part with no material",
        ),
        (
            Character {
                weighted: false,
                ..Default::default()
            },
            "has no bone weights",
        ),
        (
            Character {
                normal_tex_coord: 1,
                ..Default::default()
            },
            "reads a texture through other texture coordinates than the first",
        ),
    ];
    for (character, part) in cases {
        let report = character.check(Some(ROLES));
        assert!(has(&report, part), "{part}: {}", report.summary());
    }
    let mut nan = Character::default();
    nan.weights[0].1 = [f32::NAN, 0.0, 0.0, 0.0];
    let mut negative = Character::default();
    negative.weights[1].1 = [1.5, -0.5, 0.0, 0.0];
    for character in [nan, negative] {
        let report = character.check(None);
        assert!(
            has(
                &report,
                "1 vertices have a negative weight, or one that isn't a number"
            ),
            "{}",
            report.summary()
        );
    }
}

#[test]
fn materials_no_mesh_uses_are_not_checked() {
    let report = Character {
        unused_material: true,
        ..Default::default()
    }
    .check(Some(ROLES));
    assert!(report.problems.is_empty(), "{}", report.summary());
    assert_eq!(report.materials, 1);
}

#[test]
fn a_bone_named_with_no_weight_is_not_used() {
    // Every vertex names Root first, with no weight: Root still deforms nothing.
    let mut character = Character {
        bones: vec!["Root", "pelvis", "spine", "tip"],
        ..Default::default()
    };
    for (joints, _) in &mut character.weights {
        joints[3] = 3;
    }
    let report = character.check(Some(ROLES));
    assert!(
        has(&report, "bone `tip` deforms nothing"),
        "{}",
        report.summary()
    );
}

#[test]
fn a_file_without_indices_counts_its_triangles_by_vertex() {
    let report = Character {
        unindexed: true,
        ..Default::default()
    }
    .check(Some(ROLES));
    assert_eq!(report.triangles, 1, "{}", report.summary());
}

#[test]
fn the_summary_lists_each_problem() {
    let report = Character {
        morph: true,
        alpha: "BLEND",
        ..Default::default()
    }
    .check(Some(ROLES));
    let summary = report.summary();
    assert!(summary.contains("; 2 problems:"), "{summary}");
    assert!(
        summary.contains("\n  - mesh `body` has morph targets"),
        "{summary}"
    );
    assert!(
        summary.contains("\n  - material `skin` is blended"),
        "{summary}"
    );
}

#[test]
fn a_character_has_exactly_one_skin() {
    for skins in [0, 2] {
        let report = Character {
            skins,
            skinned: skins > 0,
            ..Default::default()
        }
        .check(None);
        let expected = format!("a character has one skin, not {skins}");
        assert!(has(&report, &expected), "{}", report.summary());
    }
}

#[test]
fn a_weight_sum_within_tolerance_is_fine() {
    let mut close = Character::default();
    // Off by a little less than the tolerance, as Blender's float weights can be.
    close.weights[1].1 = [0.5, 0.5015, 0.0, 0.0];
    let report = close.check(Some(ROLES));
    assert!(report.problems.is_empty(), "{}", report.summary());
}

#[test]
fn a_cut_out_material_is_fine() {
    let report = Character {
        alpha: "MASK",
        ..Default::default()
    }
    .check(Some(ROLES));
    assert!(report.problems.is_empty(), "{}", report.summary());
}

#[test]
fn bones_are_checked_against_the_role_file() {
    let report = Character::default().check(Some("root Root\nhead head\n"));
    assert!(has(
        &report,
        "the role file's bone `head` isn't in the skin"
    ));
    // A leaf bone that deforms nothing, and that no role names, should go. One that a role
    // names, or that other bones hang from, stays.
    let tip = Character {
        bones: vec!["Root", "pelvis", "spine", "tip"],
        ..Default::default()
    };
    assert!(has(&tip.check(Some(ROLES)), "bone `tip` deforms nothing"));
    let report = tip.check(Some("tip tip\n"));
    assert!(!has(&report, "`tip`"), "{}", report.summary());
    assert!(
        !has(&report, "`Root`"),
        "Root carries the others: {}",
        report.summary()
    );
}

#[test]
fn role_files_are_read_and_checked() {
    let roles = Roles::parse("# a rig\nspine a b  # a chain\n\nhead c\n").unwrap();
    assert_eq!(roles.bones().collect::<Vec<_>>(), ["a", "b", "c"]);
    let error = |text: &str| Roles::parse(text).unwrap_err();
    assert!(error("spine\n").contains("line 1: role `spine` names no bones"));
    assert!(error("a x\na y\n").contains("line 2: role `a` is given twice"));
    assert!(error("a x\nb x\n").contains("line 2: bone `x` already plays another role"));
    assert!(error("# nothing\n").contains("at least one role"));
}

#[test]
fn the_game_engine_rigs_role_file_reads() {
    let text = include_str!("../../../characters/rigs/game_engine.roles");
    let roles = Roles::parse(text).unwrap();
    assert_eq!(
        roles.bones().count(),
        53,
        "every bone of the rig plays a role"
    );
}
