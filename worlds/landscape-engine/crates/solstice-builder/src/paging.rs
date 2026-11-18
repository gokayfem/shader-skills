//! Divides a finished scene into pages for on-demand loading.
//!
//! Page ids run coarse to fine, and a page only holds sets from one simplification round. A
//! set's parents always come from a later round, so page dependencies can't form a cycle, and
//! they always point at a lower id.

use rustc_hash::FxHashMap;
use solstice_asset::{
    gpu::{pack_triangle_words, triangle_words, CLUSTER_CHILDREN_MISSING, NODE_LEAF, VERTEX_WORDS},
    paged::{
        upper_mip_count, ClusterSet, LoadedScene, MeshInfo, PageInfo, PagedScene, PAGE_BYTES,
        ROOT_ROUND,
    },
    PackedNode, PackedVertex, SceneAsset, TextureLayer, VirtualMesh,
};
use std::ops::Range;

/// Identical payloads may share an offset within a page. Keys compare every byte, so this
/// preserves quantized positions, attributes, skin weights and triangle order exactly.
///
/// `data` is the page as it will be stored: its triangles are words of three-byte triangles,
/// where the clusters coming in have a word for each.
#[derive(Default)]
struct PageRecords {
    data: ClusterRecords,
    vertices: FxHashMap<Vec<u8>, u32>,
    triangles: FxHashMap<Vec<u32>, u32>,
    clusters: usize,
}

// External cluster tables reserve the largest page's record count for every pool slot.
// Sharing a tiny payload must not pack unbounded metadata into one otherwise empty page.
const MAX_PAGE_CLUSTERS: usize = PAGE_BYTES / std::mem::size_of::<solstice_asset::PackedCluster>();

struct PageCluster {
    id: u32,
    vertices: Range<usize>,
    triangles: Range<usize>,
}

impl PageRecords {
    fn append(&mut self, source: &ClusterRecords, ranges: &[PageCluster], out: &mut PagedScene) {
        self.clusters += ranges.len();
        for range in ranges {
            let vertices = &source.vertices[range.vertices.clone()];
            let vertex_key: &[u8] = bytemuck::cast_slice(vertices);
            let vertex_offset = match self.vertices.get(vertex_key) {
                Some(&offset) => offset,
                None => {
                    let offset = self.data.vertices.len() as u32;
                    self.data.vertices.extend_from_slice(vertices);
                    self.vertices.insert(vertex_key.to_vec(), offset);
                    offset
                }
            };
            let mut triangles = Vec::new();
            pack_triangle_words(&source.triangles[range.triangles.clone()], &mut triangles);
            let triangle_offset = match self.triangles.get(&triangles) {
                Some(&offset) => offset,
                None => {
                    let offset = self.data.triangles.len() as u32;
                    self.data.triangles.extend_from_slice(&triangles);
                    self.triangles.insert(triangles, offset);
                    offset
                }
            };
            // Both in words, as the shaders find them.
            let cluster = &mut out.clusters[range.id as usize];
            cluster.vertex_offset = vertex_offset * VERTEX_WORDS;
            cluster.triangle_offset = triangle_offset;
        }
    }

    fn rollback(&mut self, vertices: usize, triangles: usize, clusters: usize) {
        self.clusters = clusters;
        self.data.vertices.truncate(vertices);
        self.data.triangles.truncate(triangles);
        self.vertices
            .retain(|_, offset| (*offset as usize) < vertices);
        self.triangles
            .retain(|_, offset| (*offset as usize) < triangles);
    }

    fn clear(&mut self) {
        self.clusters = 0;
        self.data.vertices.clear();
        self.data.triangles.clear();
        self.vertices.clear();
        self.triangles.clear();
    }
}

/// One set on its way into a page.
struct Item {
    round: u32,
    mesh: u32,
    /// Position along a space-filling curve within the mesh, so a page covers one area.
    curve: u32,
    set: ClusterSet,
    /// Where this mesh's clusters begin in the scene.
    cluster_base: u32,
}

/// Supplies the vertices and triangles of the clusters being paged, which may not be in memory.
pub trait ClusterStore {
    /// Appends a cluster's records, found at these offsets within its mesh.
    fn read(&mut self, mesh: u32, range: ClusterRange, out: &mut ClusterRecords);
}

/// Where one cluster's records are within its mesh.
#[derive(Clone, Copy, Debug)]
pub struct ClusterRange {
    pub first_vertex: usize,
    pub vertices: usize,
    pub first_triangle: usize,
    pub triangles: usize,
}

/// The records of the page being filled.
#[derive(Default)]
pub struct ClusterRecords {
    pub vertices: Vec<PackedVertex>,
    pub triangles: Vec<u32>,
}

/// Reads clusters from the meshes of a scene held in memory.
struct InMemory<'a>(&'a [VirtualMesh]);

impl ClusterStore for InMemory<'_> {
    fn read(&mut self, mesh: u32, range: ClusterRange, out: &mut ClusterRecords) {
        let mesh = &self.0[mesh as usize];
        out.vertices
            .extend_from_slice(&mesh.vertices[range.first_vertex..][..range.vertices]);
        out.triangles
            .extend_from_slice(&mesh.triangles[range.first_triangle..][..range.triangles]);
    }
}

/// Divides `scene` into pages. Textures split into the base levels and those above.
pub fn paginate(scene: &SceneAsset) -> LoadedScene {
    let mut pages = Vec::new();
    let Ok(mut out) = paginate_from(scene, &mut InMemory(&scene.meshes), &mut |page, _| {
        pages.push(page);
        Ok::<(), std::convert::Infallible>(())
    });
    let upper_textures = split_textures(scene, &mut out);
    LoadedScene {
        scene: out,
        pages,
        upper_textures,
        collision: crate::collision::build_collision(scene),
    }
}

/// Receives each page that is not pinned as it is finished, in order: its bytes and its sizes.
pub type PageSink<'a, E> = dyn FnMut(Vec<u8>, &PageInfo) -> Result<(), E> + 'a;

/// Divides the meshes of `scene` into pages, without ever holding their vertices and triangles
/// together: `store` supplies them a cluster at a time, and `sink` receives each page that is not
/// pinned, in order, with its sizes. Meshes may have left their `vertices` and `triangles` empty.
/// An error from `sink` stops the work and is returned.
///
/// Textures and collision are left to the caller, which may not want them in memory either.
pub fn paginate_from<E>(
    scene: &SceneAsset,
    store: &mut dyn ClusterStore,
    sink: &mut PageSink<'_, E>,
) -> Result<PagedScene, E> {
    let mut out = PagedScene {
        materials: scene.materials.clone(),
        instances: scene.instances.clone(),
        lights: scene.lights.clone(),
        viewpoint: scene.viewpoint,
        sea: scene.sea.clone(),
        still_water: scene.still_water.clone(),
        gameplay: scene.gameplay.clone(),
        sources: scene.sources.clone(),
        hierarchy_depth: 1,
        source_triangles: scene.total_source_triangles(),
        unique_triangles: scene.unique_source_triangles(),
        skeletons: scene.skeletons.clone(),
        poses: scene.poses.clone(),
        instance_poses: scene.instance_poses.clone(),
        clips: scene.clips.clone(),
        ..Default::default()
    };

    let mut items = Vec::new();
    for (mesh_index, mesh) in scene.meshes.iter().enumerate() {
        let node_base = out.nodes.len() as u32;
        let cluster_base = out.clusters.len() as u32;
        let generated_base = out.generated.len() as u32;

        out.meshes.push(MeshInfo {
            root_node: (!mesh.clusters.is_empty()).then_some(mesh.root_node + node_base),
            quant_origin: mesh.quant_origin,
            quant_step: mesh.quant_step,
            uv_origin: mesh.uv_origin,
            uv_step: mesh.uv_step,
            bone_lists: mesh
                .skinned()
                .then(|| out.cluster_bones.len() as i32 - cluster_base as i32),
        });
        out.cluster_bones.extend_from_slice(&mesh.cluster_bones);
        out.hierarchy_depth = out.hierarchy_depth.max(mesh.hierarchy_depth);
        out.nodes.extend(mesh.nodes.iter().map(|node| PackedNode {
            first: node.first
                + if node.is_leaf != 0 {
                    cluster_base
                } else {
                    node_base
                },
            ..*node
        }));
        out.clusters.extend_from_slice(&mesh.clusters);
        out.generated
            .extend(mesh.generated.iter().map(|&c| c + cluster_base));

        let leaf_of_set = leaf_nodes(mesh);
        let curve = space_curve(mesh, &leaf_of_set);
        for (index, set) in mesh.sets.iter().enumerate() {
            // A set with no parents to fall back on must always be there.
            let round = if set.generated_count == 0 {
                ROOT_ROUND
            } else {
                set.round
            };
            items.push(Item {
                round,
                mesh: mesh_index as u32,
                curve: curve[index],
                cluster_base,
                set: ClusterSet {
                    first_cluster: set.first_cluster + cluster_base,
                    first_generated: set.first_generated + generated_base,
                    leaf_node: leaf_of_set[index] + node_base,
                    round,
                    ..*set
                },
            });
        }
    }

    // Coarsest first. The roots lead, as `ROOT_ROUND` is the largest round. Within a round
    // meshes keep the order they were made in, which puts like beside like: sorted instead by
    // where their instances stand, every view of the scene this was measured on wanted 3 to 4%
    // more pages (`docs/streaming.md`).
    items.sort_by_key(|item| (std::cmp::Reverse(item.round), item.mesh, item.curve));

    let mut filling = Filling::default();
    let mut current_round = None;
    let mut placed = vec![false; items.len()];
    for index in 0..items.len() {
        if placed[index] {
            continue;
        }
        let item = &items[index];
        if current_round.is_some_and(|round| round != item.round)
            || filling.records.clusters + item.set.cluster_count as usize > MAX_PAGE_CLUSTERS
        {
            finish_page(&mut out, &mut filling, current_round, sink)?;
        }
        current_round = Some(item.round);

        // Try the whole sibling set before closing a page: shared payloads can make it fit
        // where its unshared size would not. An overflowing set moves intact to the next page.
        if !filling.place(scene, item, store, &mut out) {
            // First, though, the room it leaves is filled from the few sets that follow it in
            // its mesh and round, where any are small enough: a pool of fixed slots holds only
            // as much as its pages are full. Only the few: they lie beside it, so a view that
            // wants this page wants them too. Filled from further off, pages were fuller and
            // every view wanted a quarter more of them.
            for later in index + 1..items.len().min(index + 1 + FILL_WINDOW) {
                let room = PAGE_BYTES - filling.current.raw_bytes();
                let candidate = &items[later];
                if candidate.round != item.round
                    || candidate.mesh != item.mesh
                    || room < SMALLEST_FILL
                {
                    break;
                }
                if !placed[later]
                    && unshared_bytes(scene, candidate) <= room
                    && filling.records.clusters + candidate.set.cluster_count as usize
                        <= MAX_PAGE_CLUSTERS
                {
                    placed[later] = filling.place(scene, candidate, store, &mut out);
                }
            }
            finish_page(&mut out, &mut filling, current_round, sink)?;
            let fits = filling.place(scene, item, store, &mut out);
            assert!(fits, "a sibling set must fit in a page");
        }
    }
    if filling.current.set_count > 0 {
        finish_page(&mut out, &mut filling, current_round, sink)?;
    }

    // Nothing is installed yet: leaves know their page, and generated clusters stand in for
    // the sets simplified into them.
    for set in &out.sets {
        out.nodes[set.leaf_node as usize].is_leaf = NODE_LEAF | set.page;
    }
    for &cluster in &out.generated {
        out.clusters[cluster as usize].counts |= CLUSTER_CHILDREN_MISSING;
    }
    Ok(out)
}

/// How many of the sets that follow one that won't fit are looked through for some that will.
const FILL_WINDOW: usize = 12;
/// Room in a page not worth looking for a set to fill.
const SMALLEST_FILL: usize = 512;

/// What a set takes of a page if it shares nothing with what is already there.
fn unshared_bytes(scene: &SceneAsset, item: &Item) -> usize {
    let mesh = &scene.meshes[item.mesh as usize];
    let skinned = mesh.skinned();
    let first = (item.set.first_cluster - item.cluster_base) as usize;
    mesh.clusters[first..first + item.set.cluster_count as usize]
        .iter()
        .map(|cluster| {
            cluster.vertex_records(skinned) as usize * std::mem::size_of::<PackedVertex>()
                + triangle_words(cluster.triangle_count()) as usize * 4
        })
        .sum()
}

/// The page being filled.
#[derive(Default)]
struct Filling {
    records: PageRecords,
    current: PageInfo,
    incoming: ClusterRecords,
    ranges: Vec<PageCluster>,
}

impl Filling {
    /// Adds a set to the page, unless the page would be too large with it: then the page is
    /// left as it was, and this returns false.
    fn place(
        &mut self,
        scene: &SceneAsset,
        item: &Item,
        store: &mut dyn ClusterStore,
        out: &mut PagedScene,
    ) -> bool {
        assert!(
            unshared_bytes(scene, item) <= PAGE_BYTES,
            "a sibling set must fit in a page"
        );
        let mesh = &scene.meshes[item.mesh as usize];
        let skinned = mesh.skinned();
        self.incoming.vertices.clear();
        self.incoming.triangles.clear();
        self.ranges.clear();
        for c in item.set.first_cluster..item.set.first_cluster + item.set.cluster_count {
            let cluster = mesh.clusters[(c - item.cluster_base) as usize];
            let vertex_start = self.incoming.vertices.len();
            let triangle_start = self.incoming.triangles.len();
            store.read(
                item.mesh,
                ClusterRange {
                    first_vertex: cluster.vertex_offset as usize,
                    vertices: cluster.vertex_records(skinned) as usize,
                    first_triangle: cluster.triangle_offset as usize,
                    triangles: cluster.triangle_count() as usize,
                },
                &mut self.incoming,
            );
            self.ranges.push(PageCluster {
                id: c,
                vertices: vertex_start..self.incoming.vertices.len(),
                triangles: triangle_start..self.incoming.triangles.len(),
            });
        }
        let previous_clusters = self.records.clusters;
        self.records.append(&self.incoming, &self.ranges, out);
        if self.records.data.vertices.len() * std::mem::size_of::<PackedVertex>()
            + self.records.data.triangles.len() * 4
            > PAGE_BYTES
        {
            self.records.rollback(
                self.current.vertex_count as usize,
                self.current.triangle_words as usize,
                previous_clusters,
            );
            return false;
        }
        self.current.vertex_count = self.records.data.vertices.len() as u32;
        self.current.triangle_words = self.records.data.triangles.len() as u32;
        self.current.set_count += 1;
        out.sets.push(ClusterSet {
            page: out.pages.len() as u32,
            ..item.set
        });
        if item.round == ROOT_ROUND {
            out.pinned_pages = out.pages.len() as u32 + 1;
        }
        true
    }
}

fn finish_page<E>(
    out: &mut PagedScene,
    filling: &mut Filling,
    round: Option<u32>,
    sink: &mut PageSink<'_, E>,
) -> Result<(), E> {
    let current = &mut filling.current;
    let records = &mut filling.records;
    let mut bytes = Vec::with_capacity(current.raw_bytes());
    bytes.extend_from_slice(bytemuck::cast_slice(&records.data.vertices));
    bytes.extend_from_slice(bytemuck::cast_slice(&records.data.triangles));
    // The roots lead, so pinned pages are the first of them.
    if round == Some(ROOT_ROUND) {
        out.pinned_data.extend_from_slice(&bytes);
    } else {
        sink(bytes, current)?;
    }
    current.first_set = out.sets.len() as u32 - current.set_count;
    out.pages.push(*current);
    *current = PageInfo::default();
    records.clear();
    Ok(())
}

/// Finds the hierarchy leaf of each sibling set.
fn leaf_nodes(mesh: &VirtualMesh) -> Vec<u32> {
    let mut set_of_first = rustc_hash::FxHashMap::default();
    for (index, set) in mesh.sets.iter().enumerate() {
        set_of_first.insert(set.first_cluster, index);
    }
    let mut leaves = vec![u32::MAX; mesh.sets.len()];
    for (index, node) in mesh.nodes.iter().enumerate() {
        if node.is_leaf != 0 {
            leaves[set_of_first[&node.first]] = index as u32;
        }
    }
    assert!(
        leaves.iter().all(|&leaf| leaf != u32::MAX),
        "every sibling set has a leaf"
    );
    leaves
}

/// Places each set on a Morton curve through the mesh's bounds.
fn space_curve(mesh: &VirtualMesh, leaf_of_set: &[u32]) -> Vec<u32> {
    let centers: Vec<[f32; 3]> = leaf_of_set
        .iter()
        .map(|&leaf| mesh.nodes[leaf as usize].center)
        .collect();
    morton_codes(&centers)
}

/// Where each point is along a Morton curve through the box that holds them all.
fn morton_codes(centers: &[[f32; 3]]) -> Vec<u32> {
    fn spread(mut x: u32) -> u32 {
        x &= 0x3ff;
        x = (x | x << 16) & 0x030000ff;
        x = (x | x << 8) & 0x0300f00f;
        x = (x | x << 4) & 0x030c30c3;
        (x | x << 2) & 0x09249249
    }
    let mut low = [f32::MAX; 3];
    let mut high = [f32::MIN; 3];
    for center in centers {
        for axis in 0..3 {
            low[axis] = low[axis].min(center[axis]);
            high[axis] = high[axis].max(center[axis]);
        }
    }
    let extent = (0..3)
        .map(|axis| high[axis] - low[axis])
        .fold(1.0e-6, f32::max);
    centers
        .iter()
        .map(|center| {
            let cell = |axis: usize| ((center[axis] - low[axis]) / extent * 1023.0) as u32;
            spread(cell(0)) | spread(cell(1)) << 1 | spread(cell(2)) << 2
        })
        .collect()
}

/// Moves each layer's levels from [`BASE_TEXTURE_EDGE`](solstice_asset::paged::BASE_TEXTURE_EDGE)
/// down to the manifest, and returns those above it.
fn split_textures(scene: &SceneAsset, out: &mut PagedScene) -> Vec<TextureLayer> {
    out.texture_size = scene.texture_size;
    out.texture_encoding = scene.texture_encoding;
    let upper_count = upper_mip_count(scene.texture_size);
    let mut upper = Vec::with_capacity(scene.textures.len());
    for layer in &scene.textures {
        upper.push(TextureLayer {
            color_mips: layer.color_mips[..upper_count].to_vec(),
            normal_mips: layer.normal_mips[..upper_count].to_vec(),
            material_mips: layer.material_mips[..upper_count].to_vec(),
        });
        out.base_textures.push(TextureLayer {
            color_mips: layer.color_mips[upper_count..].to_vec(),
            normal_mips: layer.normal_mips[upper_count..].to_vec(),
            material_mips: layer.material_mips[upper_count..].to_vec(),
        });
    }
    upper
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dag::tests::{bumpy_sphere, open_edges};
    use crate::Sphere;
    use crate::{build_virtual_mesh, BuildOptions};
    use glam::{Mat4, Vec3};
    use solstice_asset::gpu::{triangle_at, NODE_PAGE_MASK};
    use solstice_asset::{unpack_triangle, Instance, Material};

    fn scene(subdivisions: &[u32]) -> SceneAsset {
        let meshes: Vec<VirtualMesh> = subdivisions
            .iter()
            .map(|&s| build_virtual_mesh(&bumpy_sphere(s), &BuildOptions::default()))
            .collect();
        SceneAsset {
            instances: (0..meshes.len() as u32)
                .map(|mesh| Instance {
                    mesh,
                    material: 0,
                    transform: Mat4::IDENTITY,
                })
                .collect(),
            meshes,
            materials: vec![Material::default()],
            ..Default::default()
        }
    }

    #[test]
    fn identical_payloads_share_storage_but_different_attributes_do_not() {
        let mut source = scene(&[0, 0]);
        let loaded = paginate(&source);
        assert_eq!(loaded.scene.pages.len(), 1);
        let [first, second] = loaded.scene.clusters.as_slice() else {
            panic!("each simple mesh should make one cluster");
        };
        assert_eq!(first.vertex_offset, second.vertex_offset);
        assert_eq!(first.triangle_offset, second.triangle_offset);
        assert_eq!(loaded.scene.pages[0].vertex_count, first.vertex_count());
        assert_eq!(
            loaded.scene.pages[0].triangle_words,
            triangle_words(first.triangle_count())
        );
        // One bit of a packed normal must prevent vertex sharing. Topology can still share.
        source.meshes[1].vertices[0].data[3] ^= 1;
        let loaded = paginate(&source);
        let first = &loaded.scene.clusters[0];
        let second = &loaded.scene.clusters[1];
        assert_ne!(first.vertex_offset, second.vertex_offset);
        assert_eq!(first.triangle_offset, second.triangle_offset);
        assert_eq!(loaded.scene.pages[0].vertex_count, first.vertex_count() * 2);
        let encoded = solstice_asset::file::encode(&loaded);
        let decoded = solstice_asset::file::decode(&encoded).unwrap();
        assert_eq!(decoded.scene.clusters, loaded.scene.clusters);
        assert_eq!(decoded.scene.pinned_data, loaded.scene.pinned_data);
    }

    #[test]
    fn sharing_payloads_does_not_make_a_pages_cluster_table_unbounded() {
        let mut source = scene(&[0]);
        source
            .meshes
            .resize(MAX_PAGE_CLUSTERS + 1, source.meshes[0].clone());
        let loaded = paginate(&source);
        assert_eq!(loaded.scene.pages.len(), 2);
        assert!(loaded.scene.max_page_clusters() as usize <= MAX_PAGE_CLUSTERS);
    }

    #[test]
    fn a_skinned_cluster_keeps_its_skins_behind_its_vertices_in_its_page() {
        use solstice_asset::PackedSkin;
        let source = bumpy_sphere(4);
        let skins: Vec<PackedSkin> = (0..source.vertices.len())
            .map(|i| PackedSkin::new([(i % 7) as u8, 0, 0, 0], [1.0, 0.0, 0.0, 0.0]))
            .collect();
        let scene = SceneAsset {
            meshes: vec![crate::build_skinned_mesh(
                &source,
                &skins,
                &BuildOptions::default(),
            )],
            instances: vec![Instance {
                mesh: 0,
                material: 0,
                transform: Mat4::IDENTITY,
            }],
            materials: vec![Material::default()],
            skeletons: vec![solstice_asset::Skeleton::default()],
            poses: vec![0],
            instance_poses: vec![0],
            ..Default::default()
        };
        let loaded = paginate(&scene);
        let paged = &loaded.scene;
        assert_eq!(paged.poses, [0]);
        assert_eq!(paged.instance_poses, [0]);
        assert_eq!(paged.meshes[0].bone_lists, Some(0));
        assert_eq!(paged.cluster_bones.len(), paged.clusters.len());
        for set in &paged.sets {
            let page = loaded.page(set.page);
            let info = paged.pages[set.page as usize];
            let records: &[PackedVertex] = bytemuck::cast_slice(&page[..info.vertex_bytes()]);
            for c in set.first_cluster..set.first_cluster + set.cluster_count {
                let cluster = paged.clusters[c as usize];
                let source_cluster = scene.meshes[0].clusters[c as usize];
                let (at, from) = (
                    (cluster.vertex_offset / VERTEX_WORDS) as usize,
                    source_cluster.vertex_offset as usize,
                );
                let count = cluster.vertex_records(true) as usize;
                assert_eq!(
                    records[at..at + count],
                    scene.meshes[0].vertices[from..from + count]
                );
            }
        }
    }

    #[test]
    fn every_cluster_lands_in_one_page_with_its_own_data() {
        let source = scene(&[5, 2, 3]);
        let loaded = paginate(&source);
        let paged = &loaded.scene;
        let mut seen = vec![0; paged.clusters.len()];
        for set in &paged.sets {
            let page = paged.pages[set.page as usize];
            for c in set.first_cluster..set.first_cluster + set.cluster_count {
                seen[c as usize] += 1;
                let cluster = paged.clusters[c as usize];
                assert_eq!(cluster.vertex_offset % VERTEX_WORDS, 0);
                assert!(
                    cluster.vertex_offset / VERTEX_WORDS + cluster.vertex_count()
                        <= page.vertex_count
                );
                assert!(
                    cluster.triangle_offset + triangle_words(cluster.triangle_count())
                        <= page.triangle_words
                );
            }
        }
        assert!(seen.iter().all(|&count| count == 1));

        // The triangles a page yields are the triangles the mesh built, in position.
        let mut base = 0u32;
        for mesh in &source.meshes {
            for (local, original) in mesh.clusters.iter().enumerate() {
                let cluster = paged.clusters[base as usize + local];
                let set = paged
                    .sets
                    .iter()
                    .find(|s| {
                        (s.first_cluster..s.first_cluster + s.cluster_count)
                            .contains(&(base + local as u32))
                    })
                    .unwrap();
                let bytes = loaded.page(set.page);
                let page = paged.pages[set.page as usize];
                let (v, t) = bytes.split_at(page.vertex_bytes());
                let v: &[PackedVertex] = bytemuck::cast_slice(v);
                let t: Vec<u32> = t
                    .chunks_exact(4)
                    .map(|w| u32::from_le_bytes(w.try_into().unwrap()))
                    .collect();
                for i in 0..original.triangle_count() {
                    let a = triangle_at(&t[cluster.triangle_offset as usize..], i);
                    let b = mesh.triangles[(original.triangle_offset + i) as usize];
                    assert_eq!(a, b);
                    for corner in unpack_triangle(a) {
                        assert_eq!(
                            v[(cluster.vertex_offset / VERTEX_WORDS) as usize + corner as usize],
                            mesh.vertices[original.vertex_offset as usize + corner as usize]
                        );
                    }
                }
            }
            base += mesh.clusters.len() as u32;
        }
    }

    #[test]
    fn room_left_in_a_page_is_filled_from_the_sets_that_follow() {
        let source = scene(&[6, 5, 4, 2]);
        let loaded = paginate(&source);
        let paged = &loaded.scene;
        // Which mesh a set belongs to: each mesh's clusters follow the one before's.
        let mesh_of = |set: &ClusterSet| {
            let mut first = 0;
            source
                .meshes
                .iter()
                .position(|mesh| {
                    first += mesh.clusters.len() as u32;
                    set.first_cluster < first
                })
                .unwrap()
        };
        let sets_of = |page: &PageInfo| {
            &paged.sets[page.first_set as usize..(page.first_set + page.set_count) as usize]
        };
        let size = |set: &ClusterSet| -> usize {
            paged.clusters[set.first_cluster as usize..][..set.cluster_count as usize]
                .iter()
                .map(|c| {
                    c.vertex_count() as usize * std::mem::size_of::<PackedVertex>()
                        + triangle_words(c.triangle_count()) as usize * 4
                })
                .sum()
        };
        let mut pairs = 0;
        for pair in paged.pages.windows(2) {
            let (page, next) = (&pair[0], &pair[1]);
            if sets_of(page)[0].round != sets_of(next)[0].round {
                continue;
            }
            // The next page opens with the set that would not fit. The set after it there, if
            // of the same mesh, was looked at before this page closed, and would have gone in
            // it had there been room.
            let room = PAGE_BYTES - page.raw_bytes();
            let following = sets_of(next);
            if room >= SMALLEST_FILL
                && (page.set_count as usize) < FILL_WINDOW
                && following.len() > 1
                && mesh_of(&following[0]) == mesh_of(&following[1])
            {
                pairs += 1;
                let set = &following[1];
                assert!(size(set) > room, "a set of {} fits {room}", size(set));
            }
        }
        assert!(pairs > 0, "the scene is too small to test this");
    }

    #[test]
    fn pages_fit_and_hold_one_round() {
        let loaded = paginate(&scene(&[6, 2]));
        let paged = &loaded.scene;
        assert!(paged.pages.len() > 2);
        for (index, page) in paged.pages.iter().enumerate() {
            assert!(page.raw_bytes() <= PAGE_BYTES);
            assert_eq!(loaded.page(index as u32).len(), page.raw_bytes());
            let sets =
                &paged.sets[page.first_set as usize..(page.first_set + page.set_count) as usize];
            assert!(sets
                .iter()
                .all(|s| s.page == index as u32 && s.round == sets[0].round));
        }
    }

    #[test]
    fn roots_are_pinned_and_dependencies_only_point_coarser() {
        let loaded = paginate(&scene(&[6, 4]));
        let paged = &loaded.scene;
        assert!(paged.pinned_pages > 0);
        assert!(paged.pinned_pages as usize <= paged.pages.len());
        for set in &paged.sets {
            assert_eq!(set.page < paged.pinned_pages, set.round == ROOT_ROUND);
        }
        let dependencies = paged.page_dependencies();
        for (page, list) in dependencies.iter().enumerate() {
            assert!(
                list.iter().all(|&d| (d as usize) < page),
                "page {page}: {list:?}"
            );
            if page < paged.pinned_pages as usize {
                assert!(list.is_empty(), "pinned pages depend on nothing");
            } else {
                assert!(!list.is_empty(), "other pages have parents");
            }
        }
    }

    #[test]
    fn nodes_and_clusters_start_uninstalled() {
        let loaded = paginate(&scene(&[4]));
        let paged = &loaded.scene;
        for set in &paged.sets {
            assert_eq!(
                paged.nodes[set.leaf_node as usize].is_leaf,
                NODE_LEAF | set.page
            );
            assert_eq!(paged.nodes[set.leaf_node as usize].first, set.first_cluster);
        }
        let finest = paged.clusters.iter().filter(|c| c.lod_error == 0.0).count();
        let waiting = paged
            .clusters
            .iter()
            .filter(|c| c.counts & CLUSTER_CHILDREN_MISSING != 0)
            .count();
        assert_eq!(finest + waiting, paged.clusters.len());
    }

    /// Selects a cut with the same rule as `vg/cull.wgsl`, given which pages are installed. A
    /// one-mesh scene keeps the mesh's cluster numbering, so the mesh's data can be read back.
    fn select_resident_cut(
        paged: &PagedScene,
        installed: &[bool],
        error_of: &dyn Fn(Sphere, f32) -> f32,
        threshold: f32,
    ) -> Vec<u32> {
        let mut children_missing = vec![false; paged.clusters.len()];
        for set in &paged.sets {
            if !installed[set.page as usize] {
                let range = set.first_generated as usize
                    ..(set.first_generated + set.generated_count) as usize;
                for &parent in &paged.generated[range] {
                    children_missing[parent as usize] = true;
                }
            }
        }

        let mut selected = Vec::new();
        let mut stack = vec![paged.meshes[0].root_node.unwrap()];
        while let Some(index) = stack.pop() {
            let node = paged.nodes[index as usize];
            let sphere = Sphere {
                center: node.lod_center.into(),
                radius: node.lod_radius,
            };
            if error_of(sphere, node.max_parent_error) <= threshold {
                continue;
            }
            if node.is_leaf == 0 {
                stack.extend(node.first..node.first + node.count);
                continue;
            }
            if !installed[(node.is_leaf & NODE_PAGE_MASK) as usize] {
                continue;
            }
            for c in node.first..node.first + node.count {
                let cluster = paged.clusters[c as usize];
                let sphere = Sphere {
                    center: cluster.lod_center.into(),
                    radius: cluster.lod_radius,
                };
                if error_of(sphere, cluster.lod_error) <= threshold || children_missing[c as usize]
                {
                    selected.push(c);
                }
            }
        }
        selected
    }

    /// Picks a random set of pages that includes every dependency of each page it holds.
    fn random_closed_pages(paged: &PagedScene, next: &mut impl FnMut() -> u64) -> Vec<bool> {
        let dependencies = paged.page_dependencies();
        let keep = next() % 100;
        let mut installed = vec![false; paged.pages.len()];
        // Dependencies have lower ids, so a parent has been decided before its children.
        for page in 0..paged.pages.len() {
            installed[page] = page < paged.pinned_pages as usize
                || (next() % 100 < keep
                    && dependencies[page].iter().all(|&d| installed[d as usize]));
        }
        installed
    }

    fn perspective(viewpoint: Vec3) -> impl Fn(Sphere, f32) -> f32 {
        move |sphere: Sphere, error: f32| {
            let distance = (sphere.center.distance(viewpoint) - sphere.radius).max(0.01);
            1000.0 * error / distance
        }
    }

    /// The heart of streaming: whichever valid set of pages is loaded, from wherever it's seen,
    /// there are no holes, cracks or overlaps.
    #[test]
    fn partly_loaded_scenes_are_watertight_from_any_viewpoint() {
        let source = scene(&[6]);
        let loaded = paginate(&source);
        let paged = &loaded.scene;
        assert!(
            paged.pages.len() > 4,
            "this test needs a mesh of many pages"
        );

        let mut state = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        let viewpoints = [
            Vec3::new(0.0, 0.0, 1.5),
            Vec3::new(3.0, 0.5, 0.0),
            Vec3::new(-1.2, 1.2, 1.2),
            Vec3::new(0.0, -8.0, 0.0),
            Vec3::new(40.0, 40.0, 40.0),
        ];
        let mut partial_cuts = 0;
        for round in 0..200 {
            let installed = random_closed_pages(paged, &mut next);
            let project = perspective(viewpoints[round % viewpoints.len()]);
            let cut = select_resident_cut(paged, &installed, &project, 1.0);
            assert!(!cut.is_empty());
            assert_eq!(open_edges(&source.meshes[0], &cut), 0, "round {round}");
            if installed.iter().any(|&page| !page) {
                partial_cuts += 1;
            }
        }
        assert!(partial_cuts > 100, "too few sets were actually partial");
    }

    #[test]
    fn pinned_pages_alone_cover_the_whole_mesh_and_all_pages_change_nothing() {
        let source = scene(&[6]);
        let paged = &paginate(&source).scene;
        let near = perspective(Vec3::new(0.0, 0.0, 1.5));

        let mut pinned = vec![false; paged.pages.len()];
        pinned[..paged.pinned_pages as usize].fill(true);
        let coarse = select_resident_cut(paged, &pinned, &near, 1.0);
        assert_eq!(open_edges(&source.meshes[0], &coarse), 0);

        let all = vec![true; paged.pages.len()];
        let fine = select_resident_cut(paged, &all, &near, 1.0);
        let triangles = |cut: &[u32]| -> u32 {
            cut.iter()
                .map(|&c| paged.clusters[c as usize].triangle_count())
                .sum()
        };
        assert!(triangles(&coarse) < triangles(&fine));

        // With everything loaded, the flag never overrides the plain LOD rule.
        for &c in &fine {
            let cluster = paged.clusters[c as usize];
            let sphere = Sphere {
                center: cluster.lod_center.into(),
                radius: cluster.lod_radius,
            };
            assert!(near(sphere, cluster.lod_error) <= 1.0);
        }
    }

    /// Loading pages parent-first, never skipping one, keeps every step watertight.
    #[test]
    fn each_step_of_loading_is_watertight() {
        let source = scene(&[6]);
        let paged = &paginate(&source).scene;
        let near = perspective(Vec3::new(0.0, 0.0, 1.5));
        let mut installed = vec![false; paged.pages.len()];
        installed[..paged.pinned_pages as usize].fill(true);
        let mut previous = 0;
        for page in paged.pinned_pages as usize..paged.pages.len() {
            installed[page] = true;
            let cut = select_resident_cut(paged, &installed, &near, 1.0);
            assert_eq!(open_edges(&source.meshes[0], &cut), 0, "page {page}");
            let triangles: u32 = cut
                .iter()
                .map(|&c| paged.clusters[c as usize].triangle_count())
                .sum();
            assert!(triangles >= previous, "loading made the view coarser");
            previous = triangles;
        }
    }
}
