//! The scene file: a small manifest, then pages and texture levels that can each be read alone.
//!
//! ```text
//! header    magic, version, where the manifest and data area are
//! manifest  deflated: everything in a [`PagedScene`], then optional tagged sections
//! data      each page deflated on its own, then texture levels as they are
//! ```
//!
//! Each optional section is a four-byte tag, its length in bytes, and its contents. Readers skip
//! tags they don't know, and older readers stop at the manifest's fixed part, so a section can be
//! added without changing the version: files stay readable both ways.
//!
//! Block-compressed textures barely shrink further, so they aren't deflated. Pages are deflated
//! separately so a client can fetch any one with an HTTP Range request. All values are
//! little-endian, and loading is one bounds-checked copy per array.

use crate::collision::{encode_shape, Collision, CollisionIndex, HeightfieldEntry, ShapeEntry};
use crate::gpu::{ClusterBones, PackedCluster, PackedNode};
use crate::paged::{
    upper_mip_count, ClusterSet, ExternalTables, LoadedScene, MeshInfo, PageInfo, PagedScene,
    PAGE_BYTES,
};
use crate::scene::{
    Card, CardImage, CardVertex, Cards, GameFile, GlowMaps, Instance, Light, Material, Sea, Source,
    TextureEncoding, TextureLayer, Viewpoint,
};
use crate::skeleton::{Bone, BoneTrack, Clip, Skeleton, MAX_BONES, NO_POSE};
use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Quat, Vec3};
use std::fmt;

const MAGIC: [u8; 8] = *b"SOLSTICE";
const VERSION: u32 = 5;
/// How hard to compress when baking. Unpacking takes the same time at any level.
const DEFLATE_LEVEL: u8 = 9;
/// The optional section holding the scene's [`Sea`]: its level, then its ocean's text.
const SEA_TAG: [u8; 4] = *b"SEA ";
/// Still water: a count of ponds, five numbers for each, then the water's description as text.
const POND_TAG: [u8; 4] = *b"POND";
/// The optional section saying which entity of an editor project made which instances: how many
/// [`Source`]s, then each one's first instance, its end, its placement (16 floats, by column),
/// and its id's length and text. Written only for scenes baked from a project.
const SOURCE_TAG: [u8; 4] = *b"SRCE";
/// The optional section holding the scene's gameplay: how many [`GameFile`]s, then each one's
/// name and text. Written only for scenes that have gameplay.
const GAME_TAG: [u8; 4] = *b"GAME";
/// The optional section listing the scene's collision shapes, whose data is in the data area.
const COLLISION_TAG: [u8; 4] = *b"COLL";
/// The optional section holding a scene's skins and skeletons, written only if it has any.
const SKIN_TAG: [u8; 4] = *b"SKIN";
/// The optional section holding a scene's animation clips, written only if it has any.
const CLIP_TAG: [u8; 4] = *b"CLIP";
/// The optional section holding the emissive maps, written only if there are any.
const GLOW_TAG: [u8; 4] = *b"GLOW";
/// The optional section holding the transparent glowing cards, written only if there are any.
const CARD_TAG: [u8; 4] = *b"CARD";
/// The optional section that says a manifest leaves out its clusters and pinned pages.
const EXTERNAL_TAG: [u8; 4] = *b"EXTP";
/// In a skin section's mesh list: a mesh that isn't skinned.
const NO_BONE_LISTS: i32 = i32::MIN;

/// Why a scene file could not be decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeError {
    BadMagic,
    UnsupportedVersion(u32),
    Truncated,
    BadInstanceReference,
    BadTextures,
    BadPages,
    BadSea,
    BadStillWater,
    BadGameplay,
    BadCollision,
    BadSkin,
    BadGlow,
    BadCards,
    BadSources,
    /// The file made for browsers, read where the whole scene's file is wanted.
    SplitForBrowsers,
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadMagic => write!(f, "not a Solstice scene file"),
            Self::UnsupportedVersion(v) => {
                write!(f, "unsupported file version {v} (expected {VERSION})")
            }
            Self::Truncated => write!(f, "file is truncated"),
            Self::BadInstanceReference => {
                write!(
                    f,
                    "an instance refers to a mesh or material that does not exist"
                )
            }
            Self::BadTextures => write!(f, "the texture data does not match its stated format"),
            Self::BadPages => write!(f, "the page tables are inconsistent"),
            Self::BadSea => write!(f, "the sea's description is not text"),
            Self::BadStillWater => write!(f, "the scene's ponds are malformed"),
            Self::BadGameplay => write!(f, "the scene's gameplay files are malformed"),
            Self::BadCollision => write!(f, "the scene's collision shapes are malformed"),
            Self::BadSkin => write!(f, "the scene's skeletons or skins are malformed"),
            Self::BadGlow => write!(f, "the scene's emissive maps are malformed"),
            Self::BadCards => write!(f, "the scene's cards are malformed"),
            Self::BadSources => write!(f, "the map of the scene's entities is malformed"),
            Self::SplitForBrowsers => {
                write!(
                    f,
                    "this is the file made for browsers: open the .solstice file"
                )
            }
        }
    }
}

impl std::error::Error for DecodeError {}

/// The fixed start of a file. It says how to fetch the rest.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Pod, Zeroable)]
pub struct Header {
    magic: [u8; 8],
    version: u32,
    _pad: u32,
    /// Where the deflated manifest begins.
    pub manifest_offset: u64,
    /// Size of the deflated manifest. The data area follows it.
    pub manifest_bytes: u64,
}

/// Bytes a client must fetch first, to read a [`Header`].
pub const HEADER_BYTES: usize = std::mem::size_of::<Header>();

/// Parses the start of a file.
pub fn decode_header(bytes: &[u8]) -> Result<Header, DecodeError> {
    if bytes.len() < MAGIC.len() || bytes[..MAGIC.len()] != MAGIC {
        return Err(DecodeError::BadMagic);
    }
    let raw = bytes.get(..HEADER_BYTES).ok_or(DecodeError::Truncated)?;
    let header: Header = bytemuck::pod_read_unaligned(raw);
    if header.version != VERSION {
        return Err(DecodeError::UnsupportedVersion(header.version));
    }
    Ok(header)
}

/// On-disk form of an [`Instance`].
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct RawInstance {
    mesh: u32,
    material: u32,
    transform: [f32; 16],
}

/// On-disk form of a [`MeshInfo`].
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct RawMesh {
    /// `u32::MAX` for a mesh with no geometry.
    root_node: u32,
    quant_origin: [f32; 3],
    quant_step: f32,
    uv_origin: [f32; 2],
    uv_step: [f32; 2],
}

/// Serialises a scene.
pub fn encode(loaded: &LoadedScene) -> Vec<u8> {
    let pinned = loaded.scene.pinned_pages as usize;
    let upper_count = upper_mip_count(loaded.scene.texture_size);

    let mut scene = loaded.scene.clone();
    let mut data = Vec::new();
    for (info, raw) in scene.pages[pinned..].iter_mut().zip(&loaded.pages) {
        let packed = deflate_page(raw, info);
        info.offset = data.len() as u64;
        info.stored_bytes = packed.len() as u32;
        data.extend_from_slice(&packed);
    }
    scene.upper_mip_offsets.clear();
    for layer in &loaded.upper_textures {
        for mip in 0..upper_count {
            scene.upper_mip_offsets.push(data.len() as u64);
            data.extend_from_slice(&layer.color_mips[mip]);
            data.extend_from_slice(&layer.normal_mips[mip]);
            data.extend_from_slice(&layer.material_mips[mip]);
        }
    }

    if let Some(collision) = &loaded.collision {
        scene.collision = Some(append_collision(collision, &mut data));
    }

    let manifest = miniz_oxide::deflate::compress_to_vec(&encode_manifest(&scene), DEFLATE_LEVEL);
    let header = Header {
        magic: MAGIC,
        version: VERSION,
        _pad: 0,
        manifest_offset: HEADER_BYTES as u64,
        manifest_bytes: manifest.len() as u64,
    };
    let mut out = Vec::with_capacity(HEADER_BYTES + manifest.len() + data.len());
    out.extend_from_slice(bytemuck::bytes_of(&header));
    out.extend_from_slice(&manifest);
    out.extend_from_slice(&data);
    out
}

/// Cluster records as they are stored and as the GPU reads them.
pub fn cluster_bytes(clusters: &[PackedCluster]) -> &[u8] {
    bytemuck::cast_slice(clusters)
}

/// Packs a page as [`encode`] does, for writers that lay out the data area themselves:
/// filtered, see [`filter_page`], and deflated.
pub fn deflate_page(raw: &[u8], info: &PageInfo) -> Vec<u8> {
    miniz_oxide::deflate::compress_to_vec(&filter_page(raw, info), DEFLATE_LEVEL)
}

/// The `u16` fields of a vertex record.
const VERTEX_FIELDS: usize = std::mem::size_of::<crate::gpu::PackedVertex>() / 2;

/// Rewrites a page's vertices so that they deflate smaller, to the same size and with nothing
/// lost: [`unfilter_page`] undoes it. The pages of a large garden scene stored a fifth smaller
/// for it, see `docs/streaming.md`.
///
/// Each field of a vertex becomes its difference from the same field of the vertex before,
/// folded so that a small step either way is a small number, and each byte of each field goes
/// in a run of its own. Neighbouring vertices are close together, so the high bytes are mostly
/// nought and the low ones small. Triangles are left alone: they pack no better for it.
pub fn filter_page(raw: &[u8], info: &PageInfo) -> Vec<u8> {
    let count = info.vertex_count as usize;
    let vertex_bytes = info.vertex_bytes().min(raw.len());
    let mut out = raw.to_vec();
    if vertex_bytes != count * 2 * VERTEX_FIELDS {
        return out;
    }
    let mut before = [0u16; VERTEX_FIELDS];
    for (index, vertex) in raw[..vertex_bytes]
        .chunks_exact(2 * VERTEX_FIELDS)
        .enumerate()
    {
        for field in 0..VERTEX_FIELDS {
            let value = u16::from_le_bytes([vertex[field * 2], vertex[field * 2 + 1]]);
            let step = value.wrapping_sub(before[field]);
            let folded = (step << 1) ^ (((step as i16) >> 15) as u16);
            out[field * 2 * count + index] = folded as u8;
            out[(field * 2 + 1) * count + index] = (folded >> 8) as u8;
            before[field] = value;
        }
    }
    out
}

/// Undoes [`filter_page`]: the page as the GPU reads it, from the page as a file stores it once
/// inflated.
pub fn unfilter_page(stored: &[u8], info: &PageInfo) -> Vec<u8> {
    let count = info.vertex_count as usize;
    let vertex_bytes = info.vertex_bytes().min(stored.len());
    let mut out = stored.to_vec();
    if vertex_bytes != count * 2 * VERTEX_FIELDS {
        return out;
    }
    let mut before = [0u16; VERTEX_FIELDS];
    for (index, vertex) in out[..vertex_bytes]
        .chunks_exact_mut(2 * VERTEX_FIELDS)
        .enumerate()
    {
        for field in 0..VERTEX_FIELDS {
            let folded = u16::from(stored[field * 2 * count + index])
                | u16::from(stored[(field * 2 + 1) * count + index]) << 8;
            let step = (folded >> 1) ^ (folded & 1).wrapping_neg();
            let value = before[field].wrapping_add(step);
            vertex[field * 2..field * 2 + 2].copy_from_slice(&value.to_le_bytes());
            before[field] = value;
        }
    }
    out
}

/// The start of a file: its header and manifest, for a writer that has placed every page and
/// texture level itself, with offsets into the data area that follows. A scene too large to hold
/// in memory is written this way.
pub fn encode_header_and_manifest(scene: &PagedScene) -> Vec<u8> {
    let manifest = miniz_oxide::deflate::compress_to_vec(&encode_manifest(scene), DEFLATE_LEVEL);
    let header = Header {
        magic: MAGIC,
        version: VERSION,
        _pad: 0,
        manifest_offset: HEADER_BYTES as u64,
        manifest_bytes: manifest.len() as u64,
    };
    let mut out = Vec::with_capacity(HEADER_BYTES + manifest.len());
    out.extend_from_slice(bytemuck::bytes_of(&header));
    out.extend_from_slice(&manifest);
    out
}

fn encode_manifest(scene: &PagedScene) -> Vec<u8> {
    let mut w = Writer::default();
    w.slice(&scene.clusters);
    w.slice(&scene.nodes);
    w.slice(&scene.sets);
    w.slice(&scene.generated);
    w.slice(&scene.pages);
    w.pod(&scene.pinned_pages);
    w.slice(&scene.pinned_data);

    let meshes: Vec<RawMesh> = scene
        .meshes
        .iter()
        .map(|mesh| RawMesh {
            root_node: mesh.root_node.unwrap_or(u32::MAX),
            quant_origin: mesh.quant_origin.to_array(),
            quant_step: mesh.quant_step,
            uv_origin: mesh.uv_origin,
            uv_step: mesh.uv_step,
        })
        .collect();
    w.slice(&meshes);
    let instances: Vec<RawInstance> = scene
        .instances
        .iter()
        .map(|i| RawInstance {
            mesh: i.mesh,
            material: i.material,
            transform: i.transform.to_cols_array(),
        })
        .collect();
    w.slice(&instances);
    w.slice(&scene.materials);
    w.slice(&scene.lights);
    w.pod(&u32::from(scene.viewpoint.is_some()));
    w.pod(&scene.viewpoint.unwrap_or_default());
    w.pod(&scene.hierarchy_depth);
    w.pod(&scene.source_triangles);
    w.pod(&scene.unique_triangles);

    w.pod(&scene.texture_size);
    w.pod(&match scene.texture_encoding {
        TextureEncoding::Raw => 0u32,
        TextureEncoding::Block => 1u32,
    });
    w.pod(&(scene.base_textures.len() as u32));
    for layer in &scene.base_textures {
        w.pod(&(layer.color_mips.len() as u32));
        for mip in layer
            .color_mips
            .iter()
            .chain(&layer.normal_mips)
            .chain(&layer.material_mips)
        {
            w.slice(mip);
        }
    }
    w.slice(&scene.upper_mip_offsets);

    if let Some(sea) = &scene.sea {
        let mut contents = sea.level.to_le_bytes().to_vec();
        contents.extend_from_slice(sea.ocean.as_bytes());
        w.section(SEA_TAG, &contents);
    }
    if let Some(still) = &scene.still_water {
        let mut contents = (still.ponds.len() as u32).to_le_bytes().to_vec();
        for pond in &still.ponds {
            for value in pond {
                contents.extend_from_slice(&value.to_le_bytes());
            }
        }
        contents.extend_from_slice(still.water.as_bytes());
        w.section(POND_TAG, &contents);
    }
    if !scene.gameplay.is_empty() {
        w.section(GAME_TAG, &encode_gameplay(&scene.gameplay));
    }
    if !scene.sources.is_empty() {
        w.section(SOURCE_TAG, &encode_sources(&scene.sources));
    }
    if let Some(index) = &scene.collision {
        w.section(COLLISION_TAG, &encode_collision_index(index));
    }
    if !scene.skeletons.is_empty() {
        w.section(SKIN_TAG, &encode_skin(scene));
    }
    if !scene.clips.is_empty() {
        w.section(CLIP_TAG, &encode_clips(&scene.clips));
    }
    if !scene.glow.layers.is_empty() {
        w.section(GLOW_TAG, &encode_glow(&scene.glow));
    }
    if !scene.cards.cards.is_empty() {
        w.section(CARD_TAG, &encode_cards(&scene.cards));
    }
    if let Some(tables) = scene.external {
        let mut contents = tables.clusters.to_le_bytes().to_vec();
        contents.extend_from_slice(&tables.pinned_bytes.to_le_bytes());
        w.section(EXTERNAL_TAG, &contents);
    }
    w.finish()
}

/// Bytes in one level of a BC7 texture: 16 per block, and a small level still fills one.
fn bc7_level_bytes(width: u32, height: u32, level: usize) -> usize {
    let side = |edge: u32| (edge >> level).max(1).div_ceil(4) as usize;
    side(width) * side(height) * 16
}

/// Whether these are a whole mip chain of a BC7 texture of this size.
fn is_whole_chain(mips: &[Vec<u8>], width: u32, height: u32) -> bool {
    width.is_multiple_of(4)
        && height.is_multiple_of(4)
        && width > 0
        && height > 0
        && mips.len() == width.max(height).ilog2() as usize + 1
        && mips
            .iter()
            .enumerate()
            .all(|(level, mip)| mip.len() == bc7_level_bytes(width, height, level))
}

fn encode_glow(glow: &GlowMaps) -> Vec<u8> {
    let mut w = Writer::default();
    w.pod(&glow.edge);
    w.pod(&(glow.layers.len() as u32));
    for layer in &glow.layers {
        w.pod(&(layer.len() as u32));
        for mip in layer {
            w.slice(mip);
        }
    }
    w.finish()
}

fn decode_glow(bytes: &[u8]) -> Result<GlowMaps, DecodeError> {
    let bad = |_| DecodeError::BadGlow;
    let mut r = Reader { bytes };
    let edge: u32 = r.pod().map_err(bad)?;
    let count: u32 = r.pod().map_err(bad)?;
    if count as usize > bytes.len() {
        return Err(DecodeError::BadGlow);
    }
    let mut layers = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let mip_count: u32 = r.pod().map_err(bad)?;
        if mip_count > 32 {
            return Err(DecodeError::BadGlow);
        }
        let mut mips = Vec::with_capacity(mip_count as usize);
        for _ in 0..mip_count {
            mips.push(r.slice::<u8>().map_err(bad)?);
        }
        if !edge.is_power_of_two() || !is_whole_chain(&mips, edge, edge) {
            return Err(DecodeError::BadGlow);
        }
        layers.push(mips);
    }
    if !r.bytes.is_empty() {
        return Err(DecodeError::BadGlow);
    }
    Ok(GlowMaps { edge, layers })
}

fn encode_cards(cards: &Cards) -> Vec<u8> {
    let mut w = Writer::default();
    w.slice(&cards.vertices);
    w.slice(&cards.cards);
    w.pod(&(cards.images.len() as u32));
    for image in &cards.images {
        w.pod(&image.width);
        w.pod(&image.height);
        w.pod(&(image.color_mips.len() as u32));
        for mip in image.color_mips.iter().chain(&image.glow_mips) {
            w.slice(mip);
        }
    }
    w.finish()
}

fn decode_cards(bytes: &[u8]) -> Result<Cards, DecodeError> {
    let bad = |_| DecodeError::BadCards;
    let mut r = Reader { bytes };
    let vertices: Vec<CardVertex> = r.slice().map_err(bad)?;
    let cards: Vec<Card> = r.slice().map_err(bad)?;
    let count: u32 = r.pod().map_err(bad)?;
    if count as usize > bytes.len() {
        return Err(DecodeError::BadCards);
    }
    let mut images = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let width: u32 = r.pod().map_err(bad)?;
        let height: u32 = r.pod().map_err(bad)?;
        let mip_count: u32 = r.pod().map_err(bad)?;
        if mip_count > 32 {
            return Err(DecodeError::BadCards);
        }
        let read = |r: &mut Reader<'_>| -> Result<Vec<Vec<u8>>, DecodeError> {
            (0..mip_count)
                .map(|_| r.slice::<u8>().map_err(bad))
                .collect()
        };
        let color_mips = read(&mut r)?;
        let glow_mips = read(&mut r)?;
        if !is_whole_chain(&color_mips, width, height) || !is_whole_chain(&glow_mips, width, height)
        {
            return Err(DecodeError::BadCards);
        }
        images.push(CardImage {
            width,
            height,
            color_mips,
            glow_mips,
        });
    }
    let cards_ok = cards.iter().all(|card| {
        card.vertex_count.is_multiple_of(3)
            && u64::from(card.first_vertex) + u64::from(card.vertex_count) <= vertices.len() as u64
            && (card.image as usize) < images.len()
    });
    if !r.bytes.is_empty() || !cards_ok {
        return Err(DecodeError::BadCards);
    }
    Ok(Cards {
        vertices,
        cards,
        images,
    })
}

fn encode_clips(clips: &[Clip]) -> Vec<u8> {
    let mut w = Writer::default();
    w.pod(&(clips.len() as u32));
    for clip in clips {
        w.slice(clip.name.as_bytes());
        w.pod(&clip.skeleton);
        w.pod(&clip.frames);
        w.pod(&u32::from(clip.loops));
        w.slice(&clip.tracks);
        w.slice(&clip.rotations);
        w.slice(&clip.translations);
    }
    w.finish()
}

fn decode_clips(bytes: &[u8]) -> Result<Vec<Clip>, DecodeError> {
    let bad = |_| DecodeError::BadSkin;
    let mut r = Reader { bytes };
    let count: u32 = r.pod().map_err(bad)?;
    if count as usize > bytes.len() {
        return Err(DecodeError::BadSkin);
    }
    let mut clips = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let name: Vec<u8> = r.slice().map_err(bad)?;
        let skeleton: u32 = r.pod().map_err(bad)?;
        let frames: u32 = r.pod().map_err(bad)?;
        let loops: u32 = r.pod().map_err(bad)?;
        let tracks: Vec<BoneTrack> = r.slice().map_err(bad)?;
        let rotations: Vec<[u16; 3]> = r.slice().map_err(bad)?;
        let translations: Vec<[u16; 3]> = r.slice().map_err(bad)?;
        clips.push(Clip {
            name: String::from_utf8(name).map_err(|_| DecodeError::BadSkin)?,
            skeleton,
            frames,
            loops: loops != 0,
            tracks,
            rotations,
            translations,
        });
    }
    if !r.bytes.is_empty() {
        return Err(DecodeError::BadSkin);
    }
    Ok(clips)
}

/// Checks that every sample a clip's tracks name is there, and every clip's skeleton exists and
/// has as many bones as it has tracks.
fn clips_are_consistent(scene: &PagedScene) -> bool {
    scene.clips.iter().all(|clip| {
        let Some(skeleton) = scene.skeletons.get(clip.skeleton as usize) else {
            return false;
        };
        clip.frames > 0
            && clip.tracks.len() == skeleton.bones.len()
            && clip.tracks.iter().all(|track| {
                (track.rotations_frames == 1 || track.rotations_frames == clip.frames)
                    && (track.translations_frames == 1 || track.translations_frames == clip.frames)
                    && u64::from(track.rotations_first) + u64::from(track.rotations_frames)
                        <= clip.rotations.len() as u64
                    && u64::from(track.translations_first) + u64::from(track.translations_frames)
                        <= clip.translations.len() as u64
            })
    })
}

/// A scene's skins: the bone lists, each mesh's way into them, the skeletons, and who follows what.
fn encode_skin(scene: &PagedScene) -> Vec<u8> {
    let mut w = Writer::default();
    w.slice(&scene.cluster_bones);
    let lists: Vec<i32> = scene
        .meshes
        .iter()
        .map(|mesh| mesh.bone_lists.unwrap_or(NO_BONE_LISTS))
        .collect();
    w.slice(&lists);
    w.pod(&(scene.skeletons.len() as u32));
    for skeleton in &scene.skeletons {
        w.pod(&(skeleton.bones.len() as u32));
        for bone in &skeleton.bones {
            w.slice(bone.name.as_bytes());
            w.pod(&bone.parent.unwrap_or(u32::MAX));
            w.pod(&bone.translation.to_array());
            w.pod(&bone.rotation.to_array());
            w.pod(&bone.inverse_bind.to_cols_array());
            w.pod(&bone.center.to_array());
            w.pod(&bone.radius);
        }
    }
    w.slice(&scene.poses);
    w.slice(&scene.instance_poses);
    w.finish()
}

/// What [`encode_skin`] writes: the bone lists, each mesh's way into them, the skeletons, the
/// poses, and which pose each instance follows.
struct SkinSection {
    cluster_bones: Vec<ClusterBones>,
    lists: Vec<i32>,
    skeletons: Vec<Skeleton>,
    poses: Vec<u32>,
    instance_poses: Vec<u32>,
}

fn decode_skin(bytes: &[u8]) -> Result<SkinSection, DecodeError> {
    let mut r = Reader { bytes };
    let cluster_bones: Vec<ClusterBones> = r.slice().map_err(|_| DecodeError::BadSkin)?;
    let lists: Vec<i32> = r.slice().map_err(|_| DecodeError::BadSkin)?;
    let count: u32 = r.pod().map_err(|_| DecodeError::BadSkin)?;
    let mut skeletons = Vec::new();
    for _ in 0..count {
        let bone_count: u32 = r.pod().map_err(|_| DecodeError::BadSkin)?;
        if bone_count as usize > MAX_BONES {
            return Err(DecodeError::BadSkin);
        }
        let mut bones = Vec::new();
        for _ in 0..bone_count {
            let name: Vec<u8> = r.slice().map_err(|_| DecodeError::BadSkin)?;
            let parent: u32 = r.pod().map_err(|_| DecodeError::BadSkin)?;
            let translation: [f32; 3] = r.pod().map_err(|_| DecodeError::BadSkin)?;
            let rotation: [f32; 4] = r.pod().map_err(|_| DecodeError::BadSkin)?;
            let inverse_bind: [f32; 16] = r.pod().map_err(|_| DecodeError::BadSkin)?;
            let center: [f32; 3] = r.pod().map_err(|_| DecodeError::BadSkin)?;
            let radius: f32 = r.pod().map_err(|_| DecodeError::BadSkin)?;
            bones.push(Bone {
                name: String::from_utf8(name).map_err(|_| DecodeError::BadSkin)?,
                parent: (parent != u32::MAX).then_some(parent),
                translation: Vec3::from_array(translation),
                rotation: Quat::from_array(rotation),
                inverse_bind: Mat4::from_cols_array(&inverse_bind),
                center: Vec3::from_array(center),
                radius,
            });
        }
        let skeleton = Skeleton { bones };
        if !skeleton.is_well_formed() {
            return Err(DecodeError::BadSkin);
        }
        skeletons.push(skeleton);
    }
    let poses: Vec<u32> = r.slice().map_err(|_| DecodeError::BadSkin)?;
    let instance_poses: Vec<u32> = r.slice().map_err(|_| DecodeError::BadSkin)?;
    if !r.bytes.is_empty() {
        return Err(DecodeError::BadSkin);
    }
    Ok(SkinSection {
        cluster_bones,
        lists,
        skeletons,
        poses,
        instance_poses,
    })
}

/// Checks everything a skin section's numbers point at, which the renderer would index blindly.
fn skin_is_consistent(scene: &PagedScene) -> bool {
    let poses_ok = scene
        .poses
        .iter()
        .all(|&skeleton| (skeleton as usize) < scene.skeletons.len());
    let instances_ok = scene.instance_poses.len() <= scene.instances.len()
        && scene
            .instance_poses
            .iter()
            .all(|&pose| pose == NO_POSE || (pose as usize) < scene.poses.len());
    // Shaders load a bone list by adding these to a cluster's number. Out of range, a load reads
    // zeros rather than anything else, so this only refuses numbers that cannot be right.
    let clusters = scene.clusters.len() as i64;
    let lists = scene.cluster_bones.len() as i64;
    let lists_ok = scene.meshes.iter().all(|mesh| {
        mesh.bone_lists
            .is_none_or(|delta| i64::from(delta) + clusters >= 0 && i64::from(delta) <= lists)
    });
    poses_ok && instances_ok && lists_ok
}

/// Writes a scene's collision shapes into the data area, each deflated on its own so that it
/// can be fetched alone, and lists where they went.
fn append_collision(collision: &Collision, data: &mut Vec<u8>) -> CollisionIndex {
    let mut store = |raw: &[u8]| {
        let packed = miniz_oxide::deflate::compress_to_vec(raw, DEFLATE_LEVEL);
        let offset = data.len() as u64;
        data.extend_from_slice(&packed);
        (offset, packed.len() as u32)
    };
    let shapes = collision
        .shapes
        .iter()
        .map(|shape| {
            let (offset, stored_bytes) = store(&encode_shape(shape));
            ShapeEntry {
                offset,
                stored_bytes,
                hull: u32::from(shape.hull),
                vertices: shape.vertices.len() as u32,
                triangles: shape.triangles.len() as u32,
            }
        })
        .collect();
    let heightfield = collision.heightfield.as_ref().map(|ground| {
        let (offset, stored_bytes) = store(bytemuck::cast_slice(&ground.heights));
        HeightfieldEntry {
            offset,
            stored_bytes,
            columns: ground.columns,
            rows: ground.rows,
            origin: ground.origin,
            spacing: ground.spacing,
            _pad: 0,
        }
    });
    CollisionIndex {
        shapes,
        instance_shapes: collision.instance_shapes.clone(),
        heightfield,
    }
}

fn encode_collision_index(index: &CollisionIndex) -> Vec<u8> {
    let mut w = Writer::default();
    w.slice(&index.shapes);
    w.slice(&index.instance_shapes);
    let heightfield: Vec<HeightfieldEntry> = index.heightfield.iter().copied().collect();
    w.slice(&heightfield);
    w.finish()
}

fn decode_collision_index(bytes: &[u8]) -> Result<CollisionIndex, DecodeError> {
    let mut r = Reader { bytes };
    let bad = |_| DecodeError::BadCollision;
    let shapes: Vec<ShapeEntry> = r.slice().map_err(bad)?;
    let instance_shapes: Vec<u32> = r.slice().map_err(bad)?;
    let heightfield: Vec<HeightfieldEntry> = r.slice().map_err(bad)?;
    if heightfield.len() > 1 || !r.bytes.is_empty() {
        return Err(DecodeError::BadCollision);
    }
    Ok(CollisionIndex {
        shapes,
        instance_shapes,
        heightfield: heightfield.first().copied(),
    })
}

/// A gameplay section's contents: a count, then each file's name and text, each a length and
/// its UTF-8 bytes.
fn encode_gameplay(files: &[GameFile]) -> Vec<u8> {
    let mut bytes = (files.len() as u32).to_le_bytes().to_vec();
    for file in files {
        for text in [&file.name, &file.text] {
            bytes.extend_from_slice(&(text.len() as u32).to_le_bytes());
            bytes.extend_from_slice(text.as_bytes());
        }
    }
    bytes
}

fn encode_sources(sources: &[Source]) -> Vec<u8> {
    let mut bytes = (sources.len() as u32).to_le_bytes().to_vec();
    for source in sources {
        for number in [source.instances.start, source.instances.end] {
            bytes.extend_from_slice(&number.to_le_bytes());
        }
        for value in source.placement.to_cols_array() {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        bytes.extend_from_slice(&(source.id.len() as u32).to_le_bytes());
        bytes.extend_from_slice(source.id.as_bytes());
    }
    bytes
}

fn decode_sources(mut bytes: &[u8], instances: u32) -> Result<Vec<Source>, DecodeError> {
    fn number(bytes: &mut &[u8]) -> Result<u32, DecodeError> {
        let (word, rest) = bytes
            .split_first_chunk::<4>()
            .ok_or(DecodeError::BadSources)?;
        *bytes = rest;
        Ok(u32::from_le_bytes(*word))
    }
    let count = number(&mut bytes)? as usize;
    // Each takes at least 76 bytes, so a count beyond that is a lie, not a large map.
    if count > bytes.len() / 76 {
        return Err(DecodeError::BadSources);
    }
    let mut sources = Vec::with_capacity(count);
    for _ in 0..count {
        let (start, end) = (number(&mut bytes)?, number(&mut bytes)?);
        let mut columns = [0.0; 16];
        for value in &mut columns {
            *value = f32::from_bits(number(&mut bytes)?);
        }
        let placement = Mat4::from_cols_array(&columns);
        if !placement.is_finite() {
            return Err(DecodeError::BadSources);
        }
        let len = number(&mut bytes)?;
        let (id, rest) = bytes
            .split_at_checked(len as usize)
            .ok_or(DecodeError::BadSources)?;
        bytes = rest;
        if start > end || end > instances {
            return Err(DecodeError::BadSources);
        }
        sources.push(Source {
            id: String::from_utf8(id.to_vec()).map_err(|_| DecodeError::BadSources)?,
            instances: start..end,
            placement,
        });
    }
    Ok(sources)
}

fn decode_gameplay(mut bytes: &[u8]) -> Result<Vec<GameFile>, DecodeError> {
    fn take<'a>(bytes: &mut &'a [u8], len: usize) -> Result<&'a [u8], DecodeError> {
        let (head, rest) = bytes
            .split_at_checked(len)
            .ok_or(DecodeError::BadGameplay)?;
        *bytes = rest;
        Ok(head)
    }
    fn number(bytes: &mut &[u8]) -> Result<usize, DecodeError> {
        let word = take(bytes, 4)?;
        Ok(u32::from_le_bytes(word.try_into().expect("four bytes")) as usize)
    }
    fn text(bytes: &mut &[u8]) -> Result<String, DecodeError> {
        let len = number(bytes)?;
        String::from_utf8(take(bytes, len)?.to_vec()).map_err(|_| DecodeError::BadGameplay)
    }
    let count = number(&mut bytes)?;
    // Each file takes at least eight bytes, so a count beyond that is a lie, not a large file.
    if count > bytes.len() / 8 {
        return Err(DecodeError::BadGameplay);
    }
    let mut files = Vec::with_capacity(count);
    for _ in 0..count {
        let name = text(&mut bytes)?;
        let text = text(&mut bytes)?;
        files.push(GameFile { name, text });
    }
    if !bytes.is_empty() {
        return Err(DecodeError::BadGameplay);
    }
    Ok(files)
}

/// Parses an unpacked manifest. `data_offset` is where the file's data area begins.
pub fn decode_manifest(bytes: &[u8], data_offset: u64) -> Result<PagedScene, DecodeError> {
    let mut r = Reader { bytes };
    let clusters: Vec<PackedCluster> = r.slice()?;
    let nodes: Vec<PackedNode> = r.slice()?;
    let sets: Vec<ClusterSet> = r.slice()?;
    let generated: Vec<u32> = r.slice()?;
    let pages: Vec<PageInfo> = r.slice()?;
    let pinned_pages: u32 = r.pod()?;
    let pinned_data: Vec<u8> = r.slice()?;

    let raw_meshes: Vec<RawMesh> = r.slice()?;
    let meshes: Vec<MeshInfo> = raw_meshes
        .iter()
        .map(|mesh| MeshInfo {
            root_node: (mesh.root_node != u32::MAX).then_some(mesh.root_node),
            quant_origin: Vec3::from_array(mesh.quant_origin),
            quant_step: mesh.quant_step,
            uv_origin: mesh.uv_origin,
            uv_step: mesh.uv_step,
            bone_lists: None,
        })
        .collect();
    let raw_instances: Vec<RawInstance> = r.slice()?;
    let instances: Vec<Instance> = raw_instances
        .iter()
        .map(|raw| Instance {
            mesh: raw.mesh,
            material: raw.material,
            transform: Mat4::from_cols_array(&raw.transform),
        })
        .collect();
    let materials: Vec<Material> = r.slice()?;
    let lights: Vec<Light> = r.slice()?;
    let references_valid = instances
        .iter()
        .all(|i| (i.mesh as usize) < meshes.len() && (i.material as usize) < materials.len());
    if !references_valid {
        return Err(DecodeError::BadInstanceReference);
    }

    let has_viewpoint: u32 = r.pod()?;
    let viewpoint: Viewpoint = r.pod()?;
    let hierarchy_depth: u32 = r.pod()?;
    let source_triangles: u64 = r.pod()?;
    let unique_triangles: u64 = r.pod()?;

    let texture_size: u32 = r.pod()?;
    let texture_encoding = match r.pod::<u32>()? {
        0 => TextureEncoding::Raw,
        1 => TextureEncoding::Block,
        _ => return Err(DecodeError::BadTextures),
    };
    let layer_count: u32 = r.pod()?;
    let mut base_textures = Vec::new();
    for _ in 0..layer_count {
        let mip_count: u32 = r.pod()?;
        let mut layer = TextureLayer::default();
        for _ in 0..mip_count {
            layer.color_mips.push(r.slice()?);
        }
        for _ in 0..mip_count {
            layer.normal_mips.push(r.slice()?);
        }
        for _ in 0..mip_count {
            layer.material_mips.push(r.slice()?);
        }
        if !base_is_well_formed(&layer, texture_size, texture_encoding) {
            return Err(DecodeError::BadTextures);
        }
        base_textures.push(layer);
    }
    let upper_mip_offsets: Vec<u64> = r.slice()?;

    let mut sea = None;
    let mut still_water = None;
    let mut gameplay = Vec::new();
    let mut sources = Vec::new();
    let mut collision = None;
    let mut skin = None;
    let mut clips = Vec::new();
    let mut glow = GlowMaps::default();
    let mut cards = Cards::default();
    let mut external = None;
    while !r.bytes.is_empty() {
        let tag: [u8; 4] = r.pod()?;
        let contents: Vec<u8> = r.slice()?;
        if tag == SEA_TAG {
            let (level, text) = contents
                .split_first_chunk::<4>()
                .ok_or(DecodeError::Truncated)?;
            sea = Some(Sea {
                level: f32::from_le_bytes(*level),
                ocean: String::from_utf8(text.to_vec()).map_err(|_| DecodeError::BadSea)?,
            });
        } else if tag == POND_TAG {
            let (count, rest) = contents
                .split_first_chunk::<4>()
                .ok_or(DecodeError::BadStillWater)?;
            let count = u32::from_le_bytes(*count) as usize;
            let numbers = count.checked_mul(20).ok_or(DecodeError::BadStillWater)?;
            if rest.len() < numbers {
                return Err(DecodeError::BadStillWater);
            }
            let (ponds, text) = rest.split_at(numbers);
            let ponds: Vec<[f32; 5]> = ponds
                .chunks_exact(20)
                .map(|pond| {
                    std::array::from_fn(|k| {
                        f32::from_le_bytes(pond[k * 4..k * 4 + 4].try_into().expect("four bytes"))
                    })
                })
                .collect();
            if ponds.iter().flatten().any(|v| !v.is_finite()) {
                return Err(DecodeError::BadStillWater);
            }
            still_water = Some(crate::scene::StillWater {
                ponds,
                water: String::from_utf8(text.to_vec()).map_err(|_| DecodeError::BadStillWater)?,
            });
        } else if tag == GAME_TAG {
            gameplay = decode_gameplay(&contents)?;
        } else if tag == SOURCE_TAG {
            sources = decode_sources(&contents, instances.len() as u32)?;
        } else if tag == COLLISION_TAG {
            collision = Some(decode_collision_index(&contents)?);
        } else if tag == SKIN_TAG {
            skin = Some(decode_skin(&contents)?);
        } else if tag == CLIP_TAG {
            clips = decode_clips(&contents)?;
        } else if tag == GLOW_TAG {
            glow = decode_glow(&contents)?;
        } else if tag == CARD_TAG {
            cards = decode_cards(&contents)?;
        } else if tag == EXTERNAL_TAG {
            let (clusters, pinned_bytes) = <[u8; 16]>::try_from(contents.as_slice())
                .map(|b| {
                    let word = |at: usize| {
                        u64::from_le_bytes(b[at..at + 8].try_into().expect("eight bytes"))
                    };
                    (word(0), word(8))
                })
                .map_err(|_| DecodeError::BadPages)?;
            external = Some(ExternalTables {
                clusters,
                pinned_bytes,
            });
        }
    }
    let glow_ok = materials.iter().all(|material| {
        material
            .glow_layer()
            .is_none_or(|l| (l as usize) < glow.layers.len())
    });
    if !glow_ok {
        return Err(DecodeError::BadGlow);
    }
    let mut meshes = meshes;
    let (cluster_bones, skeletons, poses, instance_poses) = match skin {
        None => Default::default(),
        Some(skin) => {
            if skin.lists.len() != meshes.len() {
                return Err(DecodeError::BadSkin);
            }
            for (mesh, list) in meshes.iter_mut().zip(skin.lists) {
                mesh.bone_lists = (list != NO_BONE_LISTS).then_some(list);
            }
            (
                skin.cluster_bones,
                skin.skeletons,
                skin.poses,
                skin.instance_poses,
            )
        }
    };

    if let Some(index) = &collision {
        index.validate(instances.len(), data_offset)?;
    }

    let scene = PagedScene {
        clusters,
        nodes,
        sets,
        generated,
        pages,
        pinned_pages,
        pinned_data,
        meshes,
        instances,
        materials,
        lights,
        viewpoint: (has_viewpoint != 0).then_some(viewpoint),
        sea,
        still_water,
        gameplay,
        sources,
        collision,
        hierarchy_depth,
        source_triangles,
        unique_triangles,
        texture_size,
        texture_encoding,
        base_textures,
        upper_mip_offsets,
        data_offset,
        cluster_bones,
        skeletons,
        poses,
        instance_poses,
        clips,
        glow,
        cards,
        external,
    };
    if !tables_are_consistent(&scene) {
        return Err(DecodeError::BadPages);
    }
    if !skin_is_consistent(&scene) || !clips_are_consistent(&scene) {
        return Err(DecodeError::BadSkin);
    }
    Ok(scene)
}

/// Unpacks a whole file held in memory, pages and all.
pub fn decode(bytes: &[u8]) -> Result<LoadedScene, DecodeError> {
    let header = decode_header(bytes)?;
    let end = header
        .manifest_offset
        .checked_add(header.manifest_bytes)
        .ok_or(DecodeError::Truncated)?;
    let packed = usize::try_from(header.manifest_offset)
        .ok()
        .zip(usize::try_from(end).ok())
        .and_then(|(start, end)| bytes.get(start..end))
        .ok_or(DecodeError::Truncated)?;
    let manifest = inflate(packed)?;
    let scene = decode_manifest(&manifest, end)?;
    drop(manifest);
    // Its cluster records and pinned pages are not where a whole file's are.
    if scene.external.is_some() {
        return Err(DecodeError::SplitForBrowsers);
    }

    let mut pages = Vec::new();
    for page in 0..scene.pages.len() as u32 {
        if page >= scene.pinned_pages {
            let packed = stored_range(bytes, page_range(&scene, page))?;
            pages.push(unpack_page(&scene, page, packed)?);
        }
    }

    let upper_count = upper_mip_count(scene.texture_size);
    let mut upper_textures = Vec::new();
    for layer in 0..scene.base_textures.len() {
        let mut out = TextureLayer::default();
        for mip in 0..upper_count {
            let range = upper_mip_range(&scene, layer, mip).ok_or(DecodeError::BadTextures)?;
            let stored = stored_range(bytes, range)?;
            let edge = scene.texture_size >> mip;
            let color = scene.texture_encoding.color_mip_bytes(edge);
            let data = scene.texture_encoding.data_mip_bytes(edge);
            out.color_mips.push(stored[..color].to_vec());
            out.normal_mips.push(stored[color..color + data].to_vec());
            out.material_mips.push(stored[color + data..].to_vec());
        }
        upper_textures.push(out);
    }

    let collision = match &scene.collision {
        None => None,
        Some(index) => {
            let mut pieces = Vec::new();
            for ((offset, stored), raw) in index.ranges().into_iter().zip(index.raw_sizes()) {
                let start = scene.data_offset + offset;
                let packed = stored_range(bytes, (start, start + u64::from(stored)))?;
                pieces.push(inflate_limited(packed, raw)?);
            }
            Some(index.decode(&pieces)?)
        }
    };

    Ok(LoadedScene {
        scene,
        pages,
        upper_textures,
        collision,
    })
}

/// The bytes of a page in the file, as absolute offsets.
pub fn page_range(scene: &PagedScene, page: u32) -> (u64, u64) {
    let info = &scene.pages[page as usize];
    let start = scene.data_offset + info.offset;
    (start, start + u64::from(info.stored_bytes))
}

/// The bytes of one upper mip level of one layer in the file, as absolute offsets: its colour,
/// normal and material data, in that order. `None` if the file has no such level.
pub fn upper_mip_range(scene: &PagedScene, layer: usize, mip: usize) -> Option<(u64, u64)> {
    let count = upper_mip_count(scene.texture_size);
    if mip >= count {
        return None;
    }
    let offset = *scene.upper_mip_offsets.get(layer * count + mip)?;
    let edge = scene.texture_size >> mip;
    let encoding = scene.texture_encoding;
    let bytes = encoding.color_mip_bytes(edge) + 2 * encoding.data_mip_bytes(edge);
    let start = scene.data_offset + offset;
    Some((start, start + bytes as u64))
}

fn stored_range(bytes: &[u8], (start, end): (u64, u64)) -> Result<&[u8], DecodeError> {
    usize::try_from(start)
        .ok()
        .zip(usize::try_from(end).ok())
        .and_then(|(start, end)| bytes.get(start..end))
        .ok_or(DecodeError::Truncated)
}

/// Unpacks a deflated block.
pub fn inflate(packed: &[u8]) -> Result<Vec<u8>, DecodeError> {
    miniz_oxide::inflate::decompress_to_vec(packed).map_err(|_| DecodeError::Truncated)
}

/// Unpacks a deflated block that should come to `limit` bytes, refusing to unpack more.
pub fn inflate_limited(packed: &[u8], limit: u64) -> Result<Vec<u8>, DecodeError> {
    miniz_oxide::inflate::decompress_to_vec_with_limit(packed, limit as usize)
        .map_err(|_| DecodeError::BadCollision)
}

/// Unpacks a page and checks that it is the size the tables say.
pub fn unpack_page(scene: &PagedScene, page: u32, packed: &[u8]) -> Result<Vec<u8>, DecodeError> {
    let info = &scene.pages[page as usize];
    let stored = inflate(packed)?;
    if stored.len() != info.raw_bytes() {
        return Err(DecodeError::BadPages);
    }
    Ok(unfilter_page(&stored, info))
}

/// Checks everything the renderer would otherwise index blindly while installing pages. A bad
/// file should fail here rather than panic later.
fn tables_are_consistent(scene: &PagedScene) -> bool {
    let clusters = scene.cluster_count() as u64;
    let generated = scene.generated.len() as u64;
    let sets = scene.sets.len() as u64;
    let pages = scene.pages.len() as u64;
    let sets_ok = scene.sets.iter().all(|set| {
        u64::from(set.first_cluster) + u64::from(set.cluster_count) <= clusters
            && u64::from(set.first_generated) + u64::from(set.generated_count) <= generated
            && u64::from(set.page) < pages
            && (set.leaf_node as usize) < scene.nodes.len()
    });
    let pages_ok = scene.pages.iter().all(|page| {
        page.raw_bytes() <= PAGE_BYTES
            && u64::from(page.first_set) + u64::from(page.set_count) <= sets
    });
    let pinned = scene.pinned_pages as usize;
    let pinned_ok = pinned <= scene.pages.len()
        && scene.pages[..pinned]
            .iter()
            .map(PageInfo::raw_bytes)
            .sum::<usize>() as u64
            == scene
                .external
                .map_or(scene.pinned_data.len() as u64, |tables| tables.pinned_bytes);
    // Left out means left out: a file that says so and has them anyway is not one this wrote.
    let external_ok =
        scene.external.is_none() || (scene.clusters.is_empty() && scene.pinned_data.is_empty());
    external_ok
        && sets_ok
        && pages_ok
        && pinned_ok
        && scene.generated.iter().all(|&c| u64::from(c) < clusters)
        && scene.meshes.iter().all(|mesh| {
            mesh.root_node
                .is_none_or(|root| (root as usize) < scene.nodes.len())
        })
}

/// Checks that every level is the size the GPU will expect, which a bad file could otherwise
/// turn into a validation error much later.
fn base_is_well_formed(layer: &TextureLayer, size: u32, encoding: TextureEncoding) -> bool {
    if !size.is_power_of_two() || (encoding == TextureEncoding::Block && size < 4) {
        return false;
    }
    let upper = upper_mip_count(size);
    let levels = size.ilog2() as usize + 1 - upper;
    let edge = |level: usize| size >> (level + upper);
    layer.color_mips.len() == levels
        && (0..levels).all(|level| {
            layer.color_mips[level].len() == encoding.color_mip_bytes(edge(level))
                && layer.normal_mips[level].len() == encoding.data_mip_bytes(edge(level))
                && layer.material_mips[level].len() == encoding.data_mip_bytes(edge(level))
        })
}

#[derive(Default)]
struct Writer {
    out: Vec<u8>,
}

impl Writer {
    fn bytes(&mut self, data: &[u8]) {
        self.out.extend_from_slice(data);
    }

    fn pod<T: Pod>(&mut self, value: &T) {
        self.bytes(bytemuck::bytes_of(value));
    }

    fn slice<T: Pod>(&mut self, values: &[T]) {
        self.pod(&(values.len() as u64));
        self.bytes(bytemuck::cast_slice(values));
    }

    /// An optional section: its tag, then its contents with their length.
    fn section(&mut self, tag: [u8; 4], contents: &[u8]) {
        self.bytes(&tag);
        self.slice(contents);
    }

    fn finish(self) -> Vec<u8> {
        self.out
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
}

impl<'a> Reader<'a> {
    fn take(&mut self, count: usize) -> Result<&'a [u8], DecodeError> {
        if count > self.bytes.len() {
            return Err(DecodeError::Truncated);
        }
        let (head, tail) = self.bytes.split_at(count);
        self.bytes = tail;
        Ok(head)
    }

    fn pod<T: Pod>(&mut self) -> Result<T, DecodeError> {
        let raw = self.take(std::mem::size_of::<T>())?;
        Ok(bytemuck::pod_read_unaligned(raw))
    }

    fn slice<T: Pod>(&mut self) -> Result<Vec<T>, DecodeError> {
        let len: u64 = self.pod()?;
        let byte_len = usize::try_from(len)
            .ok()
            .and_then(|n| n.checked_mul(std::mem::size_of::<T>()))
            .ok_or(DecodeError::Truncated)?;
        let raw = self.take(byte_len)?;
        Ok(raw
            .chunks_exact(std::mem::size_of::<T>())
            .map(bytemuck::pod_read_unaligned)
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collision::{CollisionShape, Heightfield};
    use crate::gpu::{pack_triangle, PackedVertex, NODE_LEAF};

    /// One pinned page and one that loads on demand, with textures above and below the base.
    fn sample_scene() -> LoadedScene {
        let page_bytes = |vertex: u8| {
            let vertices = [PackedVertex {
                data: [u16::from(vertex); 6],
            }; 3];
            let mut bytes: Vec<u8> = bytemuck::cast_slice(&vertices).to_vec();
            bytes.extend_from_slice(bytemuck::bytes_of(&pack_triangle(0, 1, 2)));
            bytes
        };
        let info = PageInfo {
            vertex_count: 3,
            triangle_words: 1,
            set_count: 1,
            ..Default::default()
        };
        let edge = 256u32;
        let encoding = TextureEncoding::Raw;
        let mip =
            |edge: u32, bytes: usize, fill: u8| vec![fill; edge as usize * edge as usize * bytes];
        let chain = |first: u32, bytes: usize, fill: u8| -> Vec<Vec<u8>> {
            (first..=edge.ilog2())
                .map(|m| mip(edge >> m, bytes, fill))
                .collect()
        };
        let scene = PagedScene {
            clusters: vec![
                PackedCluster {
                    radius: 2.0,
                    counts: 3 | 1 << 8,
                    ..Default::default()
                };
                2
            ],
            nodes: vec![
                PackedNode {
                    is_leaf: NODE_LEAF,
                    ..Default::default()
                },
                PackedNode {
                    is_leaf: NODE_LEAF | 1,
                    first: 1,
                    count: 1,
                    ..Default::default()
                },
            ],
            sets: vec![
                ClusterSet {
                    cluster_count: 1,
                    ..Default::default()
                },
                ClusterSet {
                    first_cluster: 1,
                    cluster_count: 1,
                    generated_count: 1,
                    leaf_node: 1,
                    page: 1,
                    ..Default::default()
                },
            ],
            generated: vec![0],
            pages: vec![
                info,
                PageInfo {
                    first_set: 1,
                    ..info
                },
            ],
            pinned_pages: 1,
            pinned_data: page_bytes(1),
            meshes: vec![MeshInfo {
                root_node: Some(0),
                quant_step: 0.5,
                ..Default::default()
            }],
            instances: vec![Instance {
                mesh: 0,
                material: 0,
                transform: Mat4::from_translation(Vec3::X),
            }],
            materials: vec![Material::default()],
            lights: vec![Light {
                range: 4.0,
                ..Default::default()
            }],
            viewpoint: Some(Viewpoint {
                position: [1.0, 2.0, 3.0],
                yaw: 0.5,
                pitch: -0.25,
            }),
            hierarchy_depth: 3,
            source_triangles: 7,
            unique_triangles: 5,
            texture_size: edge,
            texture_encoding: encoding,
            base_textures: vec![TextureLayer {
                color_mips: chain(1, 4, 1),
                normal_mips: chain(1, 2, 2),
                material_mips: chain(1, 2, 3),
            }],
            ..Default::default()
        };
        LoadedScene {
            scene,
            pages: vec![page_bytes(2)],
            upper_textures: vec![TextureLayer {
                color_mips: vec![mip(edge, 4, 4)],
                normal_mips: vec![mip(edge, 2, 5)],
                material_mips: vec![mip(edge, 2, 6)],
            }],
            collision: None,
        }
    }

    #[test]
    fn round_trip_preserves_everything() {
        let loaded = sample_scene();
        let decoded = decode(&encode(&loaded)).unwrap();
        let (a, b) = (&decoded.scene, &loaded.scene);
        assert_eq!(a.clusters, b.clusters);
        assert_eq!(a.nodes, b.nodes);
        assert_eq!(a.sets, b.sets);
        assert_eq!(a.generated, b.generated);
        assert_eq!(a.pinned_pages, 1);
        assert_eq!(a.pinned_data, b.pinned_data);
        assert_eq!(a.materials, b.materials);
        assert_eq!(a.lights, b.lights);
        assert_eq!(a.viewpoint, b.viewpoint);
        assert_eq!(a.sea, b.sea);
        assert_eq!(a.still_water, b.still_water);
        assert_eq!(a.meshes, b.meshes);
        assert_eq!(a.instances[0].transform, b.instances[0].transform);
        assert_eq!(a.hierarchy_depth, 3);
        assert_eq!(a.source_triangles, 7);
        assert_eq!(
            a.base_textures[0].normal_mips,
            b.base_textures[0].normal_mips
        );
        assert_eq!(decoded.pages, loaded.pages);
        assert_eq!(
            decoded.upper_textures[0].color_mips,
            loaded.upper_textures[0].color_mips
        );
        assert_eq!(
            decoded.upper_textures[0].normal_mips,
            loaded.upper_textures[0].normal_mips
        );
        assert_eq!(
            decoded.upper_textures[0].material_mips,
            loaded.upper_textures[0].material_mips
        );
        assert_eq!(decoded.page(0), loaded.page(0));
        assert_eq!(decoded.page(1), loaded.page(1));
    }

    /// A BC7 chain of this size, filled with `fill`.
    fn chain_of(width: u32, height: u32, fill: u8) -> Vec<Vec<u8>> {
        (0..=width.max(height).ilog2() as usize)
            .map(|level| vec![fill; bc7_level_bytes(width, height, level)])
            .collect()
    }

    fn sample_glow() -> GlowMaps {
        GlowMaps {
            edge: 16,
            layers: vec![chain_of(16, 16, 7), chain_of(16, 16, 9)],
        }
    }

    fn sample_cards() -> Cards {
        Cards {
            vertices: vec![CardVertex::default(); 6],
            cards: vec![Card {
                first_vertex: 3,
                vertex_count: 3,
                image: 0,
                opacity: 0.5,
                emissive: [2.0, 1.0, 0.5],
            }],
            images: vec![CardImage {
                width: 8,
                height: 64,
                color_mips: chain_of(8, 64, 1),
                glow_mips: chain_of(8, 64, 2),
            }],
        }
    }

    #[test]
    fn glow_and_cards_travel_with_their_scene() {
        let mut loaded = sample_scene();
        loaded.scene.materials[0].flags |= 2 << Material::GLOW_SHIFT;
        loaded.scene.glow = sample_glow();
        loaded.scene.cards = sample_cards();
        let decoded = decode(&encode(&loaded)).unwrap().scene;
        assert_eq!(decoded.glow, sample_glow());
        assert_eq!(decoded.cards, sample_cards());
        assert_eq!(decoded.materials[0].glow_layer(), Some(1));
    }

    #[test]
    fn a_file_with_neither_section_still_loads() {
        let loaded = sample_scene();
        let bytes = encode(&loaded);
        // Version 5 is version 4 with pages laid out more tightly: optional sections came and
        // went without moving it.
        assert_eq!(bytes[8..12], 5u32.to_le_bytes(), "the version hasn't moved");
        let manifest = encode_manifest(&loaded.scene);
        assert!(!manifest
            .windows(4)
            .any(|tag| tag == GLOW_TAG || tag == CARD_TAG));
        let decoded = decode(&bytes).unwrap();
        assert!(decoded.scene.glow.layers.is_empty() && decoded.scene.cards.cards.is_empty());
        assert_eq!(decoded.scene.materials, loaded.scene.materials);
        assert_eq!(decoded.pages, loaded.pages);
    }

    /// What is written from here on must not trouble a reader that doesn't know the sections.
    #[test]
    fn sections_a_reader_does_not_know_are_skipped() {
        let mut loaded = sample_scene();
        loaded.scene.cards = sample_cards();
        let manifest = encode_manifest(&loaded.scene);
        let renamed: Vec<u8> = {
            let at = manifest
                .windows(4)
                .position(|tag| tag == CARD_TAG)
                .expect("written");
            let mut bytes = manifest.clone();
            bytes[at..at + 4].copy_from_slice(b"NEW!");
            bytes
        };
        let scene = decode_manifest(&renamed, 0).unwrap();
        assert!(scene.cards.cards.is_empty());
        assert_eq!(scene.materials, loaded.scene.materials);
    }

    #[test]
    fn a_material_cannot_name_a_glow_layer_that_is_not_there() {
        let mut loaded = sample_scene();
        loaded.scene.materials[0].flags |= 3 << Material::GLOW_SHIFT;
        loaded.scene.glow = sample_glow();
        assert_eq!(decode(&encode(&loaded)).err(), Some(DecodeError::BadGlow));
    }

    #[test]
    fn broken_glow_and_cards_are_errors() {
        let section = encode_glow(&sample_glow());
        for cut in 0..section.len() {
            assert_eq!(
                decode_glow(&section[..cut]),
                Err(DecodeError::BadGlow),
                "cut at {cut}"
            );
        }
        let mut wrong = sample_glow();
        wrong.layers[1].pop();
        assert_eq!(decode_glow(&encode_glow(&wrong)), Err(DecodeError::BadGlow));

        let section = encode_cards(&sample_cards());
        for cut in 0..section.len() {
            assert_eq!(
                decode_cards(&section[..cut]),
                Err(DecodeError::BadCards),
                "cut at {cut}"
            );
        }
        let bad = |change: fn(&mut Cards)| {
            let mut cards = sample_cards();
            change(&mut cards);
            assert_eq!(
                decode_cards(&encode_cards(&cards)),
                Err(DecodeError::BadCards)
            );
        };
        bad(|c| c.cards[0].image = 1);
        bad(|c| c.cards[0].first_vertex = 4);
        bad(|c| c.cards[0].vertex_count = 2);
        bad(|c| c.images[0].width = 6);
        bad(|c| c.images[0].glow_mips[0].pop().map(|_| ()).unwrap());
    }

    /// What a browser reads: the small manifest, with the big tables told apart from it.
    fn without_tables(mut scene: PagedScene) -> PagedScene {
        let external = ExternalTables {
            clusters: scene.clusters.len() as u64,
            pinned_bytes: scene.pinned_data.len() as u64,
        };
        scene.clusters.clear();
        scene.pinned_data.clear();
        scene.external = Some(external);
        scene
    }

    #[test]
    fn a_manifest_can_leave_out_its_big_tables() {
        let scene = sample_scene().scene;
        let (clusters, pinned) = (scene.clusters.len(), scene.pinned_data.len());
        let small = without_tables(scene.clone());
        let decoded = decode_manifest(&encode_manifest(&small), 0).unwrap();
        assert!(decoded.clusters.is_empty() && decoded.pinned_data.is_empty());
        assert_eq!(decoded.cluster_count(), clusters);
        assert_eq!(
            decoded.external.unwrap().pinned_bytes,
            pinned as u64,
            "and says how much there is"
        );
        assert_eq!(decoded.page_dependencies(), scene.page_dependencies());
        // Read as a whole scene's file, it is refused and not taken for a damaged one.
        assert_eq!(
            decode(&encode_header_and_manifest(&small)).err(),
            Some(DecodeError::SplitForBrowsers)
        );
    }

    #[test]
    fn a_manifest_that_leaves_out_too_little_or_too_much_is_refused() {
        let small = without_tables(sample_scene().scene);
        let bad = |change: fn(&mut PagedScene)| {
            let mut scene = small.clone();
            change(&mut scene);
            assert_eq!(
                decode_manifest(&encode_manifest(&scene), 0).err(),
                Some(DecodeError::BadPages)
            );
        };
        bad(|s| s.external.as_mut().unwrap().clusters = 0);
        bad(|s| s.external.as_mut().unwrap().pinned_bytes += 1);
        bad(|s| s.pinned_data.push(0));
    }

    #[test]
    fn the_map_of_entities_travels_with_its_scene() {
        let mut loaded = sample_scene();
        let count = loaded.scene.instances.len() as u32;
        let sources = vec![
            Source {
                id: "a91fXq2b".into(),
                instances: 0..1,
                placement: Mat4::from_translation(Vec3::new(1.0, 2.0, 3.0)),
            },
            Source {
                id: "terrain".into(),
                instances: 1..count,
                placement: Mat4::IDENTITY,
            },
        ];
        loaded.scene.sources = sources.clone();
        assert_eq!(decode(&encode(&loaded)).unwrap().scene.sources, sources);
        // A scene not baked from a project has none, and writes no section for it.
        assert!(decode(&encode(&sample_scene()))
            .unwrap()
            .scene
            .sources
            .is_empty());
        // One that names instances the scene doesn't have is refused.
        loaded.scene.sources[1].instances = 1..count + 1;
        assert_eq!(
            decode(&encode(&loaded)).err(),
            Some(DecodeError::BadSources)
        );
    }

    #[test]
    fn a_damaged_map_of_entities_is_refused() {
        let good = encode_sources(&[Source {
            id: "a91fXq2b".into(),
            instances: 0..2,
            placement: Mat4::IDENTITY,
        }]);
        assert!(decode_sources(&good, 2).is_ok());
        // Cut short anywhere.
        for len in 0..good.len() {
            assert_eq!(
                decode_sources(&good[..len], 2),
                Err(DecodeError::BadSources),
                "{len}"
            );
        }
        // A count, or an id's length, far beyond what is there.
        let mut huge = good.clone();
        huge[..4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(decode_sources(&huge, 2), Err(DecodeError::BadSources));
        let mut long = good.clone();
        long[76..80].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(decode_sources(&long, 2), Err(DecodeError::BadSources));
        // An id that isn't text, and a range that runs backwards.
        let mut bad_text = good.clone();
        bad_text[80] = 0xff;
        assert_eq!(decode_sources(&bad_text, 2), Err(DecodeError::BadSources));
        let mut not_a_number = good.clone();
        not_a_number[12..16].copy_from_slice(&f32::NAN.to_le_bytes());
        assert_eq!(
            decode_sources(&not_a_number, 2),
            Err(DecodeError::BadSources)
        );
        assert_eq!(decode_sources(&bad_text, 2), Err(DecodeError::BadSources));
        let mut backwards = good;
        backwards[4..8].copy_from_slice(&2u32.to_le_bytes());
        backwards[8..12].copy_from_slice(&1u32.to_le_bytes());
        assert_eq!(decode_sources(&backwards, 2), Err(DecodeError::BadSources));
    }

    #[test]
    fn a_sea_travels_with_its_scene() {
        let mut loaded = sample_scene();
        let sea = Sea {
            level: -0.6,
            ocean: "# Clear water\nabsorption 0.35 0.06 0.015\n".to_string(),
        };
        loaded.scene.sea = Some(sea.clone());
        assert_eq!(decode(&encode(&loaded)).unwrap().scene.sea, Some(sea));
        // And a scene without one has none.
        assert_eq!(decode(&encode(&sample_scene())).unwrap().scene.sea, None);
    }

    #[test]
    fn ponds_travel_with_their_scene() {
        let mut loaded = sample_scene();
        let still = crate::scene::StillWater {
            ponds: vec![[-2.0, -37.0, 4.0, -23.0, 1.5], [24.0, -6.0, 59.0, 5.0, 1.9]],
            water: "absorption 2.1 1.5 2.0\nturbidity 0.5\n".to_string(),
        };
        loaded.scene.still_water = Some(still.clone());
        let decoded = decode(&encode(&loaded)).unwrap().scene;
        assert_eq!(decoded.still_water, Some(still));
        // Clear water has no description, and a scene without ponds has none.
        let clear = crate::scene::StillWater {
            ponds: vec![[0.0, 0.0, 1.0, 1.0, 0.0]],
            water: String::new(),
        };
        loaded.scene.still_water = Some(clear.clone());
        assert_eq!(
            decode(&encode(&loaded)).unwrap().scene.still_water,
            Some(clear)
        );
        assert_eq!(
            decode(&encode(&sample_scene())).unwrap().scene.still_water,
            None
        );
        // A pond that isn't a number is a damaged file, not water to draw.
        loaded.scene.still_water = Some(crate::scene::StillWater {
            ponds: vec![[0.0, 0.0, f32::NAN, 1.0, 0.0]],
            water: String::new(),
        });
        assert!(matches!(
            decode(&encode(&loaded)),
            Err(DecodeError::BadStillWater)
        ));
    }

    #[test]
    fn gameplay_travels_with_its_scene() {
        let mut loaded = sample_scene();
        let files = vec![
            GameFile {
                name: "cove/gate.actor".to_string(),
                text: "name gate\nscript gate.star\n".to_string(),
            },
            GameFile {
                name: "cove/gate.star".to_string(),
                text: "def on_begin(self):\n    pass  # «unicode» too\n".to_string(),
            },
            GameFile::default(),
        ];
        loaded.scene.gameplay = files.clone();
        assert_eq!(decode(&encode(&loaded)).unwrap().scene.gameplay, files);
    }

    fn sample_collision() -> Collision {
        Collision {
            shapes: vec![
                CollisionShape {
                    hull: false,
                    vertices: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.5]],
                    triangles: vec![[0, 1, 2]],
                },
                CollisionShape {
                    hull: true,
                    vertices: vec![[2.0, 0.0, 0.0], [0.0, 2.0, 0.0], [0.0, 0.0, 2.0], [1.0; 3]],
                    triangles: vec![],
                },
            ],
            instance_shapes: vec![1],
            heightfield: Some(Heightfield {
                columns: 2,
                rows: 3,
                origin: [-5.0, 2.0],
                spacing: [1.5, 2.5],
                heights: vec![0.0, 0.5, 1.0, 1.5, 2.0, 2.5],
            }),
        }
    }

    #[test]
    fn collision_travels_with_its_scene_and_stays_out_of_the_manifest() {
        let mut loaded = sample_scene();
        let plain = encode(&loaded);
        let mut collision = sample_collision();
        collision.instance_shapes = vec![1; loaded.scene.instances.len()];
        loaded.collision = Some(collision.clone());
        let bytes = encode(&loaded);
        let decoded = decode(&bytes).unwrap();
        assert_eq!(decoded.collision, loaded.collision);
        // The shapes are in the data area, each to be fetched alone.
        let index = decoded.scene.collision.as_ref().unwrap();
        assert_eq!(index.ranges().len(), 3);
        let mut pieces = Vec::new();
        for (offset, stored) in index.ranges() {
            let start = (decoded.scene.data_offset + offset) as usize;
            pieces.push(inflate(&bytes[start..start + stored as usize]).unwrap());
        }
        assert_eq!(index.decode(&pieces).unwrap(), collision);
        // The data shares nothing with the rest: pages and textures stay where they were.
        assert_eq!(decode(&plain).unwrap().scene.collision, None);
        assert_eq!(decoded.pages, sample_scene().pages);
    }

    #[test]
    fn broken_collision_is_an_error_not_a_guess() {
        let collision = sample_collision();
        let mut data = Vec::new();
        let index = append_collision(&collision, &mut data);
        let mut pieces: Vec<Vec<u8>> = index
            .ranges()
            .iter()
            .map(|&(o, s)| inflate(&data[o as usize..o as usize + s as usize]).unwrap())
            .collect();
        assert_eq!(index.decode(&pieces).unwrap(), collision);
        let bad = |index: &CollisionIndex, pieces: &[Vec<u8>]| {
            assert_eq!(index.decode(pieces), Err(DecodeError::BadCollision));
        };
        bad(&index, &pieces[..2]);
        let mut wrong = index.clone();
        wrong.shapes[0].vertices = 4;
        bad(&wrong, &pieces);
        let mut wrong = index.clone();
        wrong.shapes[0].vertices = u32::MAX;
        bad(&wrong, &pieces);
        let mut wrong = index.clone();
        wrong.instance_shapes[0] = 2;
        bad(&wrong, &pieces);
        let mut wrong = index.clone();
        wrong.heightfield.as_mut().unwrap().rows = 2;
        bad(&wrong, &pieces);
        let mut wrong = index.clone();
        wrong.heightfield.as_mut().unwrap().spacing[0] = 0.0;
        bad(&wrong, &pieces);
        // A triangle naming a vertex that isn't there.
        let saved = pieces[0].clone();
        let at = pieces[0].len() - 4;
        pieces[0][at..].copy_from_slice(&3u32.to_le_bytes());
        bad(&index, &pieces);
        pieces[0] = saved;
        // A vertex that isn't a number, or is further out than physics can work with.
        let saved = pieces[1].clone();
        pieces[1][0..4].copy_from_slice(&f32::NAN.to_le_bytes());
        bad(&index, &pieces);
        pieces[1][0..4].copy_from_slice(&1.0e7_f32.to_le_bytes());
        bad(&index, &pieces);
        pieces[1] = saved;
        // A height too high.
        let at = pieces[2].len() - 4;
        pieces[2][at..].copy_from_slice(&1.0e7_f32.to_le_bytes());
        bad(&index, &pieces);
        // A section cut short.
        let section = encode_collision_index(&index);
        for cut in 0..section.len() {
            assert_eq!(
                decode_collision_index(&section[..cut]),
                Err(DecodeError::BadCollision),
                "cut at {cut}"
            );
        }
    }

    #[test]
    fn a_collision_index_is_checked_before_anything_is_fetched() {
        let mut data = Vec::new();
        let index = append_collision(&sample_collision(), &mut data);
        assert_eq!(index.validate(1, 100), Ok(()));
        assert_eq!(index.raw_sizes(), [48, 48, 24]);
        let bad = |index: &CollisionIndex| {
            assert_eq!(index.validate(1, 100), Err(DecodeError::BadCollision));
        };
        assert_eq!(
            index.validate(2, 100),
            Err(DecodeError::BadCollision),
            "one too few"
        );
        let mut wrong = index.clone();
        wrong.instance_shapes = vec![9];
        bad(&wrong);
        let mut wrong = index.clone();
        wrong.shapes[0].vertices = u32::MAX;
        bad(&wrong);
        let mut wrong = index.clone();
        wrong.shapes[0].stored_bytes = 1_000_000;
        bad(&wrong);
        let mut wrong = index.clone();
        wrong.shapes[0].offset = u64::MAX;
        bad(&wrong);
        let mut wrong = index.clone();
        wrong.heightfield.as_mut().unwrap().columns = u32::MAX;
        bad(&wrong);
        // Unpacking stops at the size it should be.
        let packed = miniz_oxide::deflate::compress_to_vec(&[7; 1000], 6);
        assert_eq!(inflate_limited(&packed, 1000).unwrap().len(), 1000);
        assert_eq!(
            inflate_limited(&packed, 999),
            Err(DecodeError::BadCollision)
        );
    }

    #[test]
    fn a_collision_index_cannot_ask_for_too_much() {
        use crate::collision::{ShapeEntry, MAX_COLLISION_BYTES, MAX_HULL_POINTS};
        let mesh = |vertices: u32| ShapeEntry {
            vertices,
            ..ShapeEntry::default()
        };
        let index = |shapes: Vec<ShapeEntry>| CollisionIndex {
            shapes,
            ..CollisionIndex::default()
        };
        // As much as is allowed, then one vertex more, spread over two pieces.
        assert_eq!(
            MAX_COLLISION_BYTES,
            1 << 30,
            "what a browser is asked to set aside"
        );
        let most = (MAX_COLLISION_BYTES / 12) as u32;
        assert_eq!(index(vec![mesh(most)]).validate(0, 0), Ok(()));
        assert_eq!(
            index(vec![mesh(most), mesh(1)]).validate(0, 0),
            Err(DecodeError::BadCollision),
            "too much in all, though each piece would fit"
        );
        // Hulls are limited, at what the bake can make. Meshes are not.
        let hull = |vertices: u32| ShapeEntry {
            hull: 1,
            ..mesh(vertices)
        };
        let most = MAX_HULL_POINTS as u32;
        assert_eq!(index(vec![hull(most)]).validate(0, 0), Ok(()));
        assert_eq!(
            index(vec![hull(most + 1)]).validate(0, 0),
            Err(DecodeError::BadCollision)
        );
        assert_eq!(index(vec![mesh(most + 1)]).validate(0, 0), Ok(()));
        // A grid whose size wraps round to nothing in 64 bits.
        let ground = CollisionIndex {
            heightfield: Some(crate::collision::HeightfieldEntry {
                columns: 1 << 31,
                rows: 1 << 31,
                ..Default::default()
            }),
            ..CollisionIndex::default()
        };
        assert_eq!(ground.validate(0, 0), Err(DecodeError::BadCollision));
    }

    /// A scene with collision, written to a file.
    fn scene_with_collision() -> (LoadedScene, Vec<u8>) {
        let mut loaded = sample_scene();
        let mut collision = sample_collision();
        collision.instance_shapes = vec![1; loaded.scene.instances.len()];
        loaded.collision = Some(collision);
        let bytes = encode(&loaded);
        (loaded, bytes)
    }

    #[test]
    fn a_manifest_with_a_bad_collision_index_is_refused_when_it_is_read() {
        let (_, bytes) = scene_with_collision();
        let scene = decode(&bytes).unwrap().scene;
        assert!(decode_manifest(&encode_manifest(&scene), 0).is_ok());
        let mut bad = scene;
        bad.collision.as_mut().unwrap().shapes[0].vertices = u32::MAX;
        assert_eq!(
            decode_manifest(&encode_manifest(&bad), 0).err(),
            Some(DecodeError::BadCollision)
        );
    }

    #[test]
    fn a_collision_piece_is_unpacked_with_its_limit_when_a_whole_file_is_read() {
        let (_, mut bytes) = scene_with_collision();
        let scene = decode(&bytes).unwrap().scene;
        let (offset, stored) = scene.collision.as_ref().unwrap().ranges()[0];
        let start = (scene.data_offset + offset) as usize;
        // Not deflate at all. Only the limited way of unpacking calls that bad collision, as
        // the plain one would say the file was truncated.
        bytes[start..start + stored as usize].fill(0xff);
        assert_eq!(decode(&bytes).err(), Some(DecodeError::BadCollision));
    }

    #[test]
    fn a_scene_without_gameplay_is_written_exactly_as_before() {
        // No section at all, so viewers from before gameplay read it as they always did.
        let plain = sample_scene();
        let manifest = encode_manifest(&plain.scene);
        assert!(!manifest.windows(4).any(|w| w == GAME_TAG));
        assert!(!manifest.windows(4).any(|w| w == COLLISION_TAG));
        let mut with = sample_scene();
        with.scene.gameplay = vec![GameFile {
            name: "a".to_string(),
            text: "b".to_string(),
        }];
        let longer = encode_manifest(&with.scene);
        assert_eq!(&longer[..manifest.len()], &manifest[..]);
        assert_eq!(decode_manifest(&manifest, 0).unwrap().gameplay, vec![]);
        // Collision only adds a section too, so what comes before it is unchanged.
        let (with_collision, _) = scene_with_collision();
        let mut index = sample_scene().scene;
        index.collision = Some(append_collision(
            with_collision.collision.as_ref().unwrap(),
            &mut Vec::new(),
        ));
        let longer = encode_manifest(&index);
        assert_eq!(&longer[..manifest.len()], &manifest[..]);
    }

    #[test]
    fn broken_gameplay_is_an_error_not_a_guess() {
        let good = encode_gameplay(&[GameFile {
            name: "a.star".to_string(),
            text: "x = 1\n".to_string(),
        }]);
        assert!(decode_gameplay(&good).is_ok());
        for cut in 0..good.len() {
            assert_eq!(
                decode_gameplay(&good[..cut]),
                Err(DecodeError::BadGameplay),
                "cut at {cut}"
            );
        }
        let mut extra = good.clone();
        extra.push(0);
        assert_eq!(decode_gameplay(&extra), Err(DecodeError::BadGameplay));
        let mut huge = good.clone();
        huge[..4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(decode_gameplay(&huge), Err(DecodeError::BadGameplay));
        let mut not_text = good;
        not_text[8] = 0xff;
        assert_eq!(decode_gameplay(&not_text), Err(DecodeError::BadGameplay));
    }

    #[test]
    fn manifests_from_before_sections_still_read() {
        // A manifest of the fixed part alone, as files were before optional sections, reads
        // as a scene with no sea.
        let mut loaded = sample_scene();
        loaded.scene.sea = None;
        let manifest = encode_manifest(&loaded.scene);
        let scene = decode_manifest(&manifest, 0).unwrap();
        assert_eq!(scene.sea, None);
        assert_eq!(scene.clusters, loaded.scene.clusters);
    }

    #[test]
    fn unknown_sections_are_skipped() {
        // A section from a newer writer, before and after the sea, is passed over.
        let mut loaded = sample_scene();
        let sea = Sea {
            level: 2.0,
            ocean: String::new(),
        };
        loaded.scene.sea = Some(sea.clone());
        let mut w = Writer { out: Vec::new() };
        w.section(*b"NEW1", &[1, 2, 3]);
        let mut manifest = encode_manifest(&loaded.scene);
        // Fixed part, then the unknown section, then the sea.
        let mut fixed = sample_scene().scene;
        fixed.sea = None;
        let fixed_len = encode_manifest(&fixed).len();
        let sea_section = manifest.split_off(fixed_len);
        manifest.extend(w.finish());
        manifest.extend(sea_section);
        assert_eq!(decode_manifest(&manifest, 0).unwrap().sea, Some(sea));
        // A section cut short is an error, not a silent loss.
        manifest.truncate(manifest.len() - 2);
        assert!(decode_manifest(&manifest, 0).is_err());
    }

    #[test]
    fn filtering_a_page_loses_nothing_and_packs_smaller() {
        // A strip of vertices a step apart, as neighbours in a mesh are, then some triangles.
        let vertices: Vec<PackedVertex> = (0..600u16)
            .map(|i| PackedVertex {
                data: [
                    1000 + i * 3,
                    40_000 - i * 5,
                    i.wrapping_mul(977) % 64,
                    i.wrapping_mul(31_337),
                    i * 100,
                    65_535 - i * 7,
                ],
            })
            .collect();
        let mut raw: Vec<u8> = bytemuck::cast_slice(&vertices).to_vec();
        raw.extend((0..300u32).flat_map(|i| i.wrapping_mul(0x0101_0101).to_le_bytes()));
        let info = PageInfo {
            vertex_count: vertices.len() as u32,
            triangle_words: 300,
            ..Default::default()
        };
        assert_eq!(raw.len(), info.raw_bytes());
        let stored = filter_page(&raw, &info);
        assert_eq!(stored.len(), raw.len());
        assert_ne!(stored, raw);
        assert_eq!(unfilter_page(&stored, &info), raw);
        assert!(
            deflate_page(&raw, &info).len() * 2
                < miniz_oxide::deflate::compress_to_vec(&raw, DEFLATE_LEVEL).len()
        );
        // A page the tables are wrong about comes back as it went in, to be refused for its size.
        let short = &raw[..100];
        assert_eq!(unfilter_page(&filter_page(short, &info), &info), short);
    }

    #[test]
    fn a_page_and_a_mip_can_be_read_without_the_rest() {
        let loaded = sample_scene();
        let file = encode(&loaded);
        let header = decode_header(&file[..HEADER_BYTES]).unwrap();
        let start = header.manifest_offset as usize;
        let end = start + header.manifest_bytes as usize;
        let scene = decode_manifest(&inflate(&file[start..end]).unwrap(), end as u64).unwrap();

        let (from, to) = page_range(&scene, 1);
        let page = unpack_page(&scene, 1, &file[from as usize..to as usize]).unwrap();
        assert_eq!(page, loaded.pages[0]);

        let (from, to) = upper_mip_range(&scene, 0, 0).unwrap();
        let stored = &file[from as usize..to as usize];
        assert_eq!(stored.len(), 256 * 256 * 8);
        assert_eq!((stored[0], *stored.last().unwrap()), (4, 6));
        assert_eq!(upper_mip_range(&scene, 0, 1), None);
    }

    #[test]
    fn rejects_foreign_and_truncated_data() {
        assert_eq!(decode(b"not a scene").unwrap_err(), DecodeError::BadMagic);
        let bytes = encode(&sample_scene());
        assert_eq!(
            decode(&bytes[..bytes.len() - 1]).unwrap_err(),
            DecodeError::Truncated
        );
    }

    fn skinned_scene() -> LoadedScene {
        let bone = Bone {
            name: "pelvis".to_owned(),
            parent: None,
            translation: Vec3::new(0.0, 1.0, 0.0),
            rotation: Quat::from_rotation_y(0.3),
            inverse_bind: Mat4::from_translation(Vec3::new(0.0, -1.0, 0.0)),
            center: Vec3::ONE,
            radius: 0.5,
        };
        LoadedScene {
            scene: PagedScene {
                meshes: vec![MeshInfo {
                    bone_lists: Some(0),
                    ..Default::default()
                }],
                cluster_bones: vec![[3; 16]],
                skeletons: vec![Skeleton { bones: vec![bone] }],
                poses: vec![0],
                instance_poses: vec![0],
                instances: vec![Instance {
                    mesh: 0,
                    material: 0,
                    transform: Mat4::IDENTITY,
                }],
                materials: vec![Material::default()],
                clusters: vec![Default::default()],
                ..Default::default()
            },
            ..Default::default()
        }
    }

    #[test]
    fn a_scene_with_skins_comes_back_as_it_went_in() {
        let scene = skinned_scene();
        let read = decode(&encode(&scene)).unwrap().scene;
        assert_eq!(read.cluster_bones, scene.scene.cluster_bones);
        assert_eq!(read.skeletons, scene.scene.skeletons);
        assert_eq!(read.poses, [0]);
        assert_eq!(read.instance_poses, [0]);
        assert_eq!(read.meshes[0].bone_lists, Some(0));
    }

    #[test]
    fn clips_travel_with_their_scene_and_bad_ones_are_refused() {
        let mut scene = skinned_scene();
        scene.scene.clips = vec![Clip {
            name: "walk".to_owned(),
            skeleton: 0,
            frames: 2,
            loops: true,
            tracks: vec![BoneTrack {
                rotations_frames: 2,
                translations_frames: 1,
                ..Default::default()
            }],
            rotations: vec![[1, 2, 3], [4, 5, 6]],
            translations: vec![[7, 8, 9]],
        }];
        let read = decode(&encode(&scene)).unwrap().scene;
        assert_eq!(read.clips, scene.scene.clips);
        scene.scene.clips[0].rotations.pop();
        assert_eq!(decode(&encode(&scene)).err(), Some(DecodeError::BadSkin));
        scene.scene.clips[0] = Clip {
            tracks: vec![BoneTrack::default()],
            ..Default::default()
        };
        assert_eq!(decode(&encode(&scene)).err(), Some(DecodeError::BadSkin));
    }

    #[test]
    fn a_scene_without_skins_has_no_section_for_them() {
        let mut scene = skinned_scene();
        scene.scene.skeletons.clear();
        let read = decode(&encode(&scene)).unwrap().scene;
        assert!(read.cluster_bones.is_empty());
        assert_eq!(read.meshes[0].bone_lists, None);
    }

    #[test]
    fn numbers_a_skin_cannot_mean_are_refused() {
        let mut bad_pose = skinned_scene();
        bad_pose.scene.poses = vec![4];
        assert_eq!(decode(&encode(&bad_pose)).err(), Some(DecodeError::BadSkin));
        let mut bad_instance = skinned_scene();
        bad_instance.scene.instance_poses = vec![9];
        assert_eq!(
            decode(&encode(&bad_instance)).err(),
            Some(DecodeError::BadSkin)
        );
        let mut bad_bones = skinned_scene();
        bad_bones.scene.skeletons[0].bones[0].parent = Some(0);
        assert_eq!(
            decode(&encode(&bad_bones)).err(),
            Some(DecodeError::BadSkin)
        );
    }

    #[test]
    fn rejects_other_versions() {
        let mut bytes = encode(&sample_scene());
        bytes[8] = 3;
        assert_eq!(
            decode(&bytes).unwrap_err(),
            DecodeError::UnsupportedVersion(3)
        );
    }

    #[test]
    fn rejects_textures_of_the_wrong_size() {
        let mut scene = sample_scene();
        scene.scene.base_textures[0].normal_mips[0].pop();
        assert_eq!(
            decode(&encode(&scene)).unwrap_err(),
            DecodeError::BadTextures
        );
    }

    #[test]
    fn rejects_dangling_instance_references() {
        let mut scene = sample_scene();
        scene.scene.instances[0].mesh = 9;
        assert_eq!(
            decode(&encode(&scene)).unwrap_err(),
            DecodeError::BadInstanceReference
        );
    }

    #[test]
    fn rejects_page_tables_that_point_outside_the_scene() {
        let mut scene = sample_scene();
        scene.scene.sets[1].first_generated = 40;
        assert_eq!(decode(&encode(&scene)).unwrap_err(), DecodeError::BadPages);

        let mut scene = sample_scene();
        scene.scene.pinned_data.pop();
        assert_eq!(decode(&encode(&scene)).unwrap_err(), DecodeError::BadPages);
    }
}
