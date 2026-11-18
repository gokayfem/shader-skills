//! Memory layouts read directly by the shaders.
//!
//! Every struct here has a twin in `solstice-render/shaders/common/scene.wgsl`.
//! The two must be changed together; the size assertions at the bottom of this
//! file catch accidental drift on the Rust side.

use bytemuck::{Pod, Zeroable};

/// Maximum triangles per cluster. The visibility buffer reserves 7 bits for the
/// triangle index, so this cannot grow without changing the ID encoding.
pub const MAX_CLUSTER_TRIANGLES: usize = 128;

/// Maximum vertices per cluster, limited by the 8-bit indices in a packed triangle.
pub const MAX_CLUSTER_VERTICES: usize = 255;

/// Error (world units) given to clusters with no coarser parent, so that they are
/// always "too coarse to be replaced" and terminate the LOD cut. Finite because
/// infinities are implementation-defined in WGSL.
pub const ROOT_PARENT_ERROR: f32 = 1.0e30;

/// Set in [`PackedCluster::counts`] when the sibling set simplified into this cluster isn't
/// installed, so this cluster must draw however coarse it is.
pub const CLUSTER_CHILDREN_MISSING: u32 = 1 << 16;

/// The bits of [`PackedCluster::counts`] holding the cluster's UV density, see
/// [`encode_uv_density`]. Tells culling how sharp a texture this cluster needs on screen.
pub const CLUSTER_UV_DENSITY_SHIFT: u32 = 24;

/// Quantises texture repeats per object-space unit to a byte, at eighths of an octave. Zero means
/// the cluster has no usable UVs.
pub fn encode_uv_density(density: f32) -> u32 {
    if !density.is_finite() || density <= 0.0 {
        return 0;
    }
    ((density.log2() + 16.0) * 8.0).round().clamp(1.0, 255.0) as u32
}

/// Inverse of [`encode_uv_density`], for non-zero values. `shaders/vg/cull.wgsl` has a twin.
pub fn decode_uv_density(code: u32) -> f32 {
    (code as f32 / 8.0 - 16.0).exp2()
}

/// Set in [`PackedNode::is_leaf`] on a leaf. The low bits hold the page of its sibling set.
pub const NODE_LEAF: u32 = 1 << 31;
/// Set in [`PackedNode::is_leaf`] while that page is installed.
pub const NODE_INSTALLED: u32 = 1 << 30;
/// The bits of [`PackedNode::is_leaf`] holding a page.
pub const NODE_PAGE_MASK: u32 = NODE_INSTALLED - 1;

/// A vertex compressed to 12 bytes: three words of a page.
///
/// Positions sit on a single mesh-wide 16-bit grid. Using one grid rather than
/// one per cluster is what keeps neighbouring clusters crack-free: a shared
/// vertex always decodes to the identical position.
///
/// Layout, as little-endian `u16`s: `x y z n u v`. The normal is octahedral, a signed byte for
/// each coordinate, see [`encode_normal`]: a page pool holds a third more vertices than it did
/// with sixteen bits for each and a word of padding.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Pod, Zeroable)]
pub struct PackedVertex {
    pub data: [u16; 6],
}

/// Words of a page one vertex takes.
pub const VERTEX_WORDS: u32 = (std::mem::size_of::<PackedVertex>() / 4) as u32;

fn octahedral(n: [f32; 3]) -> [f32; 2] {
    let sum = n[0].abs() + n[1].abs() + n[2].abs();
    let p = [n[0] / sum, n[1] / sum];
    if n[2] < 0.0 {
        [
            (1.0 - p[1].abs()) * if p[0] < 0.0 { -1.0 } else { 1.0 },
            (1.0 - p[0].abs()) * if p[1] < 0.0 { -1.0 } else { 1.0 },
        ]
    } else {
        p
    }
}

/// The unit vector a packed normal stands for. `shaders/common/scene.wgsl` has a twin.
pub fn decode_normal(packed: u16) -> [f32; 3] {
    let x = f32::from(packed as u8 as i8) / 127.0;
    let y = f32::from((packed >> 8) as u8 as i8) / 127.0;
    let z = 1.0 - x.abs() - y.abs();
    let fold = (-z).max(0.0);
    let v = [
        x + if x >= 0.0 { -fold } else { fold },
        y + if y >= 0.0 { -fold } else { fold },
        z,
    ];
    let length = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    [v[0] / length, v[1] / length, v[2] / length]
}

/// Packs a unit vector into sixteen bits, as octahedral coordinates of a signed byte each.
///
/// Rounding each coordinate to the nearest step is not always the nearest direction, so the four
/// steps around the exact point are all tried and the one that decodes closest is kept.
pub fn encode_normal(normal: [f32; 3]) -> u16 {
    let exact = octahedral(normal);
    let floor = [(exact[0] * 127.0).floor(), (exact[1] * 127.0).floor()];
    let mut best = (f32::NEG_INFINITY, 0u16);
    for corner in 0..4 {
        let x = (floor[0] + (corner & 1) as f32).clamp(-127.0, 127.0) as i8;
        let y = (floor[1] + (corner >> 1) as f32).clamp(-127.0, 127.0) as i8;
        let packed = u16::from(x as u8) | u16::from(y as u8) << 8;
        let decoded = decode_normal(packed);
        let alike = decoded[0] * normal[0] + decoded[1] * normal[1] + decoded[2] * normal[2];
        if alike > best.0 {
            best = (alike, packed);
        }
    }
    best.1
}

/// How many bones a cluster's list can name. A cluster using more is drawn with the bounds of its
/// whole mesh, see [`BONES_OVERFLOW`].
pub const CLUSTER_BONES: usize = 16;

/// The bones of a skinned cluster, in ascending order, for working out where it is in a pose.
/// Unused slots repeat the last bone, so a loop stops at the first entry not above the one before.
pub type ClusterBones = [u8; CLUSTER_BONES];

/// The start of a bone list whose cluster uses more bones than a list can name: the only list
/// that ever descends at its start, which marks it. It is drawn with the bounds of the whole mesh.
pub const BONES_OVERFLOW: [u8; 2] = [255, 0];

/// Makes a cluster's bone list from the bones its vertices use, in any order.
pub fn cluster_bones(mut bones: Vec<u8>) -> ClusterBones {
    bones.sort_unstable();
    bones.dedup();
    let mut list = [0; CLUSTER_BONES];
    if bones.len() > CLUSTER_BONES {
        list[..BONES_OVERFLOW.len()].copy_from_slice(&BONES_OVERFLOW);
        return list;
    }
    let last = bones.last().copied().unwrap_or(0);
    list.fill(last);
    list[..bones.len()].copy_from_slice(&bones);
    list
}

/// The four bones that move a vertex, and how much each does: 8 bytes beside its [`PackedVertex`].
///
/// Bones are numbered as the skeleton numbers them. Weights sum to exactly 255, and the used bones
/// come first in ascending order. Two clusters sharing a vertex therefore hold identical bytes,
/// and sum the same terms in the same order, so the skinned position is the same to the last bit
/// on both sides of the seam.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Pod, Zeroable)]
pub struct PackedSkin {
    pub joints: [u8; 4],
    pub weights: [u8; 4],
}

impl PackedSkin {
    /// Quantises influences as a file gives them, in any order, with weights that need not sum
    /// to one. Weights are rounded so that they sum to 255 exactly, by giving the leftover to
    /// those that rounding hurt most. With no weight at all, it follows the first bone wholly.
    pub fn new(joints: [u8; 4], weights: [f32; 4]) -> Self {
        let mut merged: Vec<(u8, f32)> = Vec::with_capacity(4);
        for (&joint, &weight) in joints.iter().zip(&weights) {
            let weight = if weight.is_finite() {
                weight.max(0.0)
            } else {
                0.0
            };
            if weight > 0.0 {
                match merged.iter_mut().find(|(j, _)| *j == joint) {
                    Some(entry) => entry.1 += weight,
                    None => merged.push((joint, weight)),
                }
            }
        }
        if merged.is_empty() {
            merged.push((joints[0], 1.0));
        }
        let sum: f32 = merged.iter().map(|m| m.1).sum();
        let mut scaled: Vec<(u8, u32, f32)> = merged
            .iter()
            .map(|&(joint, weight)| {
                let exact = weight / sum * 255.0;
                (joint, exact.floor() as u32, exact - exact.floor())
            })
            .collect();
        let short = 255 - scaled.iter().map(|m| m.1).sum::<u32>();
        // Largest remainder first; the bone's number settles ties, so the result is repeatable.
        scaled.sort_by(|a, b| b.2.total_cmp(&a.2).then(a.0.cmp(&b.0)));
        for entry in scaled.iter_mut().take(short as usize) {
            entry.1 += 1;
        }
        scaled.retain(|entry| entry.1 > 0);
        scaled.sort_unstable_by_key(|entry| entry.0);

        let mut out = Self::default();
        for (slot, entry) in scaled.iter().enumerate() {
            out.joints[slot] = entry.0;
            out.weights[slot] = entry.1 as u8;
        }
        out
    }

    /// The bones with any weight.
    pub fn bones(self) -> impl Iterator<Item = u8> {
        self.joints
            .into_iter()
            .zip(self.weights)
            .filter(|(_, weight)| *weight > 0)
            .map(|(joint, _)| joint)
    }

    /// How much of a vertex's weight must move to turn one skin into the other, from 0 to 1.
    pub fn difference(self, other: Self) -> f32 {
        let weight_of = |skin: Self, bone: u8| {
            skin.joints
                .into_iter()
                .zip(skin.weights)
                .find(|(joint, weight)| *joint == bone && *weight > 0)
                .map_or(0, |(_, weight)| i32::from(weight))
        };
        let moved: i32 = self
            .bones()
            .map(|bone| (weight_of(self, bone) - weight_of(other, bone)).abs())
            .sum::<i32>()
            + other
                .bones()
                .filter(|&bone| weight_of(self, bone) == 0)
                .map(|bone| weight_of(other, bone))
                .sum::<i32>();
        moved as f32 / 510.0
    }
}

/// Words of a page one skin takes.
pub const SKIN_WORDS: u32 = (std::mem::size_of::<PackedSkin>() / 4) as u32;

/// How many [`PackedVertex`]-sized records hold the skins of a skinned cluster's vertices. They
/// follow the cluster's vertices, two words a skin with no gaps between them, so only skinned
/// meshes pay for them, and a page needs no separate part for them.
pub const fn skin_records(vertex_count: u32) -> u32 {
    (vertex_count * SKIN_WORDS).div_ceil(VERTEX_WORDS)
}

/// The records that hold some vertices' skins, to follow those vertices. See [`skin_records`].
pub fn pack_skins(skins: &[PackedSkin]) -> Vec<PackedVertex> {
    let mut bytes: Vec<u8> = bytemuck::cast_slice(skins).to_vec();
    bytes.resize(
        skin_records(skins.len() as u32) as usize * std::mem::size_of::<PackedVertex>(),
        0,
    );
    bytes
        .chunks_exact(std::mem::size_of::<PackedVertex>())
        .map(bytemuck::pod_read_unaligned)
        .collect()
}

/// One of the skins [`pack_skins`] packed into `records`.
pub fn skin_at(records: &[PackedVertex], index: usize) -> PackedSkin {
    let bytes: &[u8] = bytemuck::cast_slice(records);
    let size = std::mem::size_of::<PackedSkin>();
    bytemuck::pod_read_unaligned(&bytes[index * size..(index + 1) * size])
}

/// One leaf of the LOD DAG: up to [`MAX_CLUSTER_TRIANGLES`] triangles.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct PackedCluster {
    /// Object-space bounding sphere of this cluster's triangles, used for culling.
    pub center: [f32; 3],
    pub radius: f32,
    /// Object-space sphere over which `lod_error` is measured. Shared by all
    /// clusters that were generated from the same group.
    pub lod_center: [f32; 3],
    pub lod_radius: f32,
    /// Simplification error of this cluster, in object-space units.
    pub lod_error: f32,
    /// First vertex in the shared vertex pool.
    pub vertex_offset: u32,
    /// First triangle in the shared triangle pool.
    pub triangle_offset: u32,
    /// `vertex_count | triangle_count << 8`, plus [`CLUSTER_CHILDREN_MISSING`] and the UV
    /// density above [`CLUSTER_UV_DENSITY_SHIFT`].
    pub counts: u32,
}

/// A node of the per-mesh hierarchy used to skip whole subtrees of clusters.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct PackedNode {
    /// Object-space sphere bounding all clusters below this node.
    pub center: [f32; 3],
    pub radius: f32,
    /// Sphere bounding the *parent* LOD spheres below this node.
    pub lod_center: [f32; 3],
    pub lod_radius: f32,
    /// Largest parent error below this node. When even this projects below the
    /// error threshold, every cluster below is finer than necessary and the
    /// whole subtree can be skipped.
    pub max_parent_error: f32,
    /// First child node, or first cluster if this is a leaf.
    pub first: u32,
    /// Number of children or clusters.
    pub count: u32,
    /// Non-zero if `first` indexes clusters instead of nodes. In a paged scene, a leaf
    /// holds [`NODE_LEAF`], [`NODE_INSTALLED`] and its page.
    pub is_leaf: u32,
}

impl PackedCluster {
    pub fn vertex_count(&self) -> u32 {
        self.counts & 0xff
    }

    pub fn triangle_count(&self) -> u32 {
        (self.counts >> 8) & 0xff
    }

    /// How many records of the page this cluster's vertices take: its vertices, then, in a
    /// skinned mesh, their skins.
    pub fn vertex_records(&self, skinned: bool) -> u32 {
        self.vertex_count()
            + if skinned {
                skin_records(self.vertex_count())
            } else {
                0
            }
    }
}

/// Packs three cluster-local vertex indices into one `u32`.
pub fn pack_triangle(a: u8, b: u8, c: u8) -> u32 {
    u32::from(a) | u32::from(b) << 8 | u32::from(c) << 16
}

/// Inverse of [`pack_triangle`].
pub fn unpack_triangle(packed: u32) -> [u8; 3] {
    [packed as u8, (packed >> 8) as u8, (packed >> 16) as u8]
}

/// Words of a page that a cluster's triangles take: three bytes each, end to end, see
/// [`pack_triangle_words`].
pub const fn triangle_words(triangle_count: u32) -> u32 {
    (triangle_count * 3).div_ceil(4)
}

/// Lays a cluster's triangles, each as [`pack_triangle`] made it, end to end at three bytes
/// each, and appends them to `out` as whole words. A mesh being built keeps a word for each
/// triangle; a page holds them like this, a quarter smaller.
pub fn pack_triangle_words(triangles: &[u32], out: &mut Vec<u32>) {
    let mut bytes = Vec::with_capacity(triangles.len() * 3 + 3);
    for triangle in triangles {
        bytes.extend_from_slice(&triangle.to_le_bytes()[..3]);
    }
    bytes.resize(triangle_words(triangles.len() as u32) as usize * 4, 0);
    out.extend(
        bytes
            .chunks_exact(4)
            .map(|word| u32::from_le_bytes([word[0], word[1], word[2], word[3]])),
    );
}

/// One triangle of those [`pack_triangle_words`] packed, as [`pack_triangle`] made it. The
/// shaders' `load_triangle` is its twin.
pub fn triangle_at(words: &[u32], index: u32) -> u32 {
    let bit = index * 24;
    let word = (bit / 32) as usize;
    let shift = bit % 32;
    let mut packed = words[word] >> shift;
    if shift > 8 {
        packed |= words[word + 1] << (32 - shift);
    }
    packed & 0x00ff_ffff
}

const _: () = {
    assert!(std::mem::size_of::<PackedVertex>() == 12);
    assert!(std::mem::size_of::<PackedCluster>() == 48);
    assert!(std::mem::size_of::<PackedNode>() == 48);
    assert!(std::mem::size_of::<PackedSkin>() == 8);
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_skin_sums_to_255_and_lists_its_bones_in_order() {
        let skin = PackedSkin::new([9, 2, 5, 7], [0.1, 0.4, 0.3, 0.2]);
        assert_eq!(skin.joints, [2, 5, 7, 9]);
        assert_eq!(skin.weights.iter().map(|&w| u32::from(w)).sum::<u32>(), 255);
        assert_eq!(skin.weights, [102, 77, 51, 25]);
        let thirds = PackedSkin::new([1, 2, 3, 0], [1.0, 1.0, 1.0, 0.0]);
        assert_eq!(thirds.weights, [85, 85, 85, 0]);
        assert_eq!(thirds.joints, [1, 2, 3, 0]);
    }

    #[test]
    fn the_same_influences_in_any_order_pack_the_same() {
        let a = PackedSkin::new([4, 1, 0, 0], [0.75, 0.25, 0.0, 0.0]);
        let b = PackedSkin::new([1, 4, 8, 0], [0.25, 0.75, 0.0, 0.0]);
        assert_eq!(a, b);
    }

    #[test]
    fn repeats_merge_and_nothing_means_the_first_bone() {
        let repeated = PackedSkin::new([3, 3, 6, 0], [0.25, 0.25, 0.5, 0.0]);
        assert_eq!(repeated.joints, [3, 6, 0, 0]);
        assert_eq!(repeated.weights, [128, 127, 0, 0]);
        let empty = PackedSkin::new([7, 2, 0, 0], [0.0, f32::NAN, -1.0, 0.0]);
        assert_eq!(empty.joints[0], 7);
        assert_eq!(empty.weights, [255, 0, 0, 0]);
    }

    #[test]
    fn how_different_two_skins_are() {
        let hip = PackedSkin::new([1, 0, 0, 0], [1.0, 0.0, 0.0, 0.0]);
        let knee = PackedSkin::new([2, 0, 0, 0], [1.0, 0.0, 0.0, 0.0]);
        let between = PackedSkin::new([1, 2, 0, 0], [0.5, 0.5, 0.0, 0.0]);
        assert_eq!(hip.difference(hip), 0.0);
        assert!((hip.difference(knee) - 1.0).abs() < 1.0e-6);
        assert!((hip.difference(between) - 0.5).abs() < 0.01);
        assert_eq!(hip.difference(between), between.difference(hip));
    }

    #[test]
    fn a_bone_list_is_sorted_padded_and_marks_overflow() {
        let list = cluster_bones(vec![7, 3, 3, 9]);
        assert_eq!(&list[..4], &[3, 7, 9, 9]);
        assert!(list[3..].iter().all(|&b| b == 9));
        assert_eq!(cluster_bones(Vec::new()), [0; CLUSTER_BONES]);
        let full = cluster_bones((0..CLUSTER_BONES as u8).rev().collect());
        assert_eq!(full[0], 0);
        assert_eq!(full[CLUSTER_BONES - 1], CLUSTER_BONES as u8 - 1);
        let over = cluster_bones((0..=CLUSTER_BONES as u8).collect());
        assert_eq!(over[..2], BONES_OVERFLOW);
    }

    #[test]
    fn a_skinned_cluster_takes_a_record_per_two_vertices() {
        let cluster = PackedCluster {
            counts: 7,
            ..Default::default()
        };
        assert_eq!(cluster.vertex_records(false), 7);
        assert_eq!(cluster.vertex_records(true), 12);
        assert_eq!(skin_records(8), 6);
        assert_eq!(skin_records(3), 2);
    }

    #[test]
    fn skins_pack_end_to_end_and_read_back() {
        let skins: Vec<PackedSkin> = (0..5u8)
            .map(|i| PackedSkin::new([i, i + 1, 0, 0], [0.25, 0.75, 0.0, 0.0]))
            .collect();
        let records = pack_skins(&skins);
        assert_eq!(records.len(), skin_records(5) as usize);
        for (index, skin) in skins.iter().enumerate() {
            assert_eq!(skin_at(&records, index), *skin);
        }
    }

    #[test]
    fn triangles_pack_at_three_bytes_and_read_back() {
        for count in [0u32, 1, 2, 3, 4, 5, 127, 128] {
            let triangles: Vec<u32> = (0..count)
                .map(|i| pack_triangle(i as u8, (i * 7 + 1) as u8, (255 - i) as u8))
                .collect();
            let mut words = vec![0xdead_beef];
            pack_triangle_words(&triangles, &mut words);
            assert_eq!(words.len() as u32, 1 + triangle_words(count));
            for (index, &triangle) in triangles.iter().enumerate() {
                assert_eq!(triangle_at(&words[1..], index as u32), triangle);
            }
        }
        assert_eq!(triangle_words(128), 96);
    }

    #[test]
    fn a_packed_normal_is_within_a_degree_and_usually_far_closer() {
        let mut worst = 0.0f32;
        let mut sum = 0.0f64;
        let mut count = 0u32;
        // A spiral over the whole sphere, and the axes, where folding is most awkward.
        let mut normals: Vec<[f32; 3]> = (0..20_000)
            .map(|i| {
                let z = 1.0 - (i as f32 + 0.5) / 10_000.0;
                let r = (1.0 - z * z).sqrt();
                let a = i as f32 * 2.399_963;
                [r * a.cos(), r * a.sin(), z]
            })
            .collect();
        normals.extend([
            [1.0, 0.0, 0.0],
            [-1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, -1.0, 0.0],
            [0.0, 0.0, 1.0],
            [0.0, 0.0, -1.0],
        ]);
        for normal in normals {
            let back = decode_normal(encode_normal(normal));
            let alike = (back[0] * normal[0] + back[1] * normal[1] + back[2] * normal[2]).min(1.0);
            let degrees = alike.acos().to_degrees();
            worst = worst.max(degrees);
            sum += f64::from(degrees);
            count += 1;
        }
        let mean = sum / f64::from(count);
        assert!(worst < 1.0, "the worst normal is {worst} degrees out");
        assert!(mean < 0.35, "the mean is {mean} degrees out");
    }
}
