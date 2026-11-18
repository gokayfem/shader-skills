//! Gameplay's glue (step 4 of docs/gameplay.md): the text files a game is made of, and play
//! mode, which runs a scene's game and says what to draw.
//!
//! * [`actor`]: `.actor` files, an actor's components, settings and script.
//! * [`scene`]: `actor` lines, which place actors in a scene, and the list of them a baked
//!   scene carries.
//! * [`input`]: `.input` files, which map keys, the mouse, gamepads and touch to actions.
//! * [`play`]: a running game, built from a baked scene's gameplay files.
//! * [`track`]: `.track` files, a circuit's centre line and what lies beside it; [`race`], which
//!   times laps round it; and [`drive`], the player's car, its camera and its instruments.
//! * [`ground`]: a map of what open country is made of, for land that is not a circuit.
//! * [`par`]: how long a track takes a truck of some kind, worked out from the track.
//!
//! None of it touches the GPU. The viewers move the renderer's instances, and its camera, as
//! [`play::Play::frame`] tells them.

use solstice_asset::MAX_COORDINATE;

pub mod actor;
pub mod animation;
pub mod drive;
pub mod ground;
pub mod impact;
pub mod input;
pub mod par;
pub mod play;
pub mod race;
pub mod scene;
pub mod track;

pub use actor::{ActorFile, Mesh, Property, PropertyKind};
pub use drive::Hud;
pub use ground::SurfaceMap;
pub use impact::Impact;
pub use input::{Binding, Device, Devices, InputMap};
pub use play::{Message, Play, View};
pub use scene::{missing_files, ActorLine, Blocker, SceneGame, GAME_FILE};
pub use solstice_vehicle::{rut, Surface, VehicleSpec};
pub use solstice_world::Entity;
pub use track::Track;

use std::fmt;

/// A mistake in one of a game's text files: which line, and what is wrong with it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileError {
    /// Counting from 1.
    pub line: usize,
    pub message: String,
}

impl fmt::Display for FileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

impl std::error::Error for FileError {}

/// Each line's words, without comments or blank lines, numbered from 1.
pub(crate) fn lines(text: &str) -> impl Iterator<Item = (usize, Vec<&str>)> {
    text.lines().enumerate().filter_map(|(index, line)| {
        let words: Vec<&str> = line
            .split('#')
            .next()
            .unwrap_or("")
            .split_whitespace()
            .collect();
        (!words.is_empty()).then_some((index + 1, words))
    })
}

/// Reads a number, or says which word wasn't one.
pub(crate) fn number(word: &str) -> Result<f32, String> {
    word.parse::<f32>()
        .ok()
        .filter(|n| n.is_finite())
        .ok_or_else(|| format!("`{word}` is not a number"))
}

/// A number for a position or a size: one physics can work with, as a big enough one panics it.
pub(crate) fn coordinate(word: &str) -> Result<f32, String> {
    let value = number(word)?;
    if value.abs() <= MAX_COORDINATE {
        Ok(value)
    } else {
        Err(format!("`{word}` is too big: the most is {MAX_COORDINATE}"))
    }
}

/// Whether a name can be used for an actor, a property or an action: letters, digits and `_`,
/// not starting with a digit.
pub(crate) fn is_name(word: &str) -> bool {
    word.chars().next().is_some_and(|c| !c.is_ascii_digit())
        && word.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// The name of a file at `relative` from the file named `from`, as gameplay files name each
/// other: relative to the scene's text file, with `/` between folders, and `..` worked out.
///
/// `resolve("cove/gate.actor", "gate.star")` is `cove/gate.star`.
pub fn resolve(from: &str, relative: &str) -> String {
    let (from, relative) = (from.replace('\\', "/"), relative.replace('\\', "/"));
    let mut parts: Vec<&str> = from.split('/').collect();
    parts.pop();
    for part in relative.split('/') {
        match part {
            "" | "." => {}
            ".." if parts.last().is_some_and(|last| *last != "..") => {
                parts.pop();
            }
            _ => parts.push(part),
        }
    }
    parts.join("/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_resolve_from_the_file_that_uses_them() {
        assert_eq!(resolve("cove/gate.actor", "gate.star"), "cove/gate.star");
        assert_eq!(
            resolve("gate.actor", "scripts/gate.star"),
            "scripts/gate.star"
        );
        assert_eq!(resolve("a/b/c.actor", "../d.star"), "a/d.star");
        assert_eq!(resolve("a/c.actor", "../../d.star"), "../d.star");
        assert_eq!(resolve("a\\c.actor", ".\\d.star"), "a/d.star");
        assert_eq!(resolve("c.actor", "x\\y.star"), "x/y.star");
    }

    #[test]
    fn comments_and_blank_lines_are_skipped() {
        let found: Vec<_> = lines("# a note\n\nname gate  # trailing\n  \nscript x").collect();
        assert_eq!(
            found,
            vec![(3, vec!["name", "gate"]), (5, vec!["script", "x"])]
        );
    }

    #[test]
    fn numbers_and_names_are_checked() {
        assert_eq!(number("2.5"), Ok(2.5));
        assert!(number("inf").is_err() && number("x").is_err() && number("NaN").is_err());
        assert_eq!(coordinate("-1000000"), Ok(-1.0e6));
        assert!(coordinate("1000001").is_err() && coordinate("-1e30").is_err());
        assert!(is_name("open_seconds") && is_name("_a1"));
        assert!(!is_name("") && !is_name("1a") && !is_name("a-b") && !is_name("a.b"));
    }
}
