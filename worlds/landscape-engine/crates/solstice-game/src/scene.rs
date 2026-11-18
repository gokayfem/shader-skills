//! Actors placed in a scene.
//!
//! A scene's text file places an actor as it places models, at x, y, z and a yaw in degrees,
//! with settings for its properties: `actor gate.actor 20.7 0 0 90 locked=true`.
//!
//! The bake carries the scene's gameplay into the scene file as [`solstice_asset::GameFile`]s:
//! each actor file and script, the input actions, and [`GAME_FILE`], which lists the actors in
//! the same form, with the renderer instances the bake placed for each actor's mesh.

use crate::{coordinate, is_name, lines, number, resolve, ActorFile, FileError};
use glam::Vec3;
use solstice_asset::GameFile;
use std::fmt::Write;
use std::ops::Range;

/// The name of the gameplay file that lists a baked scene's actors and its input actions.
pub const GAME_FILE: &str = "scene.game";

/// One actor placed in a scene.
#[derive(Clone, Debug, PartialEq)]
pub struct ActorLine {
    /// The actor file, relative to the scene's text file.
    pub file: String,
    pub position: Vec3,
    pub yaw_degrees: f32,
    /// Property values for this placement, as written: `name=value`, a vector as `x,y,z`.
    pub settings: Vec<(String, String)>,
    /// The renderer instances drawing its mesh, once baked.
    pub instances: Option<Range<u32>>,
    /// Its model is not in this scene's file but in the one every scene of a game shares,
    /// which gives it its instances when the two are read together: `shared`, at the end.
    pub shared: bool,
}

impl ActorLine {
    /// Reads the words of an `actor` line, after `actor` itself.
    pub fn parse(words: &[&str]) -> Result<Self, String> {
        if words.len() < 5 {
            return Err("expected `actor FILE X Y Z YAW [NAME=VALUE...]`".to_owned());
        }
        let mut actor = ActorLine {
            file: words[0].to_owned(),
            position: Vec3::new(
                coordinate(words[1])?,
                coordinate(words[2])?,
                coordinate(words[3])?,
            ),
            yaw_degrees: number(words[4])?,
            settings: Vec::new(),
            instances: None,
            shared: false,
        };
        for word in &words[5..] {
            if *word == "shared" {
                actor.shared = true;
                continue;
            }
            let (name, value) = word
                .split_once('=')
                .ok_or_else(|| format!("`{word}` should be NAME=VALUE"))?;
            if name == "instances" {
                let range = value
                    .split_once('+')
                    .and_then(|(first, count)| {
                        Some((first.parse::<u32>().ok()?, count.parse::<u32>().ok()?))
                    })
                    .and_then(|(first, count)| Some(first..first.checked_add(count)?))
                    .ok_or_else(|| format!("`{word}` should be instances=FIRST+COUNT"))?;
                actor.instances = Some(range);
                continue;
            }
            if !is_name(name) || value.is_empty() {
                return Err(format!("`{word}` should be NAME=VALUE"));
            }
            if actor.settings.iter().any(|(n, _)| n == name) {
                return Err(format!("`{name}` is set twice"));
            }
            actor.settings.push((name.to_owned(), value.to_owned()));
        }
        Ok(actor)
    }

    /// The line, `actor` and all, which [`ActorLine::parse`] reads back as it is.
    pub fn to_text(&self) -> String {
        let p = self.position;
        let mut text = format!(
            "actor {} {} {} {} {}",
            self.file, p.x, p.y, p.z, self.yaw_degrees
        );
        for (name, value) in &self.settings {
            let _ = write!(text, " {name}={value}");
        }
        if let Some(instances) = &self.instances {
            let _ = write!(text, " instances={}+{}", instances.start, instances.len());
        }
        if self.shared {
            text.push_str(" shared");
        }
        text
    }
}

/// An invisible wall: `blocker X Y Z SX SY SZ YAW`, a box by its centre, its full size in metres,
/// and its yaw in degrees.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Blocker {
    pub centre: Vec3,
    pub size: Vec3,
    pub yaw_degrees: f32,
}

impl Blocker {
    /// Reads the words of a `blocker` line, after `blocker` itself.
    pub fn parse(words: &[&str]) -> Result<Self, String> {
        let [x, y, z, sx, sy, sz, yaw] = words else {
            return Err("expected `blocker X Y Z SX SY SZ YAW`".to_owned());
        };
        let size = Vec3::new(coordinate(sx)?, coordinate(sy)?, coordinate(sz)?);
        if size.min_element() <= 0.0 {
            return Err(format!("a blocker's size must be above 0, not {size}"));
        }
        Ok(Self {
            centre: Vec3::new(coordinate(x)?, coordinate(y)?, coordinate(z)?),
            size,
            yaw_degrees: number(yaw)?,
        })
    }

    /// The line, `blocker` and all.
    pub fn to_text(&self) -> String {
        let (c, s) = (self.centre, self.size);
        format!(
            "blocker {} {} {} {} {} {} {}",
            c.x, c.y, c.z, s.x, s.y, s.z, self.yaw_degrees
        )
    }
}

/// A baked scene's [`GAME_FILE`]: its actors, its invisible walls, and its input actions.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SceneGame {
    pub actors: Vec<ActorLine>,
    pub blockers: Vec<Blocker>,
    /// The `.input` file, relative to the scene's text file, or the built-in actions if none.
    pub input: Option<String>,
    /// The `.track` file, relative to the scene's text file, if the scene is a circuit.
    pub track: Option<String>,
    /// The map of what the ground is made of (`ground.rs`), if the scene has one: it says
    /// what each tyre is on, in place of the track.
    pub surfaces: Option<String>,
}

impl SceneGame {
    pub fn parse(text: &str) -> Result<Self, FileError> {
        let mut game = SceneGame::default();
        for (line, words) in lines(text) {
            let error = |message: String| FileError { line, message };
            match words[0] {
                "blocker" => game
                    .blockers
                    .push(Blocker::parse(&words[1..]).map_err(error)?),
                "actor" => game
                    .actors
                    .push(ActorLine::parse(&words[1..]).map_err(error)?),
                "input" if words.len() == 2 => {
                    if game.input.is_some() {
                        return Err(error("`input` is given twice".to_owned()));
                    }
                    game.input = Some(words[1].to_owned());
                }
                "input" => return Err(error("expected `input FILE`".to_owned())),
                "track" if words.len() == 2 => {
                    if game.track.is_some() {
                        return Err(error("`track` is given twice".to_owned()));
                    }
                    game.track = Some(words[1].to_owned());
                }
                "track" => return Err(error("expected `track FILE`".to_owned())),
                "surfaces" if words.len() == 2 => {
                    if game.surfaces.is_some() {
                        return Err(error("`surfaces` is given twice".to_owned()));
                    }
                    game.surfaces = Some(words[1].to_owned());
                }
                "surfaces" => return Err(error("expected `surfaces FILE`".to_owned())),
                other => return Err(error(format!("unknown line `{other}`"))),
            }
        }
        Ok(game)
    }

    pub fn to_text(&self) -> String {
        let mut text = String::new();
        if let Some(input) = &self.input {
            let _ = writeln!(text, "input {input}");
        }
        if let Some(track) = &self.track {
            let _ = writeln!(text, "track {track}");
        }
        if let Some(surfaces) = &self.surfaces {
            let _ = writeln!(text, "surfaces {surfaces}");
        }
        for actor in &self.actors {
            text.push_str(&actor.to_text());
            text.push('\n');
        }
        for blocker in &self.blockers {
            text.push_str(&blocker.to_text());
            text.push('\n');
        }
        text
    }
}

/// The files a scene's gameplay names that `files` lacks, each once, in the order named: for
/// a viewer that is handed a game a file at a time, to ask for the rest. [`GAME_FILE`] is the
/// first, then the input actions, track and actor files it names, then each actor's script,
/// animation and vehicle. A file that can't be read names nothing: loading the game says what
/// is wrong with it.
pub fn missing_files(files: &[GameFile]) -> Vec<String> {
    let file = |name: &str| files.iter().find(|f| f.name == name).map(|f| &*f.text);
    let mut named = Vec::new();
    match file(GAME_FILE).map(SceneGame::parse) {
        None => named.push(GAME_FILE.to_owned()),
        Some(Err(_)) => {}
        Some(Ok(scene)) => {
            named.extend(scene.input);
            named.extend(scene.track);
            named.extend(scene.surfaces);
            for line in scene.actors {
                let actor = file(&line.file).map(ActorFile::parse);
                named.push(line.file.clone());
                if let Some(Ok(actor)) = actor {
                    let vehicle = actor.vehicle.as_ref().map(|vehicle| &vehicle.path);
                    let relative = actor
                        .script
                        .iter()
                        .chain(&actor.animation)
                        .chain(vehicle)
                        .chain(&actor.kit);
                    named.extend(relative.map(|name| resolve(&line.file, name)));
                }
            }
        }
    }
    let mut missing: Vec<String> = Vec::new();
    for name in named {
        if file(&name).is_none() && !missing.contains(&name) {
            missing.push(name);
        }
    }
    missing
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(line: &str) -> Vec<&str> {
        line.split_whitespace().collect()
    }

    #[test]
    fn an_actor_line_reads_back_what_it_wrote() {
        let line =
            ActorLine::parse(&words("gate.actor 20.7 0 -3.25 90 locked=true hinge=0,1,2")).unwrap();
        assert_eq!(line.file, "gate.actor");
        assert_eq!(line.position, Vec3::new(20.7, 0.0, -3.25));
        assert_eq!(line.yaw_degrees, 90.0);
        assert_eq!(
            line.settings,
            vec![
                ("locked".to_owned(), "true".to_owned()),
                ("hinge".to_owned(), "0,1,2".to_owned())
            ]
        );
        assert_eq!(line.instances, None);
        let baked = ActorLine {
            instances: Some(120..124),
            ..line.clone()
        };
        let text = baked.to_text();
        assert_eq!(
            text,
            "actor gate.actor 20.7 0 -3.25 90 locked=true hinge=0,1,2 instances=120+4"
        );
        assert_eq!(ActorLine::parse(&words(&text)[1..]).unwrap(), baked);
    }

    #[test]
    fn mistakes_in_actor_lines_are_explained() {
        let cases = [
            ("gate.actor 1 2 3", "expected `actor FILE X Y Z YAW"),
            ("gate.actor 1 2 x 0", "`x` is not a number"),
            ("gate.actor 1 2 3 0 locked", "should be NAME=VALUE"),
            ("gate.actor 1 2 3 0 locked=", "should be NAME=VALUE"),
            ("gate.actor 1 2 3 0 a=1 a=2", "set twice"),
            ("gate.actor 1 2 3 0 instances=4", "instances=FIRST+COUNT"),
            (
                "gate.actor 1 2 3 0 instances=4294967295+2",
                "instances=FIRST+COUNT",
            ),
        ];
        for (line, part) in cases {
            let error = ActorLine::parse(&words(line)).unwrap_err();
            assert!(error.contains(part), "{line:?}: {error}");
        }
    }

    #[test]
    fn numbers_too_big_for_physics_are_mistakes() {
        for words in [
            ["1e30", "0", "0", "1", "1", "1", "0"],
            ["0", "1e30", "0", "1", "1", "1", "0"],
            ["0", "0", "1e30", "1", "1", "1", "0"],
            ["0", "0", "0", "1e30", "1", "1", "0"],
            ["0", "0", "0", "1", "1e30", "1", "0"],
            ["0", "0", "0", "1", "1", "1e30", "0"],
        ] {
            assert!(Blocker::parse(&words).is_err(), "{words:?}");
        }
        assert!(Blocker::parse(&["1e6", "0", "0", "1e6", "1", "1", "0"]).is_ok());
        for words in [
            ["a.actor", "1e30", "0", "0", "0"],
            ["a.actor", "0", "1e30", "0", "0"],
            ["a.actor", "0", "0", "1e30", "0"],
        ] {
            assert!(ActorLine::parse(&words).is_err(), "{words:?}");
        }
    }

    #[test]
    fn blockers_read_back_what_they_wrote() {
        let text = "actor a 0 0 0 0\nblocker 1 2 3 4 5 6.5 45\n";
        let game = SceneGame::parse(text).unwrap();
        assert_eq!(
            game.blockers,
            vec![Blocker {
                centre: Vec3::new(1.0, 2.0, 3.0),
                size: Vec3::new(4.0, 5.0, 6.5),
                yaw_degrees: 45.0,
            }]
        );
        assert_eq!(game.to_text(), text);
        for (line, part) in [
            ("blocker 1 2 3", "expected `blocker X Y Z"),
            ("blocker 1 2 3 4 5 x 0", "`x` is not a number"),
            ("blocker 1 2 3 4 0 6 0", "must be above 0"),
            ("blocker 1 2 3 4 5 6 0 7", "expected `blocker X Y Z"),
        ] {
            let error = SceneGame::parse(line).unwrap_err();
            assert_eq!(error.line, 1);
            assert!(error.message.contains(part), "{line}: {error}");
        }
    }

    #[test]
    fn the_files_a_game_names_and_lacks_are_listed_as_they_come_to_light() {
        let file = |name: &str, text: &str| GameFile {
            name: name.to_owned(),
            text: text.to_owned(),
        };
        let mut files = Vec::new();
        assert_eq!(missing_files(&files), [GAME_FILE]);
        files.push(file(
            GAME_FILE,
            "input keys.input
actor doors/door.actor 0 0 0 0
actor doors/door.actor 1 0 0 0
actor rock.actor 0 0 0 0
",
        ));
        assert_eq!(
            missing_files(&files),
            ["keys.input", "doors/door.actor", "rock.actor"]
        );
        files.push(file(
            "doors/door.actor",
            "name door
script door.star
animation ../door.anim
",
        ));
        files.push(file(
            "rock.actor",
            "name rock
",
        ));
        files.push(file(
            "cars/car.actor",
            "name car
vehicle car.vehicle
",
        ));
        files[0].text.push_str(
            "track ring.track
actor cars/car.actor 0 0 0 0
",
        );
        files.push(file(
            "keys.input",
            "interact key KeyF
",
        ));
        assert_eq!(
            missing_files(&files),
            [
                "ring.track",
                "doors/door.star",
                "door.anim",
                "cars/car.vehicle"
            ]
        );
        files.push(file("doors/door.star", ""));
        files.push(file("door.anim", ""));
        files.push(file("ring.track", ""));
        files.push(file("cars/car.vehicle", ""));
        assert!(missing_files(&files).is_empty());
        // A file with a mistake names nothing: loading the game reports it.
        files[0].text = "actor".to_owned();
        assert!(missing_files(&files).is_empty());
    }

    #[test]
    fn a_scene_game_reads_back_what_it_wrote() {
        let text = "input cove/play.input\nactor cove/gate.actor 20.7 0 0 90 instances=3+2\nactor cove/player.actor 0 0 0 0\n";
        let game = SceneGame::parse(text).unwrap();
        assert_eq!(game.input.as_deref(), Some("cove/play.input"));
        assert_eq!(game.actors.len(), 2);
        assert_eq!(game.to_text(), text);
        let error = SceneGame::parse("actor a 0 0 0 0\nmodel x").unwrap_err();
        assert_eq!(error.line, 2);
        assert_eq!(SceneGame::parse("input a\ninput b").unwrap_err().line, 2);
        assert_eq!(SceneGame::parse("actor a 0 0").unwrap_err().line, 1);
    }
}
