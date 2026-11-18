//! Trees and bushes, grown from a [`Species`] description.
//!
//! A plant is a trunk that carries levels of branches, each level spiralling around its parent at
//! the golden angle as real shoots do, and the last level carries leaves. Every stem is a tube of
//! its own, starting inside its parent as branches do in production tree models. That keeps each
//! one a separate piece, which the cluster builder can thin out with distance like a leaf.
//!
//! The crown's outline comes from how long the trunk's branches are at each height, which is how
//! species differ most at a glance: a rounded oak, a narrow birch, a wide bush. Conifers carry
//! their branches in whorls, rings of several at one height, rather than in a spiral.
//!
//! Each species names its [`Bark`] and [`LeafKind`], which decide both the textures to use and
//! the outline each leaf is built with.

use super::bark::BARK_REPEAT;
use super::leaf::LEAF_VARIANTS;
pub use super::species::{Crown, Leaves, Level, Species};
use super::Rng;
use crate::field::smoothstep;
use glam::{Quat, Vec2, Vec3};
use solstice_builder::{Mesh, Vertex};
use std::f32::consts::TAU;

/// The golden angle, at which successive shoots spiral around a stem.
const GOLDEN_ANGLE: f32 = 2.399_963;

/// A grown plant: wood and leaves, which take different materials.
pub struct Plant {
    pub wood: Mesh,
    pub leaves: Mesh,
    /// The youngest shoots, for a species whose [`Species::twigs`] are another colour than its
    /// bark. Empty otherwise.
    pub twigs: Mesh,
}

/// A stem as a curve: points along it, the radius at each, and the direction it grows there.
struct Stem {
    points: Vec<Vec3>,
    radii: Vec<f32>,
    length: f32,
}

impl Stem {
    /// Point, direction and radius at `t` of the way along.
    fn at(&self, t: f32) -> (Vec3, Vec3, f32) {
        let segments = self.points.len() - 1;
        let x = (t.clamp(0.0, 1.0) * segments as f32).min(segments as f32 - 1.0e-4);
        let i = x as usize;
        let f = x - i as f32;
        let point = self.points[i].lerp(self.points[i + 1], f);
        let direction = (self.points[i + 1] - self.points[i]).normalize_or(Vec3::Y);
        let radius = self.radii[i] + (self.radii[i + 1] - self.radii[i]) * f;
        (point, direction, radius)
    }
}

/// How finely a stem is divided along its length, in metres: about three of its widths, so that
/// bends stay smooth, but never so fine that twigs cost more than the leaves they carry.
fn ring_spacing(radius: f32, length: f32) -> f32 {
    // The thinnest twigs are all but straight between one leaf and the next.
    let finest = if radius < 0.008 { 0.2 } else { 0.12 };
    (radius * 6.0).clamp(finest, 0.6).min(length / 2.0)
}

/// As [`ring_spacing`], for a shoot that hangs: it turns over in its first few hand's lengths
/// and is straight from there down, so it needs no more rings than show that turn.
fn hanging_ring_spacing(length: f32) -> f32 {
    0.3_f32.min(length / 2.0)
}

/// Height above the plant's ground below which no hanging shoot grows: it stops there.
const HANG_FLOOR: f32 = 0.12;

/// Grows one stem by marching along it, turning a little at each step.
#[allow(clippy::too_many_arguments)]
fn grow(
    rng: &mut Rng,
    start: Vec3,
    direction: Vec3,
    length: f32,
    radius: f32,
    tip_radius: f32,
    level: &Level,
) -> Stem {
    let step = if level.reach > 0.0 {
        hanging_ring_spacing(length)
    } else {
        ring_spacing(radius, length)
    };
    let segments = ((length / step).ceil() as usize).max(2);
    let step = length / segments as f32;
    let mut points = vec![start];
    let mut radii = vec![radius];
    let mut direction = direction.normalize();
    let mut point = start;
    let mut grown = 0.0;
    for i in 1..=segments {
        let t = i as f32 / segments as f32;
        let wander = Vec3::new(rng.signed(), rng.signed() * 0.5, rng.signed()) * level.gnarl;
        direction = (direction
            + (wander + Vec3::Y * level.upturn - Vec3::Y * level.droop * t * 2.0) * step)
            .normalize();
        point += direction * step;
        // A hanging shoot ends where it would reach the ground.
        if level.reach > 0.0 && point.y < HANG_FLOOR && i > 2 {
            break;
        }
        grown += step;
        points.push(point);
        radii.push(radius + (tip_radius - radius) * t.powf(0.8));
    }
    if let Some(last) = radii.last_mut() {
        *last = last.min(tip_radius.max(0.0015));
    }
    Stem {
        points,
        radii,
        length: grown,
    }
}

impl Species {
    /// Grows one plant. Different seeds give different plants of the same species.
    pub fn grow(&self, seed: u32) -> Plant {
        let mut rng = Rng::new(seed, 7);
        let trunk_level = Level {
            upturn: 0.02,
            gnarl: self.trunk_gnarl,
            ..Level::default()
        };
        let lean = Vec3::new(rng.signed(), 0.0, rng.signed()) * 0.05;
        let trunk = grow(
            &mut rng,
            Vec3::ZERO - Vec3::Y * self.trunk_radius,
            Vec3::Y + lean,
            self.height + self.trunk_radius,
            self.trunk_radius,
            self.trunk_radius * 0.25,
            &trunk_level,
        );

        let mut stems = vec![trunk];
        let mut generation = vec![0usize];
        for (depth, level) in self.levels.iter().enumerate() {
            let mut next = Vec::new();
            for &parent_index in &generation {
                let parent_length = stems[parent_index].length;
                let span = level.span.1 - level.span.0;
                let count = (parent_length * span * level.per_metre).round().max(1.0) as usize;
                let mut positions = shoot_positions(level, count, &mut rng);
                // A branch does not end in a bare point: its tip carries on as a shoot of the
                // next level, nearly straight ahead. The trunk's own tip is its leader.
                if depth > 0 && level.span.1 >= 0.95 && level.whorl == 0 {
                    positions.push((0.985, rng.range(0.0, TAU)));
                }
                let last = positions.len() - 1;
                for (index, (t, spin)) in positions.into_iter().enumerate() {
                    let terminal = depth > 0 && index == last && t > 0.98;
                    let (point, along, parent_radius) = stems[parent_index].at(t);
                    let side = perpendicular(along, spin);
                    let along_span = ((t - level.span.0) / span.max(1.0e-6)).clamp(0.0, 1.0);
                    let angle = if terminal {
                        rng.range(4.0, 16.0)
                    } else {
                        level.angle.0
                            + (level.angle.1 - level.angle.0) * along_span
                            + rng.signed() * 10.0
                    };
                    let angle = angle.to_radians();
                    let direction = along * angle.cos() + side * angle.sin();
                    let mut length = parent_length * level.length * rng.range(0.75, 1.2);
                    if depth == 0 {
                        length *= self.crown.envelope(along_span);
                    } else {
                        length *= 1.0 - level.taper * along_span;
                    }
                    if level.reach > 0.0 {
                        // It hangs: as long as takes it down to where the curtain ends, ragged
                        // by half as much again, and no longer than its kind grows.
                        let drop = point.y - level.reach * rng.range(0.55, 1.6);
                        length = length.min(drop.max(0.25));
                    }
                    if length < 0.03 {
                        continue;
                    }
                    let radius = (parent_radius * level.radius).max(0.003);
                    let stem = grow(
                        &mut rng,
                        point,
                        direction,
                        length,
                        radius,
                        (radius * 0.2).max(0.0015),
                        level,
                    );
                    next.push(stems.len());
                    stems.push(stem);
                }
            }
            generation = next;
        }

        let mut wood = Mesh::default();
        let mut twigs = Mesh::default();
        let twig_radius = self.twigs.map_or(0.0, |(below, _)| below);
        for (index, stem) in stems.iter().enumerate() {
            let flare = if index == 0 { self.root_flare } else { 0.0 };
            if stem.radii[0] < twig_radius {
                add_tube(&mut twigs, stem, 0.0, &mut rng);
            } else {
                add_tube(&mut wood, stem, flare, &mut rng);
            }
        }

        let crown_center = crown_center(&stems, &generation);
        let mut leaves = Mesh::default();
        for &index in &generation {
            add_leaves(
                &mut leaves,
                &stems[index],
                &self.leaves,
                crown_center,
                &mut rng,
            );
        }
        Plant {
            wood,
            leaves,
            twigs,
        }
    }
}

/// Where shoots leave their parent: `(fraction along it, angle around it)` for each of `count`.
fn shoot_positions(level: &Level, count: usize, rng: &mut Rng) -> Vec<(f32, f32)> {
    let span = level.span.1 - level.span.0;
    let mut positions = Vec::with_capacity(count);
    if level.whorl > 0 {
        // Rings of shoots at one height, each ring turned against the last.
        let per_ring = level.whorl as usize;
        let rings = count.div_ceil(per_ring).max(1);
        for ring in 0..rings {
            let t = level.span.0 + span * (ring as f32 + rng.range(0.35, 0.65)) / rings as f32;
            let start = rng.range(0.0, TAU);
            for k in 0..per_ring {
                let spin = start + (k as f32 + rng.signed() * 0.15) / per_ring as f32 * TAU;
                positions.push((t + rng.signed() * 0.004, spin));
            }
        }
    } else {
        let mut spin = rng.range(0.0, TAU);
        for k in 0..count {
            spin += GOLDEN_ANGLE;
            positions.push((
                level.span.0 + span * (k as f32 + rng.range(0.2, 0.8)) / count as f32,
                spin,
            ));
        }
    }
    positions
}

/// A unit vector perpendicular to `axis`, turned `angle` around it.
fn perpendicular(axis: Vec3, angle: f32) -> Vec3 {
    let reference = if axis.y.abs() < 0.9 { Vec3::Y } else { Vec3::X };
    let first = axis.cross(reference).normalize();
    Quat::from_axis_angle(axis, angle) * first
}

/// The middle of where the leaves are.
fn crown_center(stems: &[Stem], leafy: &[usize]) -> Vec3 {
    let mut sum = Vec3::ZERO;
    let mut count = 0.0;
    for &index in leafy {
        for &p in &stems[index].points {
            sum += p;
            count += 1.0;
        }
    }
    if count > 0.0 {
        sum / count
    } else {
        Vec3::ZERO
    }
}

/// A stem's surface: rings of vertices along its curve, closing to a point at its tip.
fn add_tube(mesh: &mut Mesh, stem: &Stem, root_flare: f32, rng: &mut Rng) {
    // About 3.5 cm to a side, so a twig is a prism and a great trunk is round against the sky.
    let sides = (((stem.radii[0] * TAU / 0.035) as usize).clamp(3, 16)
        + ((stem.radii[0] - 0.09).max(0.0) * 60.0) as usize)
        .min(40);
    // A thick stem is never a true cylinder: it swells and hollows along its length, and bulges
    // where old limbs were. Too slight to see on a twig, so thin stems go without.
    let thick = smoothstep(0.06, 0.2, stem.radii[0]);
    let bulges = (rng.range(0.0, 100.0), rng.range(0.0, 100.0));
    // Bark wraps a whole number of times, so its seam matches.
    let around = (stem.radii[0] * TAU / BARK_REPEAT).round().max(1.0) * BARK_REPEAT;
    let lobes = rng.range(0.0, TAU);
    let base = mesh.vertices.len() as u32;

    // Frames carried along the curve without twisting.
    let mut normal = perpendicular((stem.points[1] - stem.points[0]).normalize_or(Vec3::Y), 0.0);
    let mut distance = 0.0;
    let rings = stem.points.len();
    for i in 0..rings {
        let tangent = if i + 1 < rings {
            (stem.points[i + 1] - stem.points[i]).normalize_or(Vec3::Y)
        } else {
            (stem.points[i] - stem.points[i - 1]).normalize_or(Vec3::Y)
        };
        if i > 0 {
            distance += stem.points[i].distance(stem.points[i - 1]);
        }
        normal = (normal - tangent * normal.dot(tangent)).normalize_or(perpendicular(tangent, 0.0));
        let binormal = tangent.cross(normal);
        let tip = i + 1 == rings;
        // The trunk spreads into buttresses where it meets the ground.
        let height = stem.points[i].y;
        let flare = root_flare * (1.0 - smoothstep(0.0, 1.2, height)).powi(2);
        for j in 0..=sides {
            let theta = j as f32 / sides as f32 * TAU;
            let buttress = 1.0 + flare * (0.6 + 0.4 * (theta * 5.0 + lobes).cos());
            let outward = normal * theta.cos() + binormal * theta.sin();
            let lump = if thick > 0.0 {
                // Read round a circle, so the swelling has no seam.
                let (c, s) = (theta.cos(), theta.sin());
                let broad = Vec3::new(c * 1.3, distance * 0.9 + bulges.0, s * 1.3);
                let fine = Vec3::new(c * 3.1, distance * 2.6 + bulges.1, s * 3.1);
                1.0 + thick
                    * (0.075 * crate::noise::fbm(broad, 3, 97)
                        + 0.03 * crate::noise::fbm(fine, 2, 98))
            } else {
                1.0
            };
            let radius = if tip {
                0.0
            } else {
                stem.radii[i] * buttress * lump
            };
            mesh.vertices.push(Vertex {
                position: stem.points[i] + outward * radius,
                normal: outward,
                uv: Vec2::new(j as f32 / sides as f32 * around, distance),
            });
        }
    }
    let row = sides as u32 + 1;
    for i in 0..rings as u32 - 1 {
        for j in 0..sides as u32 {
            let a = base + i * row + j;
            let (b, c, d) = (a + 1, a + row + 1, a + row);
            mesh.indices.extend_from_slice(&[a, b, c, a, c, d]);
        }
    }
}

/// Leaves along a stem, alternating around it, with a few at its tip.
fn add_leaves(mesh: &mut Mesh, stem: &Stem, leaves: &Leaves, crown: Vec3, rng: &mut Rng) {
    let count = (stem.length * leaves.per_metre).round().max(1.0) as usize;
    let total: f32 = leaves.variants.iter().sum();
    let mut spin = rng.range(0.0, TAU);
    for k in 0..count {
        // Mostly towards the tip, where the newest growth is.
        let t = 0.25 + 0.75 * ((k as f32 + rng.unit()) / count as f32).powf(0.7);
        let (point, along, radius) = stem.at(t);
        spin += GOLDEN_ANGLE;
        let side = perpendicular(along, spin);
        let out = (along * leaves.lean + side).normalize();
        let length = rng.range(leaves.length.0, leaves.length.1);
        let mut pick = rng.unit() * total;
        let mut variant = 0;
        while variant + 1 < LEAF_VARIANTS as usize && pick >= leaves.variants[variant] {
            pick -= leaves.variants[variant];
            variant += 1;
        }
        // Leaves turn their faces to the sky, give or take.
        let roll = rng.signed() * 0.6;
        add_leaf(
            mesh,
            point + side * radius,
            out,
            length,
            leaves,
            roll,
            variant as u32,
            crown,
        );
    }
}

/// Angle between a needle and its shoot.
const NEEDLE_ANGLE: f32 = 0.95;
/// Distance between fans of needles along each side of a shoot, in metres.
const NEEDLE_SPACING: f32 = 0.03;

/// A shoot's needles, splayed out either side of it in a flat spray, as on a fir. Each triangle
/// is a fan of several needles: as wide as the spacing where it leaves the shoot, so the fans
/// cover the shoot between them, and drawn to a point. The atlas draws the needles in each
/// fan. Fans are separate pieces, so the cluster builder thins them with distance and widens
/// the ones it keeps, like any other leaf.
#[allow(clippy::too_many_arguments)]
fn add_needle_spray(
    mesh: &mut Mesh,
    base: Vec3,
    axis: Vec3,
    length: f32,
    leaves: &Leaves,
    roll: f32,
    variant: u32,
    crown: Vec3,
) {
    let up_ish = (Vec3::Y - axis * axis.dot(Vec3::Y)).normalize_or(perpendicular(axis, 0.0));
    let face = Quat::from_axis_angle(axis, roll) * up_ish;
    let side = axis.cross(face).normalize();
    let curl = |s: f32| -face * (s * s * length * leaves.kind.curl());
    let lit = |at: Vec3| {
        let out = (at - crown).normalize_or(face);
        (face * (1.0 - leaves.crown_normals) + out * leaves.crown_normals).normalize()
    };
    let u = |across: f32| (variant as f32 + 0.5 + across * 0.47) / LEAF_VARIANTS as f32;
    let per_side = ((length / NEEDLE_SPACING).round() as usize).max(2);
    for i in 0..per_side {
        let s = (i as f32 + 0.5) / per_side as f32;
        let root = base + axis * (s * length) + curl(s);
        // Needles shorten towards the shoot's tip, where they are newest.
        let needle = length * leaves.width * (1.0 - 0.45 * s);
        for sign in [1.0, -1.0] {
            // Alternate sides are offset half a spacing, as needles spiral round a real shoot.
            let root = root + axis * (sign * 0.25 * NEEDLE_SPACING);
            let direction = (axis * NEEDLE_ANGLE.cos() + side * (sign * NEEDLE_ANGLE.sin())
                - face * 0.12)
                .normalize();
            let tip = root + direction * needle;
            let first = mesh.vertices.len() as u32;
            let half = NEEDLE_SPACING * 0.5;
            for (position, across, v) in [
                (root - axis * half, -sign, 0.0),
                (root + axis * half, sign, 0.0),
                (tip, 0.0, 1.0),
            ] {
                mesh.vertices.push(Vertex {
                    position,
                    normal: lit(position),
                    uv: Vec2::new(u(across), v),
                });
            }
            // Counter-clockwise seen from the face, whichever side the needle is on.
            if sign > 0.0 {
                mesh.indices
                    .extend_from_slice(&[first, first + 2, first + 1]);
            } else {
                mesh.indices
                    .extend_from_slice(&[first, first + 1, first + 2]);
            }
        }
    }
}

/// One leaf: a stalk point, two rows folded along the midrib, and a tip, curling down a little.
#[allow(clippy::too_many_arguments)]
fn add_leaf(
    mesh: &mut Mesh,
    base: Vec3,
    axis: Vec3,
    length: f32,
    leaves: &Leaves,
    roll: f32,
    variant: u32,
    crown: Vec3,
) {
    let kind = leaves.kind;
    let crown_normals = leaves.crown_normals;
    if kind.is_spray() {
        add_needle_spray(mesh, base, axis, length, leaves, roll, variant, crown);
        return;
    }
    let up_ish = (Vec3::Y - axis * axis.dot(Vec3::Y)).normalize_or(perpendicular(axis, 0.0));
    let face = Quat::from_axis_angle(axis, roll) * up_ish;
    let side = axis.cross(face).normalize();
    let width = length * leaves.width;
    let half_width = |s: f32| width * kind.half_width(s);
    let curl = |s: f32| -face * (s * s * length * kind.curl());
    let u = |across: f32| (variant as f32 + 0.5 + across * 0.47) / LEAF_VARIANTS as f32;
    let lit = |n: Vec3, at: Vec3| {
        let out = (at - crown).normalize_or(n);
        (n * (1.0 - crown_normals) + out * crown_normals).normalize()
    };

    let first = mesh.vertices.len() as u32;
    mesh.vertices.push(Vertex {
        position: base,
        normal: lit(face, base),
        uv: Vec2::new(u(0.0), 0.0),
    });
    let rows = kind.rows();
    for &s in rows {
        let center = base + axis * (s * length) + curl(s);
        let half = half_width(s);
        let fold = face * (half * kind.fold());
        for (across, offset, tilt) in [
            (-1.0, -side * half, side),
            (0.0, -fold, Vec3::ZERO),
            (1.0, side * half, -side),
        ] {
            let position = center + offset;
            let normal = (face + tilt * 0.35).normalize();
            mesh.vertices.push(Vertex {
                position,
                normal: lit(normal, position),
                uv: Vec2::new(u(across * half / (width * 0.5)), s),
            });
        }
    }
    let tip = base + axis * length + curl(1.0);
    mesh.vertices.push(Vertex {
        position: tip,
        normal: lit(face, tip),
        uv: Vec2::new(u(0.0), 1.0),
    });

    // Counter-clockwise seen from the face: the stalk's two triangles, two pairs between each
    // row and the next, and the tip's two.
    let stalk = first;
    let row = |k: u32| (first + 1 + k * 3, first + 2 + k * 3, first + 3 + k * 3);
    let count = rows.len() as u32;
    let point = first + 1 + count * 3;
    let (l0, c0, r0) = row(0);
    mesh.indices
        .extend_from_slice(&[stalk, c0, l0, stalk, r0, c0]);
    for k in 0..count - 1 {
        let (l0, c0, r0) = row(k);
        let (l1, c1, r1) = row(k + 1);
        mesh.indices
            .extend_from_slice(&[l0, c0, c1, l0, c1, l1, c0, r0, r1, c0, r1, c1]);
    }
    let (l1, c1, r1) = row(count - 1);
    mesh.indices
        .extend_from_slice(&[l1, c1, point, c1, r1, point]);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bounds(mesh: &Mesh) -> (Vec3, Vec3) {
        mesh.vertices
            .iter()
            .fold((Vec3::MAX, Vec3::MIN), |(lo, hi), v| {
                (lo.min(v.position), hi.max(v.position))
            })
    }

    #[test]
    fn a_broadleaf_tree_has_the_shape_and_size_asked_for() {
        let species = Species::oak();
        let plant = species.grow(0);
        let (lo, hi) = bounds(&plant.wood);
        assert!(
            hi.y > species.height * 0.9 && hi.y < species.height * 1.4,
            "top at {}",
            hi.y
        );
        // A rounded crown is wide, but narrower than it is tall.
        let spread = (hi.x - lo.x).max(hi.z - lo.z);
        assert!(
            spread > species.height * 0.35 && spread < species.height * 1.3,
            "spread {spread}"
        );
        let leaves = plant.leaves.triangle_count() / 8;
        assert!((8_000..60_000).contains(&leaves), "{leaves} leaves");
        let wood = plant.wood.triangle_count();
        assert!(wood < 120_000, "{wood} wood triangles");
    }

    #[test]
    fn surfaces_face_the_way_their_triangles_wind() {
        let plant = Species::oak().grow(1);
        for mesh in [&plant.wood, &plant.leaves] {
            let mut wrong = 0;
            let mut total = 0;
            for triangle in mesh.indices.chunks_exact(3) {
                let [a, b, c] = [0, 1, 2].map(|i| mesh.vertices[triangle[i] as usize]);
                let face = (b.position - a.position).cross(c.position - a.position);
                if face.length() < 1.0e-12 {
                    continue;
                }
                total += 1;
                if face.dot(a.normal + b.normal + c.normal) <= 0.0 {
                    wrong += 1;
                }
            }
            assert!(
                wrong * 100 < total,
                "{wrong} of {total} triangles face against their normals"
            );
        }
    }

    #[test]
    fn the_same_seed_grows_the_same_plant() {
        let species = Species::bush();
        assert_eq!(
            species.grow(3).leaves.vertices,
            species.grow(3).leaves.vertices
        );
        assert_ne!(
            species.grow(3).leaves.vertices,
            species.grow(4).leaves.vertices
        );
    }

    #[test]
    fn every_species_grows_to_its_height_within_budget() {
        for name in Species::names() {
            let species = Species::by_name(name).unwrap();
            let plant = species.grow(0);
            let (lo, hi) = bounds(&plant.wood);
            let spread = (hi.x - lo.x).max(hi.z - lo.z);
            let leaves = plant.leaves.triangle_count();
            let wood = plant.wood.triangle_count();
            println!(
                "{name}: top {:.1} m, spread {spread:.1} m, {leaves} leaf and {wood} wood triangles",
                hi.y
            );
            assert!(hi.y > species.height * 0.85, "{name} top at {}", hi.y);
            // A specimen tree, an old oak or a weeping willow, is grown to be stood under: it
            // has several times the leaves of a tree of the wood.
            assert!(
                leaves > 16_000 && leaves < 1_400_000,
                "{name}: {leaves} leaf triangles"
            );
            assert!(wood < 320_000, "{name}: {wood} wood triangles");
        }
        assert!(Species::by_name("baobab").is_none());
    }

    #[test]
    fn a_fir_is_a_narrow_cone_and_a_birch_is_slimmer_than_an_oak() {
        let shape = |species: Species| {
            let plant = species.grow(2);
            let (lo, hi) = bounds(&plant.leaves);
            let width = (hi.x - lo.x).max(hi.z - lo.z);
            // Width of the crown in its top third against its bottom third.
            let third = |from: f32, to: f32| {
                let (mut a, mut b) = (f32::MAX, f32::MIN);
                for v in &plant.leaves.vertices {
                    let y = (v.position.y - lo.y) / (hi.y - lo.y);
                    if y >= from && y < to {
                        a = a.min(v.position.x);
                        b = b.max(v.position.x);
                    }
                }
                b - a
            };
            (width / hi.y, third(0.67, 1.0) / third(0.0, 0.33))
        };
        let (fir_ratio, fir_taper) = shape(Species::fir());
        let (birch_ratio, _) = shape(Species::birch());
        let (oak_ratio, _) = shape(Species::oak());
        assert!(fir_taper < 0.6, "fir tapers to {fir_taper}");
        assert!(fir_ratio < 0.7, "fir is {fir_ratio} as wide as tall");
        assert!(
            birch_ratio < oak_ratio,
            "birch {birch_ratio}, oak {oak_ratio}"
        );
    }
}
