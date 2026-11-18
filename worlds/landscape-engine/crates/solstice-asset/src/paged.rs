//! A scene divided for on-demand loading.
//!
//! A [`PagedScene`] is everything that stays resident: cluster records, the hierarchy, the
//! tables that say where the rest lives. Vertices and triangles live in fixed-capacity *pages*
//! that come and go. A page holds whole sibling sets, the clusters that share one parent
//! group, so swapping a set for its parents is all-or-nothing and can't open a crack.
//!
//! See `docs/streaming.md` for the rules that keep a partly loaded scene watertight.

use crate::gpu::{PackedCluster, PackedNode, PackedVertex, VERTEX_WORDS};

use crate::scene::{
    GameFile, Instance, Light, Material, Sea, TextureEncoding, TextureLayer, Viewpoint,
};
use bytemuck::{Pod, Zeroable};
use glam::Vec3;

/// Capacity of one page, and so the size of one slot in the GPU's page pool. It must hold the
/// largest possible sibling set.
pub const PAGE_BYTES: usize = 128 * 1024;
/// Words a page can hold: a cluster's vertices and triangles are found by the word.
pub const PAGE_WORDS: u32 = (PAGE_BYTES / 4) as u32;

/// Texture mips no larger than this travel with the manifest, so every material can draw from
/// the first frame.
pub const BASE_TEXTURE_EDGE: u32 = 128;

/// [`ClusterSet::round`] of a set with no parents, which must always stay loaded.
pub const ROOT_ROUND: u32 = u32::MAX;

/// Clusters that share one parent group, and the parents simplified from them.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Pod, Zeroable)]
pub struct ClusterSet {
    pub first_cluster: u32,
    pub cluster_count: u32,
    /// Range in the `generated` list: the clusters simplified from this set. Those clusters
    /// draw in this set's place until it is installed.
    pub first_generated: u32,
    pub generated_count: u32,
    /// Which simplification pass made this set's group. Parents always come from a later
    /// pass. [`ROOT_ROUND`] for a set with no parents.
    pub round: u32,
    /// Leaf node of the hierarchy for this set. Meaningful once paged.
    pub leaf_node: u32,
    /// Page holding this set. Meaningful once paged.
    pub page: u32,
    pub _pad: u32,
}

/// Where one page is and what it holds.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Pod, Zeroable)]
pub struct PageInfo {
    /// Offset of the compressed page from the start of the file's data area.
    pub offset: u64,
    /// Size of the compressed page. Zero for a pinned page, which is held in the manifest.
    pub stored_bytes: u32,
    /// Vertex records, the skins of skinned clusters among them.
    pub vertex_count: u32,
    /// Words of triangles, which are packed at three bytes each within a cluster.
    pub triangle_words: u32,
    pub first_set: u32,
    pub set_count: u32,
    pub _pad: u32,
}

impl PageInfo {
    /// Size of the page once unpacked: all its vertices, then all its triangles.
    pub fn raw_bytes(&self) -> usize {
        self.vertex_bytes() + self.triangle_words as usize * 4
    }

    /// Size of the page's vertices, which its triangles follow.
    pub fn vertex_bytes(&self) -> usize {
        self.vertex_count as usize * std::mem::size_of::<PackedVertex>()
    }

    /// Words of the page's vertices: where its triangles begin, counted in words.
    pub fn vertex_words(&self) -> u32 {
        self.vertex_count * VERTEX_WORDS
    }
}

/// How to decode one mesh's vertices. Instances copy this.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MeshInfo {
    /// Index of the hierarchy root in [`PagedScene::nodes`], if the mesh has any geometry.
    pub root_node: Option<u32>,
    pub quant_origin: Vec3,
    pub quant_step: f32,
    pub uv_origin: [f32; 2],
    pub uv_step: [f32; 2],
    /// In a skinned mesh: what to add to one of its clusters' numbers to find its bone list in
    /// [`PagedScene::cluster_bones`].
    pub bone_lists: Option<i32>,
}

/// The resident part of a scene: all of it is uploaded when the scene opens.
#[derive(Clone, Debug, Default)]
pub struct PagedScene {
    /// Offsets are in words: a cluster's vertices from the start of its page, its triangles
    /// from the end of the page's vertices. They are rewritten on install.
    pub clusters: Vec<PackedCluster>,
    pub nodes: Vec<PackedNode>,
    /// Sorted by page.
    pub sets: Vec<ClusterSet>,
    /// See [`ClusterSet::first_generated`].
    pub generated: Vec<u32>,
    /// Pinned pages first, then coarse to fine, so a page's dependencies have lower ids.
    pub pages: Vec<PageInfo>,
    /// How many of the first `pages` are always loaded.
    pub pinned_pages: u32,
    /// The raw contents of the pinned pages, one after another.
    pub pinned_data: Vec<u8>,

    pub meshes: Vec<MeshInfo>,
    pub instances: Vec<Instance>,
    pub materials: Vec<Material>,
    pub lights: Vec<Light>,
    pub viewpoint: Option<Viewpoint>,
    pub sea: Option<Sea>,
    /// The scene's ponds, if it has any.
    pub still_water: Option<crate::scene::StillWater>,
    /// The scene's gameplay files, if it has any.
    pub gameplay: Vec<GameFile>,
    /// Which entity made which instances, for a scene baked from a project.
    pub sources: Vec<crate::scene::Source>,
    /// Where its collision shapes are, if it has any: only those playing fetch them.
    pub collision: Option<crate::collision::CollisionIndex>,
    /// Longest root-to-leaf path over all meshes, which sets the traversal dispatch count.
    pub hierarchy_depth: u32,
    /// Triangles drawn if every instance used its finest LOD.
    pub source_triangles: u64,
    /// Triangles stored once per mesh, regardless of instancing.
    pub unique_triangles: u64,

    /// Edge length of the largest level of each texture.
    pub texture_size: u32,
    pub texture_encoding: TextureEncoding,
    /// The levels of each layer from [`BASE_TEXTURE_EDGE`] down.
    pub base_textures: Vec<TextureLayer>,
    /// For each layer, for each level above the base: where its colour, normal and material
    /// data begin in the data area, in that order.
    pub upper_mip_offsets: Vec<u64>,

    /// Where the data area begins in the file. Set when a file is read.
    pub data_offset: u64,

    /// Each skinned cluster's bones, by [`MeshInfo::bone_lists`].
    pub cluster_bones: Vec<crate::gpu::ClusterBones>,
    /// See [`crate::SceneAsset::skeletons`].
    pub skeletons: Vec<crate::skeleton::Skeleton>,
    /// See [`crate::SceneAsset::poses`].
    pub poses: Vec<u32>,
    /// See [`crate::SceneAsset::instance_poses`].
    pub instance_poses: Vec<u32>,
    /// See [`crate::SceneAsset::clips`].
    pub clips: Vec<crate::skeleton::Clip>,
    /// The emissive maps of its materials.
    pub glow: crate::scene::GlowMaps,
    /// Its transparent glowing cards.
    pub cards: crate::scene::Cards,
    /// Set when `clusters` and `pinned_data` are left empty, because they come separately.
    pub external: Option<ExternalTables>,
}

/// The sizes of the tables a scene's manifest leaves out: a browser has 4 GiB of memory to work
/// in, and a scene like Zorah has 5 GiB of these alone. They follow the manifest in a file of
/// their own: the cluster records in the order pages hold them, each page's sets in turn, and
/// then the pinned pages. The host hands over a page's records with the page, so that only the
/// records of pages on the GPU are ever held.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ExternalTables {
    pub clusters: u64,
    pub pinned_bytes: u64,
}

impl PagedScene {
    /// How many cluster records a page's sets hold.
    pub fn page_cluster_count(&self, page: u32) -> u32 {
        let info = &self.pages[page as usize];
        self.sets[info.first_set as usize..(info.first_set + info.set_count) as usize]
            .iter()
            .map(|set| set.cluster_count)
            .sum()
    }

    /// The most cluster records any one page holds.
    pub fn max_page_clusters(&self) -> u32 {
        (0..self.pages.len() as u32)
            .map(|page| self.page_cluster_count(page))
            .max()
            .unwrap_or(0)
    }

    /// How many cluster records the scene has, wherever they are.
    pub fn cluster_count(&self) -> usize {
        self.external
            .map_or(self.clusters.len(), |tables| tables.clusters as usize)
    }

    /// How many mip levels each texture has above the base levels in the manifest.
    pub fn upper_mip_count(&self) -> usize {
        upper_mip_count(self.texture_size)
    }

    /// The contents of a pinned page.
    pub fn pinned_page(&self, page: u32) -> &[u8] {
        let start: usize = self.pages[..page as usize]
            .iter()
            .map(PageInfo::raw_bytes)
            .sum();
        &self.pinned_data[start..start + self.pages[page as usize].raw_bytes()]
    }

    /// For each page, the other pages that must be installed before it. These hold the
    /// parents of its clusters, which have to be swappable for those clusters. Sorted.
    pub fn page_dependencies(&self) -> Vec<Vec<u32>> {
        let mut set_page = vec![0u32; self.cluster_count()];
        for set in &self.sets {
            let range =
                set.first_cluster as usize..(set.first_cluster + set.cluster_count) as usize;
            set_page[range].fill(set.page);
        }
        let mut dependencies = vec![Vec::new(); self.pages.len()];
        for set in &self.sets {
            let range =
                set.first_generated as usize..(set.first_generated + set.generated_count) as usize;
            let list: &mut Vec<u32> = &mut dependencies[set.page as usize];
            list.extend(self.generated[range].iter().map(|&c| set_page[c as usize]));
        }
        for (page, list) in dependencies.iter_mut().enumerate() {
            list.sort_unstable();
            list.dedup();
            list.retain(|&other| other != page as u32);
        }
        dependencies
    }
}

/// How many mip levels of a texture of this edge lie above [`BASE_TEXTURE_EDGE`].
pub fn upper_mip_count(texture_size: u32) -> usize {
    let mut count = 0;
    while texture_size >> count > BASE_TEXTURE_EDGE {
        count += 1;
    }
    count
}

/// A paged scene with its loadable data in memory, as baked or as read from a whole file.
#[derive(Clone, Debug, Default)]
pub struct LoadedScene {
    pub scene: PagedScene,
    /// The raw contents of each page that is not pinned, indexed by page minus `pinned_pages`.
    pub pages: Vec<Vec<u8>>,
    /// For each layer, the levels above the base, largest first.
    pub upper_textures: Vec<TextureLayer>,
    /// The scene's collision, whole, if it has any.
    pub collision: Option<crate::collision::Collision>,
}

impl LoadedScene {
    /// The raw contents of a page.
    pub fn page(&self, page: u32) -> &[u8] {
        if page < self.scene.pinned_pages {
            self.scene.pinned_page(page)
        } else {
            &self.pages[(page - self.scene.pinned_pages) as usize]
        }
    }
}
