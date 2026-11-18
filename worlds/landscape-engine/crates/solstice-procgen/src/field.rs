//! Values across a square that tile seamlessly, and ways to combine them.
//!
//! A [`Field`] gives a number at each `(u, v)`. Every field here repeats over `0..1` in both axes,
//! and every combinator keeps that true, so anything built from them tiles without further thought.
//!
//! ```
//! use solstice_procgen::field::{fbm, Field};
//!
//! let warp = fbm(4, 4, 1);
//! let patches = fbm(6, 5, 2).warped(warp, 0.05).unit().smooth(0.4, 0.6);
//! assert!((0.0..=1.0).contains(&patches.at(0.25, 0.75)));
//! ```

use crate::noise::{lattice_random, tiling_billow, tiling_fbm, tiling_fbm_xy, tiling_ridged};
use glam::Vec2;

/// A seamlessly tiling value at every point of a square.
pub trait Field: Send + Sync {
    fn at(&self, u: f32, v: f32) -> f32;

    /// Passes each value through `f`.
    fn map(self, f: impl Fn(f32) -> f32 + Send + Sync) -> impl Field
    where
        Self: Sized,
    {
        move |u: f32, v: f32| f(self.at(u, v))
    }

    /// Maps noise in `-1..1` to `0..1`, clamped.
    fn unit(self) -> impl Field
    where
        Self: Sized,
    {
        self.map(|x| (x * 0.5 + 0.5).clamp(0.0, 1.0))
    }

    /// Rises smoothly from 0 at `low` to 1 at `high`.
    fn smooth(self, low: f32, high: f32) -> impl Field
    where
        Self: Sized,
    {
        self.map(move |x| smoothstep(low, high, x))
    }

    fn plus(self, other: impl Field) -> impl Field
    where
        Self: Sized,
    {
        move |u: f32, v: f32| self.at(u, v) + other.at(u, v)
    }

    fn times(self, other: impl Field) -> impl Field
    where
        Self: Sized,
    {
        move |u: f32, v: f32| self.at(u, v) * other.at(u, v)
    }

    /// Displaces where this field is read by `warp`, breaking up straight features. The field
    /// reads the same displacement along both axes, offset so that they differ.
    fn warped(self, warp: impl Field, amount: f32) -> impl Field
    where
        Self: Sized,
    {
        move |u: f32, v: f32| {
            let offset = Vec2::new(warp.at(u, v), warp.at(v + 0.37, u + 0.61)) * amount;
            self.at(u + offset.x, v + offset.y)
        }
    }

    /// Repeats this field a whole number of times along each axis, which stretches it when the
    /// counts differ.
    fn repeated(self, across: u32, down: u32) -> impl Field
    where
        Self: Sized,
    {
        move |u: f32, v: f32| self.at(u * across as f32, v * down as f32)
    }
}

impl<F: Fn(f32, f32) -> f32 + Send + Sync> Field for F {
    fn at(&self, u: f32, v: f32) -> f32 {
        self(u, v)
    }
}

/// Soft fractal noise in roughly `-1..1`, with `period` features across the square at its coarsest.
pub fn fbm(period: i32, octaves: u32, seed: u32) -> impl Field + Copy {
    move |u: f32, v: f32| tiling_fbm(u, v, period, octaves, seed)
}

/// Soft fractal noise in roughly `-1..1`, with `across` features over the square in X and
/// `down` in Y at its coarsest, so its features are drawn out along the axis with fewer.
pub fn fbm_xy(across: i32, down: i32, octaves: u32, seed: u32) -> impl Field + Copy {
    move |u: f32, v: f32| tiling_fbm_xy(u, v, across, down, octaves, seed)
}

/// Fractal noise in `0..1` with sharp creases, like eroded stone.
pub fn ridged(period: i32, octaves: u32, seed: u32) -> impl Field + Copy {
    move |u: f32, v: f32| tiling_ridged(u, v, period, octaves, seed)
}

/// Fractal noise in `0..1` with puffy lumps and sharp creases between them, like soil.
pub fn billow(period: i32, octaves: u32, seed: u32) -> impl Field + Copy {
    move |u: f32, v: f32| tiling_billow(u, v, period, octaves, seed)
}

/// Smooth waves across `v`, with `count` crests over the square, in `0..1`.
pub fn waves(count: u32) -> impl Field {
    move |_: f32, v: f32| (v * count as f32 * std::f32::consts::TAU).sin() * 0.5 + 0.5
}

pub fn smoothstep(low: f32, high: f32, x: f32) -> f32 {
    let t = ((x - low) / (high - low)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// What [`Cells`] finds at a point.
#[derive(Clone, Copy, Debug)]
pub struct Cell {
    /// Distance to the nearest cell's centre, in cell widths.
    pub nearest: f32,
    /// Distance to the second nearest.
    pub second: f32,
    /// A random number in `0..1` that is the same across the whole of a cell.
    pub id: f32,
}

impl Cell {
    /// How far from the border with the next cell, which is 0 on the border itself.
    pub fn border(&self) -> f32 {
        self.second - self.nearest
    }
}

/// Irregular cells: pebbles, cracked plates, crystals.
#[derive(Clone, Copy, Debug)]
pub struct Cells {
    pub period: i32,
    pub seed: u32,
}

impl Cells {
    pub fn sample(&self, u: f32, v: f32) -> Cell {
        let p = Vec2::new(u, v) * self.period as f32;
        let base = p.floor();
        let mut cell = Cell {
            nearest: f32::MAX,
            second: f32::MAX,
            id: 0.0,
        };
        for dy in -2..=2 {
            for dx in -2..=2 {
                let (cx, cy) = (base.x as i32 + dx, base.y as i32 + dy);
                let (wx, wy) = (cx.rem_euclid(self.period), cy.rem_euclid(self.period));
                let centre = Vec2::new(
                    cx as f32 + lattice_random(wx, wy, self.seed, 0),
                    cy as f32 + lattice_random(wx, wy, self.seed, 1),
                );
                let distance = centre.distance(p);
                if distance < cell.nearest {
                    cell.second = cell.nearest;
                    cell.nearest = distance;
                    cell.id = lattice_random(wx, wy, self.seed, 2);
                } else if distance < cell.second {
                    cell.second = distance;
                }
            }
        }
        cell
    }
}

/// What [`Stones`] finds at a point.
#[derive(Clone, Copy, Debug, Default)]
pub struct Stone {
    /// Height of the topmost stone's surface here, in metres, or 0 where none covers this point.
    pub height: f32,
    /// A random number in `0..1` that is the same across the whole stone.
    pub id: f32,
}

/// Rounded stones lying on the ground, overlapping as real ones do. Each is a squashed, rotated
/// ellipsoid, and the highest one at a point is what shows.
#[derive(Clone, Copy, Debug)]
pub struct Stones {
    /// Stones per side of the square.
    pub period: i32,
    pub seed: u32,
    /// Longest radius of a stone, in metres.
    pub radius: f32,
    /// The size of the square, in metres.
    pub tile_metres: f32,
    /// The share of cells that hold a stone, in `0..=1`.
    pub coverage: f32,
}

impl Stones {
    pub fn sample(&self, u: f32, v: f32) -> Stone {
        let cell_metres = self.tile_metres / self.period as f32;
        let p = Vec2::new(u, v) * self.period as f32;
        let base = p.floor();
        let reach = (self.radius / cell_metres).ceil() as i32 + 1;
        let mut top = Stone::default();
        for dy in -reach..=reach {
            for dx in -reach..=reach {
                let (cx, cy) = (base.x as i32 + dx, base.y as i32 + dy);
                let (wx, wy) = (cx.rem_euclid(self.period), cy.rem_euclid(self.period));
                let random = |channel: u32| lattice_random(wx, wy, self.seed, channel);
                if random(8) >= self.coverage {
                    continue;
                }
                let centre = Vec2::new(cx as f32 + random(0), cy as f32 + random(1));
                let size = self.radius * (0.5 + random(2) * 0.5);
                // Stones are longer than they are wide, and flatter than either.
                let (long, wide, tall) = (
                    size,
                    size * (0.6 + random(3) * 0.35),
                    size * (0.35 + random(4) * 0.3),
                );
                let axis = Vec2::from_angle(random(5) * std::f32::consts::TAU);
                let offset = (p - centre) * cell_metres;
                let along = offset.dot(axis) / long;
                let across = offset.perp_dot(axis) / wide;
                let inside = 1.0 - along * along - across * across;
                if inside > 0.0 {
                    let height = tall * inside.sqrt() + random(6) * size * 0.25;
                    if height > top.height {
                        top = Stone {
                            height,
                            id: random(7),
                        };
                    }
                }
            }
        }
        top
    }
}

/// What [`Facets`] finds at a point.
#[derive(Clone, Copy, Debug, Default)]
pub struct Facet {
    /// Height of the surface, in metres.
    pub height: f32,
    /// A random number in `0..1` that is the same across the whole block.
    pub id: f32,
}

/// Broken, angular blocks with flat faces and sharp creases, like fractured rock.
///
/// Each cell holds a many-sided pyramid with randomly turned faces, and the highest one at a point
/// is what shows. Flat faces come from the pyramid's sides, and creases from where blocks meet.
#[derive(Clone, Copy, Debug)]
pub struct Facets {
    /// Blocks per side of the square.
    pub period: i32,
    pub seed: u32,
    /// Slope of the faces, as rise over run.
    pub slope: f32,
    /// The size of the square, in metres.
    pub tile_metres: f32,
}

impl Facets {
    const FACES: u32 = 5;

    pub fn sample(&self, u: f32, v: f32) -> Facet {
        let cell_metres = self.tile_metres / self.period as f32;
        let p = Vec2::new(u, v) * self.period as f32;
        let base = p.floor();
        let mut top = Facet {
            height: f32::MIN,
            id: 0.0,
        };
        for dy in -2..=2 {
            for dx in -2..=2 {
                let (cx, cy) = (base.x as i32 + dx, base.y as i32 + dy);
                let (wx, wy) = (cx.rem_euclid(self.period), cy.rem_euclid(self.period));
                let random = |channel: u32| lattice_random(wx, wy, self.seed, channel);
                let offset =
                    (p - Vec2::new(cx as f32 + random(0), cy as f32 + random(1))) * cell_metres;
                // The farthest-reaching face decides how far down the pyramid this point is.
                let mut reach = f32::MIN;
                let turn = random(1) * std::f32::consts::TAU;
                for face in 0..Self::FACES {
                    let angle = turn
                        + (face as f32 + random(3 + face) * 0.7) / Self::FACES as f32
                            * std::f32::consts::TAU;
                    reach = reach.max(offset.dot(Vec2::from_angle(angle)));
                }
                let summit = random(2) * cell_metres * self.slope * 0.8;
                let height = summit - reach * self.slope;
                if height > top.height {
                    top = Facet {
                        height,
                        id: random(9),
                    };
                }
            }
        }
        top
    }
}

/// What [`Blades`] finds at a point.
#[derive(Clone, Copy, Debug, Default)]
pub struct Blade {
    /// Height of the topmost blade here, in metres, or 0 where none covers this point.
    pub height: f32,
    /// A random number in `0..1` that is the same along the whole blade.
    pub id: f32,
    /// How far along the blade this point is: 0 at its root and 1 at its tip.
    pub along: f32,
}

/// Blades of grass lying over each other, leaning together in the direction `flow` gives.
///
/// Each rises from its root towards its tip, so tips lie on top of the roots of their neighbours.
/// Leaning together, in drifts that change across the square, is what makes grass look combed by
/// wind and trodden by feet, where random directions look like scattered straw.
#[derive(Clone, Copy, Debug)]
pub struct Blades<F> {
    /// Cells per side of the square.
    pub period: i32,
    pub seed: u32,
    /// Blades started per cell.
    pub density: u32,
    /// Length of a blade, in metres.
    pub length: f32,
    /// Width at a blade's base, in metres.
    pub width: f32,
    /// How high a blade's tip stands above its root, in metres.
    pub rise: f32,
    /// Direction blades lean, as a fraction of a full turn at each point of the square.
    pub flow: F,
    /// How far a blade may turn from the flow, as a fraction of a full turn.
    pub spread: f32,
    /// The size of the square, in metres.
    pub tile_metres: f32,
}

impl<F: Field> Blades<F> {
    pub fn sample(&self, u: f32, v: f32) -> Blade {
        let cell_metres = self.tile_metres / self.period as f32;
        let p = Vec2::new(u, v) * self.period as f32;
        let base = p.floor();
        let reach = (self.length / cell_metres).ceil() as i32 + 1;
        let mut top = Blade::default();
        for dy in -reach..=reach {
            for dx in -reach..=reach {
                let (cx, cy) = (base.x as i32 + dx, base.y as i32 + dy);
                let (wx, wy) = (cx.rem_euclid(self.period), cy.rem_euclid(self.period));
                let flow = self.flow.at(
                    wx as f32 / self.period as f32,
                    wy as f32 / self.period as f32,
                );
                for blade in 0..self.density {
                    let channel = blade * 6;
                    let random = |offset: u32| lattice_random(wx, wy, self.seed, channel + offset);
                    let root = Vec2::new(cx as f32 + random(0), cy as f32 + random(1));
                    let turn = flow + (random(2) - 0.5) * self.spread * 2.0;
                    let direction = Vec2::from_angle(turn * std::f32::consts::TAU);
                    let length = self.length * (0.65 + random(3) * 0.35);
                    let offset = (p - root) * cell_metres;
                    let along = offset.dot(direction) / length;
                    if !(0.0..1.0).contains(&along) {
                        continue;
                    }
                    // Blades bend over slightly, and narrow to a point at the tip.
                    let bend = (random(4) - 0.5) * length * 0.3 * along * along;
                    let across = offset.perp_dot(direction) - bend;
                    let width = self.width * (1.0 - along * along).sqrt();
                    let side = across.abs() / width;
                    if side < 1.0 {
                        let rounded = (1.0 - side * side).sqrt() * self.width * 0.25;
                        let height = along * self.rise * (0.6 + random(5) * 0.4)
                            + random(5) * self.rise * 0.45
                            + rounded;
                        if height > top.height {
                            top = Blade {
                                height,
                                id: random(5),
                                along,
                            };
                        }
                    }
                }
            }
        }
        top
    }
}

/// What [`Leaves`] finds at a point.
#[derive(Clone, Copy, Debug, Default)]
pub struct Leaf {
    /// Height of the topmost leaf's surface here, in metres, or 0 where none covers this point.
    pub height: f32,
    /// A random number in `0..1` that is the same across the whole leaf.
    pub id: f32,
    /// How strongly a vein or the midrib shows here, in `0..=1`.
    pub vein: f32,
    /// A second random number for the whole leaf, for how decayed it is.
    pub age: f32,
}

/// Fallen leaves lying in a heap: pointed outlines with midribs and side veins, curled at the edges.
#[derive(Clone, Copy, Debug)]
pub struct Leaves {
    /// Cells per side of the square.
    pub period: i32,
    pub seed: u32,
    /// Leaves per cell.
    pub density: u32,
    /// Length of a leaf, in metres.
    pub length: f32,
    /// How deep the heap is, in metres.
    pub depth: f32,
    /// The size of the square, in metres.
    pub tile_metres: f32,
}

impl Leaves {
    pub fn sample(&self, u: f32, v: f32) -> Leaf {
        let cell_metres = self.tile_metres / self.period as f32;
        let p = Vec2::new(u, v) * self.period as f32;
        let base = p.floor();
        let reach = (self.length / cell_metres).ceil() as i32;
        let mut top = Leaf::default();
        for dy in -reach..=reach {
            for dx in -reach..=reach {
                let (cx, cy) = (base.x as i32 + dx, base.y as i32 + dy);
                let (wx, wy) = (cx.rem_euclid(self.period), cy.rem_euclid(self.period));
                for leaf in 0..self.density {
                    let channel = leaf * 8;
                    let random = |offset: u32| lattice_random(wx, wy, self.seed, channel + offset);
                    let centre = Vec2::new(cx as f32 + random(0), cy as f32 + random(1));
                    let axis = Vec2::from_angle(random(2) * std::f32::consts::TAU);
                    let length = self.length * (0.45 + random(3) * 0.55);
                    let offset = (p - centre) * cell_metres;
                    let t = offset.dot(axis) / length + 0.5;
                    if !(0.0..1.0).contains(&t) {
                        continue;
                    }
                    // The midrib curves, so that no two leaves lie quite straight.
                    let curve = (random(4) - 0.5) * length * 0.3 * (t - 0.5) * (t - 0.5) * 4.0;
                    let across = offset.perp_dot(axis) - curve;
                    // Widest below the middle, narrowing to a point at the tip.
                    let half_width = length
                        * (0.2 + random(5) * 0.12)
                        * (std::f32::consts::PI * t.powf(0.8))
                            .sin()
                            .max(0.0)
                            .powf(0.75);
                    if half_width <= 0.0 {
                        continue;
                    }
                    let side = across / half_width;
                    if side.abs() >= 1.0 {
                        continue;
                    }
                    // Dry leaves curl up from the midrib, or cup down, a little either way.
                    let curl = (random(6) - 0.5) * 2.0 * side * side * length * 0.035;
                    let layer = random(7) * self.depth;
                    let edge = (1.0 - side * side).sqrt() * 0.0012;
                    let midrib = (1.0 - (across.abs() / 0.0022).min(1.0)) * 0.0009;
                    let height = layer + curl.max(0.0) + edge + midrib + 0.001;
                    if height > top.height {
                        // Side veins run forward from the midrib towards the edge.
                        let phase = (t * 7.0 - side.abs() * 1.1).rem_euclid(1.0);
                        let side_vein = (1.0 - (phase - 0.5).abs() * 9.0).max(0.0) * 0.45;
                        let rib = 1.0 - (across.abs() / 0.0026).min(1.0);
                        top = Leaf {
                            height,
                            id: random(6),
                            vein: side_vein.max(rib),
                            age: random(5),
                        };
                    }
                }
            }
        }
        top
    }
}

/// What [`Strokes`] finds at a point.
#[derive(Clone, Copy, Debug, Default)]
pub struct Stroke {
    /// Height of the topmost stroke here, in `0..1`, or 0 if none covers this point.
    pub height: f32,
    /// A random number in `0..1` that is the same along the whole stroke.
    pub id: f32,
}

/// Short overlapping strokes in random directions: blades of grass, needles, fallen leaves.
#[derive(Clone, Copy, Debug)]
pub struct Strokes {
    pub period: i32,
    pub seed: u32,
    /// Length of a stroke, in cell widths. Stay below 3, the distance searched.
    pub length: f32,
    /// Width at the middle of a stroke, in cell widths.
    pub width: f32,
    /// Strokes started per cell.
    pub density: u32,
}

impl Strokes {
    pub fn sample(&self, u: f32, v: f32) -> Stroke {
        let p = Vec2::new(u, v) * self.period as f32;
        let base = p.floor();
        let reach = (self.length * 0.5 + self.width).ceil() as i32;
        let mut top = Stroke::default();
        for dy in -reach..=reach {
            for dx in -reach..=reach {
                let (cx, cy) = (base.x as i32 + dx, base.y as i32 + dy);
                let (wx, wy) = (cx.rem_euclid(self.period), cy.rem_euclid(self.period));
                for stroke in 0..self.density {
                    let channel = stroke * 5;
                    let centre = Vec2::new(
                        cx as f32 + lattice_random(wx, wy, self.seed, channel),
                        cy as f32 + lattice_random(wx, wy, self.seed, channel + 1),
                    );
                    let angle =
                        lattice_random(wx, wy, self.seed, channel + 2) * std::f32::consts::TAU;
                    let half = Vec2::from_angle(angle) * self.length * 0.5;
                    let along = ((p - centre).dot(half) / half.length_squared()).clamp(-1.0, 1.0);
                    let distance = (p - centre - half * along).length();
                    // Strokes taper towards their ends.
                    let width = self.width * (1.0 - along * along).max(0.0).sqrt();
                    if distance < width {
                        let layer = lattice_random(wx, wy, self.seed, channel + 3);
                        let height = layer * (1.0 - (distance / width).powi(2)).sqrt();
                        if height > top.height {
                            top = Stroke {
                                height,
                                id: lattice_random(wx, wy, self.seed, channel + 4),
                            };
                        }
                    }
                }
            }
        }
        top
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_tiles(field: &impl Field, name: &str) {
        for i in 0..64 {
            let t = i as f32 / 64.0;
            for (a, b) in [((0.0, t), (1.0, t)), ((t, 0.0), (t, 1.0))] {
                let difference = (field.at(a.0, a.1) - field.at(b.0, b.1)).abs();
                assert!(difference < 1.0e-3, "{name} differs by {difference} at {t}");
            }
        }
    }

    #[test]
    fn every_field_and_combinator_tiles() {
        assert_tiles(&fbm(4, 5, 1), "fbm");
        assert_tiles(&ridged(5, 4, 2), "ridged");
        assert_tiles(&billow(5, 4, 2), "billow");
        assert_tiles(&waves(8), "waves");
        assert_tiles(&fbm(4, 5, 1).unit().smooth(0.3, 0.7), "smooth");
        assert_tiles(&fbm(4, 5, 1).plus(ridged(3, 3, 2)), "plus");
        assert_tiles(&fbm(4, 5, 1).times(waves(3)), "times");
        assert_tiles(&ridged(6, 4, 2).warped(fbm(3, 3, 7), 0.1), "warped");
        assert_tiles(&fbm(3, 4, 1).repeated(2, 5), "repeated");

        let cells = Cells { period: 9, seed: 3 };
        assert_tiles(
            &move |u: f32, v: f32| cells.sample(u, v).nearest,
            "cell distance",
        );
        assert_tiles(
            &move |u: f32, v: f32| cells.sample(u, v).border(),
            "cell border",
        );
        assert_tiles(&move |u: f32, v: f32| cells.sample(u, v).id, "cell id");

        let strokes = Strokes {
            period: 12,
            seed: 4,
            length: 2.0,
            width: 0.15,
            density: 2,
        };
        assert_tiles(
            &move |u: f32, v: f32| strokes.sample(u, v).height,
            "strokes",
        );

        let leaves = Leaves {
            period: 12,
            seed: 11,
            density: 2,
            length: 0.5,
            depth: 0.03,
            tile_metres: 4.0,
        };
        assert_tiles(&move |u: f32, v: f32| leaves.sample(u, v).height, "leaves");

        let blades = Blades {
            period: 24,
            seed: 9,
            density: 2,
            length: 0.3,
            width: 0.02,
            rise: 0.1,
            flow: fbm(3, 2, 10),
            spread: 0.1,
            tile_metres: 4.0,
        };
        assert_tiles(&move |u: f32, v: f32| blades.sample(u, v).height, "blades");

        let facets = Facets {
            period: 7,
            seed: 8,
            slope: 0.5,
            tile_metres: 4.0,
        };
        assert_tiles(&move |u: f32, v: f32| facets.sample(u, v).height, "facets");

        let stones = Stones {
            period: 20,
            seed: 5,
            radius: 0.3,
            tile_metres: 4.0,
            coverage: 0.7,
        };
        assert_tiles(&move |u: f32, v: f32| stones.sample(u, v).height, "stones");
    }

    #[test]
    fn the_same_seed_gives_the_same_field_and_another_does_not() {
        let (a, b, c) = (fbm(4, 4, 1), fbm(4, 4, 1), fbm(4, 4, 2));
        assert_eq!(a.at(0.3, 0.6), b.at(0.3, 0.6));
        assert_ne!(a.at(0.3, 0.6), c.at(0.3, 0.6));
    }

    #[test]
    fn cells_stay_constant_inside_and_order_their_distances() {
        let cells = Cells {
            period: 10,
            seed: 5,
        };
        for i in 0..200 {
            let cell = cells.sample(i as f32 * 0.0137, i as f32 * 0.0071);
            assert!(cell.nearest <= cell.second);
            assert!((0.0..1.0).contains(&cell.id));
        }
    }

    #[test]
    fn strokes_cover_part_of_the_square_but_not_all() {
        let strokes = Strokes {
            period: 16,
            seed: 6,
            length: 1.5,
            width: 0.1,
            density: 1,
        };
        let covered = (0..1000)
            .filter(|&i| {
                strokes
                    .sample(i as f32 * 0.00613, i as f32 * 0.00291)
                    .height
                    > 0.0
            })
            .count();
        assert!(covered > 50 && covered < 950, "covered {covered} of 1000");
    }
}
