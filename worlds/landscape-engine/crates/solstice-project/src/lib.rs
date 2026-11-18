//! The editor's project format (`docs/editor.md`): a folder holding a `solstice.project` and
//! the scenes it lists, each a `.scene` file of entities and their components, in JSON.
//!
//! ```text
//! scenes/                    a project is a folder
//!   solstice.project         its name and its scenes
//!   cove.scene               one scene: its settings and its entities
//!   cove/gate.actor          files the scene uses, by paths relative to the folder
//! ```
//!
//! This crate only reads and writes the files' text, so the editor (in wasm) and the bake
//! (natively) share it; neither file system nor GPU is touched here. A scene is baked by
//! lowering it to the commands the scene text has always had (`apps/bake/src/project.rs`).
//!
//! Files are written with fields in a fixed order, one to a line, and short lists of numbers on
//! one line, so that they read well and diff well in git.

use serde::{Deserialize, Serialize};

/// The format version written. A file with a later one is refused rather than half read.
pub const FORMAT: u32 = 1;
/// Ids an entity can't have: a baked scene's map of which entity made which instances lists
/// what the terrain and a circuit make under these.
pub const RESERVED_IDS: [&str; 2] = ["terrain", "circuit"];
/// The name of a project's own file, at the top of its folder.
pub const PROJECT_FILE: &str = "solstice.project";

/// A project: what is in `solstice.project`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Project {
    pub format: u32,
    pub name: String,
    /// Its scenes' files, relative to the project's folder.
    pub scenes: Vec<String>,
}

/// One scene: settings that there is one of, and the entities placed in it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scene {
    pub format: u32,
    /// What a person wrote about the scene.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
    /// Edge length of the scene's square textures, if not 1024.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub texture_size: Option<u32>,
    /// Where a viewer starts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub camera: Option<Camera>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terrain: Option<Terrain>,
    /// The terrain in bands of sand, dirt and grass.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub land: Option<Land>,
    /// Real grass on the grass, out to this many metres from the origin.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meadow: Option<f32>,
    /// A flat, coloured water plane.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub water: Option<Water>,
    /// An open sea.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sea: Option<Sea>,
    /// The game the scene plays, if any beyond its actors.
    #[serde(default, skip_serializing_if = "Game::is_empty")]
    pub game: Game,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entities: Vec<Entity>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Camera {
    pub position: [f32; 3],
    /// Rotation around +Y, in radians, as the scene text has it. Zero looks along -Z.
    pub yaw_radians: f32,
    /// Rotation above the horizon, in radians.
    pub pitch_radians: f32,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Terrain {
    /// Metres across.
    pub size: f32,
    /// Cells along each side.
    pub cells: u32,
    /// How tall its hills are.
    pub height: f32,
    /// Where the shore begins, along X.
    pub shore_x: f32,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Land {
    /// Metres of sand in from the water, then of dirt; grass beyond.
    pub sand: f32,
    pub dirt: f32,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Water {
    pub level: f32,
    pub color: [f32; 3],
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sea {
    pub level: f32,
    /// A built-in ocean's name, or a `.ocean` file in the project.
    pub ocean: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Game {
    /// The input actions, if not the built-in ones.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<String>,
    /// A circuit's track.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub track: Option<String>,
    /// The land, road and planting round the track, this many metres across.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub circuit: Option<f32>,
}

impl Game {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// One thing in a scene, or a folder of them. It is named in the editor, in undo and by agents
/// by its `id`, which never changes once made; `name` is for people and can be anything.
///
/// It has at most one of the components that say what it is (`mesh`, `actor`, `light`,
/// `blocker`, `wood`, `clear`, `path`, `yard`, `pond`), and none for a folder.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entity {
    pub id: String,
    pub name: String,
    /// What a person wrote about it.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transform: Option<Transform>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mesh: Option<Mesh>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<Actor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub light: Option<Light>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocker: Option<Blocker>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wood: Option<Wood>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clear: Option<Clear>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<Path>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub yard: Option<Yard>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pond: Option<Pond>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<Entity>,
}

impl Entity {
    /// A folder: an entity with only children.
    pub fn folder(id: String, name: String) -> Self {
        Self {
            id,
            name,
            ..Default::default()
        }
    }

    /// The names of the components it has that say what it is.
    pub fn kinds(&self) -> Vec<&'static str> {
        [
            ("mesh", self.mesh.is_some()),
            ("actor", self.actor.is_some()),
            ("light", self.light.is_some()),
            ("blocker", self.blocker.is_some()),
            ("wood", self.wood.is_some()),
            ("clear", self.clear.is_some()),
            ("path", self.path.is_some()),
            ("yard", self.yard.is_some()),
            ("pond", self.pond.is_some()),
        ]
        .into_iter()
        .filter_map(|(name, has)| has.then_some(name))
        .collect()
    }
}

/// Where an entity is.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Transform {
    pub position: [f32; 3],
    /// Degrees about X, Y and Z, applied as yaw (Y), then pitch (X), then roll (Z).
    #[serde(default, skip_serializing_if = "is_zero3")]
    pub rotation: [f32; 3],
    /// Uniform scale.
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub scale: f32,
    /// If set, it stands on the terrain, with this fraction of its height below the ground, and
    /// `position[1]` is ignored.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ground: Option<f32>,
}

impl Default for Transform {
    fn default() -> Self {
        Self {
            position: [0.0; 3],
            rotation: [0.0; 3],
            scale: 1.0,
            ground: None,
        }
    }
}

impl Transform {
    pub fn at(position: [f32; 3]) -> Self {
        Self {
            position,
            ..Default::default()
        }
    }
}

/// A model, or one named piece of it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mesh {
    /// A glTF file in the project.
    pub asset: String,
    /// One named piece of the model, placed by its own origin.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<String>,
    #[serde(default, skip_serializing_if = "Collide::is_auto")]
    pub collide: Collide,
    /// It keeps its skeleton, so that it can bend.
    #[serde(default, skip_serializing_if = "is_false")]
    pub skinned: bool,
}

/// How a mesh collides.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Collide {
    /// As the bake judges from its shape.
    #[default]
    Auto,
    None,
    Hull,
    Coarse,
}

impl Collide {
    pub fn is_auto(&self) -> bool {
        *self == Self::Auto
    }
}

/// An actor: an `.actor` file and its properties' values here.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Actor {
    pub asset: String,
    /// Values as written in the scene text: `true`, `2.5`, a vector as `x,y,z`. Kept in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty", with = "ordered_map")]
    pub properties: Vec<(String, String)>,
}

/// A point light, at its entity's position.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Light {
    pub range: f32,
    /// Its colour times its brightness.
    pub intensity: [f32; 3],
}

/// An invisible wall, centred on its entity's position, turned only by its yaw.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Blocker {
    /// Its full size along X, Y and Z.
    pub size: [f32; 3],
}

/// Trees over the grass, in a circle round its entity's position.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Wood {
    pub radius: f32,
    /// Metres between trees.
    pub spacing: f32,
}

/// Nothing planted within this radius of its entity's position.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Clear {
    pub radius: f32,
}

/// A dirt path, between two points on the ground (x, z).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Path {
    pub from: [f32; 2],
    pub to: [f32; 2],
    pub width: f32,
}

/// A dirt yard, between opposite corners on the ground (x, z).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Yard {
    pub from: [f32; 2],
    pub to: [f32; 2],
}

/// Still water over a rectangle on the ground (x, z), at a level.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pond {
    pub min: [f32; 2],
    pub max: [f32; 2],
    pub level: f32,
    /// A built-in ocean's name, or a `.ocean` file, whose water the scene's ponds hold.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub water: Option<String>,
}

fn one() -> f32 {
    1.0
}

fn is_one(value: &f32) -> bool {
    *value == 1.0
}

fn is_zero3(value: &[f32; 3]) -> bool {
    *value == [0.0; 3]
}

fn is_false(value: &bool) -> bool {
    !*value
}

/// A list of pairs, written as a JSON object in the list's order and read back in the file's.
mod ordered_map {
    use serde::de::{MapAccess, Visitor};
    use serde::ser::SerializeMap;
    use serde::{Deserializer, Serializer};
    use std::fmt;

    pub fn serialize<S: Serializer>(pairs: &[(String, String)], s: S) -> Result<S::Ok, S::Error> {
        let mut map = s.serialize_map(Some(pairs.len()))?;
        for (key, value) in pairs {
            map.serialize_entry(key, value)?;
        }
        map.end()
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<(String, String)>, D::Error> {
        struct Pairs;
        impl<'de> Visitor<'de> for Pairs {
            type Value = Vec<(String, String)>;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("an object of names and values, each written as text")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut pairs = Vec::new();
                while let Some(pair) = map.next_entry::<String, String>()? {
                    if pairs.iter().any(|(k, _): &(String, String)| *k == pair.0) {
                        return Err(serde::de::Error::custom(format!(
                            "property `{}` is given twice",
                            pair.0
                        )));
                    }
                    pairs.push(pair);
                }
                Ok(pairs)
            }
        }
        d.deserialize_map(Pairs)
    }
}

/// Why a file could not be read.
#[derive(Debug, Clone, PartialEq)]
pub struct Error(pub String);

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

fn check_format(format: u32) -> Result<(), Error> {
    if format == 0 || format > FORMAT {
        Err(Error(format!(
            "format {format}: this build reads project files of format 1 to {FORMAT}"
        )))
    } else {
        Ok(())
    }
}

impl Project {
    pub fn new(name: &str) -> Self {
        Self {
            format: FORMAT,
            name: name.to_owned(),
            scenes: Vec::new(),
        }
    }

    pub fn parse(text: &str) -> Result<Self, Error> {
        let project: Self = serde_json::from_str(text).map_err(|e| Error(e.to_string()))?;
        check_format(project.format)?;
        Ok(project)
    }

    pub fn to_json(&self) -> String {
        to_json(self)
    }
}

impl Scene {
    pub fn new() -> Self {
        Self {
            format: FORMAT,
            ..Default::default()
        }
    }

    /// Reads a scene, and checks that its ids are all present and different and that no entity
    /// is two kinds of thing.
    pub fn parse(text: &str) -> Result<Self, Error> {
        let scene: Self = serde_json::from_str(text).map_err(|e| Error(e.to_string()))?;
        check_format(scene.format)?;
        scene.check()?;
        Ok(scene)
    }

    pub fn to_json(&self) -> String {
        to_json(self)
    }

    /// Every entity, parents before their children, in the file's order.
    pub fn walk(&self) -> Vec<&Entity> {
        fn visit<'a>(entities: &'a [Entity], out: &mut Vec<&'a Entity>) {
            for entity in entities {
                out.push(entity);
                visit(&entity.children, out);
            }
        }
        let mut out = Vec::new();
        visit(&self.entities, &mut out);
        out
    }

    pub fn check(&self) -> Result<(), Error> {
        let mut seen = std::collections::HashSet::new();
        for entity in self.walk() {
            if entity.id.is_empty() {
                return Err(Error(format!("`{}` has no id", entity.name)));
            }
            if RESERVED_IDS.contains(&entity.id.as_str()) {
                return Err(Error(format!(
                    "id `{}` is kept for what the scene's land makes",
                    entity.id
                )));
            }
            if !seen.insert(entity.id.as_str()) {
                return Err(Error(format!("id `{}` is used twice", entity.id)));
            }
            let kinds = entity.kinds();
            if kinds.len() > 1 {
                return Err(Error(format!(
                    "`{}` ({}) is both {}: an entity is one kind of thing",
                    entity.name,
                    entity.id,
                    kinds.join(" and ")
                )));
            }
        }
        Ok(())
    }
}

/// Makes ids: short, and different for each `seed`. The editor makes its own at random; the
/// converter makes them from where each line was, so converting twice gives the same ids.
pub fn make_id(seed: u64) -> String {
    const DIGITS: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
    // splitmix64, which spreads neighbouring seeds far apart.
    let mut z = seed.wrapping_add(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^= z >> 31;
    (0..8)
        .map(|_| {
            let digit = DIGITS[(z % 62) as usize] as char;
            z /= 62;
            digit
        })
        .collect()
}

/// Writes a value as the project's files are written: two-space indents, one field to a line,
/// and lists of numbers on one line.
pub fn to_json<T: Serialize>(value: &T) -> String {
    let pretty = serde_json::to_string_pretty(value).expect("project types always serialise");
    let mut out = String::with_capacity(pretty.len());
    let mut rest = pretty.as_str();
    // A list written over several lines starts with `[` at the end of a line. Strings never
    // hold a raw line break, so this is never inside one.
    while let Some(open) = rest.find("[\n") {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let close = after.find(']').unwrap_or(after.len());
        let inner = &after[..close];
        let numbers = !inner.trim().is_empty()
            && inner
                .chars()
                .all(|c| c.is_ascii_digit() || " \n\r,.-+eE".contains(c));
        if numbers && close < after.len() {
            let items: Vec<&str> = inner.split(',').map(str::trim).collect();
            out.push('[');
            out.push_str(&items.join(", "));
            out.push(']');
            rest = &after[close + 1..];
        } else {
            out.push('[');
            rest = after;
        }
    }
    out.push_str(rest);
    out.push('\n');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Scene {
        Scene {
            texture_size: Some(2048),
            camera: Some(Camera {
                position: [66.0, 1.5, -41.0],
                yaw_radians: 2.31,
                pitch_radians: 0.03,
            }),
            sea: Some(Sea {
                level: -0.6,
                ocean: "coastal".into(),
            }),
            entities: vec![Entity {
                children: vec![
                    Entity {
                        id: make_id(1),
                        name: "Tower".into(),
                        transform: Some(Transform {
                            rotation: [0.0, 90.0, 0.0],
                            ..Transform::at([22.0, 0.0, -22.0])
                        }),
                        mesh: Some(Mesh {
                            asset: "fort.gltf".into(),
                            part: Some("tower_round".into()),
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                    Entity {
                        id: make_id(2),
                        name: "Gate".into(),
                        note: "Walk up to it and press E.".into(),
                        transform: Some(Transform::at([20.7, 0.0, 0.0])),
                        actor: Some(Actor {
                            asset: "cove/gate.actor".into(),
                            properties: vec![
                                ("locked".into(), "true".into()),
                                ("height".into(), "3.5".into()),
                            ],
                        }),
                        ..Default::default()
                    },
                ],
                ..Entity::folder(make_id(0), "Fort".into())
            }],
            ..Scene::new()
        }
    }

    #[test]
    fn a_scene_reads_back_as_it_was_written() {
        let scene = sample();
        let text = scene.to_json();
        assert_eq!(Scene::parse(&text).unwrap(), scene);
        // And writing it again gives the same text, so saving without a change changes nothing.
        assert_eq!(Scene::parse(&text).unwrap().to_json(), text);
    }

    #[test]
    fn numbers_sit_on_one_line_and_defaults_are_left_out() {
        let text = sample().to_json();
        assert!(text.contains("\"position\": [22.0, 0.0, -22.0]"), "{text}");
        assert!(text.contains("\"rotation\": [0.0, 90.0, 0.0]"), "{text}");
        // The gate isn't turned or scaled, and collides as the bake judges.
        assert_eq!(text.matches("\"rotation\"").count(), 1, "{text}");
        assert!(!text.contains("\"scale\""), "{text}");
        assert!(!text.contains("\"collide\""), "{text}");
        // A list of things is still one to a line.
        assert!(text.contains("\"children\": [\n"), "{text}");
    }

    #[test]
    fn properties_keep_their_order() {
        let text = sample().to_json();
        let locked = text.find("\"locked\"").unwrap();
        let height = text.find("\"height\"").unwrap();
        assert!(locked < height);
        let read = Scene::parse(&text).unwrap();
        let actor = read.walk()[2].actor.clone().unwrap();
        assert_eq!(actor.properties[0].0, "locked");
    }

    #[test]
    fn mistakes_are_refused_with_a_reason() {
        let mut twice = sample();
        twice.entities[0].children[1].id = twice.entities[0].children[0].id.clone();
        assert!(Scene::parse(&twice.to_json())
            .unwrap_err()
            .0
            .contains("used twice"));

        let mut both = sample();
        both.entities[0].children[0].light = Some(Light::default());
        assert!(Scene::parse(&both.to_json())
            .unwrap_err()
            .0
            .contains("mesh and light"));

        let later = sample()
            .to_json()
            .replace("\"format\": 1", "\"format\": 99");
        assert!(Scene::parse(&later).unwrap_err().0.contains("format 99"));

        let typo = sample().to_json().replace("\"position\"", "\"positon\"");
        assert!(Scene::parse(&typo).is_err());
    }

    #[test]
    fn ids_differ_and_are_short() {
        let ids: std::collections::HashSet<String> = (0..10_000).map(make_id).collect();
        assert_eq!(ids.len(), 10_000);
        assert!(ids.iter().all(|id| id.len() == 8));
    }

    #[test]
    fn a_project_lists_its_scenes() {
        let mut project = Project::new("Site");
        project.scenes.push("cove.scene".into());
        let text = project.to_json();
        assert_eq!(text, "{\n  \"format\": 1,\n  \"name\": \"Site\",\n  \"scenes\": [\n    \"cove.scene\"\n  ]\n}\n");
        assert_eq!(Project::parse(&text).unwrap(), project);
    }
}
