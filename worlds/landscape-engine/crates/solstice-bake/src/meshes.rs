//! Procedural high-density meshes. Geometric detail is kept in the mesh rather than in
//! normal maps wherever possible, since dense source geometry is what virtualized
//! geometry exists to make affordable.

use glam::{Vec2, Vec3};
use rustc_hash::FxHashMap;
use solstice_builder::{Mesh, Vertex};
use solstice_procgen::noise::{fbm, random, ridged};
use std::f32::consts::TAU;

/// An indexed surface before UVs are assigned.
#[derive(Default)]
pub struct Surface {
    positions: Vec<Vec3>,
    faces: Vec<[u32; 3]>,
}

impl Surface {
    /// Finishes the surface: smooth normals across coincident vertices, then box-mapped
    /// UVs in world units. Box mapping needs a UV seam wherever the dominant axis of a
    /// face changes. The builder tracks topology on positions, so seams cost nothing.
    pub fn into_mesh(self) -> Mesh {
        let (positions, faces) = weld(self.positions, self.faces);
        let mut normals = vec![Vec3::ZERO; positions.len()];
        for face in &faces {
            let [a, b, c] = face.map(|i| i as usize);
            let n = (positions[b] - positions[a]).cross(positions[c] - positions[a]);
            normals[a] += n;
            normals[b] += n;
            normals[c] += n;
        }

        let mut mesh = Mesh {
            vertices: Vec::with_capacity(faces.len() * 3),
            indices: Vec::with_capacity(faces.len() * 3),
        };
        for face in &faces {
            let [a, b, c] = face.map(|i| i as usize);
            let geometric = (positions[b] - positions[a]).cross(positions[c] - positions[a]);
            let axis = geometric.abs();
            let project = |p: Vec3| {
                if axis.x >= axis.y && axis.x >= axis.z {
                    Vec2::new(p.z, p.y)
                } else if axis.y >= axis.z {
                    Vec2::new(p.x, p.z)
                } else {
                    Vec2::new(p.x, p.y)
                }
            };
            for index in [a, b, c] {
                mesh.indices.push(mesh.vertices.len() as u32);
                mesh.vertices.push(Vertex {
                    position: positions[index],
                    normal: normals[index].normalize_or(Vec3::Y),
                    uv: project(positions[index]),
                });
            }
        }
        mesh
    }

    /// Appends a `columns` by `rows` grid of quads. `at(u, v)` gives the positions and
    /// `wrap` closes the grid in U. The front face is on the side of `dV x dU`.
    fn add_grid(&mut self, columns: u32, rows: u32, wrap: bool, at: impl Fn(f32, f32) -> Vec3) {
        let base = self.positions.len() as u32;
        let stride = if wrap { columns } else { columns + 1 };
        for row in 0..=rows {
            let v = row as f32 / rows as f32;
            for column in 0..stride {
                self.positions.push(at(column as f32 / columns as f32, v));
            }
        }
        let index = |column: u32, row: u32| base + row * stride + column % stride;
        for row in 0..rows {
            for column in 0..columns {
                let (a, b) = (index(column, row), index(column + 1, row));
                let (c, d) = (index(column, row + 1), index(column + 1, row + 1));
                self.faces.push([a, c, b]);
                self.faces.push([b, c, d]);
            }
        }
    }
}

/// Merges coincident positions so normals smooth across separately generated patches.
fn weld(positions: Vec<Vec3>, faces: Vec<[u32; 3]>) -> (Vec<Vec3>, Vec<[u32; 3]>) {
    let mut remap = FxHashMap::default();
    let mut unique = Vec::with_capacity(positions.len());
    let map: Vec<u32> = positions
        .iter()
        .map(|p| {
            let key = (*p * 8192.0).round().as_ivec3().to_array();
            *remap.entry(key).or_insert_with(|| {
                unique.push(*p);
                unique.len() as u32 - 1
            })
        })
        .collect();
    let welded = faces
        .into_iter()
        .map(|f| f.map(|i| map[i as usize]))
        .filter(|f| f[0] != f[1] && f[1] != f[2] && f[2] != f[0])
        .collect();
    (unique, welded)
}

fn icosphere(subdivisions: u32) -> Surface {
    let t = (1.0 + 5.0_f32.sqrt()) / 2.0;
    let mut positions: Vec<Vec3> = [
        [-1.0, t, 0.0],
        [1.0, t, 0.0],
        [-1.0, -t, 0.0],
        [1.0, -t, 0.0],
        [0.0, -1.0, t],
        [0.0, 1.0, t],
        [0.0, -1.0, -t],
        [0.0, 1.0, -t],
        [t, 0.0, -1.0],
        [t, 0.0, 1.0],
        [-t, 0.0, -1.0],
        [-t, 0.0, 1.0],
    ]
    .map(|p| Vec3::from_array(p).normalize())
    .to_vec();
    let mut faces: Vec<[u32; 3]> = vec![
        [0, 11, 5],
        [0, 5, 1],
        [0, 1, 7],
        [0, 7, 10],
        [0, 10, 11],
        [1, 5, 9],
        [5, 11, 4],
        [11, 10, 2],
        [10, 7, 6],
        [7, 1, 8],
        [3, 9, 4],
        [3, 4, 2],
        [3, 2, 6],
        [3, 6, 8],
        [3, 8, 9],
        [4, 9, 5],
        [2, 4, 11],
        [6, 2, 10],
        [8, 6, 7],
        [9, 8, 1],
    ];
    for _ in 0..subdivisions {
        let mut midpoints: FxHashMap<(u32, u32), u32> = FxHashMap::default();
        let mut next = Vec::with_capacity(faces.len() * 4);
        for [a, b, c] in faces {
            let mut midpoint = |i: u32, j: u32| {
                *midpoints.entry((i.min(j), i.max(j))).or_insert_with(|| {
                    positions.push((positions[i as usize] + positions[j as usize]).normalize());
                    positions.len() as u32 - 1
                })
            };
            let (ab, bc, ca) = (midpoint(a, b), midpoint(b, c), midpoint(c, a));
            next.extend([[a, ab, ca], [b, bc, ab], [c, ca, bc], [ab, bc, ca]]);
        }
        faces = next;
    }
    Surface { positions, faces }
}

/// A fractured boulder about one unit in radius.
///
/// Random cutting planes give it large flat faces, which read as broken rock far better
/// than the blobby look of noise alone. Ridged noise then adds erosion at every scale.
pub fn rock(subdivisions: u32, seed: u32) -> Mesh {
    let mut surface = icosphere(subdivisions);
    let cuts: Vec<(Vec3, f32)> = (0..14)
        .map(|i| {
            let n = Vec3::new(
                random(i, seed) * 2.0 - 1.0,
                random(i, seed + 1) * 2.0 - 1.0,
                random(i, seed + 2) * 2.0 - 1.0,
            )
            .normalize_or(Vec3::Y);
            (n, 0.62 + random(i, seed + 3) * 0.3)
        })
        .collect();
    let squash = Vec3::new(
        0.8 + random(0, seed + 4) * 0.5,
        0.55 + random(0, seed + 5) * 0.4,
        0.8 + random(0, seed + 6) * 0.5,
    );

    for p in &mut surface.positions {
        let direction = *p;
        let mut point = direction;
        for &(normal, distance) in &cuts {
            let over = point.dot(normal) - distance;
            if over > 0.0 {
                point -= normal * over;
            }
        }
        let coarse = fbm(direction * 2.3, 4, seed) * 0.12;
        let crease = (ridged(direction * 5.0, 8, seed + 7) - 0.55) * 0.09;
        let grit = fbm(direction * 90.0, 3, seed + 20) * 0.0035;
        *p = (point + direction * (coarse + crease + grit)) * squash;
    }
    surface.into_mesh()
}

/// A mesh of four-cornered pieces, each given as its corners in order round it. Each faces the
/// way `facing` says for a point in it: a road's pieces face up.
pub fn quads(pieces: &[[Vec3; 4]], facing: impl Fn(Vec3) -> Vec3) -> Mesh {
    let mut surface = Surface::default();
    for piece in pieces {
        let base = surface.positions.len() as u32;
        surface.positions.extend_from_slice(piece);
        let normal = (piece[1] - piece[0]).cross(piece[2] - piece[0]);
        let (b, d) = if normal.dot(facing(piece[0])) >= 0.0 {
            (1, 3)
        } else {
            (3, 1)
        };
        surface.faces.push([base, base + b, base + 2]);
        surface.faces.push([base, base + 2, base + d]);
    }
    surface.into_mesh()
}

/// A square patch of terrain, `size` wide with `cells` quads per side, sampling `height`.
pub fn terrain(size: f32, cells: u32, height: impl Fn(f32, f32) -> f32) -> Mesh {
    let mut surface = Surface::default();
    let half = size * 0.5;
    surface.add_grid(cells, cells, false, |u, v| {
        let (x, z) = (u * size - half, v * size - half);
        Vec3::new(x, height(x, z), z)
    });
    surface.into_mesh()
}

/// A weathered, fluted column of the given height, standing on the origin.
pub fn column(height: f32, radial: u32, rows: u32, seed: u32) -> Mesh {
    /// Radius of the turned profile at `t` in `0..1` along the height, and how much
    /// of the fluting and square footprint applies there.
    fn profile(t: f32) -> (f32, f32, f32) {
        let smooth = |a: f32, b: f32, x: f32| {
            let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
            t * t * (3.0 - 2.0 * t)
        };
        // Tapered shaft with entasis: a slight swelling that corrects a perceived waist.
        let shaft = 0.62 - 0.14 * t + 0.022 * (t * std::f32::consts::PI).sin();
        let echinus = smooth(0.885, 0.955, t) * 0.26;
        let abacus = smooth(0.955, 0.958, t) * 0.05;
        let fluting = 1.0 - smooth(0.87, 0.885, t);
        let square = smooth(0.953, 0.957, t);
        (shaft + echinus + abacus, fluting, square)
    }

    let side = |u: f32, v: f32| {
        let theta = u * TAU;
        let (sin, cos) = theta.sin_cos();
        let (radius, fluting, square) = profile(v);

        // Twenty concave flutes separated by sharp arrises.
        let phase = (u * 20.0).fract() - 0.5;
        let flute = (1.0 - (phase * 2.0) * (phase * 2.0)).max(0.0).sqrt() * 0.045 * fluting;

        // Morph to a square slab at the abacus.
        let to_square = 1.0 / cos.abs().max(sin.abs());
        let shaped = radius * (1.0 + (to_square - 1.0) * square);

        let y = v * height;
        let direction = Vec3::new(cos, 0.0, sin);
        let surface_point = direction * (shaped - flute) + Vec3::Y * y;

        // Weathering: broad erosion, sharp chips near drum joints, and fine pitting.
        let sample = surface_point * 1.1;
        let joint = ((y / 1.15).fract() - 0.5).abs() * 2.0;
        let groove = ((joint - 0.985) / 0.015).clamp(0.0, 1.0) * 0.012 * fluting;
        let erosion = fbm(sample, 5, seed) * 0.018;
        let chips = (ridged(sample * 2.2, 6, seed + 3) - 0.75).max(0.0) * 0.16;
        let pits = fbm(surface_point * 38.0, 3, seed + 9) * 0.0028;
        surface_point + direction * (erosion - chips - groove + pits)
    };

    let mut surface = Surface::default();
    surface.add_grid(radial, rows, true, side);

    // Close both ends so the mesh stays manifold and casts sound shadows. The caps reuse
    // the side's own rim, displacement included, so that they weld to it exactly.
    surface.add_grid(radial, 8, true, |u, v| Vec3::ZERO.lerp(side(u, 0.0), v));
    surface.add_grid(radial, 8, true, |u, v| {
        side(u, 1.0).lerp(Vec3::Y * height, v)
    });
    surface.into_mesh()
}

/// A dressed stone block filling `-0.5..0.5` on each axis, with softened and chipped
/// edges. Instances scale it to different proportions.
pub fn stone_block(cells: u32, seed: u32) -> Mesh {
    const ROUND: f32 = 0.022;
    let inner = 0.5 - ROUND;

    let shape = move |p: Vec3| {
        // Project the box surface outward from an inner box: a rounded box.
        let clamped = p.clamp(Vec3::splat(-inner), Vec3::splat(inner));
        let outward = (p - clamped).normalize_or(p.normalize_or(Vec3::Y));
        let rounded = clamped + outward * ROUND;

        // Chips bite hardest along edges, where real masonry gets damaged.
        let second_largest = {
            let a = rounded.abs();
            let max = a.max_element();
            let min = a.min_element();
            a.x + a.y + a.z - max - min
        };
        let edge_factor = ((second_largest - 0.38) / 0.12).clamp(0.0, 1.0);
        let chips = (ridged(rounded * 5.5, 6, seed) - 0.62).max(0.0) * (0.05 + 0.34 * edge_factor);
        let tooling = fbm(rounded * 7.0, 4, seed + 5) * 0.006;
        let grit = fbm(rounded * 70.0, 3, seed + 11) * 0.0018;
        rounded + outward * (tooling + grit - chips)
    };

    let mut surface = Surface::default();
    // One patch per face. Each `at` maps (u, v) so that the normal points outward.
    let faces: [(Vec3, Vec3, Vec3); 6] = [
        (Vec3::X, Vec3::Z, Vec3::Y),
        (Vec3::NEG_X, Vec3::NEG_Z, Vec3::Y),
        (Vec3::Y, Vec3::X, Vec3::Z),
        (Vec3::NEG_Y, Vec3::Z, Vec3::X),
        (Vec3::Z, Vec3::NEG_X, Vec3::Y),
        (Vec3::NEG_Z, Vec3::X, Vec3::Y),
    ];
    for (normal, tangent, bitangent) in faces {
        surface.add_grid(cells, cells, false, move |u, v| {
            let p = normal * 0.5 + tangent * (u - 0.5) + bitangent * (v - 0.5);
            shape(p)
        });
    }
    surface.into_mesh()
}

/// A plain sphere, for lamps and test objects.
pub fn sphere(subdivisions: u32) -> Mesh {
    icosphere(subdivisions).into_mesh()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every edge shared by exactly two faces, in consistent opposite directions.
    fn assert_closed_and_consistent(mesh: &Mesh) {
        let key = |i: u32| {
            (mesh.vertices[i as usize].position * 8192.0)
                .round()
                .as_ivec3()
                .to_array()
        };
        let mut directed: FxHashMap<([i32; 3], [i32; 3]), i32> = FxHashMap::default();
        for t in mesh.indices.chunks_exact(3) {
            for side in 0..3 {
                *directed
                    .entry((key(t[side]), key(t[(side + 1) % 3])))
                    .or_insert(0) += 1;
            }
        }
        for (&(a, b), &count) in &directed {
            let reverse = directed.get(&(b, a)).copied().unwrap_or(0);
            assert_eq!(count, 1, "edge used {count} times in one direction");
            assert_eq!(
                reverse, 1,
                "edge has no opposite: the surface is open or inverted"
            );
        }
    }

    /// Outward-facing triangles enclose positive signed volume.
    fn signed_volume(mesh: &Mesh) -> f32 {
        mesh.indices
            .chunks_exact(3)
            .map(|t| {
                let [a, b, c] = [t[0], t[1], t[2]].map(|i| mesh.vertices[i as usize].position);
                a.dot(b.cross(c)) / 6.0
            })
            .sum()
    }

    #[test]
    fn rock_is_closed_and_faces_outward() {
        let mesh = rock(3, 1);
        assert_closed_and_consistent(&mesh);
        assert!(signed_volume(&mesh) > 0.0);
    }

    #[test]
    fn block_is_closed_and_faces_outward() {
        let mesh = stone_block(16, 1);
        assert_closed_and_consistent(&mesh);
        assert!(
            signed_volume(&mesh) > 0.5,
            "volume was {}",
            signed_volume(&mesh)
        );
    }

    #[test]
    fn column_is_closed_and_faces_outward() {
        let mesh = column(7.0, 40, 60, 1);
        assert_closed_and_consistent(&mesh);
        assert!(
            signed_volume(&mesh) > 1.0,
            "volume was {}",
            signed_volume(&mesh)
        );
    }

    #[test]
    fn terrain_faces_upward() {
        let mesh = terrain(10.0, 4, |_, _| 0.0);
        for t in mesh.indices.chunks_exact(3) {
            let [a, b, c] = [t[0], t[1], t[2]].map(|i| mesh.vertices[i as usize].position);
            assert!((b - a).cross(c - a).y > 0.0);
        }
    }
}
