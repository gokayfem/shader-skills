//! A racing circuit: its centre line, and what lies either side of it.
//!
//! ```text
//! # Written by tools/blender/make_circuit.py, which builds the circuit's model from the same
//! # line. Each `p` is a point of the centre line, in order round the lap.
//! name Ashcombe Park
//! sector 512                  # the point each further sector starts at; the first starts at 0
//! # p X Y Z   LX LY LZ   LEFT RIGHT   [l=SURFACE:WIDTH,...] [r=...]
//! p 0 12.5 0  -1 0 0  6.5 6.5  l=kerb:1.4,gravel:18 r=concrete:9
//! ```
//!
//! * `X Y Z` is the point, and `LX LY LZ` the way across the track to the driver's left there,
//!   along the surface: it tips with the banking.
//! * `LEFT` and `RIGHT` are how far the asphalt reaches to each side.
//! * `l=` and `r=` list what lies beyond it on each side, from the track's edge outwards, each
//!   with its width. Beyond the last, and where none is given, is grass.
//!
//! The lap starts at the first point, and the line closes back to it. From this file come the
//! surface under each tyre ([`Track::surface`]), where a car is round the lap for timing it
//! ([`Track::locate`]), and where to put a car back on the track ([`Track::place`]).

use crate::{coordinate, lines, number, FileError};
use glam::Vec3;
use solstice_vehicle::Surface;
use std::fmt::Write;

/// A strip of ground beside the track: what it is, and how wide.
pub type Band = (Surface, f32);

/// A point of the centre line.
#[derive(Clone, Debug, PartialEq)]
pub struct Sample {
    pub position: Vec3,
    /// Across the track to the left, along its surface: a unit vector.
    pub left: Vec3,
    /// How far the asphalt reaches to the left, and to the right.
    pub half_left: f32,
    pub half_right: f32,
    /// What lies beyond the asphalt on each side, from its edge outwards.
    pub left_bands: Vec<Band>,
    pub right_bands: Vec<Band>,
}

/// Where a point is, relative to the track.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Place {
    /// The sample the nearest stretch of the centre line starts at.
    pub sample: usize,
    /// How far round the lap, in metres from the start line.
    pub along: f32,
    /// How far to the left of the centre line, in metres: below zero, to the right.
    pub offset: f32,
    /// How far above the track's surface there.
    pub above: f32,
    pub surface: Surface,
}

/// A circuit.
#[derive(Clone, Debug, PartialEq)]
pub struct Track {
    pub name: String,
    pub samples: Vec<Sample>,
    /// The sample each sector starts at, rising. The first sector starts at sample 0.
    pub sectors: Vec<usize>,
    /// How far round the lap each sample is, and the lap's whole length.
    distances: Vec<f32>,
    length: f32,
}

/// How many samples either side of a hint are searched for the nearest: enough for a car at any
/// speed between two steps, and for the wheels of one car.
const NEARBY: usize = 16;
/// Every how many samples a search of the whole lap looks first, before searching near the
/// nearest of those: at most `NEARBY`, so the search near it reaches every sample between.
const COARSE: usize = 8;

impl Track {
    /// A track through these samples, closing back to the first.
    pub fn new(name: &str, samples: Vec<Sample>, sectors: Vec<usize>) -> Result<Self, String> {
        if samples.len() < 3 {
            return Err("a track needs at least three points".to_owned());
        }
        let mut distances = Vec::with_capacity(samples.len());
        let mut length = 0.0;
        for (index, sample) in samples.iter().enumerate() {
            distances.push(length);
            let next = &samples[(index + 1) % samples.len()];
            let step = sample.position.distance(next.position);
            if step < 1.0e-3 {
                return Err(format!(
                    "points {} and {} are the same",
                    index + 1,
                    index + 2
                ));
            }
            length += step;
        }
        if sectors.windows(2).any(|pair| pair[1] <= pair[0])
            || sectors
                .iter()
                .any(|&sector| sector == 0 || sector >= samples.len())
        {
            return Err("sectors must rise, and start at points after the first".to_owned());
        }
        Ok(Self {
            name: name.to_owned(),
            samples,
            sectors,
            distances,
            length,
        })
    }

    /// The lap's length along the centre line, in metres.
    pub fn length(&self) -> f32 {
        self.length
    }

    /// Where line `line` is, in metres round the lap: 0 is the start line, then each sector's.
    pub fn sector_start(&self, line: usize) -> f32 {
        match line {
            0 => 0.0,
            _ => self.distances[self.sectors[line - 1]],
        }
    }

    /// How far round the lap each sector starts, the first at 0.
    pub fn sector_starts(&self) -> Vec<f32> {
        std::iter::once(0.0)
            .chain(self.sectors.iter().map(|&sample| self.distances[sample]))
            .collect()
    }

    pub fn parse(text: &str) -> Result<Self, FileError> {
        let mut name = String::new();
        let mut samples = Vec::new();
        let mut sectors = Vec::new();
        let mut named = None;
        for (line, words) in lines(text) {
            let error = |message: String| FileError { line, message };
            match words[0] {
                "name" if words.len() >= 2 => {
                    if let Some(first) = named {
                        return Err(error(format!(
                            "a second name: the first is on line {first}"
                        )));
                    }
                    named = Some(line);
                    name = words[1..].join(" ");
                }
                "name" => return Err(error("expected `name NAME`".to_owned())),
                "sector" => {
                    let [_, index] = words[..] else {
                        return Err(error("expected `sector POINT`".to_owned()));
                    };
                    sectors.push(index.parse::<usize>().map_err(|_| {
                        error(format!("`{index}` is not a point's number, from 0"))
                    })?);
                }
                "p" => {
                    if words.len() < 9 {
                        return Err(error(
                            "expected `p X Y Z LX LY LZ LEFT RIGHT [l=...] [r=...]`".to_owned(),
                        ));
                    }
                    let read = |index: usize| coordinate(words[index]).map_err(&error);
                    let position = Vec3::new(read(1)?, read(2)?, read(3)?);
                    let left = Vec3::new(read(4)?, read(5)?, read(6)?);
                    if (left.length() - 1.0).abs() > 1.0e-3 {
                        return Err(error(
                            "the way to the left must be a unit vector".to_owned(),
                        ));
                    }
                    let (half_left, half_right) = (read(7)?, read(8)?);
                    if half_left <= 0.0 || half_right <= 0.0 {
                        return Err(error("the track's width must be above 0".to_owned()));
                    }
                    let mut sample = Sample {
                        position,
                        left,
                        half_left,
                        half_right,
                        left_bands: Vec::new(),
                        right_bands: Vec::new(),
                    };
                    for word in &words[9..] {
                        let (side, bands) = match word.split_once('=') {
                            Some(("l", bands)) => (&mut sample.left_bands, bands),
                            Some(("r", bands)) => (&mut sample.right_bands, bands),
                            _ => return Err(error(format!("`{word}` should be l=... or r=..."))),
                        };
                        if !side.is_empty() {
                            return Err(error(format!(
                                "`{word}`: a side's bands are given once, as l=A,B"
                            )));
                        }
                        for band in bands.split(',') {
                            side.push(parse_band(band).map_err(&error)?);
                        }
                    }
                    samples.push(sample);
                }
                other => {
                    return Err(error(format!(
                        "unknown line `{other}`: a track file has name, sector and p lines"
                    )))
                }
            }
        }
        Self::new(&name, samples, sectors).map_err(|message| FileError { line: 1, message })
    }

    pub fn to_text(&self) -> String {
        let mut text = String::new();
        if !self.name.is_empty() {
            let _ = writeln!(text, "name {}", self.name);
        }
        for sector in &self.sectors {
            let _ = writeln!(text, "sector {sector}");
        }
        for sample in &self.samples {
            let (p, l) = (sample.position, sample.left);
            let _ = write!(
                text,
                "p {} {} {} {} {} {} {} {}",
                p.x, p.y, p.z, l.x, l.y, l.z, sample.half_left, sample.half_right
            );
            for (key, bands) in [("l", &sample.left_bands), ("r", &sample.right_bands)] {
                if bands.is_empty() {
                    continue;
                }
                let listed: Vec<String> = bands
                    .iter()
                    .map(|(surface, width)| format!("{}:{width}", surface.name()))
                    .collect();
                let _ = write!(text, " {key}={}", listed.join(","));
            }
            text.push('\n');
        }
        text
    }

    /// Where a point is. `hint` is a sample it is known to be near, such as where it was last
    /// step, which saves searching the whole lap.
    pub fn locate(&self, point: Vec3, hint: Option<usize>) -> Place {
        let count = self.samples.len();
        let nearest = |candidates: &mut dyn Iterator<Item = usize>| {
            let mut best = (f32::INFINITY, 0, 0.0);
            for index in candidates {
                let (from, to) = (
                    self.samples[index].position,
                    self.samples[(index + 1) % count].position,
                );
                let along = to - from;
                let share = ((point - from).dot(along) / along.length_squared()).clamp(0.0, 1.0);
                let distance = point.distance_squared(from + along * share);
                if distance < best.0 {
                    best = (distance, index, share);
                }
            }
            best
        };
        let around = |hint: usize| {
            let hint = hint % count;
            nearest(&mut (0..=2 * NEARBY).map(|k| (hint + count + k - NEARBY % count) % count))
        };
        let mut best = match hint {
            Some(hint) => around(hint),
            None => nearest(&mut (0..count)),
        };
        // Where two stretches run side by side, as at a hairpin, a hint can hold on to the wrong
        // one: a point off the road the hint found may be on another, so the lap is searched
        // again. A wheel on a kerb is off the road, at most corners, so that search looks at
        // every eighth sample first and then near the nearest of them.
        if hint.is_some() {
            let found = &self.samples[best.1];
            let road = found.half_left.max(found.half_right);
            if best.0 > road * road {
                let from = |index: usize| point.distance_squared(self.samples[index].position);
                let coarse = (0..count)
                    .step_by(COARSE)
                    .min_by(|&a, &b| from(a).total_cmp(&from(b)))
                    .unwrap_or(0);
                let searched = around(coarse);
                if searched.0 < best.0 {
                    best = searched;
                }
            }
        }
        let (_, index, share) = best;
        let (here, next) = (&self.samples[index], &self.samples[(index + 1) % count]);
        let centre = here.position.lerp(next.position, share);
        let left = here.left.lerp(next.left, share).normalize_or(here.left);
        let forward = (next.position - here.position).normalize();
        let up = forward.cross(left).normalize_or(Vec3::Y);
        let from_centre = point - centre;
        let offset = from_centre.dot(left);
        let lerp = |a: f32, b: f32| a + (b - a) * share;
        let (half, bands) = if offset >= 0.0 {
            (lerp(here.half_left, next.half_left), &here.left_bands)
        } else {
            (lerp(here.half_right, next.half_right), &here.right_bands)
        };
        let mut beyond = offset.abs() - half;
        let mut surface = Surface::Asphalt;
        if beyond > 0.0 {
            surface = Surface::Grass;
            for &(band, width) in bands {
                if beyond <= width {
                    surface = band;
                    break;
                }
                beyond -= width;
            }
        }
        let segment = here.position.distance(next.position);
        Place {
            sample: index,
            along: self.distances[index] + segment * share,
            offset,
            above: from_centre.dot(up),
            surface,
        }
    }

    /// Where the centre line is this far round the lap, in metres from the start line, and the
    /// way it runs there, and the way to its left.
    pub fn place(&self, along: f32) -> (Vec3, Vec3, Vec3) {
        let along = along.rem_euclid(self.length);
        let index = self
            .distances
            .partition_point(|&distance| distance <= along)
            .saturating_sub(1);
        let (here, next) = (
            &self.samples[index],
            &self.samples[(index + 1) % self.samples.len()],
        );
        let segment = here.position.distance(next.position);
        let share = ((along - self.distances[index]) / segment).clamp(0.0, 1.0);
        (
            here.position.lerp(next.position, share),
            (next.position - here.position) / segment,
            here.left.lerp(next.left, share).normalize_or(here.left),
        )
    }
}

fn parse_band(word: &str) -> Result<Band, String> {
    let (surface, width) = word
        .split_once(':')
        .ok_or_else(|| format!("`{word}` should be SURFACE:WIDTH"))?;
    let surface = Surface::from_name(surface).ok_or_else(|| {
        format!("`{surface}` is not a surface: use asphalt, kerb, grass, gravel or concrete")
    })?;
    let width = number(width)?;
    if width <= 0.0 {
        return Err(format!("a strip's width must be above 0, not {width}"));
    }
    Ok((surface, width))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A flat oval, driven clockwise seen from above: two straights along Z, 200 m long and
    /// 100 m apart, joined by half circles. The start is half-way up the left-hand straight,
    /// heading -Z. Kerb then gravel on the left, grass on the right.
    pub(crate) fn oval() -> Track {
        let mut centre: Vec<(Vec3, Vec3)> = Vec::new();
        let step = 2.0;
        let mut z = 0.0;
        while z > -100.0 {
            centre.push((Vec3::new(0.0, 0.0, z), Vec3::NEG_Z));
            z -= step;
        }
        let turns = (std::f32::consts::PI * 50.0 / step) as usize;
        for k in 0..turns {
            let angle = std::f32::consts::PI * k as f32 / turns as f32;
            centre.push((
                Vec3::new(50.0 - 50.0 * angle.cos(), 0.0, -100.0 - 50.0 * angle.sin()),
                Vec3::new(angle.sin(), 0.0, -angle.cos()),
            ));
        }
        let mut z = -100.0;
        while z < 100.0 {
            centre.push((Vec3::new(100.0, 0.0, z), Vec3::Z));
            z += step;
        }
        for k in 0..turns {
            let angle = std::f32::consts::PI * k as f32 / turns as f32;
            centre.push((
                Vec3::new(50.0 + 50.0 * angle.cos(), 0.0, 100.0 + 50.0 * angle.sin()),
                Vec3::new(-angle.sin(), 0.0, angle.cos()),
            ));
        }
        let mut z = 100.0;
        while z > 0.5 {
            centre.push((Vec3::new(0.0, 0.0, z), Vec3::NEG_Z));
            z -= step;
        }
        let samples: Vec<Sample> = centre
            .into_iter()
            .map(|(position, forward)| Sample {
                position,
                left: Vec3::Y.cross(forward),
                half_left: 6.0,
                half_right: 6.0,
                left_bands: vec![(Surface::Kerb, 1.5), (Surface::Gravel, 10.0)],
                right_bands: Vec::new(),
            })
            .collect();
        let third = samples.len() / 3;
        Track::new("Oval", samples, vec![third, 2 * third]).unwrap()
    }

    #[test]
    fn a_track_knows_its_length_and_its_sectors() {
        let track = oval();
        let expected = 400.0 + std::f32::consts::TAU * 50.0;
        assert!(
            (track.length() - expected).abs() < 1.0,
            "{}",
            track.length()
        );
        let starts = track.sector_starts();
        assert_eq!(starts.len(), 3);
        assert!(starts[0] == 0.0 && starts[1] > 200.0 && starts[2] > starts[1]);
    }

    #[test]
    fn a_point_is_found_on_the_lap_and_across_the_track() {
        let track = oval();
        // On the first straight, heading -Z, the left is -X.
        let place = track.locate(Vec3::new(-2.0, 0.3, -31.0), None);
        assert!((place.along - 31.0).abs() < 0.01 && (place.offset - 2.0).abs() < 0.01);
        assert!((place.above - 0.3).abs() < 0.01);
        assert_eq!(place.surface, Surface::Asphalt);
        // Across to the left: kerb, gravel, then grass. To the right: grass at once.
        let at = |x: f32| track.locate(Vec3::new(x, 0.0, -31.0), None).surface;
        assert_eq!(at(-5.9), Surface::Asphalt);
        assert_eq!(at(-6.5), Surface::Kerb);
        assert_eq!(at(-8.0), Surface::Gravel);
        assert_eq!(at(-17.0), Surface::Gravel);
        assert_eq!(at(-18.0), Surface::Grass);
        assert_eq!(at(5.9), Surface::Asphalt);
        assert_eq!(at(6.1), Surface::Grass);
        // On the far straight, heading +Z, the left is +X.
        let place = track.locate(Vec3::new(103.0, 0.0, 10.0), None);
        assert!((place.offset - 3.0).abs() < 0.01);
        assert!(place.along > 300.0);
    }

    #[test]
    fn a_hint_finds_the_same_place_and_recovers_when_wrong() {
        let track = oval();
        let mut hint = None;
        let mut along = 0.0;
        while along < track.length() {
            let (centre, _, left) = track.place(along);
            let point = centre + left * 1.5;
            let hinted = track.locate(point, hint);
            let searched = track.locate(point, None);
            assert_eq!(hinted, searched, "at {along}");
            assert!((hinted.along - along).abs() < 0.05 && (hinted.offset - 1.5).abs() < 0.05);
            hint = Some(hinted.sample);
            along += 7.3;
        }
        // A hint from the other side of the lap.
        let place = track.locate(Vec3::new(0.0, 0.0, -31.0), Some(track.samples.len() / 2));
        assert!((place.along - 31.0).abs() < 0.01);
    }

    /// Two straights 30 m apart, joined by a hairpin: a car that slides from one to the other
    /// is on the other's road, not on the grass beside the first.
    #[test]
    fn a_hint_lets_go_of_a_stretch_the_car_has_left() {
        let mut centre: Vec<(Vec3, Vec3)> = Vec::new();
        for k in 0..100 {
            centre.push((Vec3::new(0.0, 0.0, -2.0 * k as f32), Vec3::NEG_Z));
        }
        for k in 0..24 {
            let angle = std::f32::consts::PI * k as f32 / 24.0;
            centre.push((
                Vec3::new(15.0 - 15.0 * angle.cos(), 0.0, -200.0 - 15.0 * angle.sin()),
                Vec3::new(angle.sin(), 0.0, -angle.cos()),
            ));
        }
        for k in 0..100 {
            centre.push((Vec3::new(30.0, 0.0, -200.0 + 2.0 * k as f32), Vec3::Z));
        }
        for k in 0..24 {
            let angle = std::f32::consts::PI * k as f32 / 24.0;
            centre.push((
                Vec3::new(15.0 + 15.0 * angle.cos(), 0.0, 15.0 * angle.sin()),
                Vec3::new(-angle.sin(), 0.0, angle.cos()),
            ));
        }
        let samples = centre
            .into_iter()
            .map(|(position, forward)| Sample {
                position,
                left: Vec3::Y.cross(forward),
                half_left: 6.0,
                half_right: 6.0,
                left_bands: vec![(Surface::Grass, 40.0)],
                right_bands: vec![(Surface::Grass, 40.0)],
            })
            .collect();
        let track = Track::new("Hairpin", samples, vec![60, 180]).unwrap();
        let first = track.locate(Vec3::new(0.0, 0.0, -100.0), None);
        let other = track.locate(Vec3::new(29.0, 0.0, -100.0), Some(first.sample));
        assert_eq!(other.surface, Surface::Asphalt, "{other:?}");
        assert!((other.offset.abs() - 1.0).abs() < 0.05, "{other:?}");
        assert_eq!(other, track.locate(Vec3::new(29.0, 0.0, -100.0), None));
    }

    #[test]
    fn a_place_wraps_round_the_lap() {
        let track = oval();
        let (start, forward, left) = track.place(0.0);
        assert_eq!(start, Vec3::ZERO);
        assert!(
            (forward - Vec3::NEG_Z).length() < 1.0e-5 && (left - Vec3::NEG_X).length() < 1.0e-5
        );
        let (again, _, _) = track.place(track.length() * 3.0);
        assert!(again.distance(start) < 0.01);
        let (behind, _, _) = track.place(-10.0);
        assert!(behind.distance(Vec3::new(0.0, 0.0, 10.0)) < 0.01);
    }

    #[test]
    fn a_file_reads_back_what_it_wrote() {
        let track = oval();
        let text = track.to_text();
        let back = Track::parse(&text).unwrap();
        assert_eq!(back.name, "Oval");
        assert_eq!(back.sectors, track.sectors);
        assert_eq!(back.samples.len(), track.samples.len());
        assert_eq!(back.to_text(), text);
        assert!((back.length() - track.length()).abs() < 0.01);
    }

    #[test]
    fn mistakes_are_reported_with_their_line() {
        let cases = [
            ("p 0 0 0 1 0 0 6", 1, "expected `p X Y Z"),
            ("name A\np 0 0 0 2 0 0 6 6", 2, "unit vector"),
            ("p 0 0 0 1 0 0 6 0", 1, "width must be above 0"),
            ("p 0 0 0 1 0 0 6 6 l=lava:3", 1, "not a surface"),
            ("p 0 0 0 1 0 0 6 6 x=kerb:3", 1, "l=... or r=..."),
            ("p 0 0 0 1 0 0 6 6 l=kerb", 1, "SURFACE:WIDTH"),
            ("lap 3", 1, "unknown line `lap`"),
            (
                "name A
name B",
                2,
                "a second name: the first is on line 1",
            ),
            (
                "p 0 0 0 1 0 0 6 6 l=kerb:1 l=grass:3",
                1,
                "a side's bands are given once",
            ),
            (
                "p 0 0 0 1 0 0 6 6\np 1 0 0 1 0 0 6 6",
                1,
                "at least three points",
            ),
            (
                "sector 9\np 0 0 0 1 0 0 6 6\np 0 0 5 1 0 0 6 6\np 5 0 5 1 0 0 6 6",
                1,
                "sectors must rise",
            ),
        ];
        for (text, line, part) in cases {
            let error = Track::parse(text).unwrap_err();
            assert_eq!(error.line, line, "{text}: {error}");
            assert!(error.message.contains(part), "{text}: {error}");
        }
    }
}
