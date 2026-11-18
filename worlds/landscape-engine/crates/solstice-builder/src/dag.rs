//! Builds the cluster LOD DAG and packs it for the GPU.

use crate::bounds::Sphere;
use crate::group::{group_clusters, GroupInput, MAX_GROUP_SIZE, MAX_SKINNED_GROUP_SIZE};
use crate::hierarchy::{build_hierarchy, Leaf};
use crate::mesh::Mesh;
use crate::partition::{partition_triangles, ClusterLimits};
use crate::simplify::{
    simplify_skinned, simplify_skinned_each, Attribute, Corner, SkinCost, Triangle,
};
use crate::thin::{find_pieces, set_aside_wide_pieces, thin, NEW_POSITION};
use glam::{Vec2, Vec3};
use rayon::prelude::*;
use rustc_hash::FxHashMap;
use solstice_asset::gpu::{
    cluster_bones, encode_normal, encode_uv_density, pack_skins, pack_triangle, ClusterBones,
    CLUSTER_UV_DENSITY_SHIFT, MAX_CLUSTER_TRIANGLES, MAX_CLUSTER_VERTICES, ROOT_PARENT_ERROR,
};
use solstice_asset::paged::{ClusterSet, ROOT_ROUND};
use solstice_asset::{PackedCluster, PackedSkin, PackedVertex, VirtualMesh};
use std::ops::Range;

/// Tuning parameters for [`build_virtual_mesh`].
#[derive(Clone, Copy, Debug)]
pub struct BuildOptions {
    pub max_cluster_triangles: usize,
    pub max_cluster_vertices: usize,
    /// A group that cannot shrink below this fraction of its triangles is left
    /// as is for this level and retried in a differently shaped group next level.
    pub min_reduction: f32,
    /// Fraction of triangles a group tries to keep per level.
    pub target_ratio: f32,
    /// Leaves the mesh as its finest clusters alone, with nothing coarser made from them: all
    /// of it is drawn however far off it is. For a small mesh whose points are to be written
    /// over as it is drawn (a thing made as the game is played), which anything simplified
    /// from them would not follow.
    pub flat: bool,
}

impl Default for BuildOptions {
    fn default() -> Self {
        Self {
            max_cluster_triangles: MAX_CLUSTER_TRIANGLES,
            max_cluster_vertices: MAX_CLUSTER_VERTICES,
            min_reduction: 0.85,
            target_ratio: 0.5,
            flat: false,
        }
    }
}

/// How far the grid of a mesh made of separate pieces extends beyond its bounds on each side, as a
/// fraction of its largest dimension.
const FOLIAGE_GRID_MARGIN: f32 = 0.25;

/// How much of a bent mesh's error to allow for beyond its error in the bind pose: simplified
/// bind-pose geometry can come out further from the truth once it bends.
const SKIN_BEND_MARGIN: f32 = 1.25;

/// How far a vertex could land from its place in a pose if its weights changed completely, as a
/// fraction of the mesh's size. It decides how long joints are kept.
const SKIN_ERROR_FRACTION: f32 = 0.025;

/// A cluster during the build. LOD links are filled in as the DAG grows.
struct BuildCluster {
    triangles: Vec<Triangle>,
    level: u32,
    bounds: Sphere,
    /// Sphere and error of the group this cluster was generated from.
    lod_bounds: Sphere,
    lod_error: f32,
    /// Whether some group has been simplified from this cluster.
    has_parent: bool,
}

/// Clusters that swap for the same parents, on their way to becoming one hierarchy leaf.
struct SiblingSet {
    members: Vec<u32>,
    parent_bounds: Sphere,
    parent_error: f32,
    round: u32,
    generated: Range<u32>,
}

/// A group of clusters that was successfully simplified.
struct BuiltGroup {
    lod_bounds: Sphere,
    error: f32,
    children: Vec<u32>,
    /// Which pass of simplification made this group.
    round: u32,
    /// The clusters simplified from this group.
    generated: Range<u32>,
}

/// Mesh data after welding onto the 16-bit grid.
struct WeldedMesh {
    positions: Vec<Vec3>,
    /// Each position's skin, if the mesh is skinned.
    skins: Option<Vec<PackedSkin>>,
    attributes: Vec<Attribute>,
    triangles: Vec<Triangle>,
    origin: Vec3,
    step: f32,
    uv_origin: Vec2,
    uv_step: Vec2,
}

/// Converts a triangle mesh to a cluster LOD DAG.
///
/// The mesh's points are snapped to one 16-bit grid across the whole of it, the same step
/// along all three axes: [`Mesh::grid_step`] says how fine that is. Points nearer each other
/// than a step become one, and what stood less than a step proud of something in another mesh
/// may end up behind it.
pub fn build_virtual_mesh(mesh: &Mesh, options: &BuildOptions) -> VirtualMesh {
    build(mesh, None, options)
}

/// As [`build_virtual_mesh`], for a mesh that bends: `skins` holds each of its vertices' skins.
///
/// The DAG is built in the bind pose. Vertices welded together have the same skin, or they would
/// not stay together in a pose, so simplification keeps each vertex's weights, and a change of
/// weights counts in the error of what it simplifies.
pub fn build_skinned_mesh(
    mesh: &Mesh,
    skins: &[PackedSkin],
    options: &BuildOptions,
) -> VirtualMesh {
    assert_eq!(mesh.vertices.len(), skins.len(), "one skin per vertex");
    build(mesh, Some(skins), options)
}

fn build(mesh: &Mesh, skins: Option<&[PackedSkin]>, options: &BuildOptions) -> VirtualMesh {
    let mut welded = weld(mesh, skins, 0.0);
    // Thinned foliage grows, so give it room on the grid beyond its original bounds.
    if !find_pieces(&welded.triangles, |_| false).pieces.is_empty() {
        welded = weld(mesh, skins, FOLIAGE_GRID_MARGIN);
    }
    let source_triangles = welded.triangles.len() as u64;
    let limits = ClusterLimits {
        max_triangles: options.max_cluster_triangles,
        max_vertices: options.max_cluster_vertices,
    };

    let mut clusters = initial_clusters(&welded, limits);
    let mut groups: Vec<BuiltGroup> = Vec::new();
    let mut active: Vec<u32> = (0..clusters.len() as u32).collect();
    let mut level = 0;

    while active.len() > 1 && !options.flat {
        level += 1;
        active = build_level(
            &mut welded,
            &mut clusters,
            &mut groups,
            active,
            level,
            limits,
            options,
        );
    }

    pack(&welded, clusters, &groups, source_triangles)
}

/// Runs one round of group, simplify, re-split. Returns the clusters that stay
/// active for the next round, or an empty list if this round made no progress.
fn build_level(
    welded: &mut WeldedMesh,
    clusters: &mut Vec<BuildCluster>,
    groups: &mut Vec<BuiltGroup>,
    active: Vec<u32>,
    level: u32,
    limits: ClusterLimits,
    options: &BuildOptions,
) -> Vec<u32> {
    let cluster_triangles: Vec<Vec<[u32; 3]>> = active
        .iter()
        .map(|&c| {
            clusters[c as usize]
                .triangles
                .iter()
                .map(|t| t.map(|corner| corner.position))
                .collect()
        })
        .collect();
    let cluster_bounds: Vec<Sphere> = active
        .iter()
        .map(|&c| clusters[c as usize].bounds)
        .collect();
    let partition = group_clusters(&GroupInput {
        triangles: &cluster_triangles,
        bounds: &cluster_bounds,
        max_group_size: if welded.skins.is_some() {
            MAX_SKINNED_GROUP_SIZE
        } else {
            MAX_GROUP_SIZE
        },
    });

    // A vertex used by two different groups is on a boundary and must not move.
    let mut owner = vec![u32::MAX; welded.positions.len()];
    let mut shared = vec![false; welded.positions.len()];
    for (group_index, group) in partition.iter().enumerate() {
        for &member in group {
            for triangle in &cluster_triangles[member as usize] {
                for &position in triangle {
                    let slot = &mut owner[position as usize];
                    if *slot == u32::MAX {
                        *slot = group_index as u32;
                    } else if *slot != group_index as u32 {
                        shared[position as usize] = true;
                    }
                }
            }
        }
    }

    struct GroupResult {
        members: Vec<u32>,
        lod_bounds: Sphere,
        error: f32,
        /// May refer to `new_positions` through `NEW_POSITION`.
        triangles: Vec<Triangle>,
        new_positions: Vec<Vec3>,
        /// The position each of `new_positions` was moved from.
        new_sources: Vec<u32>,
    }

    let clusters_view: &[BuildCluster] = clusters;
    let welded_view: &WeldedMesh = welded;
    let results: Vec<Option<GroupResult>> = partition
        .par_iter()
        .map(|group| {
            let members: Vec<u32> = group.iter().map(|&m| active[m as usize]).collect();
            let triangles: Vec<Triangle> = members
                .iter()
                .flat_map(|&c| clusters_view[c as usize].triangles.iter().copied())
                .collect();
            let target = |count: usize| ((count as f32 * options.target_ratio) as usize).max(1);

            // Separate pieces, such as blades and leaves, are thinned out. Connected surfaces
            // are simplified by edge collapse.
            let mut found = find_pieces(&triangles, |v| shared[v as usize]);
            set_aside_wide_pieces(&triangles, &mut found, &welded_view.positions);
            let mut new_positions = Vec::new();
            let mut new_sources = Vec::new();
            let mut thinning_error = 0.0f32;
            let mut reduced = Vec::with_capacity(triangles.len());
            if !found.pieces.is_empty() {
                let piece_triangles = found.pieces.iter().map(Vec::len).sum();
                let thinned = thin(
                    &triangles,
                    &found.pieces,
                    &welded_view.positions,
                    &welded_view.attributes,
                    target(piece_triangles),
                );
                reduced.extend(thinned.triangles);
                new_positions = thinned.positions;
                new_sources = thinned.sources;
                thinning_error = thinned.error;
            }
            let rest: Vec<Triangle> = if found.pieces.is_empty() {
                triangles.clone()
            } else {
                found.rest.iter().map(|&t| triangles[t as usize]).collect()
            };
            let mut simplified_error = 0.0f32;
            if !rest.is_empty() {
                let skin = welded_view.skins.as_ref().map(|skins| SkinCost {
                    skins,
                    error_per_change: welded_view.step * 65535.0 * SKIN_ERROR_FRACTION,
                });
                let simplify_to = |count: usize| {
                    simplify_skinned(
                        &rest,
                        &welded_view.positions,
                        &welded_view.attributes,
                        skin,
                        count,
                        |v| shared[v as usize],
                    )
                };
                // A group's error is the largest of its parts'. Where thinning carries the
                // reduction, the rest, such as a branch among twigs, goes only as far as it can
                // without being the coarser part, so it never holds the twigs' detail back: the
                // first of these targets, deepest first, that stays within the twigs' error.
                // One run gives all three, as simplifying to each alone would.
                let simplified = if found.pieces.is_empty() {
                    simplify_to(target(rest.len()))
                } else {
                    let budget = thinning_error.max(welded_view.step);
                    let at = |ratio: f32| ((rest.len() as f32 * ratio) as usize).max(1);
                    let mut tries = simplify_skinned_each(
                        &rest,
                        &welded_view.positions,
                        &welded_view.attributes,
                        skin,
                        &[at(0.9), at(0.75), target(rest.len())],
                        |v| shared[v as usize],
                    );
                    let deepest = tries.pop().expect("three targets");
                    let middle = tries.pop().expect("three targets");
                    let shallow = tries.pop().expect("three targets");
                    [deepest, middle, shallow]
                        .into_iter()
                        .find(|s| s.error <= budget)
                        .unwrap_or_else(|| crate::simplify::Simplified {
                            triangles: rest.clone(),
                            error: 0.0,
                        })
                };
                reduced.extend(simplified.triangles);
                simplified_error = simplified.error;
            }

            if reduced.is_empty()
                || reduced.len() as f32 > triangles.len() as f32 * options.min_reduction
            {
                return None;
            }

            // Spheres and errors must grow monotonically up the DAG. That is what
            // makes the runtime LOD test give a consistent, hole-free cut.
            let lod_bounds = Sphere::merged_all(
                members
                    .iter()
                    .map(|&c| clusters_view[c as usize].lod_bounds),
            );
            let child_error = members
                .iter()
                .map(|&c| clusters_view[c as usize].lod_error)
                .fold(0.0, f32::max);
            // The small increment keeps parents strictly coarser than children
            // even when a collapse on a perfectly flat region costs nothing.
            let error = child_error + simplified_error.max(thinning_error) + welded_view.step * 0.5;

            Some(GroupResult {
                members,
                lod_bounds,
                error,
                triangles: reduced,
                new_positions,
                new_sources,
            })
        })
        .collect();

    // Give thinned pieces' moved vertices real indices, on the mesh's grid.
    let results: Vec<Option<GroupResult>> = results
        .into_iter()
        .map(|result| {
            result.map(|mut result| {
                if !result.new_positions.is_empty() {
                    let base = welded.positions.len() as u32;
                    for (&p, &source) in result.new_positions.iter().zip(&result.new_sources) {
                        let snapped = welded.snap(p);
                        welded.positions.push(snapped);
                        // A piece keeps its skin when it is moved.
                        if let Some(skins) = &mut welded.skins {
                            let skin = skins[source as usize];
                            skins.push(skin);
                        }
                    }
                    for triangle in &mut result.triangles {
                        for corner in triangle {
                            if corner.position & NEW_POSITION != 0 {
                                corner.position = base + (corner.position & !NEW_POSITION);
                            }
                        }
                    }
                }
                result
            })
        })
        .collect();
    let welded_view: &WeldedMesh = welded;
    let split: Vec<Option<Vec<Vec<Triangle>>>> = results
        .par_iter()
        .map(|result| {
            result
                .as_ref()
                .map(|r| split_into_clusters(&r.triangles, &welded_view.positions, limits))
        })
        .collect();

    let mut next_active = Vec::new();
    let mut progressed = false;
    for ((group, result), split) in partition.iter().zip(results).zip(split) {
        let (Some(result), Some(new_clusters)) = (result, split) else {
            // Keep these clusters in play; next level they form different groups.
            next_active.extend(group.iter().map(|&m| active[m as usize]));
            continue;
        };
        progressed = true;
        for &member in &result.members {
            clusters[member as usize].has_parent = true;
        }
        let first_generated = clusters.len() as u32;
        groups.push(BuiltGroup {
            lod_bounds: result.lod_bounds,
            error: result.error,
            children: result.members,
            round: level,
            generated: first_generated..first_generated + new_clusters.len() as u32,
        });
        for triangles in new_clusters {
            let bounds = triangle_bounds(&triangles, &welded.positions);
            next_active.push(clusters.len() as u32);
            clusters.push(BuildCluster {
                triangles,
                level,
                bounds,
                lod_bounds: result.lod_bounds,
                lod_error: result.error,
                has_parent: false,
            });
        }
    }

    if progressed {
        next_active
    } else {
        Vec::new()
    }
}

fn initial_clusters(welded: &WeldedMesh, limits: ClusterLimits) -> Vec<BuildCluster> {
    split_into_clusters(&welded.triangles, &welded.positions, limits)
        .into_iter()
        .map(|triangles| {
            let bounds = triangle_bounds(&triangles, &welded.positions);
            BuildCluster {
                triangles,
                level: 0,
                bounds,
                lod_bounds: bounds,
                lod_error: 0.0,
                has_parent: false,
            }
        })
        .collect()
}

fn split_into_clusters(
    triangles: &[Triangle],
    positions: &[Vec3],
    limits: ClusterLimits,
) -> Vec<Vec<Triangle>> {
    let by_position: Vec<[u32; 3]> = triangles.iter().map(|t| t.map(|c| c.position)).collect();
    let mut clusters = Vec::new();
    for members in partition_triangles(&by_position, positions, limits) {
        let cluster: Vec<Triangle> = members.iter().map(|&t| triangles[t as usize]).collect();
        split_until_indexable(cluster, limits.max_vertices, &mut clusters);
    }
    clusters
}

/// Partitioning counts welded positions, but a cluster stores one vertex for each distinct
/// position and attribute pair. A model with many UV seams or hard edges can therefore stay
/// within the position limit and still exceed what 8-bit indices can address. Halving keeps
/// neighbouring triangles together, because partitioning emits them in the order they grew.
fn split_until_indexable(
    cluster: Vec<Triangle>,
    max_vertices: usize,
    out: &mut Vec<Vec<Triangle>>,
) {
    let mut corners: Vec<Corner> = cluster.iter().flatten().copied().collect();
    corners.sort_unstable_by_key(|c| (c.position, c.attribute));
    corners.dedup();
    if corners.len() <= max_vertices {
        out.push(cluster);
        return;
    }
    let mut first = cluster;
    let second = first.split_off(first.len() / 2);
    split_until_indexable(first, max_vertices, out);
    split_until_indexable(second, max_vertices, out);
}

fn triangle_bounds(triangles: &[Triangle], positions: &[Vec3]) -> Sphere {
    Sphere::from_points(
        triangles
            .iter()
            .flatten()
            .map(|corner| positions[corner.position as usize]),
    )
}

impl WeldedMesh {
    /// The nearest point on the mesh's 16-bit grid, kept within it.
    fn snap(&self, p: Vec3) -> Vec3 {
        let grid = ((p - self.origin) / self.step)
            .round()
            .clamp(Vec3::ZERO, Vec3::splat(65535.0));
        self.origin + grid * self.step
    }
}

/// Snaps vertices to the mesh-wide 16-bit grid and welds them.
///
/// Welding *after* snapping guarantees one grid position has one topological
/// vertex. Triangles that collapse to a line under quantisation are dropped.
fn weld(mesh: &Mesh, input_skins: Option<&[PackedSkin]>, margin: f32) -> WeldedMesh {
    let (mut min, mut max) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
    let (mut uv_min, mut uv_max) = (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN));
    for vertex in &mesh.vertices {
        min = min.min(vertex.position);
        max = max.max(vertex.position);
        uv_min = uv_min.min(vertex.uv);
        uv_max = uv_max.max(vertex.uv);
    }
    if mesh.vertices.is_empty() {
        (min, max, uv_min, uv_max) = (Vec3::ZERO, Vec3::ZERO, Vec2::ZERO, Vec2::ZERO);
    }

    let pad = Vec3::splat((max - min).max_element() * margin);
    let (min, max) = (min - pad, max + pad);
    let step = ((max - min).max_element() / 65535.0).max(1.0e-9);
    let uv_step = ((uv_max - uv_min) / 65535.0).max(Vec2::splat(1.0e-9));

    // A skin belongs to a position, so vertices that differ in skin are different positions,
    // free to part in a pose.
    let mut position_ids: FxHashMap<([u16; 3], PackedSkin), u32> = FxHashMap::default();
    let mut attribute_ids: FxHashMap<[u32; 3], u32> = FxHashMap::default();
    let mut positions = Vec::new();
    let mut skins = input_skins.map(|_| Vec::new());
    let mut attributes = Vec::new();

    let corners: Vec<Corner> = mesh
        .vertices
        .iter()
        .enumerate()
        .map(|(index, vertex)| {
            let grid = ((vertex.position - min) / step).round().as_uvec3();
            let key = [grid.x as u16, grid.y as u16, grid.z as u16];
            let skin = input_skins.map_or(PackedSkin::default(), |s| s[index]);
            let position = *position_ids.entry((key, skin)).or_insert_with(|| {
                positions.push(min + Vec3::new(key[0] as f32, key[1] as f32, key[2] as f32) * step);
                if let Some(skins) = &mut skins {
                    skins.push(skin);
                }
                positions.len() as u32 - 1
            });

            let uv = ((vertex.uv - uv_min) / uv_step).round().as_uvec2();
            let attribute_key = [u32::from(encode_octahedral(vertex.normal)), uv.x, uv.y];
            let attribute = *attribute_ids.entry(attribute_key).or_insert_with(|| {
                attributes.push(Attribute {
                    normal: vertex.normal.normalize_or(Vec3::Y),
                    uv: vertex.uv,
                });
                attributes.len() as u32 - 1
            });
            Corner {
                position,
                attribute,
            }
        })
        .collect();

    let triangles = mesh
        .indices
        .chunks_exact(3)
        .map(|t| {
            [
                corners[t[0] as usize],
                corners[t[1] as usize],
                corners[t[2] as usize],
            ]
        })
        .filter(|t| {
            t[0].position != t[1].position
                && t[1].position != t[2].position
                && t[2].position != t[0].position
        })
        .collect();

    WeldedMesh {
        positions,
        skins,
        attributes,
        triangles,
        origin: min,
        step,
        uv_origin: uv_min,
        uv_step,
    }
}

/// Encodes a unit vector as a vertex holds it: octahedral coordinates, a signed byte each.
fn encode_octahedral(normal: Vec3) -> u16 {
    encode_normal(normal.normalize_or(Vec3::Y).to_array())
}

/// Orders clusters by sibling set and emits the GPU representation.
fn pack(
    welded: &WeldedMesh,
    clusters: Vec<BuildCluster>,
    groups: &[BuiltGroup],
    source_triangles: u64,
) -> VirtualMesh {
    let mut out = VirtualMesh {
        quant_origin: welded.origin,
        quant_step: welded.step,
        uv_origin: welded.uv_origin.to_array(),
        uv_step: welded.uv_step.to_array(),
        source_triangles,
        ..Default::default()
    };
    if clusters.is_empty() {
        return out;
    }

    // Both sides of the choice between a group and its parents use the same margin.
    let bend = if welded.skins.is_some() {
        SKIN_BEND_MARGIN
    } else {
        1.0
    };
    // Sibling sets: children of one group, then parentless clusters in small runs.
    let mut sibling_sets: Vec<SiblingSet> = groups
        .iter()
        .map(|group| SiblingSet {
            members: group.children.clone(),
            parent_bounds: group.lod_bounds,
            parent_error: group.error * bend,
            round: group.round,
            generated: group.generated.clone(),
        })
        .collect();
    let roots: Vec<u32> = (0..clusters.len() as u32)
        .filter(|&c| !clusters[c as usize].has_parent)
        .collect();
    for run in roots.chunks(16) {
        sibling_sets.push(SiblingSet {
            members: run.to_vec(),
            parent_bounds: Sphere::merged_all(run.iter().map(|&c| clusters[c as usize].lod_bounds)),
            parent_error: ROOT_PARENT_ERROR,
            round: ROOT_ROUND,
            generated: 0..0,
        });
    }

    let mut packed_index = vec![0u32; clusters.len()];
    let mut leaves = Vec::with_capacity(sibling_sets.len());
    for set in &sibling_sets {
        let first_cluster = out.clusters.len() as u32;
        for &member in &set.members {
            packed_index[member as usize] = out.clusters.len() as u32;
            let packed = pack_cluster(
                welded,
                &clusters[member as usize],
                bend,
                &mut out.vertices,
                &mut out.triangles,
                &mut out.cluster_bones,
            );
            out.clusters.push(packed);
        }
        leaves.push(Leaf {
            level: clusters[set.members[0] as usize].level,
            first_cluster,
            cluster_count: set.members.len() as u32,
            bounds: Sphere::merged_all(set.members.iter().map(|&m| clusters[m as usize].bounds)),
            parent_lod_bounds: set.parent_bounds,
            parent_error: set.parent_error,
        });
    }

    // Now that clusters have their final positions, record which ones replace each set.
    for (set, leaf) in sibling_sets.iter().zip(&leaves) {
        let first_generated = out.generated.len() as u32;
        out.generated
            .extend(set.generated.clone().map(|c| packed_index[c as usize]));
        out.sets.push(ClusterSet {
            first_cluster: leaf.first_cluster,
            cluster_count: leaf.cluster_count,
            first_generated,
            generated_count: set.generated.len() as u32,
            round: set.round,
            ..Default::default()
        });
    }

    let bounds = Sphere::merged_all(leaves.iter().map(|leaf| leaf.bounds));
    out.bounds_center = bounds.center;
    out.bounds_radius = bounds.radius;

    let hierarchy = build_hierarchy(leaves);
    out.nodes = hierarchy.nodes;
    out.root_node = hierarchy.root;
    out.hierarchy_depth = hierarchy.depth;
    out
}

fn pack_cluster(
    welded: &WeldedMesh,
    cluster: &BuildCluster,
    bend: f32,
    vertices: &mut Vec<PackedVertex>,
    triangles: &mut Vec<u32>,
    bones: &mut Vec<ClusterBones>,
) -> PackedCluster {
    let vertex_offset = vertices.len() as u32;
    let triangle_offset = triangles.len() as u32;
    let mut local: FxHashMap<Corner, u8> = FxHashMap::default();
    let mut skins: Vec<PackedSkin> = Vec::new();

    for triangle in &cluster.triangles {
        let indices = triangle.map(|corner| {
            *local.entry(corner).or_insert_with(|| {
                vertices.push(pack_vertex(welded, corner));
                if let Some(all) = &welded.skins {
                    skins.push(all[corner.position as usize]);
                }
                u8::try_from(vertices.len() as u32 - 1 - vertex_offset)
                    .expect("cluster exceeds 8-bit vertex indexing")
            })
        });
        triangles.push(pack_triangle(indices[0], indices[1], indices[2]));
    }
    if welded.skins.is_some() {
        // End to end, right behind the cluster's vertices.
        vertices.extend(pack_skins(&skins));
        bones.push(cluster_bones(
            skins.iter().flat_map(|s| s.bones()).collect(),
        ));
    }

    PackedCluster {
        center: cluster.bounds.center.to_array(),
        radius: cluster.bounds.radius,
        lod_center: cluster.lod_bounds.center.to_array(),
        lod_radius: cluster.lod_bounds.radius,
        lod_error: cluster.lod_error * bend,
        vertex_offset,
        triangle_offset,
        counts: local.len() as u32
            | (cluster.triangles.len() as u32) << 8
            | encode_uv_density(uv_density(welded, &cluster.triangles)) << CLUSTER_UV_DENSITY_SHIFT,
    }
}

/// Texture repeats per object-space unit across some triangles. It's an area ratio, so seams
/// and slivers with wild UVs count only as much as their small area.
fn uv_density(welded: &WeldedMesh, triangles: &[Triangle]) -> f32 {
    let (mut uv_area, mut area) = (0.0, 0.0);
    for triangle in triangles {
        let p = triangle.map(|corner| welded.positions[corner.position as usize]);
        let uv = triangle.map(|corner| welded.attributes[corner.attribute as usize].uv);
        area += (p[1] - p[0]).cross(p[2] - p[0]).length();
        uv_area += (uv[1] - uv[0]).perp_dot(uv[2] - uv[0]).abs();
    }
    (uv_area / area.max(1.0e-12)).sqrt()
}

fn pack_vertex(welded: &WeldedMesh, corner: Corner) -> PackedVertex {
    let position = welded.positions[corner.position as usize];
    let attribute = welded.attributes[corner.attribute as usize];
    let grid = ((position - welded.origin) / welded.step)
        .round()
        .as_uvec3();
    let uv = ((attribute.uv - welded.uv_origin) / welded.uv_step)
        .round()
        .as_uvec2();
    PackedVertex {
        data: [
            grid.x as u16,
            grid.y as u16,
            grid.z as u16,
            encode_octahedral(attribute.normal),
            uv.x as u16,
            uv.y as u16,
        ],
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::mesh::Vertex;
    use solstice_asset::gpu::unpack_triangle;

    #[test]
    fn a_long_mesh_keeps_its_points_coarsely_however_thin_it_is() {
        // A strip a kilometre long, a metre wide, and 6 mm higher along one side than the
        // other: a line painted on a road, say, with the road's camber.
        let strip = |length: f32| {
            let corner = |x: f32, z: f32, y: f32| Vertex {
                position: Vec3::new(x, y, z),
                normal: Vec3::Y,
                uv: Vec2::new(x, z),
            };
            Mesh {
                vertices: vec![
                    corner(0.0, 0.0, 0.0),
                    corner(1.0, 0.0, 0.006),
                    corner(1.0, length, 0.006),
                    corner(0.0, length, 0.0),
                ],
                indices: vec![0, 2, 1, 0, 3, 2],
            }
        };
        let long = strip(1000.0);
        assert!((long.grid_step() - 1000.0 / 65535.0).abs() < 1.0e-6);
        let built = build_virtual_mesh(&long, &BuildOptions::default());
        assert_eq!(built.quant_step, long.grid_step());
        // Its 6 mm of height is less than half a step: built, it is flat.
        let heights: Vec<u16> = built.vertices.iter().map(|v| v.data[1]).collect();
        assert!(heights.iter().all(|&h| h == heights[0]), "{heights:?}");
        // A hundred metres of it keeps that height, to within a millimetre.
        let short = strip(100.0);
        assert!(short.grid_step() < 0.0016);
        let built = build_virtual_mesh(&short, &BuildOptions::default());
        let heights: Vec<u16> = built.vertices.iter().map(|v| v.data[1]).collect();
        let (low, high) = (heights.iter().min().unwrap(), heights.iter().max().unwrap());
        let risen = f32::from(high - low) * built.quant_step;
        assert!((risen - 0.006).abs() < 0.001, "it rises {risen} m");
        assert_eq!(Mesh::default().grid_step(), 0.0);
    }

    /// A closed subdivided icosphere with bumps, so simplification has real
    /// geometric error to measure.
    pub(crate) fn bumpy_sphere(subdivisions: u32) -> Mesh {
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
        let vertices = positions
            .iter()
            .map(|&p| {
                let bump =
                    1.0 + 0.05 * (p.x * 19.0).sin() * (p.y * 23.0).cos() * (p.z * 17.0).sin();
                Vertex {
                    position: p * bump,
                    normal: p,
                    uv: Vec2::new(p.x, p.y),
                }
            })
            .collect();
        let mut mesh = Mesh {
            vertices,
            indices: faces.into_iter().flatten().collect(),
        };
        mesh.compute_smooth_normals();
        mesh
    }

    /// Selects a DAG cut with the same rule as the GPU culling shader, walking the
    /// real node hierarchy. `error_of` projects a (sphere, error) pair to pixels.
    fn select_cut(
        mesh: &VirtualMesh,
        error_of: &dyn Fn(Sphere, f32) -> f32,
        threshold: f32,
    ) -> Vec<u32> {
        let mut selected = Vec::new();
        let mut stack = vec![mesh.root_node];
        while let Some(node_index) = stack.pop() {
            let node = mesh.nodes[node_index as usize];
            let parent_sphere = Sphere {
                center: node.lod_center.into(),
                radius: node.lod_radius,
            };
            if error_of(parent_sphere, node.max_parent_error) <= threshold {
                continue;
            }
            if node.is_leaf == 0 {
                stack.extend(node.first..node.first + node.count);
                continue;
            }
            for index in node.first..node.first + node.count {
                let cluster = mesh.clusters[index as usize];
                let sphere = Sphere {
                    center: cluster.lod_center.into(),
                    radius: cluster.lod_radius,
                };
                if error_of(sphere, cluster.lod_error) <= threshold {
                    selected.push(index);
                }
            }
        }
        selected
    }

    /// Counts edges in the cut that are not shared by exactly two triangles.
    pub(crate) fn open_edges(mesh: &VirtualMesh, selected: &[u32]) -> usize {
        let mut uses: FxHashMap<([u16; 3], [u16; 3]), u32> = FxHashMap::default();
        for &index in selected {
            let cluster = mesh.clusters[index as usize];
            for t in 0..cluster.triangle_count() {
                let packed = mesh.triangles[(cluster.triangle_offset + t) as usize];
                let grid = unpack_triangle(packed).map(|local| {
                    let data = mesh.vertices[cluster.vertex_offset as usize + local as usize].data;
                    [data[0], data[1], data[2]]
                });
                for side in 0..3 {
                    let (a, b) = (grid[side], grid[(side + 1) % 3]);
                    *uses.entry((a.min(b), a.max(b))).or_insert(0) += 1;
                }
            }
        }
        uses.values().filter(|&&count| count != 2).count()
    }

    /// Surface area of the triangles in a cut.
    fn cut_area(mesh: &VirtualMesh, selected: &[u32]) -> f32 {
        let mut area = 0.0;
        for &index in selected {
            let cluster = mesh.clusters[index as usize];
            for t in 0..cluster.triangle_count() {
                let packed = mesh.triangles[(cluster.triangle_offset + t) as usize];
                let [a, b, c] = unpack_triangle(packed).map(|local| {
                    let data = mesh.vertices[cluster.vertex_offset as usize + local as usize].data;
                    mesh.quant_origin
                        + Vec3::new(data[0] as f32, data[1] as f32, data[2] as f32)
                            * mesh.quant_step
                });
                area += (b - a).cross(c - a).length() * 0.5;
            }
        }
        area
    }

    /// A 10 by 10 m meadow of separate, slightly leaning blades of grass, 4 triangles each.
    fn meadow(per_side: u32) -> Mesh {
        let mut mesh = Mesh::default();
        let spacing = 10.0 / per_side as f32;
        for z in 0..per_side {
            for x in 0..per_side {
                let hash = (x * 7919 + z * 104_729) % 1000;
                let jitter = hash as f32 / 1000.0;
                let root = Vec3::new(
                    x as f32 * spacing + jitter * spacing * 0.5,
                    0.0,
                    z as f32 * spacing,
                );
                let lean = Vec3::new(jitter - 0.5, 0.0, 0.3) * 0.1;
                let height = 0.25 + jitter * 0.15;
                let base = mesh.vertices.len() as u32;
                for (i, (side, up)) in [(0.0, 0.0), (1.0, 0.0), (0.0, 0.5), (1.0, 0.5), (0.5, 1.0)]
                    .into_iter()
                    .enumerate()
                {
                    let _ = i;
                    let width = 0.012 * (1.0 - up);
                    mesh.vertices.push(Vertex {
                        position: root
                            + Vec3::X * (side - 0.5) * width
                            + Vec3::Y * up * height
                            + lean * up * up,
                        normal: Vec3::Z,
                        uv: Vec2::new(side, up),
                    });
                }
                mesh.indices
                    .extend([0, 1, 2, 1, 3, 2, 2, 3, 4].map(|i| base + i));
            }
        }
        mesh
    }

    #[test]
    fn foliage_thins_with_distance_and_keeps_its_coverage() {
        let mesh = build_virtual_mesh(&meadow(120), &BuildOptions::default());
        let source = cut_area(&mesh, &select_cut(&mesh, &|_, error| error, 0.0));
        let coarsest = select_cut(&mesh, &|_, error| error, 1.0e20);
        assert!(
            triangle_total(&mesh, &coarsest) < 2000,
            "coarsest cut still has {} triangles of {}",
            triangle_total(&mesh, &coarsest),
            mesh.source_triangles
        );

        let mut previous = u32::MAX;
        for threshold in [0.001, 0.003, 0.01, 0.03, 0.1, 0.3, 1.0, 3.0, 10.0, 1.0e20] {
            let cut = select_cut(&mesh, &|_, error| error, threshold);
            let triangles = triangle_total(&mesh, &cut);
            assert!(triangles <= previous, "a coarser cut has more triangles");
            previous = triangles;
            let area = cut_area(&mesh, &cut);
            assert!(
                (area / source - 1.0).abs() < 0.15,
                "at error {threshold}, {triangles} triangles cover {area} m² against {source} m²"
            );
        }
    }

    fn triangle_total(mesh: &VirtualMesh, selected: &[u32]) -> u32 {
        selected
            .iter()
            .map(|&c| mesh.clusters[c as usize].triangle_count())
            .sum()
    }

    fn built(subdivisions: u32) -> VirtualMesh {
        build_virtual_mesh(&bumpy_sphere(subdivisions), &BuildOptions::default())
    }

    /// A flat 2 by 2 quad with UVs running 0 to 1 repeats once per 2 units.
    #[test]
    fn uv_density_measures_repeats_per_unit() {
        let welded = WeldedMesh {
            skins: None,
            positions: vec![Vec3::ZERO, Vec3::X * 2.0, Vec3::Y * 2.0],
            attributes: vec![
                Attribute {
                    normal: Vec3::Z,
                    uv: Vec2::ZERO,
                },
                Attribute {
                    normal: Vec3::Z,
                    uv: Vec2::X,
                },
                Attribute {
                    normal: Vec3::Z,
                    uv: Vec2::Y,
                },
            ],
            triangles: Vec::new(),
            origin: Vec3::ZERO,
            step: 1.0,
            uv_origin: Vec2::ZERO,
            uv_step: Vec2::ONE,
        };
        let corner = |index| Corner {
            position: index,
            attribute: index,
        };
        let density = uv_density(&welded, &[[corner(0), corner(1), corner(2)]]);
        assert!((density - 0.5).abs() < 1.0e-5, "{density}");
    }

    #[test]
    fn clusters_respect_format_limits() {
        let mesh = built(5);
        for cluster in &mesh.clusters {
            assert!(cluster.triangle_count() as usize <= MAX_CLUSTER_TRIANGLES);
            assert!(cluster.vertex_count() as usize <= MAX_CLUSTER_VERTICES);
            assert!(cluster.triangle_count() > 0);
        }
    }

    #[test]
    fn dag_reduces_to_a_small_root_set() {
        let mesh = built(6);
        let coarsest = select_cut(&mesh, &|_, error| error, 1.0e20);
        assert!(
            triangle_total(&mesh, &coarsest) < 1000,
            "coarsest cut still has {} triangles of {}",
            triangle_total(&mesh, &coarsest),
            mesh.source_triangles
        );
    }

    #[test]
    fn finest_cut_is_exactly_the_source_mesh() {
        let mesh = built(5);
        let finest = select_cut(&mesh, &|_, error| error, 0.0);
        assert_eq!(
            u64::from(triangle_total(&mesh, &finest)),
            mesh.source_triangles
        );
    }

    /// The defining property of the whole system: any uniform error threshold gives
    /// a closed surface, even though clusters come from many different LOD levels.
    #[test]
    fn every_uniform_cut_is_watertight() {
        let mesh = built(6);
        let mut previous_triangles = u32::MAX;
        for threshold in [
            0.0, 1.0e-4, 5.0e-4, 1.0e-3, 5.0e-3, 0.01, 0.02, 0.05, 0.1, 0.3, 1.0, 10.0,
        ] {
            let cut = select_cut(&mesh, &|_, error| error, threshold);
            assert!(!cut.is_empty(), "threshold {threshold} selected nothing");
            assert_eq!(
                open_edges(&mesh, &cut),
                0,
                "cracks at threshold {threshold}"
            );
            let triangles = triangle_total(&mesh, &cut);
            assert!(
                triangles <= previous_triangles,
                "coarser cut got denser at {threshold}"
            );
            previous_triangles = triangles;
        }
    }

    /// The realistic case: the threshold varies across the mesh with distance from
    /// the camera, so a single frame mixes levels. It must still be watertight.
    #[test]
    fn perspective_cuts_are_watertight_from_any_viewpoint() {
        let mesh = built(6);
        let viewpoints = [
            Vec3::new(0.0, 0.0, 1.5),
            Vec3::new(3.0, 0.5, 0.0),
            Vec3::new(-1.2, 1.2, 1.2),
            Vec3::new(0.0, -8.0, 0.0),
            Vec3::new(40.0, 40.0, 40.0),
        ];
        for viewpoint in viewpoints {
            let project = |sphere: Sphere, error: f32| {
                let distance = (sphere.center.distance(viewpoint) - sphere.radius).max(0.01);
                1000.0 * error / distance
            };
            let cut = select_cut(&mesh, &project, 1.0);
            assert!(!cut.is_empty());
            assert_eq!(open_edges(&mesh, &cut), 0, "cracks seen from {viewpoint}");
        }
    }

    #[test]
    fn nearer_viewpoints_select_more_triangles() {
        let mesh = built(6);
        let triangles_at = |distance: f32| {
            let viewpoint = Vec3::Z * distance;
            let project = |sphere: Sphere, error: f32| {
                1000.0 * error / (sphere.center.distance(viewpoint) - sphere.radius).max(0.01)
            };
            triangle_total(&mesh, &select_cut(&mesh, &project, 1.0))
        };
        assert!(triangles_at(2.0) > triangles_at(50.0));
    }

    #[test]
    fn error_and_bounds_grow_monotonically_toward_the_root() {
        let mesh = built(5);
        fn walk(mesh: &VirtualMesh, node: u32) {
            let packed = mesh.nodes[node as usize];
            let parent = Sphere {
                center: packed.lod_center.into(),
                radius: packed.lod_radius,
            };
            if packed.is_leaf != 0 {
                for c in packed.first..packed.first + packed.count {
                    let cluster = mesh.clusters[c as usize];
                    assert!(cluster.lod_error <= packed.max_parent_error);
                    if packed.max_parent_error < ROOT_PARENT_ERROR {
                        let own = Sphere {
                            center: cluster.lod_center.into(),
                            radius: cluster.lod_radius,
                        };
                        assert!(
                            parent.contains(own),
                            "parent LOD sphere does not contain child"
                        );
                    }
                }
                return;
            }
            for child in packed.first..packed.first + packed.count {
                walk(mesh, child);
            }
        }
        walk(&mesh, mesh.root_node);
    }

    #[test]
    fn empty_mesh_builds_an_empty_result() {
        let mesh = build_virtual_mesh(&Mesh::default(), &BuildOptions::default());
        assert!(mesh.clusters.is_empty());
        assert!(mesh.nodes.is_empty());
    }

    /// Every triangle has its own attributes, as on a flat-shaded or heavily seamed model,
    /// so a full cluster holds three vertices per triangle: more than 8 bits can index.
    #[test]
    fn models_made_of_seams_build_and_stay_watertight() {
        let source = bumpy_sphere(5);
        let mut mesh = Mesh::default();
        for (face, triangle) in source.indices.chunks_exact(3).enumerate() {
            for &index in triangle {
                let mut vertex = source.vertices[index as usize];
                vertex.uv = Vec2::new(face as f32, (face % 7) as f32);
                mesh.indices.push(mesh.vertices.len() as u32);
                mesh.vertices.push(vertex);
            }
        }
        let built = build_virtual_mesh(&mesh, &BuildOptions::default());
        for cluster in &built.clusters {
            assert!(cluster.vertex_count() as usize <= MAX_CLUSTER_VERTICES);
        }
        for threshold in [0.0, 0.01, 0.1, 10.0] {
            let cut = select_cut(&built, &|_, error| error, threshold);
            assert_eq!(
                open_edges(&built, &cut),
                0,
                "cracks at threshold {threshold}"
            );
        }
    }

    /// A sphere's skins: four bones stacked up its height, blending into each other.
    fn stacked_skins(mesh: &Mesh) -> Vec<PackedSkin> {
        mesh.vertices
            .iter()
            .map(|vertex| {
                let height = (vertex.position.y + 1.2) / 2.4 * 3.0;
                let lower = (height.floor() as i32).clamp(0, 2);
                let blend = (height - lower as f32).clamp(0.0, 1.0);
                PackedSkin::new(
                    [lower as u8, lower as u8 + 1, 0, 0],
                    [1.0 - blend, blend, 0.0, 0.0],
                )
            })
            .collect()
    }

    /// Reads the position on the grid and the skin of a cluster's vertex.
    fn skinned_vertex(mesh: &VirtualMesh, cluster: u32, local: u32) -> ([u16; 3], PackedSkin) {
        let cluster = mesh.clusters[cluster as usize];
        let data = mesh.vertices[(cluster.vertex_offset + local) as usize].data;
        let skins = &mesh.vertices[(cluster.vertex_offset + cluster.vertex_count()) as usize..];
        (
            [data[0], data[1], data[2]],
            solstice_asset::gpu::skin_at(skins, local as usize),
        )
    }

    #[test]
    fn a_vertex_shared_between_clusters_has_the_same_skin_in_all_of_them() {
        let source = bumpy_sphere(5);
        let skins = stacked_skins(&source);
        let mesh = build_skinned_mesh(&source, &skins, &BuildOptions::default());
        assert!(mesh.skinned());
        let mut seen: FxHashMap<[u16; 3], PackedSkin> = FxHashMap::default();
        let mut shared = 0;
        for cluster in 0..mesh.clusters.len() as u32 {
            for local in 0..mesh.clusters[cluster as usize].vertex_count() {
                let (grid, skin) = skinned_vertex(&mesh, cluster, local);
                match seen.get(&grid) {
                    Some(other) => {
                        shared += 1;
                        assert_eq!(*other, skin, "a vertex differs between clusters");
                    }
                    None => {
                        seen.insert(grid, skin);
                    }
                }
            }
        }
        assert!(shared > 0);
    }

    /// Where a vertex goes in a pose: each bone turns about the Z axis at its own height.
    fn posed(mesh: &VirtualMesh, cluster: u32, local: u32) -> [u32; 3] {
        let (grid, skin) = skinned_vertex(mesh, cluster, local);
        let p = mesh.quant_origin
            + Vec3::new(grid[0] as f32, grid[1] as f32, grid[2] as f32) * mesh.quant_step;
        let mut sum = Vec3::ZERO;
        for (joint, weight) in skin.joints.iter().zip(skin.weights) {
            let angle = f32::from(*joint) * 0.4;
            let pivot = Vec3::Y * (f32::from(*joint) - 1.5);
            let moved = glam::Quat::from_rotation_z(angle) * (p - pivot) + pivot;
            sum += moved * f32::from(weight) / 255.0;
        }
        sum.to_array().map(f32::to_bits)
    }

    #[test]
    fn every_cut_of_a_skinned_mesh_is_watertight_in_a_pose() {
        let source = bumpy_sphere(5);
        let skins = stacked_skins(&source);
        let mesh = build_skinned_mesh(&source, &skins, &BuildOptions::default());
        for threshold in [0.0, 0.002, 0.01, 0.05, 0.3, 10.0] {
            let cut = select_cut(&mesh, &|_, error| error, threshold);
            assert!(!cut.is_empty());
            let mut uses: FxHashMap<([u32; 3], [u32; 3]), u32> = FxHashMap::default();
            for &index in &cut {
                let cluster = mesh.clusters[index as usize];
                for t in 0..cluster.triangle_count() {
                    let packed = mesh.triangles[(cluster.triangle_offset + t) as usize];
                    let corners =
                        unpack_triangle(packed).map(|local| posed(&mesh, index, u32::from(local)));
                    for side in 0..3 {
                        let (a, b) = (corners[side], corners[(side + 1) % 3]);
                        *uses.entry((a.min(b), a.max(b))).or_insert(0) += 1;
                    }
                }
            }
            let open = uses.values().filter(|&&count| count != 2).count();
            assert_eq!(open, 0, "cracks at threshold {threshold}");
        }
    }

    #[test]
    fn a_cluster_lists_every_bone_its_vertices_use() {
        let source = bumpy_sphere(5);
        let skins = stacked_skins(&source);
        let mesh = build_skinned_mesh(&source, &skins, &BuildOptions::default());
        assert_eq!(mesh.cluster_bones.len(), mesh.clusters.len());
        for cluster in 0..mesh.clusters.len() as u32 {
            let list = &mesh.cluster_bones[cluster as usize];
            for local in 0..mesh.clusters[cluster as usize].vertex_count() {
                let (_, skin) = skinned_vertex(&mesh, cluster, local);
                for bone in skin.bones() {
                    assert!(list.contains(&bone), "bone {bone} isn't on {list:?}");
                }
            }
        }
    }

    #[test]
    fn a_change_of_weights_counts_in_the_error() {
        let source = bumpy_sphere(5);
        // One bone everywhere, for a skinned mesh whose weights never change.
        let same = vec![PackedSkin::new([0, 0, 0, 0], [1.0, 0.0, 0.0, 0.0]); source.vertices.len()];
        let rigid = build_skinned_mesh(&source, &same, &BuildOptions::default());
        // Bones change sharply and often, so that simplifying must cross many boundaries.
        let hard: Vec<PackedSkin> = source
            .vertices
            .iter()
            .map(|vertex| {
                let band = (((vertex.position.y + 1.2) * 12.0) as i32).rem_euclid(2) as u8;
                PackedSkin::new([band, 0, 0, 0], [1.0, 0.0, 0.0, 0.0])
            })
            .collect();
        let bent = build_skinned_mesh(&source, &hard, &BuildOptions::default());
        let worst = |mesh: &VirtualMesh| {
            mesh.clusters
                .iter()
                .map(|c| c.lod_error)
                .fold(0.0_f32, f32::max)
        };
        assert!(
            worst(&bent) > worst(&rigid),
            "{} is not above {}",
            worst(&bent),
            worst(&rigid)
        );
    }

    /// Cards like hair: lots of separate quads, each with its own seams, so clusters are as full
    /// of vertices as they can be, and a set of them is as large as a set can be.
    #[test]
    fn a_full_set_of_skinned_cards_fits_in_a_page() {
        let mut source = Mesh::default();
        for i in 0..6000u32 {
            let (x, z) = ((i % 80) as f32 * 0.03, (i / 80) as f32 * 0.03);
            let base = source.vertices.len() as u32;
            for (u, v) in [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)] {
                source.vertices.push(Vertex {
                    position: Vec3::new(x + u * 0.02, v * 0.02, z),
                    normal: Vec3::Z,
                    uv: Vec2::new(i as f32 + u, v),
                });
            }
            source
                .indices
                .extend([0, 1, 2, 0, 2, 3].map(|corner| base + corner));
        }
        let skins =
            vec![PackedSkin::new([0, 0, 0, 0], [1.0, 0.0, 0.0, 0.0]); source.vertices.len()];
        let mesh = build_skinned_mesh(&source, &skins, &BuildOptions::default());
        let page = solstice_asset::paged::PAGE_BYTES;
        for set in &mesh.sets {
            let bytes: usize = mesh.clusters[set.first_cluster as usize..]
                [..set.cluster_count as usize]
                .iter()
                .map(|c| {
                    c.vertex_records(true) as usize * std::mem::size_of::<PackedVertex>()
                        + solstice_asset::gpu::triangle_words(c.triangle_count()) as usize * 4
                })
                .sum();
            assert!(bytes <= page, "a set takes {bytes} bytes, more than a page");
        }
    }

    #[test]
    fn a_rigid_mesh_carries_no_skin() {
        let mesh = built(4);
        assert!(!mesh.skinned());
        assert!(mesh.cluster_bones.is_empty());
    }

    #[test]
    fn single_triangle_builds() {
        let vertex = |x: f32, y: f32| Vertex {
            position: Vec3::new(x, y, 0.0),
            normal: Vec3::Z,
            uv: Vec2::ZERO,
        };
        let mesh = Mesh {
            vertices: vec![vertex(0.0, 0.0), vertex(1.0, 0.0), vertex(0.0, 1.0)],
            indices: vec![0, 1, 2],
        };
        let built = build_virtual_mesh(&mesh, &BuildOptions::default());
        assert_eq!(built.clusters.len(), 1);
        assert_eq!(built.source_triangles, 1);
    }
}
