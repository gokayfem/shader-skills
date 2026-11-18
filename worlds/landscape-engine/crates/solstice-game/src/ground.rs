//! What the ground is made of, place by place: a map for land that is not a circuit.
//!
//! A circuit's track file says what lies either side of its road. Open country has no such
//! order: a bog lies where water gathers, rock where a ridge breaks through, and a track's
//! clay wherever the track has wandered. So the bake writes a map, a grid of squares each
//! holding one [`Surface`], and each tyre reads the square it is on.
//!
//! ```text
//! # surfaces X0 Z0 SPACING COLUMNS ROWS, then a line a row: runs of one surface, each a
//! # count and a letter (`a` is the first of Surface::ALL).
//! surfaces -500 -500 1 1000 1000
//! 412i3f170i2g413i
//! ```

use crate::FileError;
use glam::Vec3;
use solstice_vehicle::Surface;
use std::fmt::Write;

/// A grid of squares of ground, each of one surface.
#[derive(Clone, Debug, PartialEq)]
pub struct SurfaceMap {
    /// World x and z of the first square's corner.
    pub origin: [f32; 2],
    /// The width of a square, in metres.
    pub spacing: f32,
    pub columns: u32,
    pub rows: u32,
    /// Each square's surface, as its place in [`Surface::ALL`]: `rows` rows of `columns`.
    pub squares: Vec<u8>,
}

impl SurfaceMap {
    /// The surface at a place. Off the map, the nearest square's.
    pub fn at(&self, point: Vec3) -> Surface {
        if self.columns == 0 || self.rows == 0 {
            return Surface::default();
        }
        let column = (((point.x - self.origin[0]) / self.spacing).floor().max(0.0) as u32)
            .min(self.columns - 1);
        let row = (((point.z - self.origin[1]) / self.spacing).floor().max(0.0) as u32)
            .min(self.rows - 1);
        Surface::from_index(self.squares[(row * self.columns + column) as usize])
            .unwrap_or_default()
    }

    pub fn parse(text: &str) -> Result<Self, FileError> {
        let mut map: Option<Self> = None;
        for (index, line) in text.lines().enumerate() {
            let line = line.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            let error = |message: String| FileError {
                line: index + 1,
                message,
            };
            let Some(map) = &mut map else {
                let words: Vec<&str> = line.split_whitespace().collect();
                let ["surfaces", x, z, spacing, columns, rows] = words[..] else {
                    return Err(error(
                        "expected `surfaces X0 Z0 SPACING COLUMNS ROWS` first".to_owned(),
                    ));
                };
                let number = |word: &str| {
                    word.parse::<f32>()
                        .ok()
                        .filter(|n| n.is_finite())
                        .ok_or_else(|| error(format!("`{word}` is not a number")))
                };
                let count = |word: &str| {
                    word.parse::<u32>()
                        .ok()
                        .filter(|&n| (1..=8192).contains(&n))
                        .ok_or_else(|| error(format!("`{word}` should be from 1 to 8192")))
                };
                let spacing = number(spacing)?;
                if spacing <= 0.0 {
                    return Err(error("the spacing must be above 0".to_owned()));
                }
                let (columns, rows) = (count(columns)?, count(rows)?);
                map = Some(Self {
                    origin: [number(x)?, number(z)?],
                    spacing,
                    columns,
                    rows,
                    squares: Vec::with_capacity((columns * rows) as usize),
                });
                continue;
            };
            // A row: runs, each a count and a letter.
            if map.squares.len() >= (map.columns * map.rows) as usize {
                return Err(error(format!("more than {} rows", map.rows)));
            }
            let mut count = 0_u32;
            let mut filled = 0_u32;
            let mut digits = false;
            for letter in line.chars() {
                if let Some(digit) = letter.to_digit(10) {
                    count = count
                        .checked_mul(10)
                        .and_then(|c| c.checked_add(digit))
                        .ok_or_else(|| error("a run is too long".to_owned()))?;
                    digits = true;
                    continue;
                }
                let surface = (letter as u32)
                    .checked_sub('a' as u32)
                    .filter(|&s| s < Surface::ALL.len() as u32)
                    .ok_or_else(|| error(format!("`{letter}` is no surface")))?;
                let run = if digits { count } else { 1 };
                filled = filled.saturating_add(run);
                if filled > map.columns {
                    return Err(error(format!("more than {} squares", map.columns)));
                }
                map.squares
                    .extend(std::iter::repeat_n(surface as u8, run as usize));
                (count, digits) = (0, false);
            }
            if digits || filled != map.columns {
                return Err(error(format!(
                    "{filled} squares, where a row has {}",
                    map.columns
                )));
            }
        }
        let map = map.ok_or(FileError {
            line: 1,
            message: "no `surfaces` line".to_owned(),
        })?;
        if map.squares.len() != (map.columns * map.rows) as usize {
            return Err(FileError {
                line: text.lines().count(),
                message: format!(
                    "{} rows, where the map has {}",
                    map.squares.len() / map.columns as usize,
                    map.rows
                ),
            });
        }
        Ok(map)
    }

    /// The file's text, which [`SurfaceMap::parse`] reads back as it is.
    pub fn to_text(&self) -> String {
        let mut text = format!(
            "surfaces {} {} {} {} {}\n",
            self.origin[0], self.origin[1], self.spacing, self.columns, self.rows
        );
        for row in self.squares.chunks(self.columns as usize) {
            let mut k = 0;
            while k < row.len() {
                let run = row[k..].iter().take_while(|&&s| s == row[k]).count();
                let letter = (b'a' + row[k]) as char;
                if run > 1 {
                    let _ = write!(text, "{run}");
                }
                text.push(letter);
                k += run;
            }
            text.push('\n');
        }
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map() -> SurfaceMap {
        let mut squares = vec![Surface::Litter.index(); 12];
        squares[5] = Surface::Bog.index();
        squares[6] = Surface::Mud.index();
        squares[7] = Surface::Mud.index();
        SurfaceMap {
            origin: [-2.0, -1.5],
            spacing: 1.0,
            columns: 4,
            rows: 3,
            squares,
        }
    }

    #[test]
    fn a_map_reads_back_what_it_wrote() {
        let map = map();
        let text = map.to_text();
        assert_eq!(text, "surfaces -2 -1.5 1 4 3\n4i\nih2g\n4i\n");
        assert_eq!(SurfaceMap::parse(&text).unwrap(), map);
    }

    #[test]
    fn each_place_reads_its_square_and_the_nearest_beyond_the_edge() {
        let map = map();
        let at = |x: f32, z: f32| map.at(Vec3::new(x, 9.0, z));
        assert_eq!(at(-0.5, 0.0), Surface::Bog);
        assert_eq!(at(0.5, 0.0), Surface::Mud);
        assert_eq!(at(1.99, -0.49), Surface::Mud);
        assert_eq!(at(-1.5, 0.0), Surface::Litter);
        assert_eq!(at(0.5, 0.6), Surface::Litter);
        // Off the map: the nearest.
        assert_eq!(at(50.0, 0.0), Surface::Mud);
        assert_eq!(at(-50.0, -50.0), Surface::Litter);
    }

    #[test]
    fn mistakes_are_reported_with_their_line() {
        for (text, line, part) in [
            ("4i", 1, "expected `surfaces"),
            ("surfaces 0 0 1 2 1\n3i", 2, "more than 2"),
            ("surfaces 0 0 1 2 1\ni", 2, "1 squares"),
            ("surfaces 0 0 1 2 1\n2z", 2, "no surface"),
            ("surfaces 0 0 1 2 2\n2a", 2, "1 rows"),
            ("surfaces 0 0 1 2 1\n2a\n2a", 3, "more than 1 rows"),
            // A run that would wrap the count round to fit.
            ("surfaces 0 0 1 2 1\n1a4294967295a", 2, "more than 2"),
            ("surfaces 0 0 0 2 1\n2a", 1, "above 0"),
            ("# nothing", 1, "no `surfaces`"),
        ] {
            let error = SurfaceMap::parse(text).unwrap_err();
            assert_eq!(error.line, line, "{text}: {error}");
            assert!(error.message.contains(part), "{text}: {error}");
        }
    }
}
