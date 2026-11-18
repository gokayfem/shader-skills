//! Splits triangles into compact, connected clusters.
//!
//! Clusters are grown greedily from seeds visited in Morton order. Each step adds
//! the frontier triangle that keeps the cluster round and adds the fewest new
//! vertices. Very small leftovers are then merged into neighbours, since a tiny
//! cluster costs as much to cull as a full one.
//!
//! Foliage is made of many separate pieces, such as blades and leaves, that share no edges. When a
//! cluster has no connected triangle left to take, it continues with the nearest unassigned piece,
//! so that a meadow packs into full, compact clusters rather than one per blade.

use glam::Vec3;
use rustc_hash::FxHashMap;

/// Size limits for one cluster.
#[derive(Clone, Copy, Debug)]
pub struct ClusterLimits {
    pub max_triangles: usize,
    pub max_vertices: usize,
}

/// Clusters smaller than this try to merge into a neighbour.
const SMALL_CLUSTER: usize = 64;
/// Rings of grid cells searched for a nearby separate piece before giving up.
const MAX_SEARCH_RINGS: i32 = 6;

/// Partitions `triangles` (indices into `positions`) and returns triangle indices
/// per cluster. Every triangle appears in exactly one cluster.
pub fn partition_triangles(
    triangles: &[[u32; 3]],
    positions: &[Vec3],
    limits: ClusterLimits,
) -> Vec<Vec<u32>> {
    if triangles.is_empty() {
        return Vec::new();
    }
    let local = LocalTriangles::new(triangles, positions);
    let adjacency = triangle_adjacency(&local.triangles);
    let clusters = grow_clusters(&local, &adjacency, limits);
    merge_small_clusters(clusters, &local, &adjacency, limits)
}

/// Triangles re-indexed to a compact vertex range, so scratch arrays stay small
/// even when partitioning a little piece of a huge mesh.
struct LocalTriangles {
    triangles: Vec<[u32; 3]>,
    centroids: Vec<Vec3>,
    vertex_count: usize,
}

impl LocalTriangles {
    fn new(triangles: &[[u32; 3]], positions: &[Vec3]) -> Self {
        let mut remap = FxHashMap::default();
        let local: Vec<[u32; 3]> = triangles
            .iter()
            .map(|t| {
                t.map(|v| {
                    let next = remap.len() as u32;
                    *remap.entry(v).or_insert(next)
                })
            })
            .collect();
        let centroids = triangles
            .iter()
            .map(|t| {
                (positions[t[0] as usize] + positions[t[1] as usize] + positions[t[2] as usize])
                    / 3.0
            })
            .collect();
        Self {
            triangles: local,
            centroids,
            vertex_count: remap.len(),
        }
    }
}

/// For each triangle, up to three edge-adjacent triangles (`u32::MAX` if none).
fn triangle_adjacency(triangles: &[[u32; 3]]) -> Vec<[u32; 3]> {
    let mut edges: Vec<(u64, u32, u8)> = Vec::with_capacity(triangles.len() * 3);
    for (index, t) in triangles.iter().enumerate() {
        for side in 0..3 {
            let (a, b) = (t[side], t[(side + 1) % 3]);
            let key = u64::from(a.min(b)) << 32 | u64::from(a.max(b));
            edges.push((key, index as u32, side as u8));
        }
    }
    edges.sort_unstable();

    let mut adjacency = vec![[u32::MAX; 3]; triangles.len()];
    for run in edges.chunk_by(|a, b| a.0 == b.0) {
        // Non-manifold edges link only their first two users; that is enough for
        // growth to continue, and the rest are reached via other edges or seeds.
        if let [(_, first, first_side), (_, second, second_side), ..] = run {
            adjacency[*first as usize][*first_side as usize] = *second;
            adjacency[*second as usize][*second_side as usize] = *first;
        }
    }
    adjacency
}

fn grow_clusters(
    local: &LocalTriangles,
    adjacency: &[[u32; 3]],
    limits: ClusterLimits,
) -> Vec<Vec<u32>> {
    let seeds = morton_order(&local.centroids);
    let mut assigned = vec![false; local.triangles.len()];
    // `vertex_stamp[v] == current cluster id` marks a vertex as used by the cluster.
    let mut vertex_stamp = vec![u32::MAX; local.vertex_count];
    let mut clusters = Vec::new();
    let mut frontier: Vec<u32> = Vec::new();

    let pieces = PieceGrid::new(&local.centroids);
    for &seed in &seeds {
        if assigned[seed as usize] {
            continue;
        }
        let cluster_id = clusters.len() as u32;
        let mut cluster = Vec::with_capacity(limits.max_triangles);
        let mut vertex_count = 0;
        let mut center_sum = Vec3::ZERO;
        frontier.clear();

        let mut next = Some(seed);
        while let Some(triangle) = next {
            assigned[triangle as usize] = true;
            cluster.push(triangle);
            center_sum += local.centroids[triangle as usize];
            for v in local.triangles[triangle as usize] {
                if vertex_stamp[v as usize] != cluster_id {
                    vertex_stamp[v as usize] = cluster_id;
                    vertex_count += 1;
                }
            }
            for neighbor in adjacency[triangle as usize] {
                if neighbor != u32::MAX && !assigned[neighbor as usize] {
                    frontier.push(neighbor);
                }
            }
            if cluster.len() >= limits.max_triangles {
                break;
            }

            // Pick the frontier triangle that best keeps the cluster compact.
            let center = center_sum / cluster.len() as f32;
            let mut best: Option<(f32, usize)> = None;
            let mut slot = 0;
            while slot < frontier.len() {
                let candidate = frontier[slot] as usize;
                if assigned[candidate] {
                    frontier.swap_remove(slot);
                    continue;
                }
                let new_vertices = local.triangles[candidate]
                    .iter()
                    .filter(|&&v| vertex_stamp[v as usize] != cluster_id)
                    .count();
                if vertex_count + new_vertices <= limits.max_vertices {
                    // Taking triangles with few free neighbours first fills dead ends
                    // that would otherwise be stranded as tiny clusters later.
                    let free_neighbors = adjacency[candidate]
                        .iter()
                        .filter(|&&n| n != u32::MAX && !assigned[n as usize])
                        .count();
                    let distance = local.centroids[candidate].distance_squared(center);
                    let score = distance
                        * (1.0 + new_vertices as f32 * 0.5)
                        * (1.0 + free_neighbors as f32 * 0.5);
                    if best.is_none_or(|(s, _)| score < s) {
                        best = Some((score, slot));
                    }
                }
                slot += 1;
            }
            next = match best {
                Some((_, slot)) => Some(frontier[slot]),
                None => pieces.nearest(
                    local,
                    &assigned,
                    &vertex_stamp,
                    cluster_id,
                    vertex_count,
                    center,
                    limits,
                ),
            };
        }
        clusters.push(cluster);
    }
    clusters
}

/// Triangle centroids bucketed on a uniform grid, for finding the nearest separate piece.
struct PieceGrid {
    origin: Vec3,
    cell: f32,
    cells: FxHashMap<[i32; 3], Vec<u32>>,
}

impl PieceGrid {
    fn new(centroids: &[Vec3]) -> Self {
        let (mut min, mut max) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        for &c in centroids {
            min = min.min(c);
            max = max.max(c);
        }
        // About eight triangles per occupied cell, assuming they spread over a surface.
        let extent = (max - min).max(Vec3::splat(1.0e-6));
        let area = extent.x * extent.y + extent.y * extent.z + extent.z * extent.x;
        let cell = (area / (centroids.len() as f32 / 8.0).max(1.0))
            .sqrt()
            .max(1.0e-6);
        let mut grid = Self {
            origin: min,
            cell,
            cells: FxHashMap::default(),
        };
        for (index, &c) in centroids.iter().enumerate() {
            grid.cells
                .entry(grid.key(c))
                .or_default()
                .push(index as u32);
        }
        grid
    }

    fn key(&self, p: Vec3) -> [i32; 3] {
        ((p - self.origin) / self.cell)
            .floor()
            .as_ivec3()
            .to_array()
    }

    /// The unassigned triangle nearest `center` that fits in the cluster.
    #[allow(clippy::too_many_arguments)]
    fn nearest(
        &self,
        local: &LocalTriangles,
        assigned: &[bool],
        vertex_stamp: &[u32],
        cluster_id: u32,
        vertex_count: usize,
        center: Vec3,
        limits: ClusterLimits,
    ) -> Option<u32> {
        let home = self.key(center);
        let mut best: Option<(f32, u32)> = None;
        for ring in 0..=MAX_SEARCH_RINGS {
            // Anything in this ring or beyond is at least this far away.
            let floor = (ring - 1).max(0) as f32 * self.cell;
            if best.is_some_and(|(d, _)| d <= floor * floor) {
                break;
            }
            for dz in -ring..=ring {
                for dy in -ring..=ring {
                    for dx in -ring..=ring {
                        if dx.abs().max(dy.abs()).max(dz.abs()) != ring {
                            continue;
                        }
                        let key = [home[0] + dx, home[1] + dy, home[2] + dz];
                        let Some(members) = self.cells.get(&key) else {
                            continue;
                        };
                        for &candidate in members {
                            if assigned[candidate as usize] {
                                continue;
                            }
                            let new_vertices = local.triangles[candidate as usize]
                                .iter()
                                .filter(|&&v| vertex_stamp[v as usize] != cluster_id)
                                .count();
                            if vertex_count + new_vertices > limits.max_vertices {
                                continue;
                            }
                            let distance =
                                local.centroids[candidate as usize].distance_squared(center);
                            if best.is_none_or(|(d, _)| distance < d) {
                                best = Some((distance, candidate));
                            }
                        }
                    }
                }
            }
        }
        best.map(|(_, candidate)| candidate)
    }
}

fn merge_small_clusters(
    mut clusters: Vec<Vec<u32>>,
    local: &LocalTriangles,
    adjacency: &[[u32; 3]],
    limits: ClusterLimits,
) -> Vec<Vec<u32>> {
    let mut owner = vec![0u32; local.triangles.len()];
    for (index, cluster) in clusters.iter().enumerate() {
        for &triangle in cluster {
            owner[triangle as usize] = index as u32;
        }
    }

    // Smallest first, so tiny fragments claim the free space before larger ones.
    let mut order: Vec<usize> = (0..clusters.len()).collect();
    order.sort_by_key(|&index| clusters[index].len());

    for index in order {
        if clusters[index].is_empty() || clusters[index].len() >= SMALL_CLUSTER {
            continue;
        }
        let mut neighbors: Vec<u32> = clusters[index]
            .iter()
            .flat_map(|&t| adjacency[t as usize])
            .filter(|&n| n != u32::MAX)
            .map(|n| owner[n as usize])
            .filter(|&c| c as usize != index)
            .collect();
        neighbors.sort_unstable();
        neighbors.dedup();

        // The fullest neighbour that still fits packs the tightest.
        let target = neighbors
            .into_iter()
            .filter(|&candidate| {
                let other = &clusters[candidate as usize];
                other.len() + clusters[index].len() <= limits.max_triangles
                    && unique_vertices(local, other, &clusters[index]) <= limits.max_vertices
            })
            .max_by_key(|&candidate| clusters[candidate as usize].len());
        if let Some(target) = target {
            let moved = std::mem::take(&mut clusters[index]);
            for &triangle in &moved {
                owner[triangle as usize] = target;
            }
            clusters[target as usize].extend(moved);
        }
    }

    clusters.retain(|c| !c.is_empty());
    clusters
}

fn unique_vertices(local: &LocalTriangles, a: &[u32], b: &[u32]) -> usize {
    let mut vertices: Vec<u32> = a
        .iter()
        .chain(b)
        .flat_map(|&t| local.triangles[t as usize])
        .collect();
    vertices.sort_unstable();
    vertices.dedup();
    vertices.len()
}

/// Indices of `points` sorted along a Morton curve.
pub fn morton_order(points: &[Vec3]) -> Vec<u32> {
    let (mut min, mut max) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
    for &p in points {
        min = min.min(p);
        max = max.max(p);
    }
    let scale = 1023.0 / (max - min).max_element().max(1.0e-12);
    let mut keyed: Vec<(u32, u32)> = points
        .iter()
        .enumerate()
        .map(|(index, &p)| {
            let q = (p - min) * scale;
            let key = spread_bits(q.x as u32)
                | spread_bits(q.y as u32) << 1
                | spread_bits(q.z as u32) << 2;
            (key, index as u32)
        })
        .collect();
    keyed.sort_unstable();
    keyed.into_iter().map(|(_, index)| index).collect()
}

/// Inserts two zero bits between each of the low 10 bits of `x`.
fn spread_bits(x: u32) -> u32 {
    let x = x & 0x3ff;
    let x = (x | x << 16) & 0x030000ff;
    let x = (x | x << 8) & 0x0300f00f;
    let x = (x | x << 4) & 0x030c30c3;
    (x | x << 2) & 0x09249249
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIMITS: ClusterLimits = ClusterLimits {
        max_triangles: 128,
        max_vertices: 255,
    };

    fn grid(size: u32) -> (Vec<[u32; 3]>, Vec<Vec3>) {
        let mut positions = Vec::new();
        for y in 0..=size {
            for x in 0..=size {
                positions.push(Vec3::new(x as f32, 0.0, y as f32));
            }
        }
        let mut triangles = Vec::new();
        let at = |x: u32, y: u32| y * (size + 1) + x;
        for y in 0..size {
            for x in 0..size {
                triangles.push([at(x, y), at(x, y + 1), at(x + 1, y)]);
                triangles.push([at(x + 1, y), at(x, y + 1), at(x + 1, y + 1)]);
            }
        }
        (triangles, positions)
    }

    #[test]
    fn every_triangle_lands_in_exactly_one_cluster() {
        let (triangles, positions) = grid(64);
        let clusters = partition_triangles(&triangles, &positions, LIMITS);
        let mut seen = vec![0u32; triangles.len()];
        for cluster in &clusters {
            for &t in cluster {
                seen[t as usize] += 1;
            }
        }
        assert!(seen.iter().all(|&count| count == 1));
    }

    #[test]
    fn clusters_respect_limits() {
        let (triangles, positions) = grid(64);
        let limits = ClusterLimits {
            max_triangles: 64,
            max_vertices: 48,
        };
        for cluster in partition_triangles(&triangles, &positions, limits) {
            assert!(cluster.len() <= limits.max_triangles);
            let mut vertices: Vec<u32> = cluster
                .iter()
                .flat_map(|&t| triangles[t as usize])
                .collect();
            vertices.sort_unstable();
            vertices.dedup();
            assert!(vertices.len() <= limits.max_vertices);
        }
    }

    #[test]
    fn clusters_are_well_filled_on_a_regular_grid() {
        let (triangles, positions) = grid(128);
        let clusters = partition_triangles(&triangles, &positions, LIMITS);
        let average = triangles.len() as f32 / clusters.len() as f32;
        assert!(average > 88.0, "average cluster size was only {average}");
    }

    /// A meadow of separate two-triangle blades, `count` by `count`.
    fn blades(count: u32) -> (Vec<[u32; 3]>, Vec<Vec3>) {
        let (mut triangles, mut positions) = (Vec::new(), Vec::new());
        for z in 0..count {
            for x in 0..count {
                let base = positions.len() as u32;
                let at = Vec3::new(x as f32 * 0.1, 0.0, z as f32 * 0.1);
                positions.extend([
                    at,
                    at + Vec3::X * 0.01,
                    at + Vec3::Y * 0.3,
                    at + Vec3::new(0.01, 0.3, 0.0),
                ]);
                triangles.push([base, base + 1, base + 2]);
                triangles.push([base + 1, base + 3, base + 2]);
            }
        }
        (triangles, positions)
    }

    #[test]
    fn separate_pieces_pack_into_full_compact_clusters() {
        let (triangles, positions) = blades(64);
        let clusters = partition_triangles(&triangles, &positions, LIMITS);
        let average = triangles.len() as f32 / clusters.len() as f32;
        assert!(average > 100.0, "average cluster size was only {average}");
        // Each cluster should hold a patch of neighbouring blades, not a scattered selection. A full
        // 128-triangle patch spans about 0.5 m from its centre. The last clusters gather
        // stragglers, so only the typical one is held to that.
        let mut spreads: Vec<f32> = clusters
            .iter()
            .map(|cluster| {
                let centroids: Vec<Vec3> = cluster
                    .iter()
                    .map(|&t| {
                        triangles[t as usize]
                            .iter()
                            .map(|&v| positions[v as usize])
                            .sum::<Vec3>()
                            / 3.0
                    })
                    .collect();
                let center = centroids.iter().sum::<Vec3>() / centroids.len() as f32;
                centroids
                    .iter()
                    .map(|c| c.distance(center))
                    .fold(0.0, f32::max)
            })
            .collect();
        spreads.sort_by(f32::total_cmp);
        let median = spreads[spreads.len() / 2];
        let worst = spreads[spreads.len() - 1];
        assert!(median < 0.7, "a typical cluster spans {median} m");
        assert!(worst < 2.0, "a cluster spans {worst} m of a 6.4 m meadow");
    }

    #[test]
    fn empty_input_produces_no_clusters() {
        assert!(partition_triangles(&[], &[], LIMITS).is_empty());
    }
}
