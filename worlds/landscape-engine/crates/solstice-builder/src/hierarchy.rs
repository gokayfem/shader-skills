//! Builds the node hierarchy used by GPU traversal.
//!
//! The leaves are *sibling sets*: clusters with the same parent group. Siblings
//! share one parent error and one parent sphere, so one leaf test decides the
//! "is the parent too coarse?" half of the LOD rule for all of them.
//!
//! Leaves are organised into one spatial tree per DAG level, and those trees hang
//! off a single root. Whole levels are thus rejected at once when they are too
//! fine for the viewer's distance.

use crate::bounds::Sphere;
use solstice_asset::PackedNode;

/// Maximum children of an interior node.
const FAN_OUT: usize = 8;

/// One sibling set, ready to become a leaf node.
#[derive(Clone, Copy, Debug)]
pub struct Leaf {
    pub level: u32,
    pub first_cluster: u32,
    pub cluster_count: u32,
    pub bounds: Sphere,
    pub parent_lod_bounds: Sphere,
    pub parent_error: f32,
}

/// The finished tree.
pub struct Hierarchy {
    pub nodes: Vec<PackedNode>,
    pub root: u32,
    /// Longest root-to-leaf path, counted in nodes.
    pub depth: u32,
}

/// Builds the hierarchy. `leaves` must not be empty.
pub fn build_hierarchy(mut leaves: Vec<Leaf>) -> Hierarchy {
    assert!(
        !leaves.is_empty(),
        "a mesh with no clusters has no hierarchy"
    );
    leaves.sort_by_key(|leaf| leaf.level);

    let mut nodes = Vec::new();
    let mut level_roots = Vec::new();
    let mut depth = 0;
    for level in leaves.chunk_by_mut(|a, b| a.level == b.level) {
        let (node, level_depth) = build_subtree(level, &mut nodes);
        level_roots.push(node);
        depth = depth.max(level_depth);
    }

    let root = summarize(&level_roots, nodes.len() as u32);
    nodes.extend(level_roots);
    nodes.push(root);

    Hierarchy {
        root: nodes.len() as u32 - 1,
        nodes,
        depth: depth + 1,
    }
}

/// Returns the subtree's root, not yet stored, and the subtree's depth. The caller
/// stores siblings together because the GPU format addresses children as a run.
fn build_subtree(leaves: &mut [Leaf], nodes: &mut Vec<PackedNode>) -> (PackedNode, u32) {
    if let [leaf] = leaves {
        return (leaf_node(leaf), 1);
    }

    let mut children = Vec::with_capacity(FAN_OUT);
    let mut depth = 0;
    for chunk in split_spatially(leaves) {
        let (child, child_depth) = build_subtree(chunk, nodes);
        children.push(child);
        depth = depth.max(child_depth);
    }

    let parent = summarize(&children, nodes.len() as u32);
    nodes.extend(children);
    (parent, depth + 1)
}

/// Splits into at most [`FAN_OUT`] contiguous, spatially coherent runs by
/// repeatedly halving the largest run along its longest axis.
fn split_spatially(leaves: &mut [Leaf]) -> Vec<&mut [Leaf]> {
    let mut runs: Vec<&mut [Leaf]> = vec![leaves];
    while runs.len() < FAN_OUT {
        let largest = (0..runs.len()).max_by_key(|&i| runs[i].len()).unwrap_or(0);
        if runs[largest].len() < 2 {
            break;
        }
        let run = runs.swap_remove(largest);
        let (mut min, mut max) = (run[0].bounds.center, run[0].bounds.center);
        for leaf in run.iter() {
            min = min.min(leaf.bounds.center);
            max = max.max(leaf.bounds.center);
        }
        let axis = (max - min).max_position();
        let middle = run.len() / 2;
        run.select_nth_unstable_by(middle, |a, b| {
            a.bounds.center[axis].total_cmp(&b.bounds.center[axis])
        });
        let (left, right) = run.split_at_mut(middle);
        runs.push(left);
        runs.push(right);
    }
    runs
}

fn leaf_node(leaf: &Leaf) -> PackedNode {
    PackedNode {
        center: leaf.bounds.center.to_array(),
        radius: leaf.bounds.radius,
        lod_center: leaf.parent_lod_bounds.center.to_array(),
        lod_radius: leaf.parent_lod_bounds.radius,
        max_parent_error: leaf.parent_error,
        first: leaf.first_cluster,
        count: leaf.cluster_count,
        is_leaf: 1,
    }
}

fn summarize(children: &[PackedNode], first: u32) -> PackedNode {
    let bounds = Sphere::merged_all(children.iter().map(|c| Sphere {
        center: c.center.into(),
        radius: c.radius,
    }));
    let lod_bounds = Sphere::merged_all(children.iter().map(|c| Sphere {
        center: c.lod_center.into(),
        radius: c.lod_radius,
    }));
    PackedNode {
        center: bounds.center.to_array(),
        radius: bounds.radius,
        lod_center: lod_bounds.center.to_array(),
        lod_radius: lod_bounds.radius,
        max_parent_error: children
            .iter()
            .map(|c| c.max_parent_error)
            .fold(0.0, f32::max),
        first,
        count: children.len() as u32,
        is_leaf: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;

    fn leaves(count: u32) -> Vec<Leaf> {
        (0..count)
            .map(|i| Leaf {
                level: i % 3,
                first_cluster: i,
                cluster_count: 1,
                bounds: Sphere {
                    center: Vec3::X * i as f32,
                    radius: 1.0,
                },
                parent_lod_bounds: Sphere {
                    center: Vec3::X * i as f32,
                    radius: 2.0,
                },
                parent_error: i as f32,
            })
            .collect()
    }

    /// Visits the tree, returning every reachable leaf's first cluster.
    fn reachable(hierarchy: &Hierarchy, node: u32, out: &mut Vec<u32>) {
        let packed = hierarchy.nodes[node as usize];
        if packed.is_leaf != 0 {
            out.push(packed.first);
            return;
        }
        for child in packed.first..packed.first + packed.count {
            reachable(hierarchy, child, out);
        }
    }

    #[test]
    fn every_leaf_is_reachable_exactly_once() {
        let hierarchy = build_hierarchy(leaves(500));
        let mut found = Vec::new();
        reachable(&hierarchy, hierarchy.root, &mut found);
        found.sort_unstable();
        assert_eq!(found, (0..500).collect::<Vec<u32>>());
    }

    #[test]
    fn stores_no_unreachable_nodes() {
        fn count(hierarchy: &Hierarchy, node: u32) -> usize {
            let packed = hierarchy.nodes[node as usize];
            if packed.is_leaf != 0 {
                return 1;
            }
            1 + (packed.first..packed.first + packed.count)
                .map(|child| count(hierarchy, child))
                .sum::<usize>()
        }
        let hierarchy = build_hierarchy(leaves(500));
        assert_eq!(count(&hierarchy, hierarchy.root), hierarchy.nodes.len());
    }

    #[test]
    fn parents_bound_their_children() {
        let hierarchy = build_hierarchy(leaves(500));
        for node in hierarchy.nodes.iter().filter(|n| n.is_leaf == 0) {
            let outer = Sphere {
                center: node.center.into(),
                radius: node.radius,
            };
            let outer_lod = Sphere {
                center: node.lod_center.into(),
                radius: node.lod_radius,
            };
            for child in &hierarchy.nodes[node.first as usize..(node.first + node.count) as usize] {
                assert!(outer.contains(Sphere {
                    center: child.center.into(),
                    radius: child.radius
                }));
                assert!(outer_lod.contains(Sphere {
                    center: child.lod_center.into(),
                    radius: child.lod_radius
                }));
                assert!(node.max_parent_error >= child.max_parent_error);
            }
        }
    }

    #[test]
    fn single_leaf_still_has_a_root() {
        let hierarchy = build_hierarchy(leaves(1));
        let mut found = Vec::new();
        reachable(&hierarchy, hierarchy.root, &mut found);
        assert_eq!(found, vec![0]);
    }

    #[test]
    fn depth_counts_the_longest_path() {
        fn measure(hierarchy: &Hierarchy, node: u32) -> u32 {
            let packed = hierarchy.nodes[node as usize];
            if packed.is_leaf != 0 {
                return 1;
            }
            1 + (packed.first..packed.first + packed.count)
                .map(|child| measure(hierarchy, child))
                .max()
                .unwrap_or(0)
        }
        let hierarchy = build_hierarchy(leaves(500));
        assert_eq!(measure(&hierarchy, hierarchy.root), hierarchy.depth);
    }
}
