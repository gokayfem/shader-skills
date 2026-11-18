//! Joins two scenes into one: a map, and what every map shares.
//!
//! A game's maps have things in common that are no part of any of them: the truck, what its
//! driver carries. Baked into every map, they are in every map's file. Kept in a file of their
//! own instead, they are read once beside whichever map is being played, and the two are joined
//! here into one scene, which is drawn as any scene is.
//!
//! Joining is renumbering. A scene's tables point into one another by index: an instance at
//! its mesh and its material, a mesh at its nodes, a node at its children or its clusters, a
//! set of clusters at its page. The second scene's tables go after the first's, and everything
//! in them that is an index has added to it how long the first's table was. The first scene's
//! own numbers do not change, so what its gameplay files say of it stays true; [`Joined`] says
//! what was added to the second's, for whoever reads the second's.
//!
//! Pages are the one thing that cannot simply follow. A scene's pinned pages, which are always
//! loaded, are its first; the rest run from coarse to fine, each needing only pages before it.
//! So the joined scene's pages are the first's pinned, the second's pinned, and then the rest
//! of both dealt together, coarse to fine: every page still follows what it needs, and the
//! coarse pages of both, which are wanted first, come first. Sets are kept in the order of
//! their pages, and move with them.
//!
//! A scene that is read from its file as it is drawn asks for ranges of bytes of the file. Of
//! a joined scene there are two files, so the second's are numbered on from the end of the
//! first's: a range at or past `second_at` is that far into the second file.
//!
//! **What the first leaves out.** Maps have more in common than their truck: the same trees,
//! the same grass and stone, the same textures under them. A map's own instances are of
//! these, so they cannot simply be the second scene's. Instead the second scene *lends*: it
//! carries a list ([`LENDS`]) of every mesh and texture it has, each with a key, which is a
//! hash of what it is. A map baked beside it leaves out of its own file whatever has a key on
//! that list, and carries a list of what it left ([`BORROWS`]): which of its meshes is an
//! empty place for which key, and which of its materials wants the texture of which key. Once
//! the two are joined, [`fill`] puts the lender's mesh in each empty place and gives each such
//! material the lender's layer. The map's instances and materials keep their own numbers, and
//! are of the lender's geometry and textures.
use crate::paged::{LoadedScene, PagedScene};
use crate::scene::Material;
use crate::skeleton::NO_POSE;

/// What was added to each of the second scene's numbers when it was joined to the first.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Joined {
    pub instances: u32,
    pub meshes: u32,
    pub materials: u32,
    pub poses: u32,
    /// And to its texture layers.
    pub layers: u32,
}

/// The gameplay file in which a scene says what it lends: a line each, `mesh INDEX KEY` for
/// one of its meshes and `texture LAYER KEY` for one of its texture layers.
pub const LENDS: &str = "shared.lends";
/// The gameplay file in which a scene says what it left out for a lender to fill: a line each,
/// `mesh INDEX KEY` for a mesh of its own that is an empty place, and `texture MATERIAL KEY`
/// for a material of its own that wants the texture.
pub const BORROWS: &str = "shared.borrows";

/// What is lent or borrowed, a line of a [`LENDS`] or [`BORROWS`] file each.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Lent {
    Mesh,
    Texture,
}

impl Lent {
    pub fn word(self) -> &'static str {
        match self {
            Self::Mesh => "mesh",
            Self::Texture => "texture",
        }
    }
}

/// The lines of a [`LENDS`] or [`BORROWS`] file: what, its number, and its key. Lines that
/// say nothing, or anything else, are passed over.
pub fn lent(text: &str) -> impl Iterator<Item = (Lent, u32, &str)> {
    text.lines().filter_map(|line| {
        let mut words = line.split_whitespace();
        let what = match words.next()? {
            "mesh" => Lent::Mesh,
            "texture" => Lent::Texture,
            _ => return None,
        };
        Some((what, words.next()?.parse().ok()?, words.next()?))
    })
}

/// Fills what a scene left out for its lender, once the lender has been joined onto it:
/// `borrows` is the scene's own list, `lends` the lender's, and `joined` what the join added
/// to the lender's numbers.
pub fn fill(
    scene: &mut PagedScene,
    borrows: &str,
    lends: &str,
    joined: Joined,
) -> Result<(), JoinError> {
    let there: std::collections::HashMap<(Lent, &str), u32> = lent(lends)
        .map(|(what, index, key)| ((what, key), index))
        .collect();
    let stale = || {
        JoinError(
            "the first was baked beside another file of what is shared than this one: bake both again, or fetch both"
                .to_owned(),
        )
    };
    for (what, index, key) in lent(borrows) {
        let theirs = *there.get(&(what, key)).ok_or_else(stale)?;
        match what {
            Lent::Mesh => {
                let at = joined.meshes.checked_add(theirs).ok_or_else(stale)?;
                let mesh = *scene.meshes.get(at as usize).ok_or_else(stale)?;
                *scene.meshes.get_mut(index as usize).ok_or_else(stale)? = mesh;
            }
            Lent::Texture => {
                let at = joined.layers.checked_add(theirs).ok_or_else(stale)?;
                if at as usize >= scene.base_textures.len() {
                    return Err(stale());
                }
                scene
                    .materials
                    .get_mut(index as usize)
                    .ok_or_else(stale)?
                    .texture_layer = i32::try_from(at).map_err(|_| stale())?;
            }
        }
    }
    Ok(())
}

/// Why two scenes could not be joined.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JoinError(pub String);

impl std::fmt::Display for JoinError {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(out, "the scenes cannot be joined: {}", self.0)
    }
}

impl std::error::Error for JoinError {}

fn refuse<T>(why: &str) -> Result<T, JoinError> {
    Err(JoinError(why.to_owned()))
}

/// Joins `second` onto `first`, table by table. `second_at` is where the second's file begins
/// in the numbering of both files' bytes: the length of the first's file. Returns what was added
/// to the second's numbers.
pub fn join(
    first: &mut PagedScene,
    second: PagedScene,
    second_at: u64,
) -> Result<Joined, JoinError> {
    join_pages(first, second, second_at).map(|(joined, _)| joined)
}

/// As [`join`], and says as well where each page that is not pinned came from, in the order
/// they now are: whether from the second scene, and which of its own it was.
fn join_pages(
    first: &mut PagedScene,
    mut second: PagedScene,
    second_at: u64,
) -> Result<(Joined, Vec<(bool, usize)>), JoinError> {
    if first.external.is_some() || second.external.is_some() {
        return refuse("one of them is split up for a browser");
    }
    let textured = |scene: &PagedScene| !scene.base_textures.is_empty();
    if textured(first) && textured(&second) {
        if first.texture_size != second.texture_size {
            return refuse("their textures are not the same size");
        }
        if first.texture_encoding != second.texture_encoding {
            return refuse("their textures are not stored the same way");
        }
    }
    let glows = |scene: &PagedScene| !scene.glow.layers.is_empty();
    if glows(first) && glows(&second) && first.glow.edge != second.glow.edge {
        return refuse("their glow maps are not the same size");
    }
    if !second.cards.cards.is_empty() {
        return refuse("the second has glowing cards, which are not joined yet");
    }
    // What is shared is what moves, with colliders of its own: a table that says nothing
    // of it collides is as good as none.
    let collides = second.collision.as_ref().is_some_and(|collision| {
        collision.heightfield.is_some()
            || collision
                .instance_shapes
                .iter()
                .any(|shape| *shape != crate::collision::NO_SHAPE)
    });
    if collides {
        return refuse("the second has collision shapes, which are not joined yet");
    }

    let added = |count: usize| count as u32;
    let (clusters, nodes, generated) = (
        added(first.clusters.len()),
        added(first.nodes.len()),
        added(first.generated.len()),
    );
    let joined = Joined {
        instances: added(first.instances.len()),
        meshes: added(first.meshes.len()),
        materials: added(first.materials.len()),
        poses: added(first.poses.len()),
        layers: added(first.base_textures.len()),
    };
    let (layers, glow_layers, skeletons, bones) = (
        first.base_textures.len() as i32,
        added(first.glow.layers.len()),
        added(first.skeletons.len()),
        first.cluster_bones.len() as i32,
    );
    // Where the second's bytes are, measured as the first's are, from the first's data area.
    let moved = (second_at + second.data_offset) as i128 - first.data_offset as i128;
    let rebase = |offset: u64| (offset as i128 + moved) as u64;

    // ---- Clusters, and the trees over them.
    first.clusters.append(&mut second.clusters);
    first
        .nodes
        .extend(second.nodes.iter().map(|node| crate::PackedNode {
            first: node.first + if node.is_leaf != 0 { clusters } else { nodes },
            ..*node
        }));
    first
        .generated
        .extend(second.generated.iter().map(|cluster| cluster + clusters));
    first
        .meshes
        .extend(second.meshes.iter().map(|mesh| crate::paged::MeshInfo {
            root_node: mesh.root_node.map(|node| node + nodes),
            bone_lists: mesh.bone_lists.map(|list| list + bones - clusters as i32),
            ..*mesh
        }));
    first.cluster_bones.append(&mut second.cluster_bones);

    // ---- Pages, and the sets in them: the pinned of both, then the rest of both, coarse
    // to fine. A page holds sets of one round of coarsening, and the rounds count down as
    // the detail gets finer: the two scenes' pages are dealt together by their rounds, each
    // scene's in the order they were in, so that a page still follows every page it needs,
    // and what is read first of either is what is seen from furthest off.
    let (pinned_a, pinned_b) = (first.pinned_pages as usize, second.pinned_pages as usize);
    let round = |scene: &PagedScene, page: usize| {
        let info = &scene.pages[page];
        scene
            .sets
            .get(info.first_set as usize)
            .map_or(0, |set| set.round)
    };
    let mut order: Vec<(bool, usize)> = (0..pinned_a).map(|page| (false, page)).collect();
    order.extend((0..pinned_b).map(|page| (true, page)));
    let (mut next_a, mut next_b) = (pinned_a, pinned_b);
    while next_a < first.pages.len() || next_b < second.pages.len() {
        let takes_b = next_a >= first.pages.len()
            || (next_b < second.pages.len() && round(&second, next_b) > round(first, next_a));
        if takes_b {
            order.push((true, next_b));
            next_b += 1;
        } else {
            order.push((false, next_a));
            next_a += 1;
        }
    }
    let mut pages = Vec::with_capacity(order.len());
    let mut sets = Vec::with_capacity(first.sets.len() + second.sets.len());
    for (new, &(of_second, old)) in order.iter().enumerate() {
        let scene: &PagedScene = if of_second { &second } else { first };
        let info = scene.pages[old];
        let first_set = sets.len() as u32;
        for set in &scene.sets[info.first_set as usize..(info.first_set + info.set_count) as usize]
        {
            sets.push(if of_second {
                crate::paged::ClusterSet {
                    first_cluster: set.first_cluster + clusters,
                    first_generated: set.first_generated + generated,
                    leaf_node: set.leaf_node + nodes,
                    page: new as u32,
                    ..*set
                }
            } else {
                crate::paged::ClusterSet {
                    page: new as u32,
                    ..*set
                }
            });
        }
        pages.push(crate::paged::PageInfo {
            first_set,
            // A pinned page is in the manifest, and has no place in a file.
            offset: if of_second && old >= pinned_b {
                rebase(info.offset)
            } else {
                info.offset
            },
            ..info
        });
    }
    // A leaf of a mesh's tree says which page its clusters are in, so that a view that
    // wants them and finds them not yet read can ask for the page: by its new number.
    for set in &sets {
        let node = &mut first.nodes[set.leaf_node as usize];
        node.is_leaf = (node.is_leaf & !crate::gpu::NODE_PAGE_MASK) | set.page;
    }
    first.sets = sets;
    first.pages = pages;
    first.pinned_pages = (pinned_a + pinned_b) as u32;
    first.pinned_data.append(&mut second.pinned_data);
    // Which pages that are not pinned came from where, in their new order.
    let rest: Vec<(bool, usize)> = order[pinned_a + pinned_b..]
        .iter()
        .map(|&(of_second, old)| (of_second, old - if of_second { pinned_b } else { pinned_a }))
        .collect();

    // ---- What is drawn with them.
    first
        .materials
        .extend(second.materials.iter().map(|material| {
            let mut material = *material;
            if material.texture_layer >= 0 {
                material.texture_layer += layers;
            }
            if let Some(layer) = material.glow_layer() {
                let below = material.flags & ((1 << Material::GLOW_SHIFT) - 1);
                material.flags = below | ((layer + glow_layers + 1) << Material::GLOW_SHIFT);
            }
            material
        }));
    if !textured(first) {
        first.texture_size = second.texture_size;
        first.texture_encoding = second.texture_encoding;
    }
    first.base_textures.append(&mut second.base_textures);
    first.upper_mip_offsets.extend(
        second
            .upper_mip_offsets
            .iter()
            .map(|offset| rebase(*offset)),
    );
    if !glows(first) {
        first.glow.edge = second.glow.edge;
    }
    first.glow.layers.append(&mut second.glow.layers);

    // ---- Instances, and the bones of those that have them.
    let count_b = second.instances.len();
    if !first.instance_poses.is_empty() || !second.instance_poses.is_empty() {
        first
            .instance_poses
            .resize(joined.instances as usize, NO_POSE);
        second.instance_poses.resize(count_b, NO_POSE);
        first
            .instance_poses
            .extend(second.instance_poses.iter().map(|pose| match *pose {
                NO_POSE => NO_POSE,
                pose => pose + joined.poses,
            }));
    }
    first
        .instances
        .extend(second.instances.iter().map(|instance| crate::Instance {
            mesh: instance.mesh + joined.meshes,
            material: instance.material + joined.materials,
            ..*instance
        }));
    first
        .poses
        .extend(second.poses.iter().map(|skeleton| skeleton + skeletons));
    first.skeletons.append(&mut second.skeletons);
    first.clips.extend(second.clips.drain(..).map(|mut clip| {
        clip.skeleton += skeletons;
        clip
    }));
    first
        .sources
        .extend(second.sources.drain(..).map(|mut source| {
            source.instances =
                source.instances.start + joined.instances..source.instances.end + joined.instances;
            source
        }));
    // Nothing of the second's collides: it is what moves, and has colliders of its own.
    if let Some(collision) = &mut first.collision {
        collision
            .instance_shapes
            .resize(first.instances.len(), crate::collision::NO_SHAPE);
    }
    first.lights.append(&mut second.lights);
    first.hierarchy_depth = first.hierarchy_depth.max(second.hierarchy_depth);
    first.source_triangles += second.source_triangles;
    first.unique_triangles += second.unique_triangles;
    Ok((joined, rest))
}

impl LoadedScene {
    /// Joins a scene that is all in memory onto this one, which is too. See [`join`].
    pub fn join(&mut self, mut second: LoadedScene) -> Result<Joined, JoinError> {
        let mut theirs: Vec<Option<Vec<u8>>> = std::mem::take(&mut second.pages)
            .into_iter()
            .map(Some)
            .collect();
        let upper = std::mem::take(&mut second.upper_textures);
        let (joined, rest) = join_pages(&mut self.scene, second.scene, 0)?;
        // The pages that are not pinned are kept in the order they now have.
        let mut mine: Vec<Option<Vec<u8>>> = std::mem::take(&mut self.pages)
            .into_iter()
            .map(Some)
            .collect();
        self.pages = rest
            .into_iter()
            .map(|(of_second, old)| {
                let from = if of_second { &mut theirs } else { &mut mine };
                from[old].take().expect("each page is placed once")
            })
            .collect();
        self.upper_textures.extend(upper);
        if let Some(collision) = &mut self.collision {
            collision
                .instance_shapes
                .resize(self.scene.instances.len(), crate::collision::NO_SHAPE);
        }
        Ok(joined)
    }
}
