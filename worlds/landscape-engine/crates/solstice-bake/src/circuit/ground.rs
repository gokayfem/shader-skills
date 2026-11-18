//! The land as it is drawn: a mesh for each kind of ground, finer where the land is steep.
//!
//! Most of the island is gentle, and a point every metre and a half draws it well. A cliff is
//! not: its whole face is a few metres across, and at that spacing its ledges and gullies are
//! lost and the line where rock meets grass is a row of teeth. So the squares of the mesh on
//! steep ground are each cut into nine, and take their heights from the island's shape itself
//! rather than from the grid a car drives on. Where a fine square meets a coarse one, the
//! points along the edge between them lie on the coarse square's straight edge, so no gap
//! shows.

use super::{Island, Land, Road, GRASS, LAND_SPACING, ROCK, SAND, SINK};
use crate::meshes;
use glam::Vec3;
use rayon::prelude::*;
use solstice_builder::Mesh;

/// How many smaller squares a side a square on steep ground is cut into.
const FINE: usize = 3;
/// Below this the sea hides its bed.
const DEEP: f32 = -5.5;

/// Which kind of ground a piece of the land is.
fn kind(height: f32, slope: f32, rocky: f32, beside_road: bool) -> usize {
    if !beside_road && (slope > 0.85 || (rocky > 0.25 && slope > 0.4)) {
        ROCK
    } else if height < 1.3 {
        SAND
    } else {
        GRASS
    }
}

/// The land as a mesh for each of its kinds: grass, rock and sand.
pub(super) fn draw(island: &Island, road: &Road, land: &Land, size: f32) -> Vec<Mesh> {
    let cells = ((size / LAND_SPACING) as usize).clamp(64, 1400);
    let step = size / cells as f32;
    let half = size * 0.5;
    let small = step / FINE as f32;

    // The squares on steep ground, and those next to them, so that the fine mesh has settled
    // back to the coarse one's shape by where they meet.
    let steep: Vec<bool> = (0..cells * cells)
        .into_par_iter()
        .map(|index| {
            let (x, z) = (
                (index % cells) as f32 * step + step * 0.5 - half,
                (index / cells) as f32 * step + step * 0.5 - half,
            );
            land.at(x, z) > -3.0 && (land.slope(x, z) > 0.45 || island.rocky(x, z) > 0.02)
        })
        .collect();
    let is = |flags: &[bool], column: isize, row: isize| {
        (0..cells as isize).contains(&column)
            && (0..cells as isize).contains(&row)
            && flags[row as usize * cells + column as usize]
    };
    let fine: Vec<bool> = (0..cells * cells)
        .into_par_iter()
        .map(|index| {
            let (column, row) = ((index % cells) as isize, (index / cells) as isize);
            (-1..=1).any(|dz| (-1..=1).any(|dx| is(&steep, column + dx, row + dz)))
        })
        .collect();
    let fine_at = |column: isize, row: isize| is(&fine, column, row);

    // A point of the land, under the road's sink where the road is.
    let sunk = |x: f32, z: f32| {
        if road
            .beside(x, z)
            .is_some_and(|beside| beside.beyond() < 1.5)
        {
            SINK
        } else {
            0.0
        }
    };
    let coarse = |x: f32, z: f32| land.at(x, z) - sunk(x, z);
    let exact = |x: f32, z: f32| land.exact(island, x, z) - sunk(x, z);
    // The height at a point of the fine lattice: `FINE` steps to a square.
    let lattice = |i: usize, j: usize| -> f32 {
        let place = |i: usize, j: usize| (i as f32 * small - half, j as f32 * small - half);
        let (column, row) = ((i / FINE) as isize, (j / FINE) as isize);
        let corner = |i: usize, j: usize| {
            let (column, row) = ((i / FINE) as isize, (j / FINE) as isize);
            let (x, z) = place(i, j);
            // A corner belongs to four squares: it is the land's own shape only if all are fine.
            if (-1..=0).all(|dz| (-1..=0).all(|dx| fine_at(column + dx, row + dz))) {
                exact(x, z)
            } else {
                coarse(x, z)
            }
        };
        let (x, z) = place(i, j);
        match (i % FINE, j % FINE) {
            (0, 0) => corner(i, j),
            // On an edge between two squares side by side.
            (0, along) => {
                if fine_at(column - 1, row) && fine_at(column, row) {
                    exact(x, z)
                } else {
                    let (a, b) = (corner(i, j - along), corner(i, j - along + FINE));
                    a + (b - a) * along as f32 / FINE as f32
                }
            }
            // On an edge between one square and the next row's.
            (along, 0) => {
                if fine_at(column, row - 1) && fine_at(column, row) {
                    exact(x, z)
                } else {
                    let (a, b) = (corner(i - along, j), corner(i - along + FINE, j));
                    a + (b - a) * along as f32 / FINE as f32
                }
            }
            _ => exact(x, z),
        }
    };

    // Each row of squares, as pieces of each kind.
    let rows: Vec<Vec<(usize, [Vec3; 4])>> = (0..cells)
        .into_par_iter()
        .map(|row| {
            let mut pieces = Vec::with_capacity(cells);
            for column in 0..cells {
                let (x, z) = (column as f32 * step - half, row as f32 * step - half);
                let beside_road = |x: f32, z: f32| {
                    road.beside(x, z)
                        .is_some_and(|beside| beside.outside() < super::SHOULDER - super::BARRIER)
                };
                if !fine[row * cells + column] {
                    let (cx, cz) = (x + step * 0.5, z + step * 0.5);
                    let corners = [
                        Vec3::new(x, coarse(x, z), z),
                        Vec3::new(x, coarse(x, z + step), z + step),
                        Vec3::new(x + step, coarse(x + step, z + step), z + step),
                        Vec3::new(x + step, coarse(x + step, z), z),
                    ];
                    // The sea bed out in deep water is never seen: it is left out.
                    if corners.iter().all(|corner| corner.y < DEEP) {
                        continue;
                    }
                    let what = kind(
                        land.at(cx, cz),
                        land.slope(cx, cz),
                        0.0,
                        beside_road(cx, cz),
                    );
                    pieces.push((what, corners));
                    continue;
                }
                let mut heights = [[0.0_f32; FINE + 1]; FINE + 1];
                for (dj, line) in heights.iter_mut().enumerate() {
                    for (di, height) in line.iter_mut().enumerate() {
                        *height = lattice(column * FINE + di, row * FINE + dj);
                    }
                }
                for dj in 0..FINE {
                    for di in 0..FINE {
                        let (x0, z0) = (x + di as f32 * small, z + dj as f32 * small);
                        let (h00, h01, h11, h10) = (
                            heights[dj][di],
                            heights[dj + 1][di],
                            heights[dj + 1][di + 1],
                            heights[dj][di + 1],
                        );
                        let corners = [
                            Vec3::new(x0, h00, z0),
                            Vec3::new(x0, h01, z0 + small),
                            Vec3::new(x0 + small, h11, z0 + small),
                            Vec3::new(x0 + small, h10, z0),
                        ];
                        let middle = (h00 + h01 + h11 + h10) * 0.25;
                        let slope = ((h10 + h11 - h00 - h01) * 0.5)
                            .hypot((h01 + h11 - h00 - h10) * 0.5)
                            / small;
                        let (cx, cz) = (x0 + small * 0.5, z0 + small * 0.5);
                        let what = kind(middle, slope, island.rocky(cx, cz), beside_road(cx, cz));
                        pieces.push((what, corners));
                    }
                }
            }
            pieces
        })
        .collect();
    let mut kinds: Vec<Vec<[Vec3; 4]>> = vec![Vec::new(); 3];
    for row in rows {
        for (what, piece) in row {
            kinds[what].push(piece);
        }
    }
    kinds
        .into_iter()
        .map(|pieces| meshes::quads(&pieces, |_| Vec3::Y))
        .collect()
}
