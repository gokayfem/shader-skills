//! Ruts: ground that stays where a tyre has pressed it.
//!
//! Soft ground does not spring back. A tyre that sinks into mud leaves a rut as deep as it
//! sank, heaps what it pushed aside into a berm either side, and prints its tread on the
//! floor of it; the next wheel along rides in that rut, and the next car finds it there. This
//! is the map of all that: how far the ground has been pressed down or heaped up at each
//! place, what the tread printed, and how wet what was churned is.
//!
//! * **One map for the wheels and for the eye.** A wheel reads its depth here
//!   ([`Ruts::depth`], through [`crate::Ground::rut`]) and presses it deeper
//!   ([`Ruts::press`]); the renderer draws the same cells, byte for byte
//!   ([`Ruts::layer`]). So a tyre sits in the rut that is drawn.
//! * **Only where something has been driven.** The map is squares of [`TILE`] cells a side,
//!   each made when it is first pressed: a map nobody has driven on holds nothing, and
//!   reading it costs one look in an empty table.
//! * **Cells of [`CELL`] metres**, each eight bytes in two layers, laid out as the renderer
//!   samples them:
//!
//!   | Layer | Byte | Holds |
//!   | --- | --- | --- |
//!   | 0 | 0, 1 | The ground's height against how it was made, in steps of [`HEIGHT_UNIT`], high byte first: [`HEIGHT_ZERO`] is untouched, less is a rut. |
//!   | 0 | 2 | How far across the tyre that printed it the cell is, over [`ACROSS_REACH`] either way. |
//!   | 0 | 3 | How wet it is, from 0 to 255. |
//!   | 1 | 0, 1 | The tread's print: where in the tread's pattern the cell was pressed, as that angle's cosine and sine, times how crisp the print is: 128 is none, 127 either way of it is 1. |
//!   | 1 | 2 | How much churned soil shows, in place of whatever covered the ground. |
//!   | 1 | 3 | 255 if anything has touched the cell. |
//!
//!   Every value is one a sampler can blend between cells: the height's two bytes blend
//!   apart and are put together after, which comes to the same as blending the height.
//!
//! Nothing here knows about the GPU, the physics engine or a car: it is pressed with a
//! [`Press`], and read.

use glam::Vec2;
use std::collections::HashMap;

/// The width of a cell, in metres: a tyre's tread is seven or eight across.
pub const CELL: f32 = 0.04;
/// Cells a side in each square of the map that is made at once: 2.56 m.
pub const TILE: usize = 64;
/// How many sizes each square is kept at for the renderer: its cells, then halved five times.
pub const LEVELS: usize = 6;
/// One step of a cell's height, in metres, and the value that is untouched ground: heights
/// run 65 cm either way.
pub const HEIGHT_UNIT: f32 = 1.0 / 50_000.0;
pub const HEIGHT_ZERO: u16 = 32_768;
/// How far either side of a tyre's middle a cell's place across it can be kept, in metres.
pub const ACROSS_REACH: f32 = 0.32;
/// How long churned ground takes to lose most of what made it wetter than it stays, in
/// seconds: a fresh rut's sheen is gone in a few minutes.
pub const DRY_SECONDS: f32 = 150.0;
/// How often a square that is drying is dried, in seconds, and how many are in one go.
const DRY_EVERY: f32 = 6.0;
const DRY_AT_ONCE: usize = 6;
/// The deepest a rut and the highest a berm can be, in metres.
pub const DEEPEST: f32 = 0.6;
const HIGHEST: f32 = 0.2;

const CELLS: usize = TILE * TILE;
/// A tile's width, in metres.
const TILE_METRES: f32 = TILE as f32 * CELL;

/// A tyre's contact with the ground over one step, or anything else that presses it: a
/// strip from where the middle of the contact was to where it is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Press {
    /// The middle of the contact as the step began and as it ended: world x and z.
    pub from: Vec2,
    pub to: Vec2,
    /// The way the tyre points along the ground: a unit vector. What it presses is laid out
    /// along this, however the tyre slid.
    pub along: Vec2,
    /// Half the tyre's width, and half the length of its contact, in metres.
    pub half_width: f32,
    pub half_length: f32,
    /// How far below the ground as it was made the bottom of the tyre is, in metres.
    pub depth: f32,
    /// The share of what is pressed out of the rut that is heaped up beside it, from 0 to
    /// 1: the rest is packed down.
    pub berm: f32,
    /// Where in the tread's pattern the contact was at each end, in radians: a lug passes
    /// with each full turn of it.
    pub phase: (f32, f32),
    /// How crisp a print the tread leaves, from 0 to 1: a rolling tyre prints, a spinning
    /// one smears.
    pub print: f32,
    /// How wet what is pressed is now, and how wet it stays once it has dried, from 0 to 1.
    pub wet: f32,
    pub stays: f32,
    /// How much churned soil is left showing, from 0 to 1.
    pub churn: f32,
}

/// One square of the map.
struct Tile {
    /// The two layers the renderer samples, four bytes a cell.
    layers: [Box<[u8]>; 2],
    /// How wet each cell stays, to dry towards.
    stays: Box<[u8]>,
    /// Counts each change, for whoever keeps a copy.
    version: u32,
    /// Whether any cell is wetter than it stays, and when it was last dried.
    damp: bool,
    dried_at: f32,
    /// Whether it is on the list of squares changed since they were last asked for.
    listed: bool,
}

impl Tile {
    fn new(clock: f32) -> Self {
        let mut first = vec![0_u8; CELLS * 4].into_boxed_slice();
        let zero = HEIGHT_ZERO.to_be_bytes();
        for cell in first.chunks_exact_mut(4) {
            cell.copy_from_slice(&[zero[0], zero[1], 128, 0]);
        }
        let mut second = vec![0_u8; CELLS * 4].into_boxed_slice();
        for cell in second.chunks_exact_mut(4) {
            cell.copy_from_slice(&[128, 128, 0, 0]);
        }
        Self {
            layers: [first, second],
            stays: vec![0_u8; CELLS].into_boxed_slice(),
            version: 0,
            damp: false,
            dried_at: clock,
            listed: false,
        }
    }

    fn height(&self, cell: usize) -> f32 {
        let raw = u16::from_be_bytes([self.layers[0][cell * 4], self.layers[0][cell * 4 + 1]]);
        (f32::from(raw) - f32::from(HEIGHT_ZERO)) * HEIGHT_UNIT
    }

    /// Everything a cell keeps: both layers' bytes, and how wet it stays.
    fn cell_bytes(&self, cell: usize) -> [u8; 9] {
        let (a, b) = (
            &self.layers[0][cell * 4..][..4],
            &self.layers[1][cell * 4..][..4],
        );
        [
            a[0],
            a[1],
            a[2],
            a[3],
            b[0],
            b[1],
            b[2],
            b[3],
            self.stays[cell],
        ]
    }

    fn set_height(&mut self, cell: usize, height: f32) {
        let raw = (height.clamp(-DEEPEST, HIGHEST) / HEIGHT_UNIT).round() + f32::from(HEIGHT_ZERO);
        let bytes = (raw as u16).to_be_bytes();
        self.layers[0][cell * 4] = bytes[0];
        self.layers[0][cell * 4 + 1] = bytes[1];
    }
}

/// From 0 to 1 as a byte, and back.
fn byte(share: f32) -> u8 {
    (share.clamp(0.0, 1.0) * 255.0).round() as u8
}

fn smoothstep(low: f32, high: f32, x: f32) -> f32 {
    let t = ((x - low) / (high - low)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The shape of a rut across the tyre that made it, and of the berm beside it: the height
/// of the ground `across` metres from the tyre's middle, for a tyre `half_width` wide sunk
/// `depth`, with a berm `berm` high and `berm_width` wide. Flat under the tread, rounded at
/// the shoulders, heaped just outside them.
fn profile(across: f32, half_width: f32, depth: f32, berm: f32, berm_width: f32) -> f32 {
    let u = across.abs() / half_width;
    let rut = depth * (1.0 - smoothstep(0.72, 1.0, u));
    let v = (across.abs() - 0.9 * half_width) / berm_width;
    let heap = if (0.0..1.0).contains(&v) {
        berm * smoothstep(0.0, 0.35, v) * (1.0 - smoothstep(0.35, 1.0, v))
    } else {
        0.0
    };
    heap - rut
}

/// How high and how wide the berm beside a rut is, in metres: what was pressed out of half
/// the rut, less what was packed down, heaped in a ridge.
fn berm_of(press: &Press) -> (f32, f32) {
    let depth = press.depth.max(0.0);
    let width = 0.1 + 0.5 * depth;
    let height = press.berm.clamp(0.0, 1.0) * depth * press.half_width / (0.5 * width);
    (height.min(0.6 * depth).min(0.12), width)
}

/// The map of what has been pressed into the ground.
#[derive(Default)]
pub struct Ruts {
    tiles: HashMap<(i32, i32), Tile>,
    /// Seconds it has been kept for.
    clock: f32,
    /// The squares changed since [`Ruts::take_changed`] was last called.
    changed: Vec<(i32, i32)>,
    /// When squares were last dried, and which was dried last.
    dried_at: f32,
    drying_from: usize,
}

impl Ruts {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether nothing has been pressed anywhere.
    pub fn is_empty(&self) -> bool {
        self.tiles.is_empty()
    }

    /// How many squares of ground have been pressed.
    pub fn tiles(&self) -> usize {
        self.tiles.len()
    }

    /// Forgets every rut: the ground is as it was made.
    pub fn clear(&mut self) {
        // Whoever keeps a copy is told each square is gone.
        for (key, _) in self.tiles.drain() {
            self.changed.push(key);
        }
    }

    /// The square a place is in, and the cell of that square.
    fn cell_at(i: i32, j: i32) -> ((i32, i32), usize) {
        let tile = (i.div_euclid(TILE as i32), j.div_euclid(TILE as i32));
        let cell = j.rem_euclid(TILE as i32) as usize * TILE + i.rem_euclid(TILE as i32) as usize;
        (tile, cell)
    }

    fn cell_height(&self, i: i32, j: i32) -> f32 {
        let (key, cell) = Self::cell_at(i, j);
        self.tiles.get(&key).map_or(0.0, |tile| tile.height(cell))
    }

    /// The ground's height at a place against how it was made, in metres: below zero in a
    /// rut, above it on a berm. Between cells' middles it is blended, as it is drawn.
    pub fn height(&self, x: f32, z: f32) -> f32 {
        if self.tiles.is_empty() {
            return 0.0;
        }
        let (u, v) = (x / CELL - 0.5, z / CELL - 0.5);
        let (i, j) = (u.floor(), v.floor());
        let (fu, fv) = (u - i, v - j);
        let (i, j) = (i as i32, j as i32);
        // All four in one square, as nearly always: one look.
        let (key, cell) = Self::cell_at(i, j);
        let (column, row) = (cell % TILE, cell / TILE);
        if column + 1 < TILE && row + 1 < TILE {
            let Some(tile) = self.tiles.get(&key) else {
                return 0.0;
            };
            let low = tile.height(cell) + (tile.height(cell + 1) - tile.height(cell)) * fu;
            let high = tile.height(cell + TILE)
                + (tile.height(cell + TILE + 1) - tile.height(cell + TILE)) * fu;
            return low + (high - low) * fv;
        }
        let low =
            self.cell_height(i, j) + (self.cell_height(i + 1, j) - self.cell_height(i, j)) * fu;
        let high = self.cell_height(i, j + 1)
            + (self.cell_height(i + 1, j + 1) - self.cell_height(i, j + 1)) * fu;
        low + (high - low) * fv
    }

    /// How deep the rut at a place is, in metres: below zero on a berm.
    pub fn depth(&self, x: f32, z: f32) -> f32 {
        -self.height(x, z)
    }

    /// Which way the pressed ground slopes at a place, and how steeply: its height's rise
    /// for each metre along x and along z, measured over `reach` metres either way.
    pub fn slope(&self, x: f32, z: f32, reach: f32) -> Vec2 {
        if self.tiles.is_empty() {
            return Vec2::ZERO;
        }
        let reach = reach.max(CELL);
        Vec2::new(
            self.height(x + reach, z) - self.height(x - reach, z),
            self.height(x, z + reach) - self.height(x, z - reach),
        ) / (2.0 * reach)
    }

    /// How wet the pressed ground at a place is, from 0 to 1: the nearest cell's.
    pub fn wet(&self, x: f32, z: f32) -> f32 {
        let (key, cell) = Self::cell_at((x / CELL).floor() as i32, (z / CELL).floor() as i32);
        self.tiles
            .get(&key)
            .map_or(0.0, |tile| f32::from(tile.layers[0][cell * 4 + 3]) / 255.0)
    }

    /// The tread's print at a place: how crisp it is, from 0 to 1, and where in the
    /// tread's pattern the cell was pressed, in radians. The nearest cell's.
    pub fn print(&self, x: f32, z: f32) -> (f32, f32) {
        let (key, cell) = Self::cell_at((x / CELL).floor() as i32, (z / CELL).floor() as i32);
        self.tiles.get(&key).map_or((0.0, 0.0), |tile| {
            let c = (f32::from(tile.layers[1][cell * 4]) - 128.0) / 127.0;
            let s = (f32::from(tile.layers[1][cell * 4 + 1]) - 128.0) / 127.0;
            (c.hypot(s).min(1.0), s.atan2(c))
        })
    }

    fn touch(&mut self, key: (i32, i32)) -> &mut Tile {
        let clock = self.clock;
        let tile = self.tiles.entry(key).or_insert_with(|| Tile::new(clock));
        if !tile.listed {
            tile.listed = true;
            self.changed.push(key);
        }
        tile.version = tile.version.wrapping_add(1).max(1);
        tile
    }

    /// Presses the ground along a strip: a tyre's contact over one step.
    ///
    /// * Under the tread the ground goes down to the tyre's depth, and never comes back up:
    ///   a rut is as deep as the deepest wheel that made it.
    /// * Beside it what was pressed out is heaped in a berm. A berm heaped where a rut
    ///   already is fills some of it.
    /// * The floor takes the tread's print wherever the tyre reaches it, and what is
    ///   pressed is as wet and as churned as the press says.
    ///
    /// Pressing the same strip twice leaves what pressing it once did.
    pub fn press(&mut self, press: &Press) {
        let finite = press.from.is_finite()
            && press.to.is_finite()
            && press.depth.is_finite()
            && press.half_width.is_finite()
            && press.half_length.is_finite();
        if !finite || press.half_width <= 0.0 {
            return;
        }
        let depth = press.depth.clamp(0.0, DEEPEST);
        // Nothing to leave: firm, dry ground under a clean tyre.
        if depth < 2.0 * HEIGHT_UNIT && press.wet <= 0.0 && press.churn <= 0.0 {
            return;
        }
        let along = press.along.try_normalize().unwrap_or(Vec2::NEG_Y);
        let side = along.perp();
        // How far the contact went along the way the tyre points. What it slid sideways
        // widens the strip: a tyre pushed across the ground scrapes a broader mark.
        let moved = press.to - press.from;
        let length = moved.dot(along);
        let slid = moved.dot(side);
        let (start, end) = (length.min(0.0), length.max(0.0));
        let half_length = press.half_length.max(CELL);
        let (berm, berm_width) = berm_of(press);
        let reach = (press.half_width + berm_width).max(press.half_width * 1.05) + slid.abs();
        // The cells the strip can reach.
        let corners = [
            press.from + along * (start - half_length) + side * reach,
            press.from + along * (start - half_length) - side * reach,
            press.from + along * (end + half_length) + side * reach,
            press.from + along * (end + half_length) - side * reach,
        ];
        let low = corners.iter().fold(Vec2::MAX, |low, c| low.min(*c));
        let high = corners.iter().fold(Vec2::MIN, |high, c| high.max(*c));
        let (i0, j0) = ((low.x / CELL).floor() as i32, (low.y / CELL).floor() as i32);
        let (i1, j1) = (
            (high.x / CELL).floor() as i32,
            (high.y / CELL).floor() as i32,
        );
        // A strip is a tyre's width and a step's length: anything far larger is a mistake,
        // a car put somewhere else between two steps.
        if i1.saturating_sub(i0) > 400 || j1.saturating_sub(j0) > 400 {
            return;
        }
        let wet = byte(press.wet);
        let stays = byte(press.stays.min(press.wet));
        let churn = press.churn.clamp(0.0, 1.0);
        let print = press.print.clamp(0.0, 1.0);
        let side_cells = TILE as i32;
        let clock = self.clock;
        for tile_j in j0.div_euclid(side_cells)..=j1.div_euclid(side_cells) {
            for tile_i in i0.div_euclid(side_cells)..=i1.div_euclid(side_cells) {
                let key = (tile_i, tile_j);
                // The square is taken out of the map while it is pressed, and made only if
                // the strip turns out to reach it.
                let mut held: Option<Tile> = None;
                // Whether the square was in the map already, and whether this changed it: a
                // square is counted changed, and copied again, only when its bytes are.
                let mut existed = false;
                let mut dirty = false;
                let rows =
                    j0.max(tile_j * side_cells)..=j1.min(tile_j * side_cells + side_cells - 1);
                for j in rows {
                    let columns =
                        i0.max(tile_i * side_cells)..=i1.min(tile_i * side_cells + side_cells - 1);
                    for i in columns {
                        let p = Vec2::new((i as f32 + 0.5) * CELL, (j as f32 + 0.5) * CELL)
                            - press.from;
                        let s = p.dot(along);
                        // Where along the strip the cell is, from 0 to 1, and how far
                        // across the tyre as it was when it passed.
                        let passed = if length.abs() > 1.0e-5 {
                            (s / length).clamp(0.0, 1.0)
                        } else {
                            1.0
                        };
                        let t = p.dot(side) - slid * passed;
                        if t.abs() >= reach {
                            continue;
                        }
                        // The contact is round at its ends: it presses less towards them.
                        let beyond = (start - s).max(s - end).max(0.0) / half_length;
                        if beyond >= 1.0 {
                            continue;
                        }
                        let weight = 1.0 - beyond * beyond;
                        let target = profile(t, press.half_width, depth, berm, berm_width) * weight;
                        let under = t.abs() < press.half_width;
                        if target.abs() < HEIGHT_UNIT && !under {
                            continue;
                        }
                        let cell = Self::cell_at(i, j).1;
                        let tile = held.get_or_insert_with(|| {
                            let kept = self.tiles.remove(&key);
                            existed = kept.is_some();
                            kept.unwrap_or_else(|| Tile::new(clock))
                        });
                        let was = tile.cell_bytes(cell);
                        'cell: {
                            let before = tile.height(cell);
                            let mut after = before;
                            if target < 0.0 {
                                after = before.min(target);
                            } else if target > 0.0 {
                                after = before.max((before + target).min(target));
                            }
                            if after != before {
                                tile.set_height(cell, after);
                            }
                            // What the tyre reaches takes its print and its wet: the floor
                            // where the tread stands on it, and the berm it heaped.
                            let touched = if under {
                                target <= before + 0.003
                            } else {
                                after != before
                            };
                            if !touched {
                                break 'cell;
                            }
                            let layers = &mut tile.layers;
                            layers[1][cell * 4 + 3] = 255;
                            if wet > layers[0][cell * 4 + 3] {
                                layers[0][cell * 4 + 3] = wet;
                                tile.damp = true;
                            }
                            tile.stays[cell] = tile.stays[cell].max(stays);
                            // Soil shows where the ground was broken: by how deep, a tyre's
                            // print in dust being soil too.
                            let broken =
                                churn * (0.35 + 0.65 * smoothstep(0.0, 0.03, depth * weight));
                            layers[1][cell * 4 + 2] = layers[1][cell * 4 + 2].max(byte(broken));
                            // The stretch this step newly rolled over takes the tread's print.
                            if under && beyond <= 0.0 {
                                let phase =
                                    press.phase.0 + (press.phase.1 - press.phase.0) * passed;
                                // Crisp under the tread, gone at its shoulders.
                                let crisp = print
                                    * (1.0 - smoothstep(0.8, 1.0, t.abs() / press.half_width));
                                layers[1][cell * 4] = byte(0.5 + 0.5 * crisp * phase.cos());
                                layers[1][cell * 4 + 1] = byte(0.5 + 0.5 * crisp * phase.sin());
                                layers[0][cell * 4 + 2] = byte(0.5 + 0.5 * t / ACROSS_REACH);
                            }
                        }
                        dirty |= tile.cell_bytes(cell) != was;
                    }
                }
                if let Some(mut tile) = held {
                    if dirty {
                        tile.version = tile.version.wrapping_add(1).max(1);
                        if !tile.listed {
                            tile.listed = true;
                            self.changed.push(key);
                        }
                    }
                    // A square made for a strip that changed nothing is not kept.
                    if dirty || existed {
                        self.tiles.insert(key, tile);
                    }
                }
            }
        }
    }

    /// Lays a lump of thrown mud where it landed: a blot `radius` metres across, as wet as
    /// `wet`, standing a little proud of the ground.
    pub fn splat(&mut self, at: Vec2, radius: f32, wet: f32) {
        // (A radius that is no number is no radius.)
        if !at.is_finite() || radius.is_nan() || radius <= 0.0 || radius > 1.0 {
            return;
        }
        let (i0, j0) = (
            ((at.x - radius) / CELL).floor() as i32,
            ((at.y - radius) / CELL).floor() as i32,
        );
        let (i1, j1) = (
            ((at.x + radius) / CELL).floor() as i32,
            ((at.y + radius) / CELL).floor() as i32,
        );
        for j in j0..=j1 {
            for i in i0..=i1 {
                let p = Vec2::new((i as f32 + 0.5) * CELL, (j as f32 + 0.5) * CELL);
                let near = 1.0 - (p.distance(at) / radius).min(1.0);
                if near <= 0.0 {
                    continue;
                }
                let (key, cell) = Self::cell_at(i, j);
                let tile = self.touch(key);
                let heaped = tile.height(cell) + 0.012 * near.sqrt() * radius / 0.08;
                // A lump lies on the ground: it does not fill a rut to its brim, nor pile
                // on other lumps without end.
                if tile.height(cell) < 0.02 {
                    tile.set_height(cell, heaped.min(0.03));
                }
                let layers = &mut tile.layers;
                layers[1][cell * 4 + 3] = 255;
                layers[1][cell * 4 + 2] = layers[1][cell * 4 + 2].max(byte(near * 3.0));
                let wetter = byte(wet * (near * 3.0).min(1.0));
                if wetter > layers[0][cell * 4 + 3] {
                    layers[0][cell * 4 + 3] = wetter;
                    tile.damp = true;
                }
                // What lands on a print covers it.
                let cover = 1.0 - (near * 3.0).min(1.0);
                for k in 0..2 {
                    let was = f32::from(layers[1][cell * 4 + k]) - 128.0;
                    layers[1][cell * 4 + k] = (128.0 + was * cover).round() as u8;
                }
            }
        }
    }

    /// Lets `seconds` pass: what was churned dries towards how wet it stays, a few squares
    /// at a time.
    pub fn advance(&mut self, seconds: f32) {
        if seconds.is_nan() || seconds <= 0.0 {
            return;
        }
        self.clock += seconds;
        if self.clock - self.dried_at < DRY_EVERY / DRY_AT_ONCE as f32 || self.tiles.is_empty() {
            return;
        }
        self.dried_at = self.clock;
        let clock = self.clock;
        // The squares due a drying, the longest left first.
        let mut due: Vec<((i32, i32), f32)> = self
            .tiles
            .iter()
            .filter(|(_, tile)| tile.damp && clock - tile.dried_at >= DRY_EVERY)
            .map(|(key, tile)| (*key, tile.dried_at))
            .collect();
        due.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)));
        self.drying_from = due.len();
        for (key, _) in due.into_iter().take(DRY_AT_ONCE) {
            let tile = self.touch(key);
            let kept = (-(clock - tile.dried_at) / DRY_SECONDS).exp();
            tile.dried_at = clock;
            let mut damp = false;
            for cell in 0..CELLS {
                let stays = f32::from(tile.stays[cell]);
                let wet = f32::from(tile.layers[0][cell * 4 + 3]);
                if wet > stays {
                    // Down to the nearest step, so that it gets there.
                    let dried = (stays + (wet - stays) * kept).floor().max(stays);
                    tile.layers[0][cell * 4 + 3] = dried as u8;
                    damp |= dried > stays;
                }
            }
            tile.damp = damp;
        }
    }

    /// How many squares were waiting to be dried when squares were last dried.
    pub fn drying(&self) -> usize {
        self.drying_from
    }

    /// The squares that have changed since this was last called, each once.
    pub fn take_changed(&mut self) -> Vec<(i32, i32)> {
        for key in &self.changed {
            if let Some(tile) = self.tiles.get_mut(key) {
                tile.listed = false;
            }
        }
        std::mem::take(&mut self.changed)
    }

    /// The square of ground a place is in.
    pub fn tile_at(x: f32, z: f32) -> (i32, i32) {
        (
            (x / TILE_METRES).floor() as i32,
            (z / TILE_METRES).floor() as i32,
        )
    }

    /// A count that changes whenever a square does: 0 for a square nothing has pressed.
    pub fn version(&self, tile: (i32, i32)) -> u32 {
        self.tiles.get(&tile).map_or(0, |tile| tile.version)
    }

    /// One layer of a square as the renderer samples it: [`TILE`] rows of [`TILE`] cells,
    /// four bytes each. `None` for a square nothing has pressed.
    pub fn layer(&self, tile: (i32, i32), layer: usize) -> Option<&[u8]> {
        self.tiles.get(&tile).map(|tile| &*tile.layers[layer])
    }

    /// One layer of a square at each smaller size, for the renderer to sample from far off:
    /// level 1 first, each half the last a side. Heights, wetness and prints are averaged,
    /// so a rut thins out with distance; whether anything is there is kept if it is true of
    /// any cell, so that a square with one rut in it is never taken for untouched ground.
    pub fn smaller(&self, tile: (i32, i32), layer: usize) -> Option<Vec<Vec<u8>>> {
        let mut last: Vec<u8> = self.layer(tile, layer)?.to_vec();
        let mut side = TILE;
        let mut levels = Vec::with_capacity(LEVELS - 1);
        for _ in 1..LEVELS {
            let half = side / 2;
            let mut next = vec![0_u8; half * half * 4];
            for row in 0..half {
                for column in 0..half {
                    let from = |dx: usize, dy: usize, byte: usize| {
                        u32::from(last[((row * 2 + dy) * side + column * 2 + dx) * 4 + byte])
                    };
                    let four = |byte: usize| {
                        from(0, 0, byte) + from(1, 0, byte) + from(0, 1, byte) + from(1, 1, byte)
                    };
                    let out = &mut next[(row * half + column) * 4..][..4];
                    if layer == 0 {
                        // The height's two bytes are one number.
                        let height = (four(0) * 256 + four(1) + 2) / 4;
                        out[0] = (height >> 8) as u8;
                        out[1] = (height & 0xff) as u8;
                        out[2] = ((four(2) + 2) / 4) as u8;
                        out[3] = ((four(3) + 2) / 4) as u8;
                    } else {
                        for (byte, out) in out.iter_mut().take(3).enumerate() {
                            *out = ((four(byte) + 2) / 4) as u8;
                        }
                        out[3] = (0..4).map(|k| from(k & 1, k >> 1, 3)).max().unwrap_or(0) as u8;
                    }
                }
            }
            levels.push(next.clone());
            last = next;
            side = half;
        }
        Some(levels)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tyre a quarter of a metre wide rolled along +x at z = 1, sunk `depth`.
    fn rolled(depth: f32, from: f32, to: f32) -> Press {
        Press {
            from: Vec2::new(from, 1.0),
            to: Vec2::new(to, 1.0),
            along: Vec2::X,
            half_width: 0.14,
            half_length: 0.12,
            depth,
            berm: 0.5,
            phase: (0.0, (to - from) / 0.08 * std::f32::consts::TAU),
            print: 1.0,
            wet: 0.9,
            stays: 0.5,
            churn: 1.0,
        }
    }

    #[test]
    fn ground_nobody_has_driven_on_is_as_it_was_made() {
        let ruts = Ruts::new();
        assert!(ruts.is_empty());
        assert_eq!(ruts.height(3.0, -7.0), 0.0);
        assert_eq!(ruts.slope(3.0, -7.0, 0.1), Vec2::ZERO);
        assert_eq!(ruts.version((0, 0)), 0);
        assert!(ruts.layer((0, 0), 0).is_none());
    }

    #[test]
    fn a_tyre_presses_a_rut_as_deep_as_it_sank() {
        let mut ruts = Ruts::new();
        ruts.press(&rolled(0.15, 0.0, 2.0));
        // Under the tread, along the strip: the depth it was pressed to.
        for x in [0.1, 0.7, 1.3, 1.9] {
            assert!((ruts.depth(x, 1.0) - 0.15).abs() < 0.002, "at {x}");
            assert!((ruts.depth(x, 1.06) - 0.15).abs() < 0.004, "off its middle");
        }
        // Well to the side, and well before and beyond it: untouched.
        assert_eq!(ruts.height(1.0, 2.0), 0.0);
        assert_eq!(ruts.height(-1.0, 1.0), 0.0);
        assert_eq!(ruts.height(3.0, 1.0), 0.0);
        assert!(!ruts.is_empty());
    }

    #[test]
    fn what_is_pressed_out_is_heaped_beside_the_rut() {
        let mut ruts = Ruts::new();
        ruts.press(&rolled(0.15, 0.0, 2.0));
        // Just outside each shoulder the ground stands above where it was.
        let highest = |side: f32| {
            (0..20)
                .map(|k| ruts.height(1.0, 1.0 + side * (0.12 + k as f32 * 0.01)))
                .fold(f32::MIN, f32::max)
        };
        for side in [-1.0, 1.0] {
            let berm = highest(side);
            assert!(berm > 0.03 && berm <= 0.12, "{berm}");
        }
        // A berm is less than the rut: what was pressed is mostly packed down. Across the
        // strip, more went down than came up.
        let across: f32 = (0..60)
            .map(|k| ruts.height(1.0, 0.4 + k as f32 * 0.02))
            .sum();
        assert!(across < 0.0, "{across}");
        // Firm ground that takes a shallow print heaps next to nothing.
        let mut firm = Ruts::new();
        firm.press(&Press {
            berm: 0.2,
            ..rolled(0.01, 0.0, 2.0)
        });
        let berm = (0..20)
            .map(|k| firm.height(1.0, 1.12 + k as f32 * 0.01))
            .fold(f32::MIN, f32::max);
        assert!(berm < 0.006, "{berm}");
    }

    #[test]
    fn a_rut_stays_and_only_a_deeper_wheel_deepens_it() {
        let mut ruts = Ruts::new();
        ruts.press(&rolled(0.15, 0.0, 2.0));
        let once: Vec<f32> = (0..50)
            .map(|k| ruts.height(1.0, 0.5 + k as f32 * 0.02))
            .collect();
        // A long time passes: the ground does not come back.
        for _ in 0..600 {
            ruts.advance(1.0);
        }
        let later: Vec<f32> = (0..50)
            .map(|k| ruts.height(1.0, 0.5 + k as f32 * 0.02))
            .collect();
        assert_eq!(once, later);
        // The same wheel again, and a lighter one: nothing changes.
        ruts.press(&rolled(0.15, 0.0, 2.0));
        ruts.press(&rolled(0.06, 0.0, 2.0));
        let again: Vec<f32> = (0..50)
            .map(|k| ruts.height(1.0, 0.5 + k as f32 * 0.02))
            .collect();
        assert_eq!(once, again);
        // A heavier one: deeper.
        ruts.press(&rolled(0.22, 0.0, 2.0));
        assert!((ruts.depth(1.0, 1.0) - 0.22).abs() < 0.002);
    }

    #[test]
    fn a_strip_pressed_in_steps_is_the_strip_pressed_at_once() {
        let mut whole = Ruts::new();
        whole.press(&rolled(0.12, 0.0, 2.0));
        let mut stepped = Ruts::new();
        for k in 0..20 {
            stepped.press(&rolled(0.12, k as f32 * 0.1, (k + 1) as f32 * 0.1));
        }
        for k in 0..40 {
            let x = 0.05 + k as f32 * 0.05;
            for z in [0.8, 0.9, 1.0, 1.1, 1.2] {
                let (a, b) = (whole.height(x, z), stepped.height(x, z));
                assert!((a - b).abs() < 0.002, "at {x} {z}: {a} and {b}");
            }
        }
    }

    #[test]
    fn a_wheel_crossing_a_rut_flattens_its_berm_and_half_fills_it() {
        let mut ruts = Ruts::new();
        ruts.press(&rolled(0.15, 0.0, 4.0));
        // Another, across it at x = 2.
        let crossing = Press {
            from: Vec2::new(2.0, -1.0),
            to: Vec2::new(2.0, 3.0),
            along: Vec2::Y,
            ..rolled(0.1, 0.0, 0.0)
        };
        ruts.press(&crossing);
        // Where the first's berm was, under the second's tread: pressed down.
        assert!(ruts.height(2.0, 1.2) < -0.09);
        // The first rut, beside the second's tread: some of the second's berm is in it,
        // and it is still a rut.
        let filled = ruts.height(2.2, 1.0);
        assert!(filled > -0.15 && filled < 0.0, "{filled}");
    }

    #[test]
    fn a_rolling_tyre_prints_its_tread_and_a_spinning_one_smears_it() {
        let mut ruts = Ruts::new();
        ruts.press(&rolled(0.05, 0.0, 2.0));
        // A lug every 8 cm: a quarter of a lug on, a quarter of a turn of the pattern.
        let (crisp, at) = ruts.print(0.82, 1.0);
        assert!(crisp > 0.9, "{crisp}");
        let turned = (at - 0.25 * std::f32::consts::TAU).rem_euclid(std::f32::consts::TAU);
        assert!(turned.min(std::f32::consts::TAU - turned) < 0.25, "{at}");
        // Beside the rut, nothing printed.
        assert_eq!(ruts.print(1.0, 2.0).0, 0.0);
        // The wheel behind it, spinning: the print is wiped.
        ruts.press(&Press {
            print: 0.0,
            ..rolled(0.05, 0.0, 2.0)
        });
        assert!(ruts.print(0.82, 1.0).0 < 0.05);
    }

    #[test]
    fn churned_ground_dries_to_how_wet_it_stays() {
        let mut ruts = Ruts::new();
        ruts.press(&rolled(0.1, 0.0, 2.0));
        assert!((ruts.wet(1.0, 1.0) - 0.9).abs() < 0.01);
        ruts.take_changed();
        let mut drying = Vec::new();
        for second in 0..900 {
            ruts.advance(1.0);
            if second % 60 == 59 {
                drying.push(ruts.wet(1.0, 1.0));
            }
        }
        assert!(
            drying.windows(2).all(|pair| pair[1] <= pair[0]),
            "{drying:?}"
        );
        assert!(
            drying[1] < 0.75 && drying[1] > 0.55,
            "after two minutes: {drying:?}"
        );
        assert!((drying[14] - 0.5).abs() < 0.02, "in the end: {drying:?}");
        // Whoever keeps a copy was told, and is told no more once it is dry.
        assert!(!ruts.take_changed().is_empty());
        ruts.advance(60.0);
        ruts.advance(60.0);
        assert!(ruts.take_changed().is_empty());
        // Ground wet from a tyre and nothing else dries right out.
        let mut tracked = Ruts::new();
        tracked.press(&Press {
            depth: 0.0,
            churn: 0.0,
            stays: 0.0,
            ..rolled(0.0, 0.0, 2.0)
        });
        assert!(tracked.wet(1.0, 1.0) > 0.8 && tracked.height(1.0, 1.0) == 0.0);
        for _ in 0..1200 {
            tracked.advance(1.0);
        }
        assert_eq!(tracked.wet(1.0, 1.0), 0.0);
    }

    /// A car standing still presses the same strip each step: once it has settled (the
    /// second press wets the tread's edge the first heaped), that changes no byte, so no
    /// square is counted changed and none is copied to the GPU again.
    #[test]
    fn pressing_what_is_already_pressed_changes_nothing() {
        let mut ruts = Ruts::new();
        let press = rolled(0.1, 0.0, 2.0);
        ruts.press(&press);
        ruts.press(&press);
        let changed = ruts.take_changed();
        assert!(!changed.is_empty());
        let versions: Vec<u32> = changed.iter().map(|&tile| ruts.version(tile)).collect();
        for _ in 0..3 {
            ruts.press(&press);
        }
        assert!(ruts.take_changed().is_empty());
        let after: Vec<u32> = changed.iter().map(|&tile| ruts.version(tile)).collect();
        assert_eq!(after, versions);
    }

    #[test]
    fn changed_squares_are_listed_once_and_counted() {
        let mut ruts = Ruts::new();
        ruts.press(&rolled(0.1, 0.0, 2.0));
        ruts.press(&rolled(0.12, 0.0, 2.0));
        let changed = ruts.take_changed();
        let tile = Ruts::tile_at(1.0, 1.0);
        assert_eq!(changed.iter().filter(|&&key| key == tile).count(), 1);
        let version = ruts.version(tile);
        assert!(version > 0);
        assert!(ruts.take_changed().is_empty());
        ruts.press(&rolled(0.2, 0.0, 2.0));
        assert!(ruts.version(tile) != version);
        assert_eq!(ruts.take_changed().len(), changed.len());
        // Cleared, each square is listed as gone.
        ruts.clear();
        assert!(ruts.is_empty() && ruts.take_changed().contains(&tile));
        assert_eq!(ruts.version(tile), 0);
    }

    #[test]
    fn a_strip_across_squares_is_whole() {
        // From one square into the next, along both axes, and where x and z are below zero.
        let mut ruts = Ruts::new();
        let press = Press {
            from: Vec2::new(-1.0, -1.0),
            to: Vec2::new(1.0, 1.0),
            along: Vec2::ONE.normalize(),
            ..rolled(0.1, 0.0, 0.0)
        };
        ruts.press(&press);
        for k in 0..=20 {
            let at = -0.9 + k as f32 * 0.09;
            assert!((ruts.depth(at, at) - 0.1).abs() < 0.003, "at {at}");
        }
        assert!(ruts.tiles() >= 3);
    }

    #[test]
    fn the_renderer_is_given_the_height_the_wheels_read() {
        let mut ruts = Ruts::new();
        ruts.press(&rolled(0.15, 0.0, 2.0));
        let tile = Ruts::tile_at(1.0, 1.0);
        let layer = ruts.layer(tile, 0).unwrap();
        assert_eq!(layer.len(), TILE * TILE * 4);
        let (i, j) = ((1.0 / CELL) as usize, (1.0 / CELL) as usize);
        let cell = (j % TILE * TILE + i % TILE) * 4;
        // As the shader puts the two bytes together.
        let height = (f32::from(layer[cell]) * 256.0 + f32::from(layer[cell + 1])
            - f32::from(HEIGHT_ZERO))
            * HEIGHT_UNIT;
        let read = ruts.height((i as f32 + 0.5) * CELL, (j as f32 + 0.5) * CELL);
        assert!((height - read).abs() < 1.0e-6 && (height + 0.15).abs() < 0.002);
        // Smaller, a square keeps its ruts, thinner, and never loses that it has any.
        let smaller = ruts.smaller(tile, 0).unwrap();
        let marks = ruts.smaller(tile, 1).unwrap();
        assert_eq!(smaller.len(), LEVELS - 1);
        for (level, texels) in smaller.iter().enumerate() {
            let side = TILE >> (level + 1);
            assert_eq!(texels.len(), side * side * 4);
        }
        let last = &marks[LEVELS - 2];
        assert!(last.chunks_exact(4).any(|cell| cell[3] == 255));
        let deepest = smaller[2]
            .chunks_exact(4)
            .map(|cell| u16::from_be_bytes([cell[0], cell[1]]))
            .min()
            .unwrap();
        assert!(deepest < HEIGHT_ZERO - 1000, "{deepest}");
    }

    #[test]
    fn a_lump_of_mud_lies_where_it_lands() {
        let mut ruts = Ruts::new();
        ruts.splat(Vec2::new(0.5, 0.5), 0.1, 1.0);
        assert!(ruts.height(0.5, 0.5) > 0.004 && ruts.height(0.5, 0.5) <= 0.03);
        assert!(ruts.wet(0.5, 0.5) > 0.9);
        assert_eq!(ruts.height(0.9, 0.5), 0.0);
        // More of them do not pile up without end.
        for _ in 0..50 {
            ruts.splat(Vec2::new(0.5, 0.5), 0.1, 1.0);
        }
        assert!(ruts.height(0.5, 0.5) <= 0.031);
        // Nonsense is left out.
        ruts.splat(Vec2::new(f32::NAN, 0.0), 0.1, 1.0);
        ruts.press(&Press {
            depth: f32::NAN,
            ..rolled(0.1, 0.0, 1.0)
        });
        ruts.press(&rolled(0.1, 0.0, 5000.0));
        assert!(ruts.tiles() <= 2);
    }
}
