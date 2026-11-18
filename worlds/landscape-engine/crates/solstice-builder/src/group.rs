//! Gathers neighbouring clusters into groups that simplify together.
//!
//! Connection strength is the number of shared edges. Favouring strongly connected
//! neighbours minimises the outer boundary of each group, and that boundary is the
//! only part of the group that cannot be simplified at this level.

use crate::bounds::Sphere;
use crate::partition::morton_order;
use rustc_hash::FxHashMap;

/// Number of clusters a group aims for.
const TARGET_GROUP_SIZE: usize = 16;
/// Groups never grow beyond this, even when merging leftovers, so that a sibling set always fits
/// in a page: full clusters of 255 vertices and 128 triangles take 4,592 bytes.
pub const MAX_GROUP_SIZE: usize = 24;
/// The same for a skinned mesh, whose full clusters take 6,640 bytes.
pub const MAX_SKINNED_GROUP_SIZE: usize = 18;
/// Groups smaller than this pull in spatial neighbours or merge into another group.
const MIN_GROUP_SIZE: usize = 6;

/// Clusters as needed for grouping.
pub struct GroupInput<'a> {
    /// `triangles[c]` holds the welded position indices of cluster `c`.
    pub triangles: &'a [Vec<[u32; 3]>],
    pub bounds: &'a [Sphere],
    /// The most clusters a group may hold.
    pub max_group_size: usize,
}

/// Returns cluster indices per group. Every cluster appears in exactly one group.
pub fn group_clusters(input: &GroupInput<'_>) -> Vec<Vec<u32>> {
    let count = input.triangles.len();
    if count == 0 {
        return Vec::new();
    }
    let adjacency = cluster_adjacency(input.triangles);
    let centers: Vec<_> = input.bounds.iter().map(|s| s.center).collect();
    let order = morton_order(&centers);

    let mut group_of = vec![u32::MAX; count];
    let mut groups: Vec<Vec<u32>> = Vec::new();

    for (position, &seed) in order.iter().enumerate() {
        if group_of[seed as usize] != u32::MAX {
            continue;
        }
        let id = groups.len() as u32;
        let mut group = vec![seed];
        group_of[seed as usize] = id;
        let mut connection: FxHashMap<u32, u32> = FxHashMap::default();
        add_connections(&adjacency, seed, &group_of, &mut connection);

        while group.len() < TARGET_GROUP_SIZE {
            let best = connection
                .iter()
                .max_by_key(|&(&cluster, &weight)| (weight, std::cmp::Reverse(cluster)))
                .map(|(&cluster, _)| cluster);
            let Some(next) = best else { break };
            connection.remove(&next);
            group.push(next);
            group_of[next as usize] = id;
            add_connections(&adjacency, next, &group_of, &mut connection);
        }

        // Disconnected islands would otherwise simplify alone forever. Pull in the
        // next clusters along the Morton curve if they are reasonably close.
        if group.len() < MIN_GROUP_SIZE {
            let reach = group_reach(input.bounds, &group);
            for &candidate in order[position + 1..].iter().take(32) {
                if group.len() >= MIN_GROUP_SIZE {
                    break;
                }
                let bounds = input.bounds[candidate as usize];
                if group_of[candidate as usize] == u32::MAX
                    && bounds.center.distance(reach.center) <= reach.radius + bounds.radius
                {
                    group.push(candidate);
                    group_of[candidate as usize] = id;
                }
            }
        }
        groups.push(group);
    }

    merge_small_groups(groups, &adjacency, &mut group_of, input.max_group_size)
}

/// A sphere around the group, inflated to define "close enough to join".
fn group_reach(bounds: &[Sphere], group: &[u32]) -> Sphere {
    let merged = Sphere::merged_all(group.iter().map(|&c| bounds[c as usize]));
    Sphere {
        center: merged.center,
        radius: merged.radius * 2.0,
    }
}

fn add_connections(
    adjacency: &[Vec<(u32, u32)>],
    cluster: u32,
    group_of: &[u32],
    connection: &mut FxHashMap<u32, u32>,
) {
    for &(neighbor, shared_edges) in &adjacency[cluster as usize] {
        if group_of[neighbor as usize] == u32::MAX {
            *connection.entry(neighbor).or_insert(0) += shared_edges;
        }
    }
}

/// For each cluster, its neighbours and the number of edges shared with each.
fn cluster_adjacency(clusters: &[Vec<[u32; 3]>]) -> Vec<Vec<(u32, u32)>> {
    let mut edges: Vec<(u64, u32)> = Vec::new();
    for (index, triangles) in clusters.iter().enumerate() {
        for t in triangles {
            for side in 0..3 {
                let (a, b) = (t[side], t[(side + 1) % 3]);
                edges.push((
                    u64::from(a.min(b)) << 32 | u64::from(a.max(b)),
                    index as u32,
                ));
            }
        }
    }
    edges.sort_unstable();
    edges.dedup();

    let mut links: Vec<(u32, u32)> = Vec::new();
    for run in edges.chunk_by(|a, b| a.0 == b.0) {
        for i in 0..run.len() {
            for j in i + 1..run.len() {
                links.push((run[i].1, run[j].1));
                links.push((run[j].1, run[i].1));
            }
        }
    }
    links.sort_unstable();

    let mut adjacency = vec![Vec::new(); clusters.len()];
    for run in links.chunk_by(|a, b| a == b) {
        adjacency[run[0].0 as usize].push((run[0].1, run.len() as u32));
    }
    adjacency
}

fn merge_small_groups(
    mut groups: Vec<Vec<u32>>,
    adjacency: &[Vec<(u32, u32)>],
    group_of: &mut [u32],
    max_group_size: usize,
) -> Vec<Vec<u32>> {
    for index in 0..groups.len() {
        if groups[index].is_empty() || groups[index].len() >= MIN_GROUP_SIZE {
            continue;
        }
        let mut weights: FxHashMap<u32, u32> = FxHashMap::default();
        for &cluster in &groups[index] {
            for &(neighbor, shared) in &adjacency[cluster as usize] {
                let other = group_of[neighbor as usize];
                if other as usize != index {
                    *weights.entry(other).or_insert(0) += shared;
                }
            }
        }
        let target = weights
            .into_iter()
            .filter(|&(g, _)| groups[g as usize].len() + groups[index].len() <= max_group_size)
            .max_by_key(|&(g, weight)| (weight, std::cmp::Reverse(g)))
            .map(|(g, _)| g);
        if let Some(target) = target {
            let moved = std::mem::take(&mut groups[index]);
            for &cluster in &moved {
                group_of[cluster as usize] = target;
            }
            groups[target as usize].extend(moved);
        }
    }
    groups.retain(|g| !g.is_empty());
    groups
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;

    /// A strip of quads, one cluster per quad, sharing one edge with each neighbour.
    fn strip(count: u32) -> (Vec<Vec<[u32; 3]>>, Vec<Sphere>) {
        let clusters = (0..count)
            .map(|i| {
                let [a, b, c, d] = [i * 2, i * 2 + 1, i * 2 + 2, i * 2 + 3];
                vec![[a, b, c], [c, b, d]]
            })
            .collect();
        let bounds = (0..count)
            .map(|i| Sphere {
                center: Vec3::X * i as f32,
                radius: 0.8,
            })
            .collect();
        (clusters, bounds)
    }

    #[test]
    fn every_cluster_lands_in_exactly_one_group() {
        let (triangles, bounds) = strip(100);
        let groups = group_clusters(&GroupInput {
            triangles: &triangles,
            bounds: &bounds,
            max_group_size: MAX_GROUP_SIZE,
        });
        let mut seen = vec![0; 100];
        for group in &groups {
            for &cluster in group {
                seen[cluster as usize] += 1;
            }
        }
        assert!(seen.iter().all(|&count| count == 1));
    }

    #[test]
    fn groups_hold_several_clusters_and_respect_the_cap() {
        let (triangles, bounds) = strip(200);
        let groups = group_clusters(&GroupInput {
            triangles: &triangles,
            bounds: &bounds,
            max_group_size: MAX_GROUP_SIZE,
        });
        assert!(
            groups.len() < 50,
            "expected real grouping, got {} groups",
            groups.len()
        );
        assert!(groups.iter().all(|g| g.len() <= MAX_GROUP_SIZE));
    }
}
