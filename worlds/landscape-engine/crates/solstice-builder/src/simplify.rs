//! Quadric-error simplification by half-edge collapse.
//!
//! Half-edge collapses are used rather than optimal placement: every surviving
//! vertex is an original vertex, so the 16-bit position grid stays valid and no
//! new attributes have to be invented.
//!
//! Topology is tracked on welded *positions*, while attributes live on triangle
//! corners. A UV seam or hard edge is therefore not a topological boundary: the
//! surface collapses straight across it and each corner keeps a matching attribute.
//!
//! Flat surfaces have no geometric error to speak of, so shape alone would let them collapse
//! without limit, and report that nothing changed, while the texture across them slides and
//! smears. So each collapse also measures how far the texture moves, in world units, and that
//! counts towards both its cost and the reported error. The renderer then keeps finer clusters
//! until the movement is too small to see.

use glam::{DVec2, DVec3, Vec2, Vec3};
use rustc_hash::FxHashMap;
use solstice_asset::PackedSkin;
use std::cell::RefCell;
use std::cmp::Ordering;
use std::collections::BinaryHeap;

/// Shading attributes of a welded vertex.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Attribute {
    pub normal: Vec3,
    pub uv: Vec2,
}

/// A triangle corner: which welded position and which attribute it uses.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Corner {
    pub position: u32,
    pub attribute: u32,
}

pub type Triangle = [Corner; 3];

/// Result of [`simplify`].
pub struct Simplified {
    pub triangles: Vec<Triangle>,
    /// Largest geometric deviation introduced, in world units.
    pub error: f32,
}

/// Rejects a collapse that would rotate a face by more than about 78 degrees.
const MIN_NORMAL_AGREEMENT: f64 = 0.2;
/// Weight of the edge-length term that breaks ties on flat regions, relative to
/// squared length. Without it flat areas collapse in arbitrary order into slivers.
const EDGE_LENGTH_WEIGHT: f64 = 1.0e-4;
/// How much a normal discontinuity across an edge raises its collapse cost.
const NORMAL_DIFFERENCE_WEIGHT: f64 = 0.02;
/// How much a texture displacement counts, relative to moving the surface by the same distance.
/// A silhouette changing is more noticeable than a pattern moving a little within a surface.
const TEXTURE_ERROR_WEIGHT: f64 = 0.5;
/// Largest distance outside a triangle, in triangle sizes, to which its texture mapping is
/// trusted. Slivers would otherwise extrapolate wildly.
const MAX_EXTRAPOLATION: f64 = 16.0;

/// Reduces `triangles` towards `target_triangles`.
///
/// Vertices for which `is_locked` returns true never move, and neither do the ends
/// of any edge that is not shared by exactly two triangles.
#[cfg(test)]
pub fn simplify(
    triangles: &[Triangle],
    positions: &[Vec3],
    attributes: &[Attribute],
    target_triangles: usize,
    is_locked: impl Fn(u32) -> bool,
) -> Simplified {
    simplify_skinned(
        triangles,
        positions,
        attributes,
        None,
        target_triangles,
        is_locked,
    )
}

/// What [`simplify_skinned`] needs of a skinned mesh.
#[derive(Clone, Copy)]
pub struct SkinCost<'a> {
    /// Each welded position's skin.
    pub skins: &'a [PackedSkin],
    /// How far, in world units, a vertex may land from where it belongs in a pose if its weights
    /// change completely: the error a change of weights counts for.
    pub error_per_change: f32,
}

/// [`simplify`] for a mesh that bends: a collapse also pays for changing the weights of what it
/// moves, so joints, where weights change fastest, are kept longer than a flat surface would be.
pub fn simplify_skinned(
    triangles: &[Triangle],
    positions: &[Vec3],
    attributes: &[Attribute],
    skin: Option<SkinCost<'_>>,
    target_triangles: usize,
    is_locked: impl Fn(u32) -> bool,
) -> Simplified {
    Simplifier::new(triangles, positions, attributes, skin, &is_locked).run(target_triangles)
}

/// [`simplify_skinned`] towards each of several targets, in one run: the result at each, in the
/// order given, is what simplifying to it alone gives.
pub fn simplify_skinned_each(
    triangles: &[Triangle],
    positions: &[Vec3],
    attributes: &[Attribute],
    skin: Option<SkinCost<'_>>,
    targets: &[usize],
    is_locked: impl Fn(u32) -> bool,
) -> Vec<Simplified> {
    // One run reaches the largest target first, so they are taken largest first and handed back
    // in the order asked for.
    let mut order: Vec<usize> = (0..targets.len()).collect();
    order.sort_by_key(|&i| std::cmp::Reverse(targets[i]));
    let descending: Vec<usize> = order.iter().map(|&i| targets[i]).collect();
    let results = Simplifier::new(triangles, positions, attributes, skin, &is_locked)
        .run_to_each(&descending);
    let mut out: Vec<Option<Simplified>> = (0..targets.len()).map(|_| None).collect();
    for (result, i) in results.into_iter().zip(order) {
        out[i] = Some(result);
    }
    out.into_iter()
        .map(|r| r.expect("one result for each target"))
        .collect()
}

/// A symmetric 4x4 error quadric plus the accumulated area weight.
#[derive(Clone, Copy, Default)]
struct Quadric {
    aa: f64,
    ab: f64,
    ac: f64,
    ad: f64,
    bb: f64,
    bc: f64,
    bd: f64,
    cc: f64,
    cd: f64,
    dd: f64,
    weight: f64,
}

impl Quadric {
    fn from_plane(normal: DVec3, d: f64, weight: f64) -> Self {
        let (a, b, c) = (normal.x, normal.y, normal.z);
        Self {
            aa: a * a * weight,
            ab: a * b * weight,
            ac: a * c * weight,
            ad: a * d * weight,
            bb: b * b * weight,
            bc: b * c * weight,
            bd: b * d * weight,
            cc: c * c * weight,
            cd: c * d * weight,
            dd: d * d * weight,
            weight,
        }
    }

    fn add(&mut self, o: &Self) {
        self.aa += o.aa;
        self.ab += o.ab;
        self.ac += o.ac;
        self.ad += o.ad;
        self.bb += o.bb;
        self.bc += o.bc;
        self.bd += o.bd;
        self.cc += o.cc;
        self.cd += o.cd;
        self.dd += o.dd;
        self.weight += o.weight;
    }

    /// Weighted mean squared distance from `p` to the accumulated planes.
    fn mean_squared_distance(&self, p: DVec3) -> f64 {
        if self.weight <= 0.0 {
            return 0.0;
        }
        let value = self.aa * p.x * p.x
            + 2.0 * self.ab * p.x * p.y
            + 2.0 * self.ac * p.x * p.z
            + 2.0 * self.ad * p.x
            + self.bb * p.y * p.y
            + 2.0 * self.bc * p.y * p.z
            + 2.0 * self.bd * p.y
            + self.cc * p.z * p.z
            + 2.0 * self.cd * p.z
            + self.dd;
        value.max(0.0) / self.weight
    }
}

/// A candidate collapse of `from` into `to`, valid while both stamps match.
struct Candidate {
    cost: f64,
    from: u32,
    to: u32,
    from_stamp: u32,
    to_stamp: u32,
}

impl PartialEq for Candidate {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}
impl Eq for Candidate {}
impl PartialOrd for Candidate {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Candidate {
    /// Reversed on cost so that `BinaryHeap` pops the cheapest collapse. Vertex ids
    /// break ties so the result does not depend on heap internals.
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .cost
            .total_cmp(&self.cost)
            .then_with(|| other.from.cmp(&self.from))
            .then_with(|| other.to.cmp(&self.to))
    }
}

/// What is known about the triangles around a vertex, worked out once and kept until they change.
/// A collapse's cost asks for these of the same few vertices tens of times, and working them out
/// afresh each time was nearly all of a build's time.
#[derive(Clone, Default)]
struct Around {
    /// Whether this holds anything yet.
    ready: bool,
    /// The vertex's version this was worked out at.
    version: u32,
    /// Neighbouring vertices, sorted.
    neighbors: Vec<u32>,
    /// The attributes the vertex's corners use, sorted.
    wedges: Vec<u32>,
    /// The neighbours across a UV seam, sorted.
    seams: Vec<u32>,
    /// The shading normal of the vertex's first corner.
    normal: Vec3,
}

struct Simplifier<'a> {
    attributes: &'a [Attribute],
    skin: Option<SkinCost<'a>>,
    /// Pose error built up at each local vertex by earlier collapses, in world units.
    skin_errors: Vec<f64>,
    /// Global position id per local vertex.
    global: Vec<u32>,
    positions: Vec<DVec3>,
    locked: Vec<bool>,
    quadrics: Vec<Quadric>,
    stamps: Vec<u32>,
    /// Texture displacement built up at each local vertex by earlier collapses, in world units.
    texture_errors: Vec<f64>,
    /// Alive triangles touching each local vertex.
    vertex_triangles: Vec<Vec<u32>>,
    /// Triangles whose corners' `position` holds local vertex ids.
    triangles: Vec<Triangle>,
    alive: Vec<bool>,
    alive_count: usize,
    heap: BinaryHeap<Candidate>,
    max_error: f64,
    /// Raised whenever the triangles around a vertex change, or the corners of one of them.
    versions: Vec<u32>,
    /// What is known around each vertex, kept while its version holds.
    around: RefCell<Vec<Around>>,
}

impl<'a> Simplifier<'a> {
    fn new(
        input: &[Triangle],
        positions: &[Vec3],
        attributes: &'a [Attribute],
        skin: Option<SkinCost<'a>>,
        is_locked: &dyn Fn(u32) -> bool,
    ) -> Self {
        let mut remap: FxHashMap<u32, u32> = FxHashMap::default();
        let mut global = Vec::new();
        let triangles: Vec<Triangle> = input
            .iter()
            .map(|t| {
                t.map(|corner| {
                    let local = *remap.entry(corner.position).or_insert_with(|| {
                        global.push(corner.position);
                        global.len() as u32 - 1
                    });
                    Corner {
                        position: local,
                        attribute: corner.attribute,
                    }
                })
            })
            .collect();

        let vertex_count = global.len();
        let local_positions: Vec<DVec3> = global
            .iter()
            .map(|&g| positions[g as usize].as_dvec3())
            .collect();
        let mut locked: Vec<bool> = global.iter().map(|&g| is_locked(g)).collect();
        let mut vertex_triangles = vec![Vec::new(); vertex_count];
        let mut quadrics = vec![Quadric::default(); vertex_count];
        let mut edge_uses: FxHashMap<u64, u32> = FxHashMap::default();

        for (index, t) in triangles.iter().enumerate() {
            let [a, b, c] = t.map(|corner| corner.position);
            let (pa, pb, pc) = (
                local_positions[a as usize],
                local_positions[b as usize],
                local_positions[c as usize],
            );
            let cross = (pb - pa).cross(pc - pa);
            let double_area = cross.length();
            if double_area > 0.0 {
                let normal = cross / double_area;
                let quadric = Quadric::from_plane(normal, -normal.dot(pa), double_area * 0.5);
                for v in [a, b, c] {
                    quadrics[v as usize].add(&quadric);
                }
            }
            for (v, next) in [(a, b), (b, c), (c, a)] {
                vertex_triangles[v as usize].push(index as u32);
                let key = u64::from(v.min(next)) << 32 | u64::from(v.max(next));
                *edge_uses.entry(key).or_insert(0) += 1;
            }
        }

        // Borders and non-manifold edges stay fixed. A border here is either the
        // group's outer boundary or a genuine hole in the source mesh.
        for (key, uses) in edge_uses {
            if uses != 2 {
                locked[(key >> 32) as usize] = true;
                locked[(key & 0xffff_ffff) as usize] = true;
            }
        }

        let alive_count = triangles.len();
        let alive = vec![true; alive_count];
        Self {
            attributes,
            skin,
            skin_errors: vec![0.0; vertex_count],
            global,
            positions: local_positions,
            locked,
            quadrics,
            stamps: vec![0; vertex_count],
            texture_errors: vec![0.0; vertex_count],
            vertex_triangles,
            triangles,
            alive,
            alive_count,
            heap: BinaryHeap::new(),
            max_error: 0.0,
            versions: vec![0; vertex_count],
            around: RefCell::new(vec![Around::default(); vertex_count]),
        }
    }

    /// In tests, after each collapse: everything kept as up to date is what working it out
    /// afresh gives, so a change that should have raised a version and didn't is caught.
    #[cfg(test)]
    fn check_kept(&self) {
        let around = self.around.borrow();
        for (v, kept) in around.iter().enumerate() {
            if !kept.ready || kept.version != self.versions[v] {
                continue;
            }
            let v = v as u32;
            let neighbors = self.find_neighbors(v);
            assert_eq!(kept.neighbors, neighbors, "neighbours of {v}");
            assert_eq!(kept.wedges, self.find_wedges(v), "wedges of {v}");
            assert_eq!(
                kept.seams,
                self.find_seam_neighbors(v, &neighbors),
                "seams of {v}"
            );
            assert_eq!(kept.normal, self.find_normal(v), "normal of {v}");
        }
    }

    /// Makes sure what is known around `v` is up to date.
    fn fresh(&self, v: u32) {
        let version = self.versions[v as usize];
        {
            let around = self.around.borrow();
            let entry = &around[v as usize];
            if entry.ready && entry.version == version {
                return;
            }
        }
        let neighbors = self.find_neighbors(v);
        let seams = self.find_seam_neighbors(v, &neighbors);
        let entry = Around {
            ready: true,
            version,
            wedges: self.find_wedges(v),
            normal: self.find_normal(v),
            neighbors,
            seams,
        };
        self.around.borrow_mut()[v as usize] = entry;
    }

    /// Simplifies towards each of `targets`, largest first, and gives the result at each. The
    /// collapses come in the same order whatever the target, so each result is what a run to it
    /// alone would give.
    fn run_to_each(mut self, targets: &[usize]) -> Vec<Simplified> {
        assert!(targets.windows(2).all(|w| w[0] >= w[1]), "targets descend");
        for v in 0..self.positions.len() as u32 {
            self.push_edges_around(v, &|n| n < v);
        }
        let mut results = Vec::with_capacity(targets.len());
        for &target in targets {
            self.collapse_to(target);
            results.push(self.snapshot());
        }
        results
    }

    /// Collapses until no more than `target` triangles are left, or nothing more can be.
    fn collapse_to(&mut self, target: usize) {
        while self.alive_count > target {
            let Some(candidate) = self.heap.pop() else {
                break;
            };
            if self.stamps[candidate.from as usize] != candidate.from_stamp
                || self.stamps[candidate.to as usize] != candidate.to_stamp
            {
                continue;
            }
            if self.can_collapse(candidate.from, candidate.to) {
                self.collapse(candidate.from, candidate.to);
            }
        }
    }

    fn run(self, target: usize) -> Simplified {
        self.run_to_each(&[target]).pop().expect("one target")
    }

    fn neighbors(&self, v: u32) -> Vec<u32> {
        self.fresh(v);
        self.around.borrow()[v as usize].neighbors.clone()
    }

    fn find_neighbors(&self, v: u32) -> Vec<u32> {
        let mut result: Vec<u32> = self.vertex_triangles[v as usize]
            .iter()
            .flat_map(|&t| self.triangles[t as usize])
            .map(|corner| corner.position)
            .filter(|&p| p != v)
            .collect();
        result.sort_unstable();
        result.dedup();
        result
    }

    /// Queues both collapses of each edge from `v`, but those to neighbours `skip` passes over,
    /// which are queued from the other end: an edge queued twice would be popped twice, to the
    /// same effect, so once is the same simplification for half the work.
    fn push_edges_around(&mut self, v: u32, skip: &dyn Fn(u32) -> bool) {
        for neighbor in self.neighbors(v) {
            if skip(neighbor) {
                continue;
            }
            for (from, to) in [(v, neighbor), (neighbor, v)] {
                if !self.locked[from as usize] {
                    let candidate = Candidate {
                        cost: self.collapse_cost(from, to),
                        from,
                        to,
                        from_stamp: self.stamps[from as usize],
                        to_stamp: self.stamps[to as usize],
                    };
                    self.heap.push(candidate);
                }
            }
        }
    }

    fn collapse_cost(&self, from: u32, to: u32) -> f64 {
        let mut quadric = self.quadrics[from as usize];
        quadric.add(&self.quadrics[to as usize]);
        let target = self.positions[to as usize];
        let length_squared = self.positions[from as usize].distance_squared(target);
        let texture = self.texture_error(from, to) * TEXTURE_ERROR_WEIGHT;
        let skin = self.skin_error(from, to);
        quadric.mean_squared_distance(target)
            + texture * texture
            + skin * skin
            + length_squared
                * (EDGE_LENGTH_WEIGHT + NORMAL_DIFFERENCE_WEIGHT * self.normal_difference(from, to))
    }

    /// How far from its place a vertex could land in a pose if `from` took on `to`'s weights,
    /// including what earlier collapses already cost at `from`.
    fn skin_error(&self, from: u32, to: u32) -> f64 {
        let Some(skin) = self.skin else {
            return 0.0;
        };
        let of = |v: u32| skin.skins[self.global[v as usize] as usize];
        f64::from(of(from).difference(of(to)) * skin.error_per_change)
            + self.skin_errors[from as usize]
    }

    /// How far the texture would slide across the surface if `from` collapsed into `to`, in world
    /// units, including what earlier collapses already moved at `from`.
    fn texture_error(&self, from: u32, to: u32) -> f64 {
        self.fresh(from);
        self.fresh(to);
        let around = self.around.borrow();
        let wedges = &around[to as usize].wedges;
        let worst = self.vertex_triangles[from as usize]
            .iter()
            .map(|&t| self.triangles[t as usize])
            .filter(|t| !t.iter().any(|corner| corner.position == to))
            .map(|t| self.moved_corner(&t, from, to, wedges).1)
            .fold(0.0, f64::max);
        let seams = &around[from as usize].seams;
        worst.max(self.seam_movement(seams, from, to)) + self.texture_errors[from as usize]
    }

    /// How far the collapse moves a UV seam. Each side's mapping can stay perfectly continuous
    /// while one side grows over the other, replacing what the surface showed there.
    fn seam_movement(&self, seams: &[u32], from: u32, to: u32) -> f64 {
        let start = self.positions[from as usize];
        let end = self.positions[to as usize];
        match *seams {
            [] => 0.0,
            // Sliding along a seam only moves it as far as the seam bends here.
            [a, b] if a == to || b == to => {
                let other = self.positions[(if a == to { b } else { a }) as usize];
                distance_to_segment(start, other, end)
            }
            // Leaving the seam, or the end of one or a junction of several, which must stay put.
            _ => start.distance(end),
        }
    }

    /// The neighbours of `v` connected by an edge whose two sides use different textures.
    fn find_seam_neighbors(&self, v: u32, neighbors: &[u32]) -> Vec<u32> {
        let uv_of = |triangle: &Triangle, vertex: u32| {
            triangle
                .iter()
                .find(|corner| corner.position == vertex)
                .map(|corner| self.attributes[corner.attribute as usize].uv)
        };
        let mut seams = neighbors.to_vec();
        seams.retain(|&n| {
            let mut sides = self.vertex_triangles[v as usize]
                .iter()
                .map(|&t| &self.triangles[t as usize])
                .filter(|t| t.iter().any(|corner| corner.position == n));
            let (Some(first), Some(second)) = (sides.next(), sides.next()) else {
                return false;
            };
            uv_of(first, v) != uv_of(second, v) || uv_of(first, n) != uv_of(second, n)
        });
        seams
    }

    /// The attribute that the corner of `triangle` at `from` should take on moving to `to`, and
    /// how far the texture slides as a result.
    ///
    /// What the corner should show there is found by continuing the triangle's own texture
    /// mapping out to `to`. On a surface with a continuous mapping, a wedge at `to` matches that
    /// exactly. Across a UV seam none does, which is what the returned distance reveals.
    fn moved_corner(&self, triangle: &Triangle, from: u32, to: u32, wedges: &[u32]) -> (u32, f64) {
        let index = triangle
            .iter()
            .position(|corner| corner.position == from)
            .expect("triangle touches the vertex being moved");
        let current = triangle[index].attribute;
        let uv_of = |corner: Corner| self.attributes[corner.attribute as usize].uv.as_dvec2();
        let (next, last) = (triangle[(index + 1) % 3], triangle[(index + 2) % 3]);
        let origin = self.positions[from as usize];
        let edge_a = self.positions[next.position as usize] - origin;
        let edge_b = self.positions[last.position as usize] - origin;
        let offset = self.positions[to as usize] - origin;
        let uv_origin = self.attributes[current as usize].uv.as_dvec2();
        let uv_a = uv_of(next) - uv_origin;
        let uv_b = uv_of(last) - uv_origin;

        let (aa, ab, bb) = (edge_a.dot(edge_a), edge_a.dot(edge_b), edge_b.dot(edge_b));
        let determinant = aa * bb - ab * ab;
        let uv_area = uv_a.perp_dot(uv_b).abs();
        if determinant <= 1.0e-12 * aa * bb || uv_area <= 0.0 {
            // No usable mapping here, as for a sliver or a mesh without UVs.
            return (best_wedge(self.attributes, current, wedges, uv_origin), 0.0);
        }
        let along_a = (bb * offset.dot(edge_a) - ab * offset.dot(edge_b)) / determinant;
        let along_b = (aa * offset.dot(edge_b) - ab * offset.dot(edge_a)) / determinant;
        let limit = MAX_EXTRAPOLATION;
        let wanted =
            uv_origin + uv_a * along_a.clamp(-limit, limit) + uv_b * along_b.clamp(-limit, limit);

        let chosen = best_wedge(self.attributes, current, wedges, wanted);
        let world_per_uv = (determinant.sqrt() / uv_area).sqrt();
        let moved = self.attributes[chosen as usize]
            .uv
            .as_dvec2()
            .distance(wanted);
        (chosen, moved * world_per_uv)
    }

    /// How much the shading normals disagree between the two ends, in `0..=1`.
    fn normal_difference(&self, from: u32, to: u32) -> f64 {
        self.fresh(from);
        self.fresh(to);
        let around = self.around.borrow();
        let (a, b) = (around[from as usize].normal, around[to as usize].normal);
        f64::from((1.0 - a.dot(b)) * 0.5)
    }

    /// The shading normal of the first of `v`'s corners.
    fn find_normal(&self, v: u32) -> Vec3 {
        self.vertex_triangles[v as usize]
            .iter()
            .flat_map(|&t| self.triangles[t as usize])
            .find(|corner| corner.position == v)
            .map_or(Vec3::Y, |corner| {
                self.attributes[corner.attribute as usize].normal
            })
    }

    fn can_collapse(&self, from: u32, to: u32) -> bool {
        // Link condition: the only vertices adjacent to both ends must be the two
        // opposite the shared edge, otherwise the collapse pinches the surface.
        self.fresh(from);
        self.fresh(to);
        let shared = {
            let around = self.around.borrow();
            let to_neighbors = &around[to as usize].neighbors;
            around[from as usize]
                .neighbors
                .iter()
                .filter(|n| to_neighbors.binary_search(n).is_ok())
                .count()
        };
        if shared != 2 {
            return false;
        }

        // Reject collapses that flip or degenerate any surviving triangle.
        let new_position = self.positions[to as usize];
        for &triangle in &self.vertex_triangles[from as usize] {
            let corners = self.triangles[triangle as usize].map(|c| c.position);
            if corners.contains(&to) {
                continue;
            }
            let old = corners.map(|v| self.positions[v as usize]);
            let new = corners.map(|v| {
                if v == from {
                    new_position
                } else {
                    self.positions[v as usize]
                }
            });
            let old_normal = (old[1] - old[0]).cross(old[2] - old[0]);
            let new_normal = (new[1] - new[0]).cross(new[2] - new[0]);
            let new_length = new_normal.length();
            if new_length <= 0.0 {
                return false;
            }
            if old_normal.dot(new_normal) < MIN_NORMAL_AGREEMENT * old_normal.length() * new_length
            {
                return false;
            }
        }
        true
    }

    fn collapse(&mut self, from: u32, to: u32) {
        let target = self.positions[to as usize];
        let mut merged = self.quadrics[to as usize];
        merged.add(&self.quadrics[from as usize]);
        let texture_error = self.texture_error(from, to);
        let weighted = texture_error * TEXTURE_ERROR_WEIGHT;
        let skin_error = self.skin_error(from, to);
        self.max_error = self
            .max_error
            .max(merged.mean_squared_distance(target))
            .max(weighted * weighted)
            .max(skin_error * skin_error);
        self.skin_errors[to as usize] = self.skin_errors[to as usize].max(skin_error);
        self.quadrics[to as usize] = merged;
        self.texture_errors[to as usize] = self.texture_errors[to as usize].max(texture_error);

        let target_wedges = self.wedges_at(to);
        let from_triangles = std::mem::take(&mut self.vertex_triangles[from as usize]);
        let mut changed = vec![from, to];
        for triangle in from_triangles {
            let index = triangle as usize;
            if !self.alive[index] {
                continue;
            }
            let corners = self.triangles[index];
            changed.extend(corners.iter().map(|c| c.position));
            if corners.iter().any(|c| c.position == to) {
                self.alive[index] = false;
                self.alive_count -= 1;
                for corner in corners {
                    if corner.position != from {
                        self.vertex_triangles[corner.position as usize].retain(|&t| t != triangle);
                    }
                }
            } else {
                let (attribute, _) = self.moved_corner(&corners, from, to, &target_wedges);
                for corner in &mut self.triangles[index] {
                    if corner.position == from {
                        corner.position = to;
                        corner.attribute = attribute;
                    }
                }
                self.vertex_triangles[to as usize].push(triangle);
            }
        }

        for v in changed {
            self.versions[v as usize] += 1;
        }
        #[cfg(test)]
        self.check_kept();

        // Invalidate every queued collapse touching the changed neighbourhood.
        self.stamps[from as usize] += 1;
        self.stamps[to as usize] += 1;
        let neighbors = self.neighbors(to);
        for &n in &neighbors {
            self.stamps[n as usize] += 1;
        }
        // Every edge from `to`, then each neighbour's other edges: those round the ring of
        // neighbours from its lower end, and all the rest.
        self.push_edges_around(to, &|_| false);
        for &n in &neighbors {
            let ring = &neighbors;
            self.push_edges_around(n, &|m| m == to || (m < n && ring.binary_search(&m).is_ok()));
        }
    }

    fn wedges_at(&self, v: u32) -> Vec<u32> {
        self.fresh(v);
        self.around.borrow()[v as usize].wedges.clone()
    }

    fn find_wedges(&self, v: u32) -> Vec<u32> {
        let mut wedges: Vec<u32> = self.vertex_triangles[v as usize]
            .iter()
            .flat_map(|&t| self.triangles[t as usize])
            .filter(|corner| corner.position == v)
            .map(|corner| corner.attribute)
            .collect();
        wedges.sort_unstable();
        wedges.dedup();
        wedges
    }

    fn snapshot(&self) -> Simplified {
        let global = &self.global;
        let triangles = self
            .triangles
            .iter()
            .zip(&self.alive)
            .filter(|(_, &alive)| alive)
            .map(|(t, _)| {
                t.map(|corner| Corner {
                    position: global[corner.position as usize],
                    attribute: corner.attribute,
                })
            })
            .collect();
        Simplified {
            triangles,
            error: self.max_error.sqrt() as f32,
        }
    }
}

fn distance_to_segment(point: DVec3, start: DVec3, end: DVec3) -> f64 {
    let along = end - start;
    let length_squared = along.length_squared();
    if length_squared <= 0.0 {
        return point.distance(start);
    }
    let t = ((point - start).dot(along) / length_squared).clamp(0.0, 1.0);
    point.distance(start + along * t)
}

/// Picks the attribute nearest `wanted` among the wedges at the collapse target and the corner's
/// `current` one. Wedges on the far side of a hard edge do not qualify.
fn best_wedge(attributes: &[Attribute], current: u32, wedges: &[u32], wanted: DVec2) -> u32 {
    let normal = attributes[current as usize].normal;
    let distance = |wedge: u32| attributes[wedge as usize].uv.as_dvec2().distance(wanted);
    wedges
        .iter()
        .copied()
        .filter(|&wedge| attributes[wedge as usize].normal.dot(normal) > 0.7)
        .chain(std::iter::once(current))
        .min_by(|&a, &b| distance(a).total_cmp(&distance(b)))
        .unwrap_or(current)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A closed UV sphere made of `segments * rings * 2`-ish triangles.
    fn closed_sphere(segments: u32, rings: u32) -> (Vec<Triangle>, Vec<Vec3>, Vec<Attribute>) {
        let mut positions = vec![Vec3::Y, -Vec3::Y];
        for ring in 1..rings {
            let theta = std::f32::consts::PI * ring as f32 / rings as f32;
            for segment in 0..segments {
                let phi = std::f32::consts::TAU * segment as f32 / segments as f32;
                positions.push(Vec3::new(
                    theta.sin() * phi.cos(),
                    theta.cos(),
                    theta.sin() * phi.sin(),
                ));
            }
        }
        let attributes: Vec<Attribute> = positions
            .iter()
            .map(|p| Attribute {
                normal: p.normalize(),
                uv: Vec2::ZERO,
            })
            .collect();
        let at = |ring: u32, segment: u32| 2 + (ring - 1) * segments + segment % segments;
        let mut indices: Vec<[u32; 3]> = Vec::new();
        for s in 0..segments {
            indices.push([0, at(1, s + 1), at(1, s)]);
            indices.push([1, at(rings - 1, s), at(rings - 1, s + 1)]);
            for r in 1..rings - 1 {
                indices.push([at(r, s), at(r, s + 1), at(r + 1, s)]);
                indices.push([at(r, s + 1), at(r + 1, s + 1), at(r + 1, s)]);
            }
        }
        let triangles = indices
            .into_iter()
            .map(|t| {
                t.map(|v| Corner {
                    position: v,
                    attribute: v,
                })
            })
            .collect();
        (triangles, positions, attributes)
    }

    /// A flat square of `cells` per side, with its texture shifted by `jump` from `seam_x` on.
    /// Vertices on the seam have one attribute for each side, as a UV seam does.
    fn flat_with_seam(
        cells: u32,
        seam_x: u32,
        jump: f32,
    ) -> (Vec<Triangle>, Vec<Vec3>, Vec<Attribute>) {
        let at = |x: u32, z: u32| z * (cells + 1) + x;
        let count = (cells + 1) * (cells + 1);
        let normal = Vec3::Y;
        let mut positions = Vec::new();
        let mut attributes = Vec::new();
        for z in 0..=cells {
            for x in 0..=cells {
                positions.push(Vec3::new(x as f32, 0.0, z as f32));
                let shift = if x > seam_x { jump } else { 0.0 };
                attributes.push(Attribute {
                    normal,
                    uv: Vec2::new(x as f32 + shift, z as f32),
                });
            }
        }
        // The second set is only used by the seam's right-hand corners.
        for z in 0..=cells {
            for x in 0..=cells {
                attributes.push(Attribute {
                    normal,
                    uv: Vec2::new(x as f32 + jump, z as f32),
                });
            }
        }
        let mut triangles = Vec::new();
        for z in 0..cells {
            for x in 0..cells {
                let corner = |px: u32, pz: u32| Corner {
                    position: at(px, pz),
                    attribute: if x >= seam_x {
                        at(px, pz) + count
                    } else {
                        at(px, pz)
                    },
                };
                let (a, b, c, d) = (
                    corner(x, z),
                    corner(x + 1, z),
                    corner(x, z + 1),
                    corner(x + 1, z + 1),
                );
                triangles.push([a, c, b]);
                triangles.push([b, c, d]);
            }
        }
        (triangles, positions, attributes)
    }

    /// Texture offset of a corner: 0 or `jump` when the mapping is intact.
    fn texture_shift(positions: &[Vec3], attributes: &[Attribute], corner: Corner) -> f32 {
        attributes[corner.attribute as usize].uv.x - positions[corner.position as usize].x
    }

    #[test]
    fn a_continuous_texture_collapses_freely_without_error() {
        let (triangles, positions, attributes) = flat_with_seam(16, 100, 0.0);
        // Its border is locked, which is as far as an open square can go.
        let result = simplify(&triangles, &positions, &attributes, 70, |_| false);
        assert!(result.triangles.len() <= 70);
        assert!(result.error < 1.0e-3, "error {}", result.error);
        for corner in result.triangles.iter().flatten() {
            assert!(texture_shift(&positions, &attributes, *corner).abs() < 1.0e-4);
        }
    }

    #[test]
    fn collapses_do_not_stretch_a_texture_across_a_seam() {
        const JUMP: f32 = 50.0;
        let (triangles, positions, attributes) = flat_with_seam(16, 8, JUMP);
        let result = simplify(&triangles, &positions, &attributes, 120, |_| false);
        assert!(result.triangles.len() <= 120);
        for triangle in &result.triangles {
            let shifts = triangle.map(|c| texture_shift(&positions, &attributes, c));
            assert!(
                (shifts[0] - shifts[1]).abs() < 1.0e-3 && (shifts[1] - shifts[2]).abs() < 1.0e-3,
                "a triangle spans both sides of the seam: {shifts:?}"
            );
        }
    }

    #[test]
    fn removing_a_patch_of_texture_counts_as_error() {
        // A flat square with one area in the middle mapped to a different part of the texture,
        // like a stain. Each side's mapping stays continuous as the patch is collapsed away.
        const CELLS: u32 = 8;
        let at = |x: u32, z: u32| z * (CELLS + 1) + x;
        let mut positions = Vec::new();
        let mut attributes = Vec::new();
        for z in 0..=CELLS {
            for x in 0..=CELLS {
                positions.push(Vec3::new(x as f32, 0.0, z as f32));
                attributes.push(Attribute {
                    normal: Vec3::Y,
                    uv: Vec2::new(x as f32, z as f32),
                });
            }
        }
        let patch = attributes.len() as u32;
        for z in 0..=CELLS {
            for x in 0..=CELLS {
                attributes.push(Attribute {
                    normal: Vec3::Y,
                    uv: Vec2::new(x as f32 + 50.0, z as f32),
                });
            }
        }
        let mut triangles = Vec::new();
        for z in 0..CELLS {
            for x in 0..CELLS {
                let inside = (3..5).contains(&x) && (3..5).contains(&z);
                let corner = |px: u32, pz: u32| Corner {
                    position: at(px, pz),
                    attribute: at(px, pz) + if inside { patch } else { 0 },
                };
                let (a, b, c, d) = (
                    corner(x, z),
                    corner(x + 1, z),
                    corner(x, z + 1),
                    corner(x + 1, z + 1),
                );
                triangles.push([a, c, b]);
                triangles.push([b, c, d]);
            }
        }
        let result = simplify(&triangles, &positions, &attributes, 32, |_| false);
        assert!(result.error > 0.2, "error {}", result.error);
    }

    #[test]
    fn texture_movement_counts_as_error_on_flat_ground() {
        // A flat hexagon whose centre has a texture coordinate out of line with its rim, so
        // removing the centre changes what the surface shows but not its shape at all.
        let mut positions = vec![Vec3::ZERO];
        let mut attributes = vec![Attribute {
            normal: Vec3::Y,
            uv: Vec2::new(3.0, 0.0),
        }];
        for i in 0..6 {
            let angle = std::f32::consts::TAU * i as f32 / 6.0;
            let p = Vec3::new(angle.cos(), 0.0, angle.sin());
            positions.push(p);
            attributes.push(Attribute {
                normal: Vec3::Y,
                uv: Vec2::new(p.x, p.z),
            });
        }
        let triangles: Vec<Triangle> = (0..6u32)
            .map(|i| {
                [0, 1 + (i + 1) % 6, 1 + i].map(|v| Corner {
                    position: v,
                    attribute: v,
                })
            })
            .collect();
        let result = simplify(&triangles, &positions, &attributes, 4, |_| false);
        assert_eq!(result.triangles.len(), 4);
        assert!(result.error > 0.1, "error {}", result.error);
    }

    #[test]
    fn one_run_to_several_targets_gives_what_a_run_to_each_does() {
        for (triangles, positions, attributes) in
            [closed_sphere(24, 16), flat_with_seam(12, 5, 0.3)]
        {
            let count = triangles.len();
            // In any order: each result is the one for the target in its place.
            let targets = [count / 2, count * 9 / 10, count / 5, count * 3 / 4];
            let together =
                simplify_skinned_each(&triangles, &positions, &attributes, None, &targets, |_| {
                    false
                });
            for (target, result) in targets.iter().zip(&together) {
                let alone = simplify(&triangles, &positions, &attributes, *target, |_| false);
                assert_eq!(result.triangles, alone.triangles, "target {target}");
                assert_eq!(
                    result.error.to_bits(),
                    alone.error.to_bits(),
                    "target {target}"
                );
            }
        }
    }

    #[test]
    fn reaches_target_on_a_closed_surface() {
        let (triangles, positions, attributes) = closed_sphere(32, 16);
        let target = triangles.len() / 2;
        let result = simplify(&triangles, &positions, &attributes, target, |_| false);
        assert!(result.triangles.len() <= target);
    }

    #[test]
    fn locked_vertices_survive() {
        let (triangles, positions, attributes) = closed_sphere(32, 16);
        let locked: Vec<u32> = (0..positions.len() as u32).step_by(3).collect();
        let result = simplify(&triangles, &positions, &attributes, 20, |v| {
            locked.contains(&v)
        });
        for &v in &locked {
            assert!(
                result.triangles.iter().flatten().any(|c| c.position == v),
                "locked vertex {v} disappeared"
            );
        }
    }

    #[test]
    fn simplified_surface_stays_closed() {
        let (triangles, positions, attributes) = closed_sphere(32, 16);
        let result = simplify(&triangles, &positions, &attributes, 100, |_| false);
        let mut uses: FxHashMap<(u32, u32), i32> = FxHashMap::default();
        for t in &result.triangles {
            for side in 0..3 {
                let (a, b) = (t[side].position, t[(side + 1) % 3].position);
                *uses.entry((a.min(b), a.max(b))).or_insert(0) += 1;
            }
        }
        assert!(uses.values().all(|&count| count == 2));
    }

    #[test]
    fn error_grows_as_the_target_shrinks() {
        let (triangles, positions, attributes) = closed_sphere(32, 16);
        let gentle = simplify(&triangles, &positions, &attributes, 500, |_| false);
        let harsh = simplify(&triangles, &positions, &attributes, 60, |_| false);
        assert!(harsh.error > gentle.error);
        assert!(gentle.error > 0.0);
    }
}
