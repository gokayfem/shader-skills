//! Where the sea's surface is drawn: a quadtree of square patches, finer near the camera,
//! after Strugar's continuous distance-dependent level of detail (CDLOD, 2010).
//!
//! The patches are fixed in the world, so their vertices never slide over the waves as the
//! camera moves. Each frame, the patches near enough to need detail are chosen by their distance
//! from the camera. Every vertex then blends towards the next coarser grid as its own distance
//! nears the end of its level's range, so that where a fine patch meets a coarse one their edges
//! hold exactly the same points: no cracks, and nothing pops as levels change.
//!
//! `shaders/ocean/surface.wgsl` places the vertices exactly as [`vertex_position`] does, which
//! the tests here check.

use glam::{Vec2, Vec3};

/// Cells across a whole patch on desktop GPUs: vertices about 11 to 23 pixels apart at 1080p.
/// Integrated GPUs use half as many, a quarter of the triangles. Patches drawn for a quarter of
/// a node have half as many again.
pub const PATCH_CELLS: u32 = 32;
/// The finest patches, in metres across: their cells are 12.5 cm at 32 cells.
pub const FINEST_PATCH: f32 = 4.0;
/// Levels of patch size, each twice the last: the coarsest is 4 · 2¹³ = 32 km across.
pub const LEVELS: u32 = 14;
/// Each level is used out to this many of its patches' widths from the camera. Large enough that
/// a coarse level never starts blending where a finer one meets it, which would crack.
const RANGE_PER_PATCH: f32 = 2.8;
/// Vertices start blending towards the coarser grid at this share of their level's range.
const MORPH_START: f32 = 0.8;

/// Width of a level's patches, in metres.
pub fn patch_size(level: u32) -> f32 {
    FINEST_PATCH * (1u32 << level) as f32
}

/// How far from the camera a level is used, in metres.
pub fn range(level: u32) -> f32 {
    RANGE_PER_PATCH * patch_size(level)
}

/// Where a level's vertices begin and finish blending to the coarser grid.
pub fn morph_range(level: u32) -> (f32, f32) {
    let end = range(level);
    (end * MORPH_START, end)
}

/// A square of sea to draw.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Patch {
    /// The corner with the least x and z, in metres.
    pub origin: Vec2,
    /// Width, in metres.
    pub size: f32,
    /// The level whose grid spacing and blending it uses.
    pub level: u32,
    /// Cells across: `PATCH_CELLS`, or half that for a quarter of a node.
    pub cells: u32,
}

impl Patch {
    /// Metres between vertices.
    pub fn spacing(&self) -> f32 {
        self.size / self.cells as f32
    }
}

/// The patches to draw for a camera at `camera`, over a sea whose surface is at `sea_level`,
/// with `cells` across each whole patch: an even number.
pub fn select(camera: Vec3, sea_level: f32, cells: u32) -> Vec<Patch> {
    assert!(
        cells >= 4 && cells.is_multiple_of(4),
        "patches need a multiple of four cells, not {cells}"
    );
    let mut patches = Vec::new();
    let top = LEVELS - 1;
    let size = patch_size(top);
    // The nine coarsest nodes around the camera, fixed to their own grid.
    let centre = (Vec2::new(camera.x, camera.z) / size).floor();
    for dz in -1..=1 {
        for dx in -1..=1 {
            let origin = (centre + Vec2::new(dx as f32, dz as f32)) * size;
            if !select_node(origin, top, camera, sea_level, cells, &mut patches) {
                // Out of reach of even the coarsest range, as from very high up.
                patches.push(Patch {
                    origin,
                    size,
                    level: top,
                    cells,
                });
            }
        }
    }
    patches
}

/// Selects node `origin` at `level`, or its children. Returns false if it is beyond its level's
/// range, for its parent to draw it instead.
fn select_node(
    origin: Vec2,
    level: u32,
    camera: Vec3,
    sea_level: f32,
    cells: u32,
    patches: &mut Vec<Patch>,
) -> bool {
    let size = patch_size(level);
    if !reaches(origin, size, camera, sea_level, range(level)) {
        return false;
    }
    if level == 0 || !reaches(origin, size, camera, sea_level, range(level - 1)) {
        patches.push(Patch {
            origin,
            size,
            level,
            cells,
        });
        return true;
    }
    let half = size / 2.0;
    for (dx, dz) in [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)] {
        let child = origin + Vec2::new(dx, dz) * half;
        if !select_node(child, level - 1, camera, sea_level, cells, patches) {
            // Too far for the finer level: this level draws the quarter, at its own spacing.
            patches.push(Patch {
                origin: child,
                size: half,
                level,
                cells: cells / 2,
            });
        }
    }
    true
}

/// Whether any of the square at `origin`, `size` across on the sea's surface, lies within
/// `radius` of the camera.
fn reaches(origin: Vec2, size: f32, camera: Vec3, sea_level: f32, radius: f32) -> bool {
    let nearest = Vec2::new(camera.x, camera.z).clamp(origin, origin + Vec2::splat(size));
    let offset = Vec3::new(
        nearest.x - camera.x,
        sea_level - camera.y,
        nearest.y - camera.z,
    );
    offset.length_squared() <= radius * radius
}

/// Where vertex `(x, z)` of `patch`'s grid lies on the calm sea, in metres, after blending
/// towards the coarser grid by its distance from `camera`.
pub fn vertex_position(patch: &Patch, x: u32, z: u32, camera: Vec3, sea_level: f32) -> Vec2 {
    vertex(patch, x, z, camera, sea_level).0
}

/// Vertex `(x, z)` of `patch`: where it lies on the calm sea, and the spacing whose detail its
/// waves are read at. That spacing blends to the coarser grid's along with the position, so
/// where two levels meet, both read the waves at the same place and the same level of detail,
/// and their heights agree.
pub fn vertex(patch: &Patch, x: u32, z: u32, camera: Vec3, sea_level: f32) -> (Vec2, f32) {
    let spacing = patch.spacing();
    let grid = Vec2::new(x as f32, z as f32);
    let unmorphed = patch.origin + grid * spacing;
    let distance = Vec3::new(unmorphed.x, sea_level, unmorphed.y).distance(camera);
    let (start, end) = morph_range(patch.level);
    let morph = ((distance - start) / (end - start)).clamp(0.0, 1.0);
    // Odd vertices slide onto their even neighbours, so the grid becomes the coarser one's.
    let odd = grid - (grid * 0.5).floor() * 2.0;
    let position = patch.origin + (grid - odd * morph) * spacing;
    (position, spacing * (1.0 + morph))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cameras() -> Vec<Vec3> {
        vec![
            Vec3::new(0.0, 1.5, 0.0),
            Vec3::new(3.7, 0.4, -12.2),
            Vec3::new(66.0, 4.0, -41.0),
            Vec3::new(-1234.5, 40.0, 987.6),
            Vec3::new(20_000.0, 300.0, -17_000.0),
            Vec3::new(5.0, 3000.0, 5.0),
        ]
    }

    #[test]
    fn patches_tile_the_sea_without_gaps_or_overlaps() {
        for camera in cameras() {
            let patches = select(camera, 0.0, PATCH_CELLS);
            let area: f64 = patches.iter().map(|p| (p.size as f64).powi(2)).sum();
            let expected = 9.0 * (patch_size(LEVELS - 1) as f64).powi(2);
            assert!(
                (area / expected - 1.0).abs() < 1.0e-9,
                "{camera}: {area} of {expected}"
            );
            for (i, a) in patches.iter().enumerate() {
                for b in &patches[i + 1..] {
                    let overlap = (a.origin.x.max(b.origin.x)
                        < (a.origin.x + a.size).min(b.origin.x + b.size))
                        && (a.origin.y.max(b.origin.y)
                            < (a.origin.y + a.size).min(b.origin.y + b.size));
                    assert!(!overlap, "{camera}: {a:?} overlaps {b:?}");
                }
            }
        }
    }

    #[test]
    fn detail_follows_the_camera_and_stays_affordable() {
        for camera in cameras() {
            let patches = select(camera, 0.0, PATCH_CELLS);
            assert!(patches.len() < 500, "{camera}: {} patches", patches.len());
            // Triangles, at the desktop and integrated resolutions.
            for (cells, budget) in [(PATCH_CELLS, 700_000), (PATCH_CELLS / 2, 180_000)] {
                let triangles: u32 = select(camera, 0.0, cells)
                    .iter()
                    .map(|p| p.cells * p.cells * 2)
                    .sum();
                assert!(
                    triangles < budget,
                    "{camera}, {cells} cells: {triangles} triangles"
                );
            }
            // Below the camera, the finest patches it can need: right at the surface, the finest
            // level of all.
            let below = Vec2::new(camera.x, camera.z);
            let under = patches
                .iter()
                .find(|p| {
                    below.cmpge(p.origin).all() && below.cmplt(p.origin + Vec2::splat(p.size)).all()
                })
                .unwrap();
            if camera.y < range(0) {
                assert_eq!(under.level, 0, "{camera}");
            }
        }
    }

    #[test]
    fn neighbouring_patches_share_every_edge_point() {
        // Wherever two patches touch, each point of one's edge, blended as the vertex shader
        // blends it, lies on the other's edge as it is drawn: between two of its points, on the
        // straight line the rasterizer draws. And where a point of each meets, both read the
        // waves at the same level of detail, so their heights agree too: the same place alone
        // is not enough, as a finer mip holds different heights.
        for (camera, cells) in cameras()
            .into_iter()
            .flat_map(|c| [(c, PATCH_CELLS), (c, PATCH_CELLS / 2)])
        {
            let patches = select(camera, 0.0, cells);
            for a in &patches {
                for b in &patches {
                    if a == b || a.spacing() > b.spacing() {
                        continue;
                    }
                    // `a` is the finer, or as fine. Their shared edge, if any.
                    for (axis, a_side, b_side) in [
                        (0, a.origin.x + a.size, b.origin.x),
                        (0, a.origin.x, b.origin.x + b.size),
                        (1, a.origin.y + a.size, b.origin.y),
                        (1, a.origin.y, b.origin.y + b.size),
                    ] {
                        if (a_side - b_side).abs() > 1.0e-3 {
                            continue;
                        }
                        let along = 1 - axis;
                        let a_from = a.origin[along];
                        let a_to = a_from + a.size;
                        let b_from = b.origin[along];
                        let b_to = b_from + b.size;
                        if a_from.max(b_from) >= a_to.min(b_to) {
                            continue;
                        }
                        let edge = |p: &Patch, at_far_side: bool| -> Vec<(Vec2, f32)> {
                            (0..=p.cells)
                                .map(|i| {
                                    let fixed = if at_far_side { p.cells } else { 0 };
                                    let (x, z) = if axis == 0 { (fixed, i) } else { (i, fixed) };
                                    vertex(p, x, z, camera, 0.0)
                                })
                                .collect()
                        };
                        let a_far = (a.origin[axis] + a.size - a_side).abs() < 1.0e-3;
                        let b_far = (b.origin[axis] + b.size - b_side).abs() < 1.0e-3;
                        let fine = edge(a, a_far);
                        let coarse = edge(b, b_far);
                        for (point, detail) in fine {
                            let t = point[along];
                            if t < b_from - 1.0e-3 || t > b_to + 1.0e-3 {
                                continue;
                            }
                            // The coarse edge's segment around t, and the point on it.
                            let on_coarse = coarse.windows(2).find_map(|w| {
                                let (p, q) = (w[0].0[along], w[1].0[along]);
                                let (lo, hi) = (p.min(q), p.max(q));
                                (t >= lo - 1.0e-4 && t <= hi + 1.0e-4).then_some(())
                            });
                            assert!(
                                on_coarse.is_some(),
                                "{camera}: {point} of {a:?} is off the edge of {b:?}"
                            );
                            // Vertices across the edge must be exactly the coarse ones, not
                            // just between them: the fine side has as many points or more, so
                            // any it has that the coarse side lacks must sit on the coarse
                            // side's points once blended.
                            if a.spacing() < b.spacing() {
                                let matched = coarse.iter().find(|c| {
                                    (c.0[along] - t).abs() < 1.0e-3 * b.spacing().max(1.0)
                                });
                                let Some((_, coarse_detail)) = matched else {
                                    panic!(
                                        "{camera}: {point} of {a:?} is not blended onto a vertex of {b:?}"
                                    );
                                };
                                assert!(
                                    (detail / coarse_detail - 1.0).abs() < 1.0e-3,
                                    "{camera}: at {point}, {a:?} reads waves at {detail} m detail and {b:?} at {coarse_detail} m"
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn vertices_stay_put_as_the_camera_moves() {
        // A vertex well inside its level's range does not move when the camera does: the
        // patches are fixed to the world, so nothing slides over the waves.
        let camera = Vec3::new(10.0, 2.0, 10.0);
        let moved = camera + Vec3::new(0.37, 0.0, -0.21);
        let before = select(camera, 0.0, PATCH_CELLS);
        let after = select(moved, 0.0, PATCH_CELLS);
        let shared: Vec<_> = before.iter().filter(|p| after.contains(p)).collect();
        assert!(shared.len() > before.len() / 2);
        for patch in shared {
            for (x, z) in [(0, 0), (5, 9), (patch.cells, patch.cells / 2)] {
                let a = vertex_position(patch, x, z, camera, 0.0);
                let b = vertex_position(patch, x, z, moved, 0.0);
                let distance = Vec3::new(a.x, 0.0, a.y).distance(camera);
                if distance < morph_range(patch.level).0 * 0.95 {
                    assert_eq!(a, b, "{patch:?} vertex ({x}, {z}) slid");
                }
            }
        }
    }
}
