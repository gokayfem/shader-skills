//! The land and the seabed: one height function, cut into patches by what the ground is made of.
//!
//! North is -Z and east is +X. The sea stands at height 0 and lies to the west. `docs/harbour.md`
//! has the map this follows.

use crate::geometry::Geometry;
use glam::{Vec2, Vec3};
use solstice_procgen::noise::fbm;

/// What the ground is made of, which decides its material.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Surface {
    Sand,
    Grass,
    Dirt,
    Rock,
}

pub const SURFACES: [Surface; 4] = [Surface::Sand, Surface::Grass, Surface::Dirt, Surface::Rock];

/// The square the ground covers: it is centred inland of the quay, and its edges lie under the sea.
pub const CENTRE_X: f32 = 150.0;
pub const SIZE: f32 = 1000.0;
/// Patches per side. Each is small enough for coarse collision to be cheap.
pub const PATCHES: u32 = 4;
/// Metres per grid cell.
pub const CELL: f32 = 1.0;

/// Deck height of the quay.
pub const QUAY_DECK: f32 = 1.9;
/// The x the land starts at along the quay: inside the quay's solid, so that the jump from
/// seabed to land is hidden.
const QUAY_SHORE: f32 = 4.0;
const DEEP: f32 = 11.0;
/// The ground at the town wall. The upper town climbs steeply to it, so that from the water each
/// row of houses stands clear above the one in front.
const WALL_FOOT: f32 = 20.0;

fn smoothstep(low: f32, high: f32, x: f32) -> f32 {
    let t = ((x - low) / (high - low)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// Where the waterline is at each z, before its wander: (z, x), north to south.
const COAST: [(f32, f32); 17] = [
    (-420.0, 230.0),
    (-260.0, 40.0),
    (-140.0, -65.0),
    (-130.0, -60.0),
    (-100.0, -40.0),
    (-90.0, -30.0),
    (-75.0, -15.0),
    (-62.0, -4.0),
    (-54.0, QUAY_SHORE),
    (50.0, QUAY_SHORE),
    (54.0, -20.0),
    (60.0, -25.0),
    (100.0, -45.0),
    (130.0, -75.0),
    (140.0, -125.0),
    (165.0, -125.0),
    (420.0, 230.0),
];

/// How much of this z lies along the quay, where the coast is built rather than natural.
fn quay_share(z: f32) -> f32 {
    smoothstep(-66.0, -54.0, z) * (1.0 - smoothstep(50.0, 56.0, z))
}

/// How far east the waterline lies at `z`.
fn shoreline(z: f32) -> f32 {
    let line = if z <= COAST[0].0 {
        COAST[0].1
    } else {
        let next = COAST
            .iter()
            .position(|&(along, _)| along > z)
            .unwrap_or(COAST.len() - 1);
        let (a, b) = (COAST[next - 1], COAST[next]);
        lerp(a.1, b.1, ((z - a.0) / (b.0 - a.0)).clamp(0.0, 1.0))
    };
    line + fbm(Vec3::new(0.0, 7.0, z) * 0.02, 4, 405) * 16.0 * (1.0 - quay_share(z))
}

/// The town's ground, by how far inland: level along the quay, a step up to the market square,
/// then the climb to the town wall and the hill behind.
fn town(x: f32, z: f32) -> f32 {
    if x < 10.0 {
        // Just under the quay's deck where it covers the land; level with it beyond the ends.
        return if z > -52.0 && z < 50.0 {
            QUAY_DECK - 0.2
        } else {
            QUAY_DECK
        };
    }
    let square = 3.0 + (x - 24.0).max(0.0) / 60.0;
    if x <= 56.0 {
        return lerp(QUAY_DECK, square, smoothstep(16.0, 24.0, x));
    }
    let edge = 3.0 + 32.0 / 60.0;
    if x <= 125.0 {
        return lerp(edge, WALL_FOOT, (x - 56.0) / 69.0);
    }
    if x <= 133.0 {
        return WALL_FOOT;
    }
    lerp(WALL_FOOT, 52.0, smoothstep(133.0, 300.0, x))
}

/// How much of a point lies in a headland, rather than the low ground around the town and beach.
fn headland(z: f32) -> f32 {
    smoothstep(-58.0, -105.0, z) + smoothstep(138.0, 165.0, z)
}

/// Height of the ground above the sea.
pub fn height(x: f32, z: f32) -> f32 {
    let inland = x - shoreline(z);
    let along_quay = quay_share(z);

    let ground = if inland < 0.0 {
        // The seabed. Dredged against the quay, a natural shelving bottom elsewhere.
        let natural = -DEEP * (1.0 - (inland / 130.0).exp());
        let dredged = (-3.5 + inland * 3.5 / 75.0).max(-DEEP);
        lerp(natural, dredged, along_quay)
    } else {
        let rolling = fbm(Vec3::new(x, 0.0, z) * 0.02, 4, 401) * 5.0;
        let wild = smoothstep(133.0, 190.0, x).max(smoothstep(62.0, 110.0, z.abs()));
        let levels = town(x, z) + rolling * wild;
        // A beach rises 1 in 12 from the waterline to its crest, then meets the town's levels.
        let beach = inland / 12.0;
        let backed = lerp(beach, levels.max(beach), smoothstep(30.0, 70.0, inland));
        let low = lerp(backed, levels, along_quay);
        // Headlands stand up steeply from the water.
        let rise = headland(z) * smoothstep(0.0, 14.0, inland);
        let craggy = fbm(Vec3::new(x, 3.0, z) * 0.05, 4, 403) * 8.0 * rise;
        low + rise * 26.0 + craggy
    };
    // The rest of the island's coast, so that the ground's edges lie under the sea.
    let (dx, dz) = (x - 180.0, z);
    let radius = dx.hypot(dz).max(1.0);
    let around = Vec3::new(dx / radius, 0.0, dz / radius);
    let coast = 360.0 + fbm(around * 1.6 + Vec3::Y * 3.0, 3, 407) * 45.0;
    let offshore = smoothstep(0.0, 120.0, radius - coast);
    lerp(ground, -DEEP, offshore)
}

/// What the ground at a point is made of, given how steep it is.
pub fn surface(x: f32, z: f32, slope: f32) -> Surface {
    let y = height(x, z);
    let inland = x - shoreline(z);
    if slope > 0.8 && y > -1.0 {
        return Surface::Rock;
    }
    if inland < 0.0 || (y < 3.0 && x - shoreline(z) < 34.0 && z > 50.0) {
        return Surface::Sand;
    }
    if headland(z) > 0.6 && inland < 40.0 {
        return Surface::Rock;
    }
    if x < 125.0 && z.abs() < 52.0 {
        return Surface::Dirt;
    }
    Surface::Grass
}

/// The surface of the ground's normal at a point, and how steep it is there.
fn normal_and_slope(x: f32, z: f32) -> (Vec3, f32) {
    let step = CELL;
    let dx = (height(x + step, z) - height(x - step, z)) / (2.0 * step);
    let dz = (height(x, z + step) - height(x, z - step)) / (2.0 * step);
    (Vec3::new(-dx, 1.0, -dz).normalize(), dx.hypot(dz))
}

/// One patch of ground: its triangles for each of [`SURFACES`], with no vertex shared between
/// surfaces, since a material ends at its edge.
pub fn patch(column: u32, row: u32) -> Vec<Geometry> {
    let span = SIZE / PATCHES as f32;
    let cells = (span / CELL) as u32;
    let origin = Vec2::new(
        CENTRE_X - SIZE * 0.5 + column as f32 * span,
        -SIZE * 0.5 + row as f32 * span,
    );
    let corners = (cells + 1) as usize;
    let at = |i: u32, j: u32| origin + Vec2::new(i as f32, j as f32) * CELL;
    let grid: Vec<(Vec3, Vec3, f32)> = (0..=cells)
        .flat_map(|j| (0..=cells).map(move |i| (i, j)))
        .map(|(i, j)| {
            let p = at(i, j);
            let (normal, slope) = normal_and_slope(p.x, p.y);
            (Vec3::new(p.x, height(p.x, p.y), p.y), normal, slope)
        })
        .collect();

    let mut out = vec![Geometry::default(); SURFACES.len()];
    // For each surface, which grid corner has become which vertex.
    let mut seen = vec![vec![u32::MAX; corners * corners]; SURFACES.len()];
    for j in 0..cells as usize {
        for i in 0..cells as usize {
            let centre = at(i as u32, j as u32) + Vec2::splat(CELL * 0.5);
            let slope = grid[j * corners + i].2;
            let surface = surface(centre.x, centre.y, slope);
            let which = SURFACES
                .iter()
                .position(|&s| s == surface)
                .expect("every surface is listed");
            let geometry = &mut out[which];
            let mut vertex = |corner: usize| {
                if seen[which][corner] == u32::MAX {
                    seen[which][corner] = geometry.positions.len() as u32;
                    let (position, normal, _) = grid[corner];
                    geometry.positions.push(position);
                    geometry.normals.push(normal);
                    geometry.uvs.push(Vec2::new(position.x, position.z));
                }
                seen[which][corner]
            };
            let a = vertex(j * corners + i);
            let b = vertex(j * corners + i + 1);
            let c = vertex((j + 1) * corners + i);
            let d = vertex((j + 1) * corners + i + 1);
            geometry.indices.extend([a, c, b, b, c, d]);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_edges_of_the_ground_lie_under_the_sea() {
        let half = SIZE * 0.5 - 2.0;
        for i in 0..=100 {
            let along = -half + i as f32 / 100.0 * 2.0 * half;
            for (x, z) in [
                (CENTRE_X - half, along),
                (CENTRE_X + half, along),
                (CENTRE_X + along, half),
                (CENTRE_X + along, -half),
            ] {
                assert!(height(x, z) < -3.0, "({x}, {z}) is {}", height(x, z));
            }
        }
    }

    #[test]
    fn the_basin_deepens_from_the_quay_to_the_mouth() {
        assert!((height(0.0, 0.0) + 3.7).abs() < 0.4, "{}", height(0.0, 0.0));
        assert!(
            (height(-72.0, 0.0) + 7.0).abs() < 0.4,
            "{}",
            height(-72.0, 0.0)
        );
    }

    #[test]
    fn the_town_stands_at_the_levels_on_the_map() {
        for (x, z, expected) in [
            (12.0, 0.0, QUAY_DECK),
            (24.0, 0.0, 3.0),
            (56.0, 0.0, 3.53),
            (125.0, 0.0, WALL_FOOT),
        ] {
            assert!(
                (height(x, z) - expected).abs() < 0.05,
                "({x}, {z}) is {}",
                height(x, z)
            );
        }
    }

    #[test]
    fn the_beach_runs_up_from_the_water() {
        for z in [70.0, 100.0, 125.0] {
            let waterline = shoreline(z);
            assert!(height(waterline - 12.0, z) < -0.3);
            assert!(height(waterline + 12.0, z) > 0.5);
            assert!(height(waterline + 12.0, z) < 2.0, "too steep at {z}");
        }
    }

    #[test]
    fn the_headlands_stand_well_above_the_sea() {
        assert!(height(40.0, -130.0) > 15.0);
        assert!(height(-60.0, 170.0) > 15.0);
    }
}
