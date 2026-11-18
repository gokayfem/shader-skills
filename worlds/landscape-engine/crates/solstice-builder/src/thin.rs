//! Simplification for meshes made of many small separate pieces: blades of grass, leaves, needles,
//! pebbles.
//!
//! Edge collapse cannot reduce such a mesh. Every edge of a blade lies on its outline, and an
//! outline must stay put or the shape tears, so a meadow would keep every triangle at every
//! distance. Instead each level keeps about half the pieces and enlarges the survivors so the
//! total area stays the same, as Unreal does for Nanite foliage when it preserves area. From far
//! away a meadow is then fewer, wider blades that cover the ground just as densely, rather than
//! the same blades thinning to nothing.
//!
//! Flat pieces grow across their narrow side first, until they are as wide as they are long, so
//! grass keeps its height, and only then grow evenly. Tubes, such as twigs, grow thicker around
//! their own axis, so a curved twig stays the curve it was.

use crate::simplify::{Attribute, Corner, Triangle};
use glam::{DMat3, DVec3, Vec3};
use rustc_hash::FxHashMap;

/// Fewer separate pieces than this in a group are left to ordinary simplification, so a lone
/// object is never mistaken for foliage.
const MIN_PIECES: usize = 8;
/// A piece holding more than this fraction of its group's triangles is a surface, not foliage.
const MAX_PIECE_SHARE: f32 = 0.125;
/// Pieces more than this many times wider than the group's median piece are not thinned.
const WIDE: f64 = 3.0;

/// A group's triangles sorted into separate pieces and everything else.
pub struct Pieces {
    /// Triangle indices of each piece.
    pub pieces: Vec<Vec<u32>>,
    /// Triangles that belong to no piece, for ordinary simplification.
    pub rest: Vec<u32>,
}

/// Splits `triangles` into connected pieces. Only pieces that lie wholly inside the group, touching
/// no locked vertex, can be removed, so the rest is everything else.
pub fn find_pieces(triangles: &[Triangle], is_locked: impl Fn(u32) -> bool) -> Pieces {
    let mut local: FxHashMap<u32, u32> = FxHashMap::default();
    for triangle in triangles {
        for corner in triangle {
            let next = local.len() as u32;
            local.entry(corner.position).or_insert(next);
        }
    }
    let mut parent: Vec<u32> = (0..local.len() as u32).collect();
    fn root(parent: &mut [u32], mut v: u32) -> u32 {
        while parent[v as usize] != v {
            parent[v as usize] = parent[parent[v as usize] as usize];
            v = parent[v as usize];
        }
        v
    }
    for triangle in triangles {
        let [a, b, c] = triangle.map(|corner| local[&corner.position]);
        let ra = root(&mut parent, a);
        for other in [b, c] {
            let ro = root(&mut parent, other);
            parent[ro as usize] = ra;
        }
    }

    let mut component: FxHashMap<u32, usize> = FxHashMap::default();
    let mut members: Vec<Vec<u32>> = Vec::new();
    let mut touches_lock: Vec<bool> = Vec::new();
    for (index, triangle) in triangles.iter().enumerate() {
        let id = root(&mut parent, local[&triangle[0].position]);
        let slot = *component.entry(id).or_insert_with(|| {
            members.push(Vec::new());
            touches_lock.push(false);
            members.len() - 1
        });
        members[slot].push(index as u32);
        if triangle.iter().any(|corner| is_locked(corner.position)) {
            touches_lock[slot] = true;
        }
    }

    let largest = (triangles.len() as f32 * MAX_PIECE_SHARE).max(1.0) as usize;
    let mut pieces = Vec::new();
    let mut rest = Vec::new();
    for (piece, locked) in members.into_iter().zip(touches_lock) {
        if !locked && piece.len() <= largest {
            pieces.push(piece);
        } else {
            rest.extend(piece);
        }
    }
    if pieces.len() < MIN_PIECES {
        rest.extend(pieces.drain(..).flatten());
        rest.sort_unstable();
    }
    Pieces { pieces, rest }
}

/// Result of [`thin`].
pub struct Thinned {
    /// The surviving pieces. A corner whose position is `NEW_POSITION | i` uses `positions[i]`.
    pub triangles: Vec<Triangle>,
    pub positions: Vec<Vec3>,
    /// The existing position each of `positions` was moved from, so that it can keep its skin.
    pub sources: Vec<u32>,
    /// Furthest any surviving vertex moved, in world units.
    pub error: f32,
}

/// Marks a position index as one of [`Thinned::positions`] rather than an existing vertex.
pub const NEW_POSITION: u32 = 1 << 31;

/// Keeps pieces until at least `target_triangles` remain, and enlarges them to keep the total area.
///
/// Which pieces survive depends only on where they are, so the choice is the same however the
/// pieces happen to be grouped.
pub fn thin(
    triangles: &[Triangle],
    pieces: &[Vec<u32>],
    positions: &[Vec3],
    attributes: &[Attribute],
    target_triangles: usize,
) -> Thinned {
    let position = |corner: Corner| positions[corner.position as usize].as_dvec3();
    let area = |piece: &[u32]| -> f64 {
        piece
            .iter()
            .map(|&t| {
                let [a, b, c] = triangles[t as usize].map(position);
                (b - a).cross(c - a).length() * 0.5
            })
            .sum()
    };

    let mut order: Vec<(u64, usize)> = pieces
        .iter()
        .enumerate()
        .map(|(index, piece)| {
            (
                placement_hash(piece_centroid(triangles, piece, positions)),
                index,
            )
        })
        .collect();
    order.sort_unstable();

    let mut kept = Vec::new();
    let mut kept_triangles = 0;
    for &(_, index) in &order {
        if kept_triangles >= target_triangles {
            break;
        }
        kept.push(index);
        kept_triangles += pieces[index].len();
    }

    let total_area: f64 = pieces.iter().map(|p| area(p)).sum();
    let kept_area: f64 = kept.iter().map(|&i| area(&pieces[i])).sum();
    let growth = if kept_area > 0.0 {
        total_area / kept_area
    } else {
        1.0
    };

    let mut out = Thinned {
        triangles: Vec::with_capacity(kept_triangles),
        positions: Vec::new(),
        sources: Vec::new(),
        error: 0.0,
    };
    for &index in &kept {
        let piece = &pieces[index];
        let mut moved: FxHashMap<u32, u32> = FxHashMap::default();
        let vertices: Vec<u32> = {
            let mut v: Vec<u32> = piece
                .iter()
                .flat_map(|&t| triangles[t as usize].map(|c| c.position))
                .collect();
            v.sort_unstable();
            v.dedup();
            v
        };
        let points: Vec<DVec3> = vertices
            .iter()
            .map(|&v| positions[v as usize].as_dvec3())
            .collect();
        let tube = tube_radius(triangles, piece, positions);
        let enlarge = enlargement(triangles, piece, positions, &points, growth);
        // Each vertex's outward direction, from the normals of the corners that use it.
        let mut outward: FxHashMap<u32, DVec3> = FxHashMap::default();
        if tube.is_some() {
            for &t in piece {
                for corner in triangles[t as usize] {
                    *outward.entry(corner.position).or_default() +=
                        attributes[corner.attribute as usize].normal.as_dvec3();
                }
            }
        }
        for (&v, &p) in vertices.iter().zip(&points) {
            let to = match tube {
                // A tube's area grows with its radius, so every point moves out by the same
                // fraction of the average radius.
                Some(radius) => p + outward[&v].normalize_or_zero() * (radius * (growth - 1.0)),
                None => enlarge(p),
            };
            out.error = out.error.max(to.distance(p) as f32);
            moved.insert(v, NEW_POSITION | out.positions.len() as u32);
            out.positions.push(to.as_vec3());
            out.sources.push(v);
        }
        for &t in piece {
            out.triangles
                .push(triangles[t as usize].map(|corner| Corner {
                    position: moved[&corner.position],
                    attribute: corner.attribute,
                }));
        }
    }
    out
}

/// A piece's centre, the directions it is longest and widest in, and how long and wide it is.
struct Shape {
    center: DVec3,
    across: DVec3,
    length: f64,
    width: f64,
}

fn shape(triangles: &[Triangle], piece: &[u32], positions: &[Vec3], points: &[DVec3]) -> Shape {
    let center = points.iter().sum::<DVec3>() / points.len() as f64;
    let mut normal = DVec3::ZERO;
    for &t in piece {
        let [a, b, c] =
            triangles[t as usize].map(|corner| positions[corner.position as usize].as_dvec3());
        normal += (b - a).cross(c - a);
    }
    let normal = normal.normalize_or(DVec3::Y);

    // The direction the piece is longest in, within its own plane.
    let mut covariance = DMat3::ZERO;
    for &p in points {
        let d = p - center;
        let d = d - normal * d.dot(normal);
        covariance += DMat3::from_cols(d * d.x, d * d.y, d * d.z);
    }
    let mut axis = normal.any_orthonormal_vector();
    for _ in 0..16 {
        axis = (covariance * axis).normalize_or(axis);
    }
    let across = normal.cross(axis).normalize_or(DVec3::X);
    let extent = |direction: DVec3| {
        let (lo, hi) = points.iter().fold((f64::MAX, f64::MIN), |(lo, hi), p| {
            let t = (*p - center).dot(direction);
            (lo.min(t), hi.max(t))
        });
        hi - lo
    };
    Shape {
        center,
        across,
        length: extent(axis),
        width: extent(across).max(1.0e-9),
    }
}

fn piece_points(
    triangles: &[Triangle],
    piece: &[u32],
    positions: &[Vec3],
) -> (Vec<u32>, Vec<DVec3>) {
    let mut vertices: Vec<u32> = piece
        .iter()
        .flat_map(|&t| triangles[t as usize].map(|c| c.position))
        .collect();
    vertices.sort_unstable();
    vertices.dedup();
    let points = vertices
        .iter()
        .map(|&v| positions[v as usize].as_dvec3())
        .collect();
    (vertices, points)
}

/// Moves pieces much wider than is typical in their group, such as a branch among twigs, out of
/// `pieces` and into `rest`. Thinning enlarges the survivors, which moves a wide one so far that
/// it would hold back the whole group's detail; edge collapse shortens such a piece instead.
pub fn set_aside_wide_pieces(triangles: &[Triangle], found: &mut Pieces, positions: &[Vec3]) {
    if found.pieces.is_empty() {
        return;
    }
    let widths: Vec<f64> = found
        .pieces
        .iter()
        .map(|piece| match tube_radius(triangles, piece, positions) {
            Some(radius) => radius * 2.0,
            None => {
                let (_, points) = piece_points(triangles, piece, positions);
                shape(triangles, piece, positions, &points).width
            }
        })
        .collect();
    let mut sorted = widths.clone();
    sorted.sort_by(f64::total_cmp);
    let limit = sorted[sorted.len() / 2] * WIDE;
    let mut kept = Vec::with_capacity(found.pieces.len());
    for (piece, width) in found.pieces.drain(..).zip(widths) {
        if width <= limit {
            kept.push(piece);
        } else {
            found.rest.extend(piece);
        }
    }
    if kept.len() < MIN_PIECES {
        found.rest.extend(kept.drain(..).flatten());
    }
    found.pieces = kept;
    found.rest.sort_unstable();
}

/// The average radius of a piece that is a closed tube, or `None` if it is flat. A tube's faces
/// point every way, so their normals all but cancel; a blade's or a leaf's add up.
fn tube_radius(triangles: &[Triangle], piece: &[u32], positions: &[Vec3]) -> Option<f64> {
    let mut sum = DVec3::ZERO;
    let mut area = 0.0;
    let (_, points) = piece_points(triangles, piece, positions);
    for &t in piece {
        let [a, b, c] =
            triangles[t as usize].map(|corner| positions[corner.position as usize].as_dvec3());
        let face = (b - a).cross(c - a);
        sum += face;
        area += face.length() * 0.5;
    }
    if sum.length() * 0.5 > area * 0.3 {
        return None;
    }
    // A tube's area is its circumference times its length.
    let center = points.iter().sum::<DVec3>() / points.len() as f64;
    let mut covariance = DMat3::ZERO;
    for &p in &points {
        let d = p - center;
        covariance += DMat3::from_cols(d * d.x, d * d.y, d * d.z);
    }
    let mut axis = DVec3::new(0.57, 0.58, 0.58);
    for _ in 0..16 {
        axis = (covariance * axis).normalize_or(axis);
    }
    let (lo, hi) = points.iter().fold((f64::MAX, f64::MIN), |(lo, hi), p| {
        let t = (*p - center).dot(axis);
        (lo.min(t), hi.max(t))
    });
    let length = (hi - lo).max(1.0e-9);
    Some(area / (std::f64::consts::TAU * length))
}

/// How to move a piece's vertices so its area grows by `growth`.
fn enlargement(
    triangles: &[Triangle],
    piece: &[u32],
    positions: &[Vec3],
    points: &[DVec3],
    growth: f64,
) -> impl Fn(DVec3) -> DVec3 {
    let Shape {
        center,
        across,
        length,
        width,
        ..
    } = shape(triangles, piece, positions, points);
    let widen = growth.min((length / width).max(1.0));
    let scale = (growth / widen).sqrt();

    move |p: DVec3| {
        let d = p - center;
        center + (d + across * (d.dot(across) * (widen - 1.0))) * scale
    }
}

fn piece_centroid(triangles: &[Triangle], piece: &[u32], positions: &[Vec3]) -> Vec3 {
    let sum: Vec3 = piece
        .iter()
        .flat_map(|&t| triangles[t as usize].map(|c| positions[c.position as usize]))
        .sum();
    sum / (piece.len() * 3) as f32
}

/// A well-mixed number from a position, so survivors are spread evenly rather than in rows.
fn placement_hash(p: Vec3) -> u64 {
    let mut x = (p.x.to_bits() as u64)
        ^ (p.y.to_bits() as u64).rotate_left(21)
        ^ (p.z.to_bits() as u64).rotate_left(42);
    x ^= x >> 30;
    x = x.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x ^= x >> 27;
    x = x.wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A row of `count` separate upright blades, 1 cm wide and 30 cm tall, two triangles each.
    fn blades(count: u32) -> (Vec<Triangle>, Vec<Vec3>) {
        let (mut triangles, mut positions) = (Vec::new(), Vec::new());
        let corner = |position: u32| Corner {
            position,
            attribute: 0,
        };
        for i in 0..count {
            let base = positions.len() as u32;
            let at = Vec3::new(i as f32 * 0.1, 0.0, (i % 7) as f32 * 0.05);
            positions.extend([
                at,
                at + Vec3::X * 0.01,
                at + Vec3::Y * 0.3,
                at + Vec3::new(0.01, 0.3, 0.0),
            ]);
            triangles.push([corner(base), corner(base + 1), corner(base + 2)]);
            triangles.push([corner(base + 1), corner(base + 3), corner(base + 2)]);
        }
        (triangles, positions)
    }

    fn total_area(triangles: &[Triangle], lookup: impl Fn(u32) -> Vec3) -> f32 {
        triangles
            .iter()
            .map(|t| {
                let [a, b, c] = t.map(|corner| lookup(corner.position));
                (b - a).cross(c - a).length() * 0.5
            })
            .sum()
    }

    #[test]
    fn separate_blades_are_pieces_and_a_lone_object_is_not() {
        let (triangles, _) = blades(40);
        let found = find_pieces(&triangles, |_| false);
        assert_eq!(found.pieces.len(), 40);
        assert!(found.rest.is_empty());

        let (few, _) = blades(3);
        let found = find_pieces(&few, |_| false);
        assert!(found.pieces.is_empty());
        assert_eq!(found.rest.len(), 6);
    }

    #[test]
    fn pieces_on_the_group_boundary_stay() {
        let (triangles, _) = blades(40);
        // The first blade's vertices are shared with another group.
        let found = find_pieces(&triangles, |v| v < 4);
        assert_eq!(found.pieces.len(), 39);
        assert_eq!(found.rest, vec![0, 1]);
    }

    #[test]
    fn thinning_halves_the_blades_and_keeps_their_area_and_height() {
        let (triangles, positions) = blades(40);
        let found = find_pieces(&triangles, |_| false);
        let thinned = thin(
            &triangles,
            &found.pieces,
            &positions,
            &[Attribute::default()],
            40,
        );

        assert_eq!(thinned.triangles.len(), 40);
        let before = total_area(&triangles, |v| positions[v as usize]);
        let after = total_area(&thinned.triangles, |v| {
            thinned.positions[(v & !NEW_POSITION) as usize]
        });
        assert!(
            (after / before - 1.0).abs() < 0.01,
            "area went from {before} to {after}"
        );

        // Blades get wider, not taller.
        let heights = thinned.positions.iter().map(|p| p.y);
        let top = heights.fold(0.0, f32::max);
        assert!((top - 0.3).abs() < 1.0e-4, "blades grew to {top}");
        // Doubling a 1 cm blade's width moves each edge out by 0.5 cm.
        assert!(
            (thinned.error - 0.005).abs() < 1.0e-3,
            "error {}",
            thinned.error
        );
    }

    #[test]
    fn the_same_pieces_survive_however_they_are_ordered() {
        let (triangles, positions) = blades(40);
        let found = find_pieces(&triangles, |_| false);
        let mut reversed = found.pieces.clone();
        reversed.reverse();
        let a = thin(
            &triangles,
            &found.pieces,
            &positions,
            &[Attribute::default()],
            40,
        );
        let b = thin(
            &triangles,
            &reversed,
            &positions,
            &[Attribute::default()],
            40,
        );
        let mut pa: Vec<[u32; 3]> = a
            .positions
            .iter()
            .map(|p| p.to_array().map(f32::to_bits))
            .collect();
        let mut pb: Vec<[u32; 3]> = b
            .positions
            .iter()
            .map(|p| p.to_array().map(f32::to_bits))
            .collect();
        pa.sort_unstable();
        pb.sort_unstable();
        assert_eq!(pa, pb);
    }

    /// `count` curved, closed tubes, like twigs: 6 sides, 2 cm across, bowed 10 cm over 50 cm.
    fn twigs(count: u32) -> (Vec<Triangle>, Vec<Vec3>, Vec<Attribute>) {
        const SIDES: u32 = 6;
        const RINGS: u32 = 8;
        let (mut triangles, mut positions, mut attributes) = (Vec::new(), Vec::new(), Vec::new());
        for i in 0..count {
            let base = positions.len() as u32;
            for r in 0..RINGS {
                let t = r as f32 / (RINGS - 1) as f32;
                let center = Vec3::new(i as f32, t * 0.5, (t * std::f32::consts::PI).sin() * 0.1);
                for k in 0..SIDES {
                    let angle = k as f32 / SIDES as f32 * std::f32::consts::TAU;
                    let out = Vec3::new(angle.cos(), 0.0, angle.sin());
                    positions.push(center + out * 0.01);
                    attributes.push(Attribute {
                        normal: out,
                        uv: glam::Vec2::ZERO,
                    });
                }
            }
            for r in 0..RINGS - 1 {
                for k in 0..SIDES {
                    let a = base + r * SIDES + k;
                    let b = base + r * SIDES + (k + 1) % SIDES;
                    let (c, d) = (b + SIDES, a + SIDES);
                    let corner = |v: u32| Corner {
                        position: v,
                        attribute: v,
                    };
                    triangles.push([corner(a), corner(d), corner(c)]);
                    triangles.push([corner(a), corner(c), corner(b)]);
                }
            }
        }
        (triangles, positions, attributes)
    }

    #[test]
    fn tubes_grow_thicker_and_keep_their_curve() {
        let (triangles, positions, attributes) = twigs(20);
        let found = find_pieces(&triangles, |_| false);
        let target = triangles.len() / 2;
        let thinned = thin(&triangles, &found.pieces, &positions, &attributes, target);
        // Doubling a 1 cm radius moves the surface out by 1 cm, and no more.
        assert!(thinned.error < 0.012, "a twig moved {} m", thinned.error);
        let before = total_area(&triangles, |v| positions[v as usize]);
        let after = total_area(&thinned.triangles, |v| {
            thinned.positions[(v & !NEW_POSITION) as usize]
        });
        assert!(
            (after / before - 1.0).abs() < 0.05,
            "area went from {before} to {after}"
        );
    }

    #[test]
    fn round_pieces_scale_evenly() {
        // Small square pebbles lying flat.
        let corner = |position: u32| Corner {
            position,
            attribute: 0,
        };
        let (mut triangles, mut positions) = (Vec::new(), Vec::new());
        for i in 0..20 {
            let base = positions.len() as u32;
            let at = Vec3::new(i as f32, 0.0, 0.0);
            positions.extend([
                at,
                at + Vec3::X * 0.1,
                at + Vec3::Z * 0.1,
                at + Vec3::new(0.1, 0.0, 0.1),
            ]);
            triangles.push([corner(base), corner(base + 2), corner(base + 1)]);
            triangles.push([corner(base + 1), corner(base + 2), corner(base + 3)]);
        }
        let found = find_pieces(&triangles, |_| false);
        let thinned = thin(
            &triangles,
            &found.pieces,
            &positions,
            &[Attribute::default()],
            20,
        );
        let piece = &thinned.positions[0..4];
        let width = piece.iter().map(|p| p.x).fold(f32::MIN, f32::max)
            - piece.iter().map(|p| p.x).fold(f32::MAX, f32::min);
        let depth = piece.iter().map(|p| p.z).fold(f32::MIN, f32::max)
            - piece.iter().map(|p| p.z).fold(f32::MAX, f32::min);
        assert!((width - 0.1 * 2f32.sqrt()).abs() < 1.0e-3, "width {width}");
        assert!(
            (depth - width).abs() < 1.0e-4,
            "depth {depth} against width {width}"
        );
    }
}
