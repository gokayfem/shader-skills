//! A crack network grown the way drying really cracks a surface.
//!
//! Cell patterns such as [`Cells`](crate::field::Cells) meet in Y-shaped junctions, but real shrinkage
//! cracks almost always meet in T-shapes: a surface cracks where it is most stressed, which is
//! furthest from any crack that has already relieved it, and a new crack grows until it runs into
//! an old one, turning to meet it square on. A scan of dried mud measured 81% T-shaped junctions and
//! 5% Y-shaped, with plates ranging over about 7x in width. Growing cracks in that order, generation
//! by generation, gives all of this without asking for any of it.

use crate::noise::{lattice_random, tiling_fbm};
use std::collections::VecDeque;

/// One pass of cracking.
#[derive(Clone, Copy, Debug)]
pub struct Generation {
    /// A new crack starts only where the nearest crack is at least this far away, in metres.
    pub spacing: f32,
    /// Crack width, in metres.
    pub width: f32,
    /// The chance per metre that a crack gives out before reaching another.
    pub give_up_per_metre: f32,
    /// The chance that a place that could crack does not, which leaves some plates larger.
    pub skip: f32,
}

/// What a [`CrackNetwork`] knows about a point.
#[derive(Clone, Copy, Debug)]
pub struct CrackSample {
    /// Distance to the centre line of the nearest crack, in metres.
    pub distance: f32,
    /// Width of that crack here, in metres.
    pub width: f32,
    /// A random number in `0..1` that is the same across the whole plate, or 0 inside a crack.
    pub plate: f32,
}

/// A tiling network of cracks, held as a grid of what is nearest to each cell.
pub struct CrackNetwork {
    size: usize,
    metres_per_cell: f32,
    distance: Vec<f32>,
    width: Vec<f32>,
    plate: Vec<f32>,
}

/// How far one arm of a crack can run, in multiples of its generation's spacing.
const MAX_LENGTH: f32 = 2.2;
/// Grid cells between steps of a growing crack.
const STEP: f32 = 1.5;
/// The coarse distance grid that steers growth has one cell per this many fine ones.
const STEERING_CELLS: usize = 4;

impl CrackNetwork {
    /// Grows a network over a square `tile_metres` wide, on a `size`x`size` grid.
    pub fn grow(size: usize, tile_metres: f32, seed: u32, generations: &[Generation]) -> Self {
        let metres_per_cell = tile_metres / size as f32;
        let mut sim = Simulation::new(size, metres_per_cell, seed);
        for (index, generation) in generations.iter().enumerate() {
            sim.run_generation(index as u32, generation);
        }
        sim.finish()
    }

    /// Looks up a point. `u` and `v` tile over `0..1`.
    pub fn sample(&self, u: f32, v: f32) -> CrackSample {
        let n = self.size as i32;
        let x = u.rem_euclid(1.0) * n as f32 - 0.5;
        let y = v.rem_euclid(1.0) * n as f32 - 0.5;
        let (fx, fy) = (x.floor(), y.floor());
        let (tx, ty) = (x - fx, y - fy);
        let (x0, y0) = (fx as i32, fy as i32);
        let at =
            |dx: i32, dy: i32| ((y0 + dy).rem_euclid(n) * n + (x0 + dx).rem_euclid(n)) as usize;
        let (a, b, c, d) = (at(0, 0), at(1, 0), at(0, 1), at(1, 1));
        let lerp = |grid: &[f32]| {
            let top = grid[a] + (grid[b] - grid[a]) * tx;
            let bottom = grid[c] + (grid[d] - grid[c]) * tx;
            top + (bottom - top) * ty
        };
        let nearest = at(i32::from(tx >= 0.5), i32::from(ty >= 0.5));
        CrackSample {
            distance: lerp(&self.distance),
            width: lerp(&self.width),
            plate: self.plate[nearest],
        }
    }

    pub fn metres_per_cell(&self) -> f32 {
        self.metres_per_cell
    }
}

struct Simulation {
    size: usize,
    metres_per_cell: f32,
    seed: u32,
    /// The crack that claimed each cell, and its width. 0 means no crack yet.
    owner: Vec<u32>,
    widths: Vec<f32>,
    /// Which step of its crack claimed each cell, so that a crack does not stop against itself.
    claimed_at: Vec<u32>,
    next_owner: u32,
    random_calls: u32,
}

impl Simulation {
    fn new(size: usize, metres_per_cell: f32, seed: u32) -> Self {
        Self {
            size,
            metres_per_cell,
            seed,
            owner: vec![0; size * size],
            widths: vec![0.0; size * size],
            claimed_at: vec![0; size * size],
            next_owner: 1,
            random_calls: 0,
        }
    }

    fn random(&mut self) -> f32 {
        self.random_calls = self.random_calls.wrapping_add(1);
        lattice_random(
            (self.random_calls & 0xffff) as i32,
            (self.random_calls >> 16) as i32,
            self.seed,
            7,
        )
    }

    fn cell(&self, x: f32, y: f32) -> usize {
        let n = self.size as i32;
        (y as i32).rem_euclid(n) as usize * self.size + (x as i32).rem_euclid(n) as usize
    }

    /// Distance from each coarse cell to the nearest crack, in metres.
    fn steering_field(&self) -> SteeringField {
        let coarse = self.size / STEERING_CELLS;
        let mut seeds = vec![false; coarse * coarse];
        for y in 0..self.size {
            for x in 0..self.size {
                if self.owner[y * self.size + x] != 0 {
                    seeds[(y / STEERING_CELLS) * coarse + x / STEERING_CELLS] = true;
                }
            }
        }
        let metres_per_cell = self.metres_per_cell * STEERING_CELLS as f32;
        let nearest = nearest_seed(&seeds, coarse);
        let distance = nearest
            .iter()
            .map(|n| n.distance() * metres_per_cell)
            .collect();
        let away = nearest
            .iter()
            .map(|n| {
                if n.found {
                    (f32::from(n.dx), f32::from(n.dy))
                } else {
                    (0.0, 0.0)
                }
            })
            .collect();
        SteeringField {
            size: coarse,
            distance,
            away,
        }
    }

    fn run_generation(&mut self, index: u32, generation: &Generation) {
        let field = self.steering_field();
        let radius = (generation.spacing / (self.metres_per_cell * STEERING_CELLS as f32))
            .ceil()
            .max(1.0) as i32;
        let n = field.size as i32;
        // Starts are the most relieved places: local peaks of distance from any crack.
        let mut starts: Vec<(f32, usize, usize)> = Vec::new();
        let stride = (radius / 3).max(1) as usize;
        for y in (0..field.size).step_by(stride) {
            for x in (0..field.size).step_by(stride) {
                let here = field.distance[y * field.size + x];
                if here >= generation.spacing {
                    // Shuffled among near-equals: an empty surface would otherwise crack in rows.
                    starts.push((here * (1.0 + self.random() * 0.3), x, y));
                }
            }
        }
        if index == 0 && starts.is_empty() {
            starts.push((f32::MAX, 0, 0));
        }
        starts.sort_by(|a, b| b.0.total_cmp(&a.0));
        // Each start reserves the area around it, so that one generation spreads out.
        let mut reserved = vec![false; field.size * field.size];
        for (_, x, y) in starts {
            if reserved[y * field.size + x] {
                continue;
            }
            for dy in -radius..=radius {
                for dx in -radius..=radius {
                    if dx * dx + dy * dy <= radius * radius {
                        let cell = (y as i32 + dy).rem_euclid(n) as usize * field.size
                            + (x as i32 + dx).rem_euclid(n) as usize;
                        reserved[cell] = true;
                    }
                }
            }
            // Some areas dried differently, which clusters large plates and small ones together.
            let place = tiling_fbm(
                x as f32 / field.size as f32,
                y as f32 / field.size as f32,
                5,
                3,
                self.seed + 40,
            ) * 0.5
                + 0.5;
            if self.random() < generation.skip * (place * 2.0) {
                continue;
            }
            let scale = STEERING_CELLS as f32;
            let (jx, jy) = (self.random() * scale, self.random() * scale);
            self.grow_crack(
                x as f32 * scale + jx,
                y as f32 * scale + jy,
                &field,
                generation,
            );
        }
    }

    /// Grows a crack both ways from a point, each arm running until it meets another crack.
    fn grow_crack(&mut self, x: f32, y: f32, field: &SteeringField, generation: &Generation) {
        let owner = self.next_owner;
        self.next_owner += 1;
        let scale = STEERING_CELLS as f32;
        let (gx, gy) = field.gradient(x / scale, y / scale);
        let mut heading = if gx == 0.0 && gy == 0.0 {
            self.random() * std::f32::consts::TAU
        } else {
            gy.atan2(gx) + std::f32::consts::PI
        };
        heading += (self.random() - 0.5) * 0.35;

        self.grow_arm(x, y, heading, owner, field, generation);
        self.grow_arm(
            x,
            y,
            heading + std::f32::consts::PI,
            owner,
            field,
            generation,
        );
    }

    fn grow_arm(
        &mut self,
        start_x: f32,
        start_y: f32,
        start_heading: f32,
        owner: u32,
        field: &SteeringField,
        generation: &Generation,
    ) {
        /// Cracks sweep in slow curves, which is what rounds the plates between them.
        const CURVE_CHANGE: f32 = 0.0005;
        const LARGEST_CURVE: f32 = 0.0032;
        /// Small jitter, and now and then a sharper break.
        const WANDER: f32 = 0.025;
        const KINK_CHANCE: f32 = 0.004;
        const KINK: f32 = 0.5;
        /// How close, in cells, before a crack is caught by the one it is approaching.
        const CAPTURE: f32 = 1.5;
        let (mut x, mut y, mut heading) = (start_x, start_y, start_heading);
        let step_metres = STEP * self.metres_per_cell;
        // A crack relieves only so much around it, so it does not run on for ever. Cracks that stop
        // short leave gaps for later generations to close, which keeps plates compact.
        let longest = (generation.spacing * MAX_LENGTH / step_metres) as u32;
        let give_up = generation.give_up_per_metre * step_metres;
        let scale = STEERING_CELLS as f32;
        let width_wobble = self.random() * 100.0;
        let mut curve = (self.random() - 0.5) * LARGEST_CURVE;

        let mut step = 0u32;
        while step < longest {
            step += 1;
            // A crack bends to meet what is ahead of it at a right angle.
            let (gx, gy) = field.gradient(x / scale, y / scale);
            let length = (gx * gx + gy * gy).sqrt();
            if length > 1.0e-5 {
                let target = (-gy).atan2(-gx);
                let near = field.at(x / scale, y / scale);
                let pull = (1.0 - near / generation.spacing.max(1.0e-3)).clamp(0.0, 1.0) * 0.32;
                heading += wrap_angle(target - heading) * pull;
            }
            curve = (curve + (self.random() - 0.5) * CURVE_CHANGE * 2.0)
                .clamp(-LARGEST_CURVE, LARGEST_CURVE);
            heading += curve + (self.random() - 0.5) * WANDER * 2.0;

            if self.random() < KINK_CHANCE {
                heading += (self.random() - 0.5) * 2.0 * KINK;
            }
            let (dx, dy) = (heading.cos(), heading.sin());
            let (nx, ny) = (x + dx * STEP, y + dy * STEP);
            // Meeting another crack ends this one, which is what makes a T-junction.
            let ahead = self.cell(nx + dx * CAPTURE, ny + dy * CAPTURE);
            let met = self.owner[ahead] != 0
                && (self.owner[ahead] != owner || self.claimed_at[ahead] + 12 < step);
            let width = generation.width
                * (0.72 + 0.56 * lattice_random((step / 14) as i32, 0, width_wobble as u32, 3));
            // Claiming the point between steps too leaves no gaps along the crack.
            self.claim(x + dx * STEP * 0.5, y + dy * STEP * 0.5, owner, width, step);
            self.claim(nx, ny, owner, width, step);

            x = nx;
            y = ny;
            if met {
                break;
            }
            if self.random() < give_up {
                break;
            }
        }
    }

    fn claim(&mut self, x: f32, y: f32, owner: u32, width: f32, step: u32) {
        let cell = self.cell(x, y);
        if self.owner[cell] == 0 {
            self.owner[cell] = owner;
            self.widths[cell] = width;
            self.claimed_at[cell] = step;
        }
    }

    fn finish(self) -> CrackNetwork {
        let n = self.size;
        let seeds: Vec<bool> = self.owner.iter().map(|&o| o != 0).collect();
        let nearest = nearest_seed(&seeds, n);
        let mut distance = vec![0.0; n * n];
        let mut width = vec![0.0; n * n];
        for i in 0..n * n {
            distance[i] = nearest[i].distance() * self.metres_per_cell;
            let source = nearest[i].y as usize * n + nearest[i].x as usize;
            width[i] = self.widths[source];
        }
        let plate = label_plates(&distance, &width, n, self.seed);
        CrackNetwork {
            size: n,
            metres_per_cell: self.metres_per_cell,
            distance,
            width,
            plate,
        }
    }
}

fn wrap_angle(angle: f32) -> f32 {
    let tau = std::f32::consts::TAU;
    (angle + std::f32::consts::PI).rem_euclid(tau) - std::f32::consts::PI
}

struct SteeringField {
    size: usize,
    distance: Vec<f32>,
    /// The direction from the nearest crack to each cell, or zero if there are no cracks yet.
    away: Vec<(f32, f32)>,
}

impl SteeringField {
    fn at(&self, x: f32, y: f32) -> f32 {
        let n = self.size as i32;
        self.distance
            [(y as i32).rem_euclid(n) as usize * self.size + (x as i32).rem_euclid(n) as usize]
    }

    /// Points away from the nearest crack. Read directly, not from differences of distance, which
    /// vanish at exactly the places new cracks start, the points furthest from any crack.
    fn gradient(&self, x: f32, y: f32) -> (f32, f32) {
        let n = self.size as i32;
        self.away[(y as i32).rem_euclid(n) as usize * self.size + (x as i32).rem_euclid(n) as usize]
    }
}

/// The seed nearest a cell, as an offset that wraps around the tile.
#[derive(Clone, Copy)]
struct Nearest {
    x: i16,
    y: i16,
    dx: i16,
    dy: i16,
    found: bool,
}

impl Nearest {
    fn distance(&self) -> f32 {
        if self.found {
            f32::from(self.dx).hypot(f32::from(self.dy))
        } else {
            // An empty grid has nothing near anywhere, which reads as far from any crack.
            f32::from(i16::MAX)
        }
    }

    fn squared(&self) -> i64 {
        if self.found {
            i64::from(self.dx).pow(2) + i64::from(self.dy).pow(2)
        } else {
            i64::MAX
        }
    }
}

/// Finds the nearest seed to every cell of a tiling grid, by sweeping known answers across it.
fn nearest_seed(seeds: &[bool], n: usize) -> Vec<Nearest> {
    let mut grid: Vec<Nearest> = seeds
        .iter()
        .enumerate()
        .map(|(i, &seed)| Nearest {
            x: (i % n) as i16,
            y: (i / n) as i16,
            dx: 0,
            dy: 0,
            found: seed,
        })
        .collect();
    let at = |x: isize, y: isize| {
        y.rem_euclid(n as isize) as usize * n + x.rem_euclid(n as isize) as usize
    };
    let consider = |grid: &mut Vec<Nearest>, x: isize, y: isize, ox: isize, oy: isize| {
        let other = grid[at(x + ox, y + oy)];
        if !other.found {
            return;
        }
        let candidate = Nearest {
            x: other.x,
            y: other.y,
            dx: other.dx - ox as i16,
            dy: other.dy - oy as i16,
            found: true,
        };
        let here = at(x, y);
        if candidate.squared() < grid[here].squared() {
            grid[here] = candidate;
        }
    };
    // Twice round, so that answers cross the tile's border and reach every cell.
    for _ in 0..2 {
        for y in 0..n as isize {
            for x in 0..n as isize {
                for (ox, oy) in [(-1, 0), (0, -1), (-1, -1), (1, -1)] {
                    consider(&mut grid, x, y, ox, oy);
                }
            }
            for x in (0..n as isize).rev() {
                consider(&mut grid, x, y, 1, 0);
            }
        }
        for y in (0..n as isize).rev() {
            for x in (0..n as isize).rev() {
                for (ox, oy) in [(1, 0), (0, 1), (1, 1), (-1, 1)] {
                    consider(&mut grid, x, y, ox, oy);
                }
            }
            for x in 0..n as isize {
                consider(&mut grid, x, y, -1, 0);
            }
        }
    }
    grid
}

/// Gives each region enclosed by cracks its own random number.
fn label_plates(distance: &[f32], width: &[f32], n: usize, seed: u32) -> Vec<f32> {
    let open = |i: usize| distance[i] > width[i] * 0.5;
    let mut plate = vec![0.0; n * n];
    let mut seen = vec![false; n * n];
    let mut queue = VecDeque::new();
    let mut count = 0i32;
    for start in 0..n * n {
        if seen[start] || !open(start) {
            continue;
        }
        count += 1;
        let value = lattice_random(count, 0, seed, 11).max(1.0e-3);
        seen[start] = true;
        queue.push_back(start);
        while let Some(i) = queue.pop_front() {
            plate[i] = value;
            let (x, y) = (i % n, i / n);
            for next in [
                y * n + (x + 1) % n,
                y * n + (x + n - 1) % n,
                ((y + 1) % n) * n + x,
                ((y + n - 1) % n) * n + x,
            ] {
                if !seen[next] && open(next) {
                    seen[next] = true;
                    queue.push_back(next);
                }
            }
        }
    }
    plate
}

#[cfg(test)]
mod tests {
    use super::*;

    fn generations() -> Vec<Generation> {
        [0.4_f32, 0.25, 0.16, 0.1, 0.06]
            .iter()
            .map(|&spacing| Generation {
                spacing,
                width: 0.01,
                give_up_per_metre: 0.3,
                skip: 0.1,
            })
            .collect()
    }

    fn network() -> CrackNetwork {
        CrackNetwork::grow(512, 2.0, 3, &generations())
    }

    #[test]
    fn growing_is_repeatable_and_the_seed_matters() {
        let (a, b) = (network(), network());
        assert_eq!(a.distance, b.distance);
        let c = CrackNetwork::grow(512, 2.0, 4, &generations());
        assert_ne!(a.distance, c.distance);
    }

    #[test]
    fn the_network_tiles() {
        let network = network();
        for i in 0..200 {
            let t = i as f32 / 200.0;
            for (a, b) in [
                (network.sample(0.0, t), network.sample(1.0, t)),
                (network.sample(t, 0.0), network.sample(t, 1.0)),
            ] {
                assert!((a.distance - b.distance).abs() < 1.0e-4);
                assert_eq!(a.plate, b.plate);
            }
        }
    }

    #[test]
    fn cracks_divide_the_surface_into_many_plates_of_varied_size() {
        let network = network();
        let mut sizes = std::collections::HashMap::new();
        for &plate in &network.plate {
            if plate > 0.0 {
                *sizes.entry(plate.to_bits()).or_insert(0u32) += 1;
            }
        }
        assert!(sizes.len() > 40, "only {} plates", sizes.len());
        let mut areas: Vec<u32> = sizes.into_values().filter(|&a| a > 30).collect();
        areas.sort_unstable();
        let spread = areas[areas.len() * 9 / 10] as f32 / areas[areas.len() / 10] as f32;
        assert!(spread > 3.0, "plate areas vary only {spread}x");
    }

    #[test]
    fn nearest_seed_wraps_around_the_tile() {
        let n = 16;
        let mut seeds = vec![false; n * n];
        seeds[0] = true;
        let nearest = nearest_seed(&seeds, n);
        assert_eq!(nearest[n * n - 1].distance(), 2.0_f32.sqrt());
        assert_eq!(nearest[n - 1].distance(), 1.0);
        assert_eq!(nearest[n * 8 + 8].distance(), 128.0_f32.sqrt());
    }

    #[test]
    fn an_empty_grid_reads_as_far_from_any_crack() {
        let nearest = nearest_seed(&[false; 16], 4);
        assert!(nearest[5].distance() > 1000.0);
    }
}
