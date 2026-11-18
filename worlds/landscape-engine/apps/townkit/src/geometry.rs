//! Closed solids, built up into one list of triangles.

use glam::{Vec2, Vec3};

/// Triangles with their normals and UVs. UVs are in metres, so that a material's scale means the
/// same thing on every piece.
#[derive(Clone, Default)]
pub struct Geometry {
    pub positions: Vec<Vec3>,
    pub normals: Vec<Vec3>,
    pub uvs: Vec<Vec2>,
    pub indices: Vec<u32>,
}

/// Projects a point along the axis its surface faces most directly.
fn planar_uv(position: Vec3, normal: Vec3) -> Vec2 {
    let facing = normal.abs();
    if facing.y >= facing.x && facing.y >= facing.z {
        Vec2::new(position.x, position.z)
    } else if facing.x >= facing.z {
        Vec2::new(position.z, position.y)
    } else {
        Vec2::new(position.x, position.y)
    }
}

/// The area-weighted normal of a polygon, by its winding.
fn newell(corners: &[Vec3]) -> Vec3 {
    let mut sum = Vec3::ZERO;
    for (i, &a) in corners.iter().enumerate() {
        sum += a.cross(corners[(i + 1) % corners.len()]);
    }
    sum.normalize_or_zero()
}

impl Geometry {
    fn vertex(&mut self, position: Vec3, normal: Vec3) -> u32 {
        self.positions.push(position);
        self.normals.push(normal);
        self.uvs.push(planar_uv(position, normal));
        self.positions.len() as u32 - 1
    }

    /// Adds a flat polygon, facing whichever way is closer to `outward`. It is fanned from its
    /// first corner, which must be able to see all the others.
    pub fn polygon(&mut self, corners: &[Vec3], outward: Vec3) {
        let mut normal = newell(corners);
        let mut ordered = corners.to_vec();
        if normal.dot(outward) < 0.0 {
            ordered[1..].reverse();
            normal = -normal;
        }
        let first = self.positions.len() as u32;
        for &corner in &ordered {
            self.vertex(corner, normal);
        }
        for i in 1..ordered.len() as u32 - 1 {
            self.indices.extend([first, first + i, first + i + 1]);
        }
    }

    pub fn cuboid(&mut self, min: Vec3, max: Vec3) {
        let at = |x: bool, y: bool, z: bool| {
            Vec3::new(
                if x { max.x } else { min.x },
                if y { max.y } else { min.y },
                if z { max.z } else { min.z },
            )
        };
        for axis in 0..3 {
            for high in [false, true] {
                let corner = |a: bool, b: bool| match axis {
                    0 => at(high, a, b),
                    1 => at(a, high, b),
                    _ => at(a, b, high),
                };
                let outward = Vec3::AXES[axis] * if high { 1.0 } else { -1.0 };
                self.polygon(
                    &[
                        corner(false, false),
                        corner(true, false),
                        corner(true, true),
                        corner(false, true),
                    ],
                    outward,
                );
            }
        }
    }

    /// A closed solid swept through `rings`, each a cross-section with the same number of corners.
    /// Hard-edged around each ring and smooth from ring to ring, so a curve has no facets. The ends
    /// are capped by fans from each ring's first corner.
    pub fn loft(&mut self, rings: &[Vec<Vec3>]) {
        let corners = rings[0].len();
        let centre = |ring: &[Vec3]| ring.iter().copied().sum::<Vec3>() / corners as f32;
        let quad = |i: usize, j: usize| {
            let next = (j + 1) % corners;
            [
                rings[i][j],
                rings[i + 1][j],
                rings[i + 1][next],
                rings[i][next],
            ]
        };
        let raw_normal = |i: usize, j: usize| {
            let [a, b, c, d] = quad(i, j);
            (c - a).cross(d - b)
        };
        // Which way round the rings are wound, from whether the quads' normals point away from
        // the middle.
        let facing: f32 = (0..corners)
            .map(|j| {
                let [a, b, c, d] = quad(0, j);
                raw_normal(0, j).dot((a + b + c + d) * 0.25 - centre(&rings[0]))
            })
            .sum();
        let sign = if facing < 0.0 { -1.0 } else { 1.0 };

        for i in 0..rings.len() - 1 {
            for j in 0..corners {
                let at_ring = |ring: usize| {
                    let mut sum = Vec3::ZERO;
                    if ring > 0 {
                        sum += raw_normal(ring - 1, j);
                    }
                    if ring + 1 < rings.len() {
                        sum += raw_normal(ring, j);
                    }
                    (sum * sign).normalize_or_zero()
                };
                let [a, b, c, d] = quad(i, j);
                let first = [
                    self.vertex(a, at_ring(i)),
                    self.vertex(b, at_ring(i + 1)),
                    self.vertex(c, at_ring(i + 1)),
                    self.vertex(d, at_ring(i)),
                ];
                if sign > 0.0 {
                    self.indices
                        .extend([first[0], first[1], first[2], first[0], first[2], first[3]]);
                } else {
                    self.indices
                        .extend([first[0], first[2], first[1], first[0], first[3], first[2]]);
                }
            }
        }
        let start = centre(&rings[0]) - centre(&rings[1]);
        let last = rings.len() - 1;
        let end = centre(&rings[last]) - centre(&rings[last - 1]);
        self.polygon(&rings[0], start);
        self.polygon(&rings[last], end);
    }

    /// A round solid standing on `base`, tapering from `bottom` to `top` radius.
    pub fn frustum(&mut self, base: Vec3, bottom: f32, top: f32, height: f32, sides: u32) {
        let ring = |radius: f32, y: f32| -> Vec<Vec3> {
            (0..sides)
                .map(|i| {
                    let angle = i as f32 / sides as f32 * std::f32::consts::TAU;
                    base + Vec3::new(angle.cos() * radius, y, angle.sin() * radius)
                })
                .collect()
        };
        let (lower, upper) = (ring(bottom, 0.0), ring(top, height));
        let slope = (bottom - top) / height;
        let normal = |corner: Vec3| {
            let out = (corner - base) * Vec3::new(1.0, 0.0, 1.0);
            (out.normalize_or_zero() + Vec3::Y * slope).normalize()
        };
        for i in 0..sides as usize {
            let next = (i + 1) % sides as usize;
            let a = self.vertex(lower[i], normal(lower[i]));
            let b = self.vertex(upper[i], normal(upper[i]));
            let c = self.vertex(upper[next], normal(upper[next]));
            let d = self.vertex(lower[next], normal(lower[next]));
            self.indices.extend([a, b, c, a, c, d]);
        }
        self.polygon(&lower, -Vec3::Y);
        self.polygon(&upper, Vec3::Y);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// The volume a closed mesh encloses, negative if it is inside out.
    fn volume(g: &Geometry) -> f32 {
        g.indices
            .chunks_exact(3)
            .map(|t| {
                let [a, b, c] = [0, 1, 2].map(|i| g.positions[t[i] as usize]);
                a.dot(b.cross(c)) / 6.0
            })
            .sum()
    }

    /// Whether every edge, once equal corners are joined, has one triangle on each side.
    fn is_closed(g: &Geometry) -> bool {
        let key = |i: u32| {
            (g.positions[i as usize] * 1000.0)
                .round()
                .as_ivec3()
                .to_array()
        };
        let mut edges: HashMap<([i32; 3], [i32; 3]), i32> = HashMap::new();
        for t in g.indices.chunks_exact(3) {
            for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
                let (a, b) = (key(a), key(b));
                if a == b {
                    continue;
                }
                let (edge, side) = if a < b { ((a, b), 1) } else { ((b, a), -1) };
                *edges.entry(edge).or_default() += side;
            }
        }
        edges.values().all(|&sum| sum == 0)
    }

    #[test]
    fn a_cuboid_is_closed_and_the_right_way_out() {
        let mut g = Geometry::default();
        g.cuboid(Vec3::ZERO, Vec3::new(2.0, 3.0, 4.0));
        assert!(is_closed(&g));
        assert!((volume(&g) - 24.0).abs() < 1.0e-3);
    }

    #[test]
    fn a_frustum_is_closed_and_the_right_way_out() {
        let mut g = Geometry::default();
        g.frustum(Vec3::new(5.0, 1.0, -3.0), 2.0, 1.0, 4.0, 32);
        assert!(is_closed(&g));
        // A cone frustum holds pi h (R^2 + R r + r^2) / 3, a little less as a 32-sided prism.
        assert!((volume(&g) - 29.3).abs() < 0.3, "{}", volume(&g));
    }

    #[test]
    fn a_loft_is_closed_and_the_right_way_out_whichever_way_it_is_wound() {
        let square = |z: f32, flip: bool| {
            let mut ring = vec![
                Vec3::new(0.0, 0.0, z),
                Vec3::new(1.0, 0.0, z),
                Vec3::new(1.0, 1.0, z),
                Vec3::new(0.0, 1.0, z),
            ];
            if flip {
                ring.reverse();
            }
            ring
        };
        for flip in [false, true] {
            let mut g = Geometry::default();
            g.loft(&[square(0.0, flip), square(1.0, flip), square(3.0, flip)]);
            assert!(is_closed(&g));
            assert!((volume(&g) - 3.0).abs() < 1.0e-3, "{}", volume(&g));
        }
    }

    #[test]
    fn a_polygon_faces_the_way_asked() {
        let corners = [Vec3::ZERO, Vec3::X, Vec3::X + Vec3::Z, Vec3::Z];
        for outward in [Vec3::Y, -Vec3::Y] {
            let mut g = Geometry::default();
            g.polygon(&corners, outward);
            assert_eq!(g.normals[0], outward);
        }
    }
}
