//! Actor files: a kind of actor, in the style of the `.ocean` and `.weather` files.
//!
//! ```text
//! # scenes/cove/gate.actor
//! name gate
//! mesh ../../target/scan/large_iron_gate/large_iron_gate.gltf 1.15   # a model, and its scale
//! script gate.star
//! property open_seconds float 2.5      # editable per placement in the scene
//! property locked bool false
//! camera 0 1.6 0                       # the player's eye, for the actor the player plays
//! collider box 3.9 3.5 0.2 at 0 1.75 0 # what it bumps into, placed relative to the actor
//! character 0.3 1.8                    # walks, as a capsule of this radius and height
//! animation keeper.anim                 # a skinned mesh's state machine, relative to this file
//! vehicle gt.vehicle                    # a car, by its numbers, relative to this file
//! ```
//!
//! `mesh PATH SCALE yaw=DEGREES` turns the model about its own origin, as a character made
//! facing +Z must be turned to face the way actors do. An actor with an `animation` has its mesh
//! imported with its skeleton. `mesh PATH SCALE part=NAME` takes one named piece of the model, by
//! its own origin. A collider
//! is `box SX SY SZ` or `capsule RADIUS HEIGHT`, then optionally `at X Y Z`, then `sensor`, which
//! blocks nothing and reports what enters and leaves it, or `dynamic MASS`, which physics moves.
//!
//! `vehicle PATH` makes the actor a car, driven by `solstice-vehicle` on the numbers in that
//! file; `vehicle PATH manual` leaves changing gear to the driver. With a `camera`, it is the
//! car the player drives. Its mesh's wheels are the pieces whose own origins are at its hubs.
//!
//! Paths are relative to the actor file. The mesh is placed by the bake, wherever the scene
//! places the actor, and moves with it. Every line but `name` may be left out.

use crate::{coordinate, is_name, lines, number, FileError};
use glam::Vec3;
use solstice_physics::{ActorCollider, BodyKind, Character, Shape};
use solstice_script::ScriptValue;
use std::fmt::Write;

/// What a property holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PropertyKind {
    Bool,
    Int,
    Float,
    /// One word of text.
    Text,
    Vec3,
}

impl PropertyKind {
    const ALL: [(Self, &'static str); 5] = [
        (Self::Bool, "bool"),
        (Self::Int, "int"),
        (Self::Float, "float"),
        (Self::Text, "text"),
        (Self::Vec3, "vec3"),
    ];

    fn from_name(name: &str) -> Option<Self> {
        Self::ALL.iter().find(|(_, n)| *n == name).map(|(k, _)| *k)
    }

    pub fn name(self) -> &'static str {
        Self::ALL
            .iter()
            .find(|(k, _)| *k == self)
            .expect("listed")
            .1
    }

    /// Reads a value of this kind from its words: one, or three for a vector.
    pub fn parse(self, words: &[&str]) -> Result<ScriptValue, String> {
        let expected = if self == Self::Vec3 { 3 } else { 1 };
        if words.len() != expected {
            return Err(format!(
                "a {} takes {expected} value{}, not {}",
                self.name(),
                if expected == 1 { "" } else { "s" },
                words.len()
            ));
        }
        let word = words[0];
        Ok(match self {
            Self::Bool => match word {
                "true" => ScriptValue::Bool(true),
                "false" => ScriptValue::Bool(false),
                _ => return Err(format!("a bool is true or false, not `{word}`")),
            },
            Self::Int => ScriptValue::Int(
                word.parse()
                    .map_err(|_| format!("`{word}` is not a whole number"))?,
            ),
            Self::Float => ScriptValue::Float(f64::from(number(word)?)),
            Self::Text => ScriptValue::from(word),
            Self::Vec3 => ScriptValue::Vec3(Vec3::new(
                number(words[0])?,
                number(words[1])?,
                number(words[2])?,
            )),
        })
    }

    /// Reads a value of this kind as a scene line writes it: one word, with a vector's three
    /// numbers joined by commas.
    pub fn parse_setting(self, word: &str) -> Result<ScriptValue, String> {
        if self == Self::Vec3 {
            self.parse(&word.split(',').collect::<Vec<_>>())
        } else {
            self.parse(&[word])
        }
    }

    /// A value's words, as [`PropertyKind::parse`] reads them.
    pub fn words(value: &ScriptValue) -> Vec<String> {
        match value {
            ScriptValue::Bool(b) => vec![b.to_string()],
            ScriptValue::Int(i) => vec![i.to_string()],
            // Shortest form that reads back as the same f32.
            ScriptValue::Float(f) => vec![(*f as f32).to_string()],
            ScriptValue::Str(s) => vec![s.to_string()],
            ScriptValue::Vec3(v) => v.to_array().iter().map(f32::to_string).collect(),
            other => vec![format!("{other:?}")],
        }
    }
}

/// A property an actor's script can read and set, with the value each placement starts from
/// unless the scene says otherwise.
#[derive(Clone, Debug, PartialEq)]
pub struct Property {
    pub name: String,
    pub kind: PropertyKind,
    pub default: ScriptValue,
}

/// A model drawn wherever the actor is, as the bake imports it.
#[derive(Clone, Debug, PartialEq)]
pub struct Mesh {
    /// A glTF file, relative to the actor file.
    pub path: String,
    pub scale: f32,
    /// One named piece of the model, placed by its own origin, rather than the whole of it.
    pub part: Option<String>,
    /// Turns the model about its own origin, in degrees.
    pub yaw_degrees: f32,
}

/// One kind of actor.
#[derive(Clone, Debug, PartialEq)]
pub struct ActorFile {
    /// Each placed actor's [`solstice_world::Name`], which scripts `find` it by.
    pub name: String,
    pub mesh: Option<Mesh>,
    /// A Starlark script, relative to the actor file.
    pub script: Option<String>,
    pub properties: Vec<Property>,
    /// Where the player's eye is, relative to the actor, if this is the actor the player plays.
    pub camera: Option<Vec3>,
    /// What it bumps into, if anything.
    pub collider: Option<ActorCollider>,
    /// Makes it walk, on the ground, with the character controller.
    pub character: Option<Character>,
    /// A state machine for a skinned mesh, relative to the actor file.
    pub animation: Option<String>,
    /// Makes it a car: its `.vehicle` file, relative to the actor file.
    pub vehicle: Option<Vehicle>,
    /// It is carried by a car's driver: with them, at their feet and turned as they are,
    /// while they are out of the car, and put away while they are in it.
    pub carried: bool,
    /// Its mesh is a kit: a bare thing and what can be fitted to it, each a named piece.
    /// This file, relative to the actor file, says which pieces are which (`kit PATH`); the
    /// bake writes beside it which instances draw each piece ([`pieces_file`]).
    pub kit: Option<String>,
}

/// The name of the gameplay file that says which of an actor's instances draw each named
/// piece of its mesh: a line a piece, `NAME FIRST COUNT`, counted from the actor's first
/// instance. The bake writes it for an actor with a `kit`.
pub fn pieces_file(actor_file: &str) -> String {
    format!("{actor_file}.pieces")
}

/// A car's numbers, and who changes gear.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Vehicle {
    /// A `.vehicle` file, relative to the actor file.
    pub path: String,
    /// Whether the gearbox changes gear by itself.
    pub automatic: bool,
}

/// Names `self` already has in a script, which a property can't take.
const RESERVED: [&str; 6] = ["entity", "position", "yaw", "walk", "vars", "instances"];

/// Reads the words of a `collider` line.
fn parse_collider(values: &[&str]) -> Result<ActorCollider, String> {
    const FORM: &str =
        "expected `collider box SX SY SZ` or `collider capsule RADIUS HEIGHT`, then optionally \
         `at X Y Z`, then `sensor` or `dynamic MASS`";
    let positive = |word: &str| {
        let value = coordinate(word)?;
        if value > 0.0 {
            Ok(value)
        } else {
            Err(format!("a collider's size must be above 0, not {value}"))
        }
    };
    let (shape, mut rest) = match values {
        ["box", x, y, z, rest @ ..] => (
            Shape::Box(Vec3::new(positive(x)?, positive(y)?, positive(z)?)),
            rest,
        ),
        ["capsule", radius, height, rest @ ..] => {
            let (radius, height) = (positive(radius)?, positive(height)?);
            if height < radius * 2.0 {
                return Err(format!(
                    "a capsule's height must be at least twice its radius, not {radius} and {height}"
                ));
            }
            (Shape::Capsule { radius, height }, rest)
        }
        _ => return Err(FORM.to_owned()),
    };
    let mut offset = Vec3::ZERO;
    if let ["at", x, y, z, after @ ..] = rest {
        offset = Vec3::new(coordinate(x)?, coordinate(y)?, coordinate(z)?);
        rest = after;
    }
    let kind = match rest {
        [] => BodyKind::Kinematic,
        ["sensor"] => BodyKind::Sensor,
        ["dynamic", mass] => BodyKind::Dynamic(positive(mass)?),
        _ => return Err(FORM.to_owned()),
    };
    Ok(ActorCollider {
        shape,
        kind,
        offset,
    })
}

impl ActorFile {
    pub fn parse(text: &str) -> Result<Self, FileError> {
        let mut name = None;
        let mut actor = ActorFile {
            name: String::new(),
            mesh: None,
            script: None,
            properties: Vec::new(),
            camera: None,
            collider: None,
            character: None,
            animation: None,
            vehicle: None,
            carried: false,
            kit: None,
        };
        for (line, words) in lines(text) {
            let error = |message: String| FileError { line, message };
            let (keyword, values) = (words[0], &words[1..]);
            let count = |range: std::ops::RangeInclusive<usize>, form: &str| {
                if range.contains(&values.len()) {
                    Ok(())
                } else {
                    Err(error(format!("expected `{keyword} {form}`")))
                }
            };
            let once = |taken: bool| {
                if taken {
                    Err(error(format!("`{keyword}` is given twice")))
                } else {
                    Ok(())
                }
            };
            match keyword {
                "name" => {
                    count(1..=1, "NAME")?;
                    once(name.is_some())?;
                    if !is_name(values[0]) {
                        return Err(error(format!(
                            "`{}` can't be a name: use letters, digits and _",
                            values[0]
                        )));
                    }
                    name = Some(values[0].to_owned());
                }
                "mesh" => {
                    count(1..=4, "PATH [SCALE] [part=NAME] [yaw=DEGREES]")?;
                    once(actor.mesh.is_some())?;
                    let (mut scale, mut part, mut yaw_degrees) = (1.0, None, 0.0);
                    for word in &values[1..] {
                        if let Some(degrees) = word.strip_prefix("yaw=") {
                            yaw_degrees = number(degrees).map_err(error)?;
                            continue;
                        }
                        match word.strip_prefix("part=") {
                            Some("") => return Err(error("`part=` needs a name".to_owned())),
                            Some(name) => part = Some(name.to_owned()),
                            None => scale = number(word).map_err(error)?,
                        }
                    }
                    if scale <= 0.0 {
                        return Err(error(format!(
                            "a mesh's scale must be above 0, not {scale}"
                        )));
                    }
                    actor.mesh = Some(Mesh {
                        path: values[0].to_owned(),
                        scale,
                        part,
                        yaw_degrees,
                    });
                }
                "animation" => {
                    count(1..=1, "PATH")?;
                    once(actor.animation.is_some())?;
                    actor.animation = Some(values[0].to_owned());
                }
                "script" => {
                    count(1..=1, "PATH")?;
                    once(actor.script.is_some())?;
                    actor.script = Some(values[0].to_owned());
                }
                "property" => {
                    if values.len() < 3 {
                        return Err(error("expected `property NAME KIND VALUE`".to_owned()));
                    }
                    let property = values[0];
                    if !is_name(property) || RESERVED.contains(&property) {
                        return Err(error(format!("`{property}` can't be a property's name")));
                    }
                    if actor.properties.iter().any(|p| p.name == property) {
                        return Err(error(format!("property `{property}` is given twice")));
                    }
                    let kind = PropertyKind::from_name(values[1]).ok_or_else(|| {
                        error(format!(
                            "`{}` is not a kind of property: use bool, int, float, text or vec3",
                            values[1]
                        ))
                    })?;
                    let default = kind
                        .parse(&values[2..])
                        .map_err(|e| error(format!("property `{property}`: {e}")))?;
                    actor.properties.push(Property {
                        name: property.to_owned(),
                        kind,
                        default,
                    });
                }
                "camera" => {
                    count(3..=3, "X Y Z")?;
                    once(actor.camera.is_some())?;
                    let [x, y, z] = [0, 1, 2].map(|i| number(values[i]));
                    actor.camera = Some(Vec3::new(
                        x.map_err(error)?,
                        y.map_err(error)?,
                        z.map_err(error)?,
                    ));
                }
                "collider" => {
                    once(actor.collider.is_some())?;
                    actor.collider = Some(parse_collider(values).map_err(error)?);
                }
                "carried" => {
                    count(0..=0, "nothing after it")?;
                    actor.carried = true;
                }
                "kit" => {
                    count(1..=1, "PATH")?;
                    once(actor.kit.is_some())?;
                    actor.kit = Some(values[0].to_owned());
                }
                "character" => {
                    count(2..=2, "RADIUS HEIGHT")?;
                    once(actor.character.is_some())?;
                    let radius = coordinate(values[0]).map_err(error)?;
                    let height = coordinate(values[1]).map_err(error)?;
                    if radius <= 0.0 || height < radius * 2.0 {
                        return Err(error(format!(
                            "a character needs a radius above 0 and a height of at least twice \
                             it, not {radius} and {height}"
                        )));
                    }
                    actor.character = Some(Character { radius, height });
                }
                "vehicle" => {
                    once(actor.vehicle.is_some())?;
                    let automatic = match values {
                        [_] => true,
                        [_, "manual"] => false,
                        _ => return Err(error("expected `vehicle PATH [manual]`".to_owned())),
                    };
                    actor.vehicle = Some(Vehicle {
                        path: values[0].to_owned(),
                        automatic,
                    });
                }
                other => {
                    return Err(error(format!(
                        "unknown line `{other}`: an actor file has name, mesh, script, property, \
                         camera, collider, character, animation and vehicle lines"
                    )))
                }
            }
        }
        actor.name = name.ok_or(FileError {
            line: 1,
            message: "an actor file needs a `name` line".to_owned(),
        })?;
        if actor.collider.is_some() && actor.character.is_some() {
            return Err(FileError {
                line: 1,
                message: "an actor is a collider or a character, not both".to_owned(),
            });
        }
        if actor.vehicle.is_some() && (actor.collider.is_some() || actor.character.is_some()) {
            return Err(FileError {
                line: 1,
                message: "a vehicle has its own body: it takes no collider or character".to_owned(),
            });
        }
        if actor.vehicle.is_some() && actor.animation.is_some() {
            return Err(FileError {
                line: 1,
                message: "a vehicle is not animated".to_owned(),
            });
        }
        Ok(actor)
    }

    /// The file's text, which [`ActorFile::parse`] reads back as it is.
    pub fn to_text(&self) -> String {
        let mut text = format!("name {}\n", self.name);
        if let Some(mesh) = &self.mesh {
            let _ = write!(text, "mesh {} {}", mesh.path, mesh.scale);
            if let Some(part) = &mesh.part {
                let _ = write!(text, " part={part}");
            }
            if mesh.yaw_degrees != 0.0 {
                let _ = write!(text, " yaw={}", mesh.yaw_degrees);
            }
            text.push('\n');
        }
        if let Some(animation) = &self.animation {
            let _ = writeln!(text, "animation {animation}");
        }
        if let Some(script) = &self.script {
            let _ = writeln!(text, "script {script}");
        }
        for property in &self.properties {
            let _ = writeln!(
                text,
                "property {} {} {}",
                property.name,
                property.kind.name(),
                PropertyKind::words(&property.default).join(" ")
            );
        }
        if let Some(eye) = self.camera {
            let _ = writeln!(text, "camera {} {} {}", eye.x, eye.y, eye.z);
        }
        if let Some(collider) = &self.collider {
            match collider.shape {
                Shape::Box(size) => {
                    let _ = write!(text, "collider box {} {} {}", size.x, size.y, size.z);
                }
                Shape::Capsule { radius, height } => {
                    let _ = write!(text, "collider capsule {radius} {height}");
                }
            }
            let at = collider.offset;
            if at != Vec3::ZERO {
                let _ = write!(text, " at {} {} {}", at.x, at.y, at.z);
            }
            match collider.kind {
                BodyKind::Kinematic => {}
                BodyKind::Sensor => text.push_str(" sensor"),
                BodyKind::Dynamic(mass) => {
                    let _ = write!(text, " dynamic {mass}");
                }
            }
            text.push('\n');
        }
        if let Some(character) = &self.character {
            let _ = writeln!(text, "character {} {}", character.radius, character.height);
        }
        if let Some(vehicle) = &self.vehicle {
            let manual = if vehicle.automatic { "" } else { " manual" };
            let _ = writeln!(text, "vehicle {}{manual}", vehicle.path);
        }
        text
    }

    pub fn property(&self, name: &str) -> Option<&Property> {
        self.properties.iter().find(|p| p.name == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GATE: &str = "\
# scenes/cove/gate.actor
name gate
mesh ../gate/gate.gltf 1.15   # the model
script gate.star
property open_seconds float 2.5
property locked bool false
property knocks int 0
property sign text closed
property hinge vec3 0 0 -1.7
camera 0 1.6 0
collider box 3.9 3.5 0.2 at 0 1.75 0
";

    #[test]
    fn an_actor_file_reads_back_what_it_wrote() {
        let gate = ActorFile::parse(GATE).unwrap();
        assert_eq!(gate.name, "gate");
        assert_eq!(
            gate.mesh,
            Some(Mesh {
                path: "../gate/gate.gltf".to_owned(),
                scale: 1.15,
                part: None,
                yaw_degrees: 0.0,
            })
        );
        assert_eq!(gate.script.as_deref(), Some("gate.star"));
        assert_eq!(gate.camera, Some(Vec3::new(0.0, 1.6, 0.0)));
        let defaults: Vec<_> = gate.properties.iter().map(|p| p.default.clone()).collect();
        assert_eq!(
            defaults,
            vec![
                ScriptValue::Float(2.5),
                ScriptValue::Bool(false),
                ScriptValue::Int(0),
                ScriptValue::from("closed"),
                ScriptValue::Vec3(Vec3::new(0.0, 0.0, -1.7)),
            ]
        );
        assert_eq!(ActorFile::parse(&gate.to_text()).unwrap(), gate);
        let bare = ActorFile::parse("name rock").unwrap();
        assert_eq!(bare.to_text(), "name rock\n");
        assert!(bare.mesh.is_none() && bare.script.is_none() && bare.camera.is_none());
    }

    #[test]
    fn mistakes_name_their_line() {
        let cases = [
            ("script a.star", 1, "needs a `name` line"),
            ("name gate\nname door", 2, "given twice"),
            ("name gate gate", 1, "expected `name NAME`"),
            ("name a-b", 1, "can't be a name"),
            ("name g\nmesh a.gltf 0", 2, "above 0"),
            ("name g\nmesh a.gltf big", 2, "`big` is not a number"),
            (
                "name g\n\nproperty locked boolean false",
                3,
                "not a kind of property",
            ),
            ("name g\nproperty locked bool yes", 2, "true or false"),
            ("name g\nproperty n int 2.5", 2, "whole number"),
            ("name g\nproperty v vec3 1 2", 2, "takes 3 values"),
            (
                "name g\nproperty position vec3 1 2 3",
                2,
                "can't be a property's name",
            ),
            (
                "name g\nproperty a int 1\nproperty a int 2",
                3,
                "given twice",
            ),
            (
                "name g\nproperty a int",
                2,
                "expected `property NAME KIND VALUE`",
            ),
            ("name g\ncamera 0 1", 2, "expected `camera X Y Z`"),
            ("name g\nsword 1 2 3", 2, "unknown line `sword`"),
            ("name g\ncollider cone 1 2", 2, "expected `collider box"),
            ("name g\ncollider box 1 2", 2, "expected `collider box"),
            ("name g\ncollider box 1 2 0", 2, "must be above 0"),
            (
                "name g\ncollider box 1 2 3 sensor loud",
                2,
                "expected `collider box",
            ),
            ("name g\ncollider box 1 2 3 dynamic 0", 2, "must be above 0"),
            (
                "name g\ncollider capsule 1 1",
                2,
                "at least twice its radius",
            ),
            (
                "name g\ncollider box 1 2 3\ncollider box 1 2 3",
                3,
                "given twice",
            ),
            (
                "name g\ncharacter 0.3",
                2,
                "expected `character RADIUS HEIGHT`",
            ),
            ("name g\ncharacter 0 2", 2, "at least twice"),
            ("name g\ncharacter 1 1", 2, "at least twice"),
            ("name g\nmesh a.gltf part=", 2, "`part=` needs a name"),
            (
                "name g\nproperty walk vec3 0 0 0",
                2,
                "can't be a property's name",
            ),
            (
                "name g\ncollider box 1 2 3\ncharacter 0.3 1.8",
                1,
                "not both",
            ),
        ];
        for (text, line, part) in cases {
            let error = ActorFile::parse(text).unwrap_err();
            assert_eq!(error.line, line, "{text:?}: {error}");
            assert!(error.message.contains(part), "{text:?}: {error}");
        }
    }

    #[test]
    fn collider_and_character_numbers_too_big_for_physics_are_mistakes() {
        for line in [
            "collider box 1e30 1 1",
            "collider box 1 1e30 1",
            "collider box 1 1 1e30",
            "collider capsule 1e30 1e31",
            "collider capsule 1 1e30",
            "collider box 1 1 1 at 1e30 0 0",
            "collider box 1 1 1 at 0 1e30 0",
            "collider box 1 1 1 at 0 0 1e30",
            "collider box 1 1 1 dynamic 1e30",
            "character 1e30 1e31",
            "character 1 1e30",
        ] {
            assert!(
                ActorFile::parse(&format!("name a\n{line}\n")).is_err(),
                "{line}"
            );
        }
        for line in [
            "collider box 1e6 1 1 at 1e6 0 0 dynamic 1e6",
            "character 1 1e6",
        ] {
            assert!(
                ActorFile::parse(&format!("name a\n{line}\n")).is_ok(),
                "{line}"
            );
        }
    }

    #[test]
    fn an_animation_and_a_turned_mesh_read_back_what_they_wrote() {
        let text = "name w\nmesh a.glb 1 yaw=180\nanimation w.anim\n";
        let walker = ActorFile::parse(text).unwrap();
        assert_eq!(walker.animation.as_deref(), Some("w.anim"));
        assert_eq!(walker.mesh.as_ref().unwrap().yaw_degrees, 180.0);
        assert_eq!(ActorFile::parse(&walker.to_text()).unwrap(), walker);
        let error = ActorFile::parse("name w\nanimation a\nanimation b\n").unwrap_err();
        assert!(error.message.contains("twice"), "{error}");
    }

    #[test]
    fn colliders_characters_and_parts_read_back_what_they_wrote() {
        let cases = [
            (
                "name a\ncollider box 1 2 3\n",
                Shape::Box(Vec3::new(1.0, 2.0, 3.0)),
                BodyKind::Kinematic,
                Vec3::ZERO,
            ),
            (
                "name a\ncollider box 1 2 3 sensor\n",
                Shape::Box(Vec3::new(1.0, 2.0, 3.0)),
                BodyKind::Sensor,
                Vec3::ZERO,
            ),
            (
                "name a\ncollider capsule 0.5 2 dynamic 80\n",
                Shape::Capsule {
                    radius: 0.5,
                    height: 2.0,
                },
                BodyKind::Dynamic(80.0),
                Vec3::ZERO,
            ),
            (
                "name a\ncollider box 1 2 3 at 0 1.5 -2\n",
                Shape::Box(Vec3::new(1.0, 2.0, 3.0)),
                BodyKind::Kinematic,
                Vec3::new(0.0, 1.5, -2.0),
            ),
            (
                "name a\ncollider box 1 2 3 at 1 0 0 dynamic 5\n",
                Shape::Box(Vec3::new(1.0, 2.0, 3.0)),
                BodyKind::Dynamic(5.0),
                Vec3::X,
            ),
        ];
        for (text, shape, kind, offset) in cases {
            let actor = ActorFile::parse(text).unwrap();
            assert_eq!(
                actor.collider,
                Some(ActorCollider {
                    shape,
                    kind,
                    offset
                }),
                "{text}"
            );
            assert_eq!(actor.to_text(), text);
        }
        let hero = ActorFile::parse("name h\ncharacter 0.3 1.8\n").unwrap();
        assert_eq!(
            hero.character,
            Some(Character {
                radius: 0.3,
                height: 1.8
            })
        );
        assert_eq!(hero.to_text(), "name h\ncharacter 0.3 1.8\n");
        for text in [
            "mesh a.gltf 1.5 part=door_02",
            "mesh a.gltf part=door_02 1.5",
        ] {
            let door = ActorFile::parse(&format!("name d\n{text}\n")).unwrap();
            let mesh = door.mesh.as_ref().unwrap();
            assert_eq!((mesh.scale, mesh.part.as_deref()), (1.5, Some("door_02")));
            assert_eq!(door.to_text(), "name d\nmesh a.gltf 1.5 part=door_02\n");
        }
    }

    #[test]
    fn settings_read_as_their_kind() {
        assert_eq!(
            PropertyKind::Vec3.parse_setting("1,2,3"),
            Ok(ScriptValue::Vec3(Vec3::new(1.0, 2.0, 3.0)))
        );
        assert_eq!(
            PropertyKind::Bool.parse_setting("true"),
            Ok(ScriptValue::Bool(true))
        );
        assert!(PropertyKind::Float.parse_setting("x").is_err());
        assert!(PropertyKind::Vec3.parse_setting("1,2").is_err());
    }
}
