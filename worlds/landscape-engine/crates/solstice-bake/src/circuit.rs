//! A racing circuit's scene, built from its track: the island it runs round, the road, and
//! everything beside it.
//!
//! The track file (`solstice_game::track`) says where the centre line runs, how wide the road
//! is, and what lies either side. From that alone this builds:
//!
//! * **The land**: an island whose broad height follows the road's, with cliffs to the east and
//!   beaches round the rest, a wooded hill in the middle, and stacks of rock standing in the
//!   sea. It is drawn level with the road wherever the road passes, so that the road lies in
//!   the land rather than on it. Grass over most of it, rock where it is steep, sand at the
//!   water.
//! * **The road**: asphalt, with a white line down each edge, the start line, and grid marks;
//!   kerbs in red and white; gravel traps and concrete where the track file has them.
//! * **Barriers** a way back from each edge, which are also what a car is stopped by.
//! * **Planting**: grass along the verges, woods beyond the barriers, boulders on the shore.
//!
//! The ground a car drives on is the same land, as a grid of heights. The game reads what is
//! under each tyre from the same track file, so what is drawn and what is driven on agree.

mod furniture;
mod ground;
mod line;
mod paving;
mod verge;

use crate::meshes;
use glam::{Mat4, Vec3};
use rayon::prelude::*;
use rustc_hash::FxHashMap;
use solstice_asset::{Heightfield, Instance, Material, SceneAsset};
use solstice_builder::{build_virtual_mesh, BuildOptions, Mesh};
use solstice_game::{Blocker, Track};
use solstice_procgen::foliage::grass::Meadow;
use solstice_procgen::foliage::Rng;
use solstice_procgen::noise::fbm;
use solstice_procgen::surface::TILE_METRES;

/// How far beyond the road and what lies beside it the land is kept level with it: room for
/// the verge and the barrier.
const SHOULDER: f32 = 11.0;
/// And how far beyond that it takes to become the land as it would have been.
const BLEND: f32 = 60.0;
/// How far out from the road's edge, and whatever lies beside it, the barrier stands.
const BARRIER: f32 = 9.0;
/// How far the land under the road is sunk below it, so that the two never show through each
/// other.
const SINK: f32 = 0.08;
/// How far the sea bed lies below the sea.
const SEA_BED: f32 = 7.0;
/// How far back from the water a cliff's top stands.
const CLIFF: f32 = 10.0;
/// Metres between the heights of the ground a car drives on.
const GROUND_SPACING: f32 = 1.0;
/// Metres between the points of the land as it is drawn.
const LAND_SPACING: f32 = 1.5;
/// The kinds of ground the land is made of, each a mesh with its own material.
const GRASS: usize = 0;
const ROCK: usize = 1;
const SAND: usize = 2;

/// What a circuit gives the scene besides what is drawn.
pub struct Built {
    /// The land, as a grid of heights to drive on.
    pub ground: Heightfield,
    /// The barriers, as walls a car is stopped by.
    pub blockers: Vec<Blocker>,
}

fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The shore at a point of the island.
struct Shore {
    /// How much of its height the land has here: none at the waterline, all of it inland.
    rise: f32,
    /// How far below the sea the bed is, where there is no land: zero or less.
    bed: f32,
    /// How much of a cliff's face this is, to be cut into ledges.
    face: f32,
}

/// The island the circuit is on, before the road is laid across it.
struct Island {
    /// Its middle, and how far the land reaches from there each way: east and west, north and
    /// south.
    centre: [f32; 2],
    reach_x: [f32; 2],
    reach_z: [f32; 2],
    /// Points of the road, thinly: the land's broad height follows the road's, so that the
    /// road is nowhere in a deep cutting or on a high bank.
    anchors: Vec<Vec3>,
    /// How far out from the middle the hill there reaches.
    hill: f32,
    /// Stacks of rock standing in the sea off the cliffs: where, how wide, how high.
    stacks: Vec<[f32; 4]>,
}

impl Island {
    /// An island to fit a track: its cliffs are close by the road on the east, and there is
    /// room for woods and beaches round the rest.
    fn new(track: &Track) -> Self {
        let mut low = Vec3::splat(f32::INFINITY);
        let mut high = Vec3::splat(f32::NEG_INFINITY);
        for sample in &track.samples {
            low = low.min(sample.position);
            high = high.max(sample.position);
        }
        let centre = [(low.x + high.x) * 0.5, (low.z + high.z) * 0.5];
        let (half_x, half_z) = ((high.x - low.x) * 0.5, (high.z - low.z) * 0.5);
        let nearest = track
            .samples
            .iter()
            .map(|s| (s.position.x - centre[0]).hypot(s.position.z - centre[1]))
            .fold(f32::INFINITY, f32::min);
        let east = centre[0] + half_x;
        Self {
            centre,
            reach_x: [half_x + 62.0, half_x + 250.0],
            reach_z: [half_z + 200.0, half_z + 300.0],
            anchors: track
                .samples
                .iter()
                .step_by(24)
                .map(|s| s.position)
                .collect(),
            hill: nearest * 0.78,
            stacks: vec![
                [east + 150.0, centre[1] - 170.0, 20.0, 15.0],
                [east + 215.0, centre[1] + 90.0, 30.0, 21.0],
                [east + 125.0, centre[1] + 240.0, 14.0, 10.0],
                [east + 260.0, centre[1] - 30.0, 11.0, 7.0],
            ],
        }
    }

    /// How far inland a point is, in metres, more or less: zero at the waterline, below it out
    /// to sea. The coast wanders.
    fn inland(&self, x: f32, z: f32) -> f32 {
        let (dx, dz) = (x - self.centre[0], z - self.centre[1]);
        let reach_x = self.reach_x[if dx > 0.0 { 0 } else { 1 }];
        let reach_z = self.reach_z[if dz > 0.0 { 1 } else { 0 }];
        let length = dx.hypot(dz);
        let around = if length > 1.0e-3 {
            Vec3::new(dx, 0.0, dz) / length
        } else {
            Vec3::X
        };
        // How far the waterline is from the middle, that way.
        let waterline = 1.0 / ((around.x / reach_x).powi(2) + (around.z / reach_z).powi(2)).sqrt();
        let wander = fbm(around * 2.2 + Vec3::new(0.0, 3.0, 0.0), 3, 311) * 0.035
            + fbm(around * 9.0 + Vec3::new(0.0, 11.0, 0.0), 3, 313) * 0.012;
        waterline * (1.0 + wander) - length
    }

    /// How far east a point is: none in the west, all of it where the cliffs are.
    fn east(&self, x: f32) -> f32 {
        smoothstep(0.15, 0.6, (x - self.centre[0]) / self.reach_x[0])
    }

    /// The land's height at a point were there no sea: its broad shape, hills, and bumps.
    fn top(&self, x: f32, z: f32) -> f32 {
        let p = Vec3::new(x, 0.0, z);
        // The road's height nearby, spread wide.
        let (mut total, mut height) = (0.0, 0.0);
        for anchor in &self.anchors {
            let d = (anchor.x - x).powi(2) + (anchor.z - z).powi(2) + 110.0 * 110.0;
            let weight = 1.0 / (d * d);
            total += weight;
            height += weight * anchor.y;
        }
        let from_middle = (x - self.centre[0]).hypot(z - self.centre[1]);
        let hill = smoothstep(self.hill, self.hill * 0.15, from_middle)
            * 17.0
            * (0.8 + fbm(p * 0.006, 3, 305) * 0.35);
        height / total
            + hill
            + fbm(p * 0.004, 4, 301) * 3.5
            + fbm(p * 0.02, 3, 302) * 1.2
            + fbm(p * 0.25, 3, 303) * 0.05
    }

    fn shore(&self, x: f32, z: f32) -> Shore {
        let p = Vec3::new(x, 0.0, z);
        let inland = self.inland(x, z);
        let east = self.east(x);
        // Round most of the island the land shelves down to a beach.
        let beach = smoothstep(0.0, 120.0, inland);
        // In the east it stands to its full height within a few metres of the sea. The cliff's
        // line is broken into buttresses and gullies, large and small.
        let broken = inland
            + fbm(p * 0.03, 3, 331) * 9.0
            + fbm(p * 0.13, 3, 332) * 2.6
            + fbm(p * 0.55, 2, 334) * 0.5;
        // Its face is in beds of rock, each standing back a little from the one below: a
        // riser, and a ledge. The beds dip along the coast.
        const BEDS: f32 = 6.0;
        let across = broken / CLIFF;
        let dip = fbm(p * 0.015, 2, 333) * 0.9 + x * 0.004;
        let bed = across.max(0.0) * BEDS + dip;
        let stepped = (bed.floor() + smoothstep(0.5, 1.0, bed.fract()) - dip) / BEDS;
        let face = if across <= 0.0 {
            0.0
        } else if across >= 1.0 {
            1.0
        } else {
            (smoothstep(0.0, 1.0, across) * 0.2 + stepped * 0.8).clamp(0.0, 1.0)
        };
        // Behind the cliff's edge the land climbs away from the sea, so that from further
        // back the sea is seen over the edge.
        let cliff = face * (0.6 + 0.4 * smoothstep(CLIFF, 62.0, broken));
        Shore {
            rise: beach + (cliff - beach) * east,
            bed: -SEA_BED * smoothstep(0.0, -70.0 + 40.0 * east, inland),
            face: east * smoothstep(-1.5, 1.0, broken) * smoothstep(CLIFF + 10.0, CLIFF, broken),
        }
    }

    /// A stack of rock standing in the sea at a point, if one does: how high, and how far
    /// in from its edge, from none to all.
    fn stack(&self, x: f32, z: f32) -> Option<(f32, f32)> {
        let p = Vec3::new(x, 0.0, z);
        let mut found: Option<(f32, f32)> = None;
        for &[sx, sz, wide, high] in &self.stacks {
            let away = (x - sx).hypot(z - sz) + fbm(p * 0.09, 3, 340) * wide * 0.35;
            if away < wide * 1.6 {
                let within = smoothstep(wide * 1.5, wide * 0.3, away);
                let height = (high + SEA_BED) * within - SEA_BED + fbm(p * 0.3, 2, 341) * 0.6;
                if found.is_none_or(|(other, _)| height > other) {
                    found = Some((height, smoothstep(wide * 1.6, wide * 1.2, away)));
                }
            }
        }
        found
    }

    /// How much of bare rock a point is: a cliff's face, or a stack.
    fn rocky(&self, x: f32, z: f32) -> f32 {
        let stack = self.stack(x, z).map_or(0.0, |(_, within)| within);
        self.shore(x, z).face.max(stack)
    }

    /// A height of the land from its top and its shore; `held` is how firmly the road holds
    /// the land there, from not at all to wholly, which the sea does not take back.
    fn finish(&self, x: f32, z: f32, top: f32, held: f32) -> f32 {
        let p = Vec3::new(x, 0.0, z);
        let shore = self.shore(x, z);
        let held = smoothstep(0.6, 1.0, held);
        let rise = shore.rise.max(held);
        let mut height = top * rise + shore.bed * (1.0 - held);
        let mut rocky = shore.face * (1.0 - held);
        let mut stacked = 0.0;
        // The stacks stand in the sea.
        if let Some((stack, within)) = self.stack(x, z) {
            if stack > height {
                height = stack;
                rocky = rocky.max(within);
                stacked = within;
            }
        }
        if rocky > 0.0 && height > 0.2 {
            // A stack is in beds too, each a ledge.
            let bed = 3.4;
            let dip = fbm(p * 0.015, 2, 333) * 0.8 + x * 0.004;
            let level = height / bed + dip;
            let ledged = (level.floor() + smoothstep(0.1, 0.9, level.fract()) - dip) * bed;
            height += (ledged - height) * 0.75 * stacked;
            // And bare rock is rough.
            height += fbm(p * 0.9, 3, 335) * 0.22 * rocky;
        }
        height
    }

    /// The island as it would be with no road on it.
    #[cfg(test)]
    fn natural(&self, x: f32, z: f32) -> f32 {
        self.finish(x, z, self.top(x, z), 0.0)
    }
}

/// Where a point is beside the road.
#[derive(Clone, Copy)]
struct Beside {
    /// How far across the road's surface from its middle: above zero to the left.
    offset: f32,
    /// How far the asphalt reaches on that side, and how wide what lies beyond it is.
    half: f32,
    bands: f32,
    /// How far out the barrier stands on that side.
    fence: f32,
    /// The height of the road's surface there, carried on level beyond its edge.
    height: f32,
}

impl Beside {
    /// How far beyond the asphalt and what lies beside it: below zero, on them.
    fn beyond(&self) -> f32 {
        self.offset.abs() - self.half - self.bands
    }

    /// How far beyond the barrier: below zero, on the road's side of it.
    fn outside(&self) -> f32 {
        self.offset.abs() - self.fence
    }
}

/// The track, with a way to find the road near a point quickly.
struct Road<'a> {
    track: &'a Track,
    cell: f32,
    near: FxHashMap<(i32, i32), Vec<u32>>,
    /// How far out from the middle of the road the barrier stands at each sample, to the left
    /// and to the right.
    fence: [Vec<f32>; 2],
}

impl<'a> Road<'a> {
    fn new(track: &'a Track) -> Self {
        // Wide enough that a point's own square and those round it hold every sample within
        // reach of it.
        let cell = 110.0;
        let mut near: FxHashMap<(i32, i32), Vec<u32>> = FxHashMap::default();
        for (index, sample) in track.samples.iter().enumerate() {
            let key = (
                (sample.position.x / cell).floor() as i32,
                (sample.position.z / cell).floor() as i32,
            );
            near.entry(key).or_default().push(index as u32);
        }
        let mut road = Self {
            track,
            cell,
            near,
            fence: [Vec::new(), Vec::new()],
        };
        // What lies beside the road changes from one sample to the next. The barrier keeps
        // clear of the widest of it nearby, and comes in and out smoothly.
        let count = track.samples.len();
        for (side, left) in [(0, true), (1, false)] {
            let edge: Vec<f32> = (0..count)
                .map(|i| {
                    let sample = &track.samples[i];
                    let half = if left {
                        sample.half_left
                    } else {
                        sample.half_right
                    };
                    half + road.bands(i, left) + BARRIER
                })
                .collect();
            let mut fence: Vec<f32> = (0..count)
                .map(|i| {
                    (0..13)
                        .map(|k| edge[(i + count + k - 6) % count])
                        .fold(0.0, f32::max)
                })
                .collect();
            for _ in 0..8 {
                fence = (0..count)
                    .map(|i| {
                        (fence[(i + count - 1) % count] + 2.0 * fence[i] + fence[(i + 1) % count])
                            * 0.25
                    })
                    .collect();
            }
            road.fence[side] = (0..count).map(|i| fence[i].max(edge[i])).collect();
        }
        road
    }

    fn bands(&self, sample: usize, left: bool) -> f32 {
        let sample = &self.track.samples[sample];
        let bands = if left {
            &sample.left_bands
        } else {
            &sample.right_bands
        };
        bands.iter().map(|band| band.1).sum()
    }

    /// The road beside a point, if it is within reach of one.
    fn beside(&self, x: f32, z: f32) -> Option<Beside> {
        let key = (
            (x / self.cell).floor() as i32,
            (z / self.cell).floor() as i32,
        );
        let samples = &self.track.samples;
        let mut best: Option<(f32, usize)> = None;
        for dz in -1..=1 {
            for dx in -1..=1 {
                let Some(list) = self.near.get(&(key.0 + dx, key.1 + dz)) else {
                    continue;
                };
                for &index in list {
                    let p = samples[index as usize].position;
                    let d = (p.x - x).powi(2) + (p.z - z).powi(2);
                    if best.is_none_or(|(nearest, _)| d < nearest) {
                        best = Some((d, index as usize));
                    }
                }
            }
        }
        let (_, nearest) = best?;
        // The stretch of the line the point is beside: before the nearest sample, or after.
        let count = samples.len();
        let mut found: Option<(f32, usize, f32)> = None;
        for first in [(nearest + count - 1) % count, nearest] {
            let (a, b) = (
                samples[first].position,
                samples[(first + 1) % count].position,
            );
            let (ax, az) = (b.x - a.x, b.z - a.z);
            let share = (((x - a.x) * ax + (z - a.z) * az) / (ax * ax + az * az)).clamp(0.0, 1.0);
            let d = (a.x + ax * share - x).powi(2) + (a.z + az * share - z).powi(2);
            if found.is_none_or(|(least, _, _)| d < least) {
                found = Some((d, first, share));
            }
        }
        let (_, first, share) = found?;
        let second = (first + 1) % count;
        let (here, next) = (&samples[first], &samples[second]);
        let centre = here.position.lerp(next.position, share);
        let left = here.left.lerp(next.left, share).normalize_or(here.left);
        let level = (left.x * left.x + left.z * left.z).sqrt().max(1.0e-3);
        let offset = ((x - centre.x) * left.x + (z - centre.z) * left.z) / (level * level);
        let on_left = offset >= 0.0;
        let between = |a: f32, b: f32| a + (b - a) * share;
        let half = if on_left {
            between(here.half_left, next.half_left)
        } else {
            between(here.half_right, next.half_right)
        };
        let bands = between(self.bands(first, on_left), self.bands(second, on_left));
        let fence = &self.fence[if on_left { 0 } else { 1 }];
        Some(Beside {
            offset,
            half,
            bands,
            fence: between(fence[first], fence[second]),
            // Beyond the asphalt the ground is level with its edge. A banked corner's road
            // tilts; were its run-off to carry that tilt on outwards, it would rise all the way
            // to the barrier, and a car arriving fast would be thrown over it.
            height: centre.y + offset.clamp(-half, half) * left.y,
        })
    }

    /// A point of the road's surface, or of what lies beside it: so far round the lap, so far
    /// to the left of the road's middle, lifted off the ground by so much. The road tilts as
    /// it is banked; beyond its edge the ground carries on level from the edge.
    fn at(&self, sample: usize, offset: f32, lift: f32) -> Vec3 {
        let sample = &self.track.samples[sample % self.track.samples.len()];
        let on_road = offset.clamp(-sample.half_right, sample.half_left);
        let level = (sample.left * Vec3::new(1.0, 0.0, 1.0)).normalize_or_zero();
        sample.position + sample.left * on_road + level * (offset - on_road) + Vec3::Y * lift
    }
}

/// The land with the road laid across it: heights on a grid, which is the ground a car drives
/// on and what everything drawn is stood on.
struct Land {
    cells: u32,
    spacing: f32,
    origin: f32,
    heights: Vec<f32>,
    /// How far the road moves the land's top at each point, and how firmly it holds it.
    moved: Vec<f32>,
    held: Vec<f32>,
}

impl Land {
    /// The island, drawn level with the road wherever the road passes and eased back to its own
    /// shape beyond.
    fn new(island: &Island, road: &Road, size: f32) -> Self {
        let cells = (size / GROUND_SPACING).ceil() as u32;
        let spacing = size / cells as f32;
        let origin = -size * 0.5;
        let stride = cells as usize + 1;
        let place = |index: usize| {
            (
                origin + (index % stride) as f32 * spacing,
                origin + (index / stride) as f32 * spacing,
            )
        };
        // At each point: the island's top, how far the road moves it, and how firmly.
        let points: Vec<(f32, f32, f32)> = (0..stride * stride)
            .into_par_iter()
            .map(|index| {
                let (x, z) = place(index);
                let top = island.top(x, z);
                match road.beside(x, z) {
                    None => (top, 0.0, 0.0),
                    Some(beside) => {
                        // Level out to a little behind the barrier, wherever that stands: it
                        // comes in and out round a gravel trap more gently than the trap does.
                        let past = beside.outside() - (SHOULDER - BARRIER);
                        let held = 1.0 - smoothstep(0.0, BLEND, past);
                        (top, (beside.height - top) * held, held)
                    }
                }
            })
            .collect();
        // Where two stretches of road pass near each other, each moves the land its own way,
        // and where their reaches meet there would be a step. Spreading what the road asks
        // of the land, everywhere but where the road holds it, smooths that away.
        let mut moved: Vec<f32> = points.iter().map(|p| p.1).collect();
        let asked = moved.clone();
        let mut line = vec![0.0_f32; stride];
        let mut spread = vec![0.0_f32; stride];
        for _ in 0..6 {
            for row in 0..stride {
                line.copy_from_slice(&moved[row * stride..(row + 1) * stride]);
                soften(&line, &mut spread, 4);
                moved[row * stride..(row + 1) * stride].copy_from_slice(&spread);
            }
            for column in 0..stride {
                for row in 0..stride {
                    line[row] = moved[row * stride + column];
                }
                soften(&line, &mut spread, 4);
                for row in 0..stride {
                    moved[row * stride + column] = spread[row];
                }
            }
            for (index, point) in points.iter().enumerate() {
                if point.2 >= 1.0 {
                    moved[index] = asked[index];
                }
            }
        }
        let heights = (0..stride * stride)
            .into_par_iter()
            .map(|index| {
                let (x, z) = place(index);
                let (top, _, held) = points[index];
                island.finish(x, z, top + moved[index], held)
            })
            .collect();
        Self {
            cells,
            spacing,
            origin,
            heights,
            held: points.iter().map(|point| point.2).collect(),
            moved,
        }
    }

    /// The land's height at a point, from the island's own shape rather than the grid: the
    /// same at the grid's points, and with all the island's detail between them.
    fn exact(&self, island: &Island, x: f32, z: f32) -> f32 {
        island.finish(
            x,
            z,
            island.top(x, z) + self.among(&self.moved, x, z),
            self.among(&self.held, x, z),
        )
    }

    /// The land's height at a point.
    fn at(&self, x: f32, z: f32) -> f32 {
        self.among(&self.heights, x, z)
    }

    /// A value at a point, from its values on the grid.
    fn among(&self, values: &[f32], x: f32, z: f32) -> f32 {
        let limit = self.cells as f32 - 1.0e-3;
        let u = ((x - self.origin) / self.spacing).clamp(0.0, limit);
        let v = ((z - self.origin) / self.spacing).clamp(0.0, limit);
        let (column, row) = (u as usize, v as usize);
        let (fu, fv) = (u - column as f32, v - row as f32);
        let stride = self.cells as usize + 1;
        let height = |column: usize, row: usize| values[row * stride + column];
        let near = height(column, row) + (height(column + 1, row) - height(column, row)) * fu;
        let far =
            height(column, row + 1) + (height(column + 1, row + 1) - height(column, row + 1)) * fu;
        near + (far - near) * fv
    }

    /// How steep the land is at a point: its rise over its run.
    fn slope(&self, x: f32, z: f32) -> f32 {
        let step = 1.5;
        let dx = (self.at(x + step, z) - self.at(x - step, z)) / (2.0 * step);
        let dz = (self.at(x, z + step) - self.at(x, z - step)) / (2.0 * step);
        dx.hypot(dz)
    }

    fn ground(self) -> Heightfield {
        Heightfield {
            columns: self.cells + 1,
            rows: self.cells + 1,
            origin: [self.origin, self.origin],
            spacing: [self.spacing, self.spacing],
            heights: self.heights,
        }
    }
}

/// Each value of a line as the mean of those within `reach` of it.
fn soften(line: &[f32], out: &mut [f32], reach: usize) {
    let last = line.len() as isize - 1;
    let at = |i: isize| line[i.clamp(0, last) as usize] as f64;
    let reach = reach as isize;
    let mut sum: f64 = (-reach..=reach).map(at).sum();
    for i in 0..line.len() as isize {
        out[i as usize] = (sum / (2 * reach + 1) as f64) as f32;
        sum += at(i + reach + 1) - at(i - reach);
    }
}

fn hash(x: i32, z: i32, channel: u32) -> u32 {
    let mut h = (x as u32).wrapping_mul(0x8da6_b343)
        ^ (z as u32).wrapping_mul(0xd816_3841)
        ^ channel.wrapping_mul(0xcb1a_b31f);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2c1b_3c6d);
    h ^ (h >> 12)
}

/// Adds a mesh with a material, once, where it was made.
fn place(scene: &mut SceneAsset, mesh: &Mesh, material: u32) {
    if mesh.indices.is_empty() {
        return;
    }
    scene.instances.push(Instance {
        mesh: scene.meshes.len() as u32,
        material,
        transform: Mat4::IDENTITY,
    });
    scene
        .meshes
        .push(build_virtual_mesh(mesh, &BuildOptions::default()));
}

/// How wide the squares of ground are that what is laid along the road is gathered into.
const CHUNK: f32 = 96.0;

/// Adds flat pieces laid along the road, as a mesh for each square of ground they lie in.
///
/// A mesh keeps its points to a 65,536th of its size. Over a whole circuit that is more than a
/// centimetre, which is more than the paint on a road stands above it: as one mesh, a white
/// line comes and goes under the asphalt. In squares this size it is a millimetre or two.
fn lay(scene: &mut SceneAsset, pieces: &[[Vec3; 4]], material: u32) {
    let mut squares: FxHashMap<(i32, i32), Vec<[Vec3; 4]>> = FxHashMap::default();
    for piece in pieces {
        let middle = (piece[0] + piece[1] + piece[2] + piece[3]) * 0.25;
        let square = (
            (middle.x / CHUNK).floor() as i32,
            (middle.z / CHUNK).floor() as i32,
        );
        squares.entry(square).or_default().push(*piece);
    }
    // In a fixed order, so that a scene bakes the same every time.
    let mut order: Vec<(i32, i32)> = squares.keys().copied().collect();
    order.sort_unstable();
    for square in order {
        place(
            scene,
            &meshes::quads(&squares[&square], |_| Vec3::Y),
            material,
        );
    }
}

fn textured(scene: &mut SceneAsset, surface: &str, tint: f32) -> u32 {
    let layer = scene.textures.len() as i32;
    scene
        .textures
        .push(crate::textures::surface(surface, scene.texture_size));
    scene.materials.push(Material {
        base_color: [tint, tint, tint, 1.0],
        roughness: 1.0,
        uv_scale: 1.0 / TILE_METRES,
        normal_strength: 1.0,
        texture_layer: layer,
        macro_variation: 1.0,
        ..Default::default()
    });
    scene.materials.len() as u32 - 1
}

/// A patch of grass's blades, as a material.
fn blades(scene: &mut SceneAsset, meadow: &Meadow) -> u32 {
    let layer = scene.textures.len() as i32;
    scene
        .textures
        .push(meadow.atlas(0).bake(scene.texture_size));
    scene.materials.push(crate::foliage::blades(layer));
    scene.materials.len() as u32 - 1
}

/// Builds the circuit into the scene: the land `size` metres across, centred on the origin.
pub fn build(scene: &mut SceneAsset, track: &Track, size: f32) -> Built {
    let road = Road::new(track);
    let island = Island::new(track);
    println!("circuit: {} of {:.0} m", track.name, track.length());
    println!("  shaping the land...");
    let land = Land::new(&island, &road, size);

    println!("  textures...");
    let grass = textured(scene, "grass", 1.0);
    let rock = textured(scene, "rock", 0.8);
    let sand = textured(scene, "sand", 0.68);

    // ---- The land, in its three kinds.
    println!("  land...");
    let kinds = ground::draw(&island, &road, &land, size);
    for (kind, material) in [(GRASS, grass), (ROCK, rock), (SAND, sand)] {
        println!("    {} triangles", kinds[kind].triangle_count());
        place(scene, &kinds[kind], material);
    }

    // ---- The road, and what is built beside it.
    println!("  road...");
    let blockers = paving::pave(scene, &road, &land, grass);
    furniture::furnish(scene, &road, &land);

    // ---- The long grass along the road.
    println!("  grass...");
    verge::grow(scene, &road, &land);

    // ---- Woods beyond the barriers: thick inland, thinning to open ground towards the cliffs.
    println!("  trees...");
    let grown = crate::foliage::grow(scene, &[("oak", 2), ("birch", 2), ("fir", 2), ("bush", 2)]);
    let plant = |scene: &mut SceneAsset, kind: usize, x: f32, z: f32, k: u32| {
        let copy = hash(x as i32, z as i32, k + 2);
        let turn = (hash(x as i32, z as i32, k) % 1000) as f32 / 1000.0 * std::f32::consts::TAU;
        let scale = 0.8 + (hash(x as i32, z as i32, k + 1) % 1000) as f32 / 1000.0 * 0.5;
        let transform = Mat4::from_translation(Vec3::new(x, land.at(x, z) - 0.05, z))
            * Mat4::from_rotation_y(turn)
            * Mat4::from_scale(Vec3::splat(scale));
        grown[kind].place(scene, copy, transform);
    };
    // Whether a tree may stand here: on dry, gentle ground, clear of the barrier.
    let open = |x: f32, z: f32, clear: f32| {
        land.at(x, z) > 2.5
            && land.slope(x, z) < 0.45
            && road.beside(x, z).is_none_or(|b| b.outside() > clear)
    };
    let mut rng = Rng::new(11, 5);
    let half = size * 0.5;
    let mut stands: Vec<(f32, f32)> = Vec::new();
    let mut taken: FxHashMap<(i32, i32), Vec<(f32, f32)>> = FxHashMap::default();
    for _ in 0..110_000 {
        let (x, z) = (rng.range(-half, half), rng.range(-half, half));
        // Woods in drifts, with open ground between; none along the cliffs, for the view.
        let drift = fbm(Vec3::new(x * 0.0045, 5.0, z * 0.0045), 3, 320);
        let shore = smoothstep(25.0, 110.0, island.inland(x, z));
        let density = smoothstep(-0.12, 0.22, drift) * shore;
        if rng.unit() > density || !open(x, z, 4.0) {
            continue;
        }
        let spacing = 11.0 - 4.0 * density;
        let key = ((x / 12.0).floor() as i32, (z / 12.0).floor() as i32);
        let crowded = (-1..=1).any(|dz| {
            (-1..=1).any(|dx| {
                taken.get(&(key.0 + dx, key.1 + dz)).is_some_and(|near| {
                    near.iter()
                        .any(|&(sx, sz)| (sx - x).powi(2) + (sz - z).powi(2) < spacing * spacing)
                })
            })
        });
        if !crowded {
            taken.entry(key).or_default().push((x, z));
            stands.push((x, z));
        }
    }
    for (index, &(x, z)) in stands.iter().enumerate() {
        let high = fbm(Vec3::new(x * 0.008, 9.0, z * 0.008), 3, 321);
        let kind = if high > 0.12 {
            2
        } else if high < -0.1 {
            1
        } else {
            0
        };
        plant(scene, kind, x, z, 9 + index as u32 % 3);
    }
    // Trees standing along the road, behind the barriers: here a row of them, there none.
    let mut lining = 0;
    for _ in 0..90_000 {
        let (x, z) = (rng.range(-half, half), rng.range(-half, half));
        let Some(beside) = road.beside(x, z) else {
            continue;
        };
        let along = fbm(Vec3::new(x * 0.011, 2.0, z * 0.011), 2, 322);
        let shore = smoothstep(60.0, 130.0, island.inland(x, z));
        if !(5.0..38.0).contains(&beside.outside())
            || rng.unit() > smoothstep(-0.1, 0.3, along) * shore * 0.75
            || !open(x, z, 5.0)
        {
            continue;
        }
        let key = ((x / 12.0).floor() as i32, (z / 12.0).floor() as i32);
        let crowded = (-1..=1).any(|dz| {
            (-1..=1).any(|dx| {
                taken.get(&(key.0 + dx, key.1 + dz)).is_some_and(|near| {
                    near.iter()
                        .any(|&(sx, sz)| (sx - x).powi(2) + (sz - z).powi(2) < 49.0)
                })
            })
        });
        if !crowded {
            taken.entry(key).or_default().push((x, z));
            let kind = if along > 0.25 {
                2
            } else if rng.unit() < 0.5 {
                1
            } else {
                0
            };
            plant(scene, kind, x, z, 21);
            lining += 1;
        }
    }
    let mut bushes = 0;
    for _ in 0..160_000 {
        let (x, z) = (rng.range(-half, half), rng.range(-half, half));
        let Some(beside) = road.beside(x, z) else {
            continue;
        };
        // Along the back of the barrier.
        if (1.5..10.0).contains(&beside.outside()) && rng.unit() < 0.3 && open(x, z, 1.5) {
            plant(scene, 3, x, z, 15);
            bushes += 1;
        }
    }
    println!("    {lining} trees along the road");
    println!("    {} trees and {bushes} bushes", stands.len());

    // ---- Rock: boulders along the shore, and crags standing out of the cliffs' faces. Each
    // size of boulder is made at its size, so that its stone is at the right scale.
    println!("  rocks...");
    const SIZES: [f32; 3] = [1.3, 3.2, 6.5];
    const EACH: u32 = 3;
    let first_rock = scene.meshes.len() as u32;
    for (which, &radius) in SIZES.iter().enumerate() {
        for seed in 0..EACH {
            let detail = if radius > 3.0 { 5 } else { 4 };
            let mut mesh = meshes::rock(detail, 40 + which as u32 * 7 + seed);
            for vertex in &mut mesh.vertices {
                vertex.position *= radius;
                vertex.uv *= radius;
            }
            scene
                .meshes
                .push(build_virtual_mesh(&mesh, &BuildOptions::default()));
        }
    }
    let boulder = |scene: &mut SceneAsset, rng: &mut Rng, size: usize, at: Vec3| {
        let scale = rng.range(0.8, 1.25);
        scene.instances.push(Instance {
            mesh: first_rock + size as u32 * EACH + rng.index(EACH as usize) as u32,
            material: rock,
            transform: Mat4::from_translation(at)
                * Mat4::from_rotation_y(rng.range(0.0, std::f32::consts::TAU))
                * Mat4::from_rotation_x(rng.range(-0.5, 0.5))
                * Mat4::from_scale(Vec3::splat(scale)),
        });
    };
    let (mut boulders, mut crags) = (0, 0);
    for _ in 0..400_000 {
        let (x, z) = (rng.range(-half, half), rng.range(-half, half));
        let height = land.at(x, z);
        if !(-2.0..60.0).contains(&height) {
            continue;
        }
        let rocky = island.rocky(x, z);
        if rocky > 0.5 && height > 2.5 {
            // On a cliff's face, or a stack's: well below its top, so that a crag stands out
            // of the face and not above it.
            let top = land
                .at(x - 14.0, z)
                .max(island.stack(x, z).map_or(0.0, |(high, _)| high + 3.0));
            if height < top - 6.0 && rng.unit() < 0.1 {
                let size = if rng.unit() < 0.6 { 2 } else { 1 };
                // Which way is down the face, on the level; the crag is set back into it.
                let fall = Vec3::new(
                    land.at(x - 2.0, z) - land.at(x + 2.0, z),
                    0.0,
                    land.at(x, z - 2.0) - land.at(x, z + 2.0),
                )
                .normalize_or_zero();
                let radius = SIZES[size];
                boulder(
                    scene,
                    &mut rng,
                    size,
                    Vec3::new(x, height - radius * 0.1, z) - fall * radius * 0.72,
                );
                crags += 1;
            }
        } else if height < 2.5 {
            // At the waterline anywhere, and thick under the cliffs.
            let east = island.east(x);
            if rng.unit() < 0.012 + 0.2 * east {
                let size = if rng.unit() < 0.3 * east { 1 } else { 0 };
                boulder(
                    scene,
                    &mut rng,
                    size,
                    Vec3::new(x, height - SIZES[size] * 0.25, z),
                );
                boulders += 1;
            }
        }
    }
    println!("    {boulders} boulders and {crags} crags");

    Built {
        ground: land.ground(),
        blockers,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use solstice_game::track::Sample;
    use solstice_physics::vehicle::Surface;

    /// A flat ring of road, 200 m in radius, 12 m wide, at a height: gravel outside it, a kerb
    /// inside.
    fn ring(height: f32) -> Track {
        let samples = (0..420)
            .map(|k| {
                let a = k as f32 / 420.0 * std::f32::consts::TAU;
                // Clockwise seen from above: the left of the road is the outside.
                let position = Vec3::new(a.cos() * 200.0, height, a.sin() * 200.0);
                let forward = Vec3::new(-a.sin(), 0.0, a.cos());
                Sample {
                    position,
                    left: Vec3::Y.cross(forward),
                    half_left: 6.0,
                    half_right: 6.0,
                    left_bands: vec![(Surface::Gravel, 10.0)],
                    right_bands: vec![(Surface::Kerb, 1.5)],
                }
            })
            .collect();
        Track::new("Ring", samples, vec![140, 280]).unwrap()
    }

    #[test]
    fn the_verge_grass_sways_as_grass() {
        let mut scene = SceneAsset {
            texture_size: 16,
            ..Default::default()
        };
        let verge = Meadow::by_name("verge").expect("a built-in grass");
        let blade = blades(&mut scene, &verge) as usize;
        let flags = scene.materials[blade].flags;
        assert_eq!(flags & Material::SWAYS, Material::SWAYS);
        assert_eq!(flags & Material::TREE, 0, "grass sways as grass");
    }

    #[test]
    fn the_land_is_level_with_the_road_and_becomes_its_own_further_off() {
        let track = ring(30.0);
        let (road, island) = (Road::new(&track), Island::new(&track));
        let land = Land::new(&island, &road, 1000.0);
        // On the road, on what lies beside it, and on the shoulder beyond: the road's height.
        for radius in [194.0, 200.0, 206.0, 215.0, 222.0, 186.0, 182.0] {
            let height = land.at(radius, 0.0);
            assert!((height - 30.0).abs() < 0.02, "at {radius}: {height}");
        }
        // Far from it, the island's own.
        for (x, z) in [(0.0, 0.0), (320.0, 0.0), (0.0, -330.0)] {
            assert!(
                (land.at(x, z) - island.natural(x, z)).abs() < 0.05,
                "at {x}, {z}"
            );
        }
        // Between, between: no step. (To the west: to the east are the cliffs.)
        let mut last = land.at(-222.0, 0.0);
        let mut x = 222.0;
        while x < 300.0 {
            x += 0.5;
            let here = land.at(-x, 0.0);
            assert!((here - last).abs() < 0.8, "a step at {x}");
            last = here;
        }
    }

    #[test]
    fn beside_a_banked_road_the_ground_is_level_with_its_edge() {
        // The ring again, banked four degrees: its outside edge, the left, is the higher.
        let mut track = ring(30.0);
        let tilt = 4.0_f32.to_radians();
        let samples = track
            .samples
            .iter()
            .map(|sample| Sample {
                left: sample.left * tilt.cos() + Vec3::Y * tilt.sin(),
                ..sample.clone()
            })
            .collect();
        track = Track::new("Banked", samples, vec![140, 280]).unwrap();
        let (road, island) = (Road::new(&track), Island::new(&track));
        let land = Land::new(&island, &road, 1000.0);
        let rise = 6.0 * tilt.sin();
        // Across the road the land tilts with it.
        assert!((land.at(200.0, 0.0) - 30.0).abs() < 0.02);
        assert!((land.at(204.0, 0.0) - (30.0 + 4.0 * tilt.sin())).abs() < 0.03);
        // Over the gravel outside and out to the barrier it is level with the outside edge,
        // and inside, with the inside edge: nowhere does it go on rising.
        for radius in [208.0, 214.0, 221.0, 225.0] {
            let height = land.at(radius, 0.0);
            assert!(
                (height - (30.0 + rise)).abs() < 0.03,
                "at {radius}: {height}"
            );
        }
        for radius in [192.0, 187.0, 183.0] {
            let height = land.at(radius, 0.0);
            assert!(
                (height - (30.0 - rise)).abs() < 0.03,
                "at {radius}: {height}"
            );
        }
        // What is laid beside the road lies on that ground.
        assert!((road.at(0, 6.0, 0.0).y - (30.0 + rise)).abs() < 1.0e-3);
        assert!((road.at(0, 16.0, 0.0).y - (30.0 + rise)).abs() < 1.0e-3);
        assert!((road.at(0, -7.5, 0.0).y - (30.0 - rise)).abs() < 1.0e-3);
        assert!((road.at(0, 16.0, 0.0).x - (200.0 + 6.0 * tilt.cos() + 10.0)).abs() < 1.0e-3);
    }

    #[test]
    fn a_point_beside_the_road_knows_its_side_and_what_lies_there() {
        let track = ring(30.0);
        let road = Road::new(&track);
        let outside = road.beside(210.0, 0.0).unwrap();
        assert!((outside.offset - 10.0).abs() < 0.05 && outside.bands == 10.0);
        assert!(outside.beyond() < 0.0, "on the gravel");
        assert!(
            (outside.outside() + 15.0).abs() < 0.05,
            "the barrier is 9 m past the gravel"
        );
        let inside = road.beside(190.0, 0.0).unwrap();
        assert!((inside.offset + 10.0).abs() < 0.05 && inside.bands == 1.5);
        assert!((inside.beyond() - 2.5).abs() < 0.05, "past the kerb");
        assert!(road.beside(600.0, 0.0).is_none(), "out of reach");
    }

    #[test]
    fn the_land_is_drawn_under_the_road_and_not_beside_it() {
        let track = ring(30.0);
        let (road, island) = (Road::new(&track), Island::new(&track));
        let land = Land::new(&island, &road, 1000.0);
        let kinds = ground::draw(&island, &road, &land, 1000.0);
        // Every point of the mesh near the road: sunk under it and under the gravel, level
        // with it on the verge.
        let mut seen = [0; 3];
        for vertex in kinds.iter().flat_map(|mesh| &mesh.vertices) {
            let p = vertex.position;
            let radius = p.x.hypot(p.z);
            let expected = if (195.0..215.0).contains(&radius) {
                seen[0] += 1;
                30.0 - SINK
            } else if (219.0..225.0).contains(&radius) || (183.0..186.0).contains(&radius) {
                seen[1] += 1;
                30.0
            } else {
                continue;
            };
            assert!((p.y - expected).abs() < 0.02, "at {radius}: {}", p.y);
        }
        assert!(seen[0] > 1000 && seen[1] > 1000);
        // The cliffs are drawn finer than the rest: there is rock, in small pieces.
        let small = kinds[ROCK]
            .indices
            .chunks_exact(3)
            .filter(|t| {
                let [a, b, c] =
                    [t[0], t[1], t[2]].map(|i| kinds[ROCK].vertices[i as usize].position);
                (a.x - b.x).abs().max((a.x - c.x).abs()) < 0.7
            })
            .count();
        assert!(small > 1000, "{small} small pieces of rock");
    }

    #[test]
    fn where_two_stretches_of_road_pass_each_other_the_land_between_has_no_step() {
        // Two straights 90 m apart, one 4 m above the other, joined at their ends far away.
        let mut samples = Vec::new();
        let mut add = |x: f32, y: f32, z: f32, forward: Vec3| {
            samples.push(Sample {
                position: Vec3::new(x, y, z),
                left: Vec3::Y.cross(forward),
                half_left: 6.0,
                half_right: 6.0,
                left_bands: Vec::new(),
                right_bands: Vec::new(),
            });
        };
        for k in 0..100 {
            add(-45.0, 20.0, -150.0 + k as f32 * 3.0, Vec3::Z);
        }
        for k in 0..30 {
            let a = k as f32 / 30.0 * std::f32::consts::PI;
            add(
                -45.0 * a.cos(),
                20.0 + 4.0 * k as f32 / 30.0,
                150.0 + 45.0 * a.sin(),
                Vec3::new(a.sin(), 0.0, a.cos()),
            );
        }
        for k in 0..100 {
            add(45.0, 24.0, 150.0 - k as f32 * 3.0, -Vec3::Z);
        }
        for k in 0..30 {
            let a = k as f32 / 30.0 * std::f32::consts::PI;
            add(
                45.0 * a.cos(),
                24.0 - 4.0 * k as f32 / 30.0,
                -150.0 - 45.0 * a.sin(),
                Vec3::new(-a.sin(), 0.0, -a.cos()),
            );
        }
        let track = Track::new("Pair", samples, vec![80, 160]).unwrap();
        let (road, island) = (Road::new(&track), Island::new(&track));
        let land = Land::new(&island, &road, 600.0);
        assert!((land.at(-45.0, 30.0) - 20.0).abs() < 0.02);
        assert!((land.at(45.0, 30.0) - 24.0).abs() < 0.02);
        let mut x = -45.0;
        let mut last = land.at(x, 30.0);
        while x < 45.0 {
            x += 0.5;
            let here = land.at(x, 30.0);
            assert!(
                (here - last).abs() < 0.3,
                "a step of {} at {x}",
                here - last
            );
            last = here;
        }
    }

    #[test]
    fn the_island_has_sea_round_it_and_cliffs_to_the_east() {
        let track = ring(30.0);
        let island = Island::new(&track);
        assert!(island.natural(0.0, 0.0) > 10.0);
        for (x, z) in [(-900.0, 0.0), (0.0, 900.0), (0.0, -900.0)] {
            assert!(island.natural(x, z) < -3.0, "sea at {x}, {z}");
        }
        // Walking east to the sea the land ends in a drop; walking west, in a beach.
        let steepest = |towards: f32| {
            let mut steepest = 0.0_f32;
            let mut x = 0.0_f32;
            while x.abs() < 700.0 {
                let rise = (island.natural(x + towards, 0.0) - island.natural(x, 0.0)).abs();
                if island.natural(x, 0.0) > 0.5 && x.abs() > 215.0 {
                    steepest = steepest.max(rise);
                }
                x += towards;
            }
            steepest
        };
        assert!(steepest(1.0) > 1.5, "a cliff: {}", steepest(1.0));
        assert!(steepest(-1.0) < 0.6, "a beach: {}", steepest(-1.0));
    }
}
