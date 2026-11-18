//! CPU-side description of a scene.

use crate::gpu::{PackedCluster, PackedNode, PackedVertex};
use crate::paged::ClusterSet;
use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};

/// A mesh preprocessed into a cluster LOD DAG, ready for upload.
#[derive(Clone, Debug, Default)]
pub struct VirtualMesh {
    pub clusters: Vec<PackedCluster>,
    pub nodes: Vec<PackedNode>,
    pub vertices: Vec<PackedVertex>,
    pub triangles: Vec<u32>,
    /// Clusters grouped by shared parent. Each hierarchy leaf is one set.
    pub sets: Vec<ClusterSet>,
    /// See [`ClusterSet::first_generated`].
    pub generated: Vec<u32>,
    /// Index of the hierarchy root in `nodes`.
    pub root_node: u32,
    /// Longest root-to-leaf path in the hierarchy, in nodes.
    pub hierarchy_depth: u32,
    /// Object-space position of the 16-bit grid's origin.
    pub quant_origin: Vec3,
    /// Object-space size of one grid step.
    pub quant_step: f32,
    /// UV decoding: `uv = uv_origin + packed * uv_step`.
    pub uv_origin: [f32; 2],
    pub uv_step: [f32; 2],
    /// Object-space bounding sphere of the whole mesh.
    pub bounds_center: Vec3,
    pub bounds_radius: f32,
    /// Triangle count of the source mesh (finest LOD only).
    pub source_triangles: u64,
    /// For a skinned mesh, each cluster's bones. Empty for one that isn't. In a skinned mesh,
    /// each cluster's vertices in `vertices` are followed by their skins.
    pub cluster_bones: Vec<crate::gpu::ClusterBones>,
}

impl VirtualMesh {
    pub fn skinned(&self) -> bool {
        !self.cluster_bones.is_empty()
    }
}

/// A physically based surface description.
///
/// `#[repr(C)]` so the runtime can upload this slice directly; mirrored by
/// `Material` in `common/scene.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct Material {
    pub base_color: [f32; 4],
    /// RGB radiance.
    pub emissive: [f32; 3],
    /// [`Material::TWO_SIDED`], [`Material::FOLIAGE`], [`Material::MASKED`],
    /// [`Material::SWAYS`] and [`Material::TREE`]. Older files hold zero here.
    pub flags: u32,
    pub roughness: f32,
    pub metallic: f32,
    /// Texture repeats per world unit of UV.
    pub uv_scale: f32,
    /// Strength of the normal map; 0 disables it.
    pub normal_strength: f32,
    /// Index into the texture array, or -1 for an untextured material.
    pub texture_layer: i32,
    /// How strongly to hide repetition, for textures that tile. Leave at 0 for textures made
    /// for one object, such as scans, whose colour would otherwise be stained.
    pub macro_variation: f32,
    /// Which texture array holds the layer right now. Set by the renderer, zero on disk.
    pub texture_tier: u32,
    /// Where in that array the layer is. Set by the renderer, zero on disk.
    pub texture_slot: u32,
}

impl Material {
    /// Drawn from both sides, with the normal turned towards the viewer.
    pub const TWO_SIDED: u32 = 1;
    /// Thin, translucent vegetation: light from behind shines through, tinted by its colour, as
    /// Unreal's two-sided foliage shading model does. Foliage should also be two-sided.
    pub const FOLIAGE: u32 = 2;
    /// Cut out: alpha in its colour map is coverage instead of cavity, and a texel is solid when
    /// that is at least 0.5. The cut-off is built into the map (`docs/cutouts.md`).
    pub const MASKED: u32 = 4;
    /// Sways in the wind as grass does: each point bends by its height above the mesh's own
    /// ground, `y = 0`, so the mesh is made standing on it (`docs/weather-effects.md`). Older
    /// files never sway.
    pub const SWAYS: u32 = 8;
    /// With [`Material::SWAYS`], sways as a tree rather than as grass: its trunk leans and
    /// sways by its height above the mesh's ground, its branches by their reach from the trunk
    /// along y, and, if it is [`Material::FOLIAGE`] too, its leaves flutter.
    pub const TREE: u32 = 16;
    /// Ground of three kinds blended: this material's, and the next two materials', whose
    /// texture layers come right after this one's. The mesh's texture coordinates hold how much
    /// of the second and the third there is, and every texture is laid by the world's x and z.
    pub const BLENDS: u32 = 32;
    /// The paint of something driven: the game can give it another colour, and it is what
    /// rusts (`Coat` in `solstice-render`). An importer marks a material whose name begins
    /// `paint`.
    pub const PAINT: u32 = 64;
    /// Inside something driven, out of the weather: its cab, its engine bay. It takes no
    /// dust or mud. An importer marks a material whose name begins `inside`.
    pub const INSIDE: u32 = 128;
    /// Glass, seen through: of the pixels it covers it is drawn in a share, its colour's
    /// alpha, and what is behind it shows in the rest, a different share each frame so that
    /// the two blend (`vg/mask.wgsl`). An importer marks a material whose alpha is blended.
    pub const GLASS: u32 = 256;
    /// Mud: where ground blended of three kinds ([`Material::BLENDS`]) is this kind, it is
    /// drawn as mud lies, wet at its heart and dried to a cracked crust at its rim
    /// (`ground/ruts.wgsl`, `docs/ruts.md`). Older files' mud is drawn as any ground is.
    pub const MUD: u32 = 512;
    /// `flags` above this bit hold one more than the material's layer in [`GlowMaps`], or zero
    /// for a material whose emissive colour is even all over.
    pub const GLOW_SHIFT: u32 = 16;
    /// The `emissive` of a material with an emissive map is what the map gives averaged over the
    /// surface, taken as this fraction of it glowing, so that what reads no map, such as the
    /// light it gives the scene, sees about the right amount. Mirrored in `vg/resolve.wgsl`.
    pub const GLOW_MAP_AVERAGE: f32 = 0.1;

    /// The material's layer in the scene's [`GlowMaps`], if it has one.
    pub fn glow_layer(&self) -> Option<u32> {
        (self.flags >> Self::GLOW_SHIFT).checked_sub(1)
    }
}

impl Default for Material {
    fn default() -> Self {
        Self {
            base_color: [0.8, 0.8, 0.8, 1.0],
            emissive: [0.0; 3],
            flags: 0,
            roughness: 0.7,
            metallic: 0.0,
            uv_scale: 1.0,
            normal_strength: 1.0,
            texture_layer: -1,
            macro_variation: 0.0,
            texture_tier: 0,
            texture_slot: 0,
        }
    }
}

/// An omnidirectional local light. Mirrored by `Light` in `common/scene.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct Light {
    pub position: [f32; 3],
    /// Distance at which the light fades to nothing.
    pub range: f32,
    /// Linear RGB intensity, in candela.
    pub intensity: [f32; 3],
    pub _pad: u32,
}

/// One texture set, with a full mip chain for each map. Their byte layout depends on the
/// scene's [`TextureEncoding`].
///
/// * `color`: RGB base colour (sRGB), A cavity, or coverage for a [`Material::MASKED`] material.
/// * `normal`: RG tangent-space normal XY.
/// * `material`: R roughness multiplier (0.5 is neutral), G metallic multiplier.
#[derive(Clone, Debug, Default)]
pub struct TextureLayer {
    pub color_mips: Vec<Vec<u8>>,
    pub normal_mips: Vec<Vec<u8>>,
    pub material_mips: Vec<Vec<u8>>,
}

/// The emissive maps of a scene's materials: one array of square BC7 sRGB textures. They hold
/// soft light and are few, so they are small and all loaded with the scene.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GlowMaps {
    /// Edge length of every layer.
    pub edge: u32,
    /// Each layer's whole mip chain, finest first.
    pub layers: Vec<Vec<Vec<u8>>>,
}

/// A corner of a [`Card`], in world space.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct CardVertex {
    pub position: [f32; 3],
    pub uv: [f32; 2],
}

/// Triangles drawn transparent and glowing over the lit scene, such as a flame on a lamp.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct Card {
    pub first_vertex: u32,
    /// A multiple of three.
    pub vertex_count: u32,
    /// Index into [`Cards::images`].
    pub image: u32,
    pub opacity: f32,
    /// Multiplies the glow map, as RGB radiance.
    pub emissive: [f32; 3],
}

/// The two maps of a card, of the same size and not necessarily square. BC7 sRGB, whole chains.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CardImage {
    pub width: u32,
    pub height: u32,
    /// Alpha is coverage.
    pub color_mips: Vec<Vec<u8>>,
    /// RGB light.
    pub glow_mips: Vec<Vec<u8>>,
}

/// A scene's transparent glowing cards, which are kept out of its clusters.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Cards {
    pub vertices: Vec<CardVertex>,
    pub cards: Vec<Card>,
    pub images: Vec<CardImage>,
}

/// How a scene's texture maps are stored.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TextureEncoding {
    /// Colour is RGBA8, normal and material are RG8.
    #[default]
    Raw,
    /// Colour is BC7, normal and material are BC5.
    Block,
}

impl TextureEncoding {
    /// Bytes in a mip level of a colour map, whose edge is `edge` texels.
    pub fn color_mip_bytes(self, edge: u32) -> usize {
        self.mip_bytes(edge, 4)
    }

    /// Bytes in a mip level of a normal or material map, whose edge is `edge` texels.
    pub fn data_mip_bytes(self, edge: u32) -> usize {
        self.mip_bytes(edge, 2)
    }

    fn mip_bytes(self, edge: u32, raw_texel_bytes: usize) -> usize {
        let edge = edge as usize;
        match self {
            Self::Raw => edge * edge * raw_texel_bytes,
            // Both block formats take 16 bytes per 4x4 block, and a small mip still fills one.
            Self::Block => edge.div_ceil(4).pow(2) * 16,
        }
    }
}

/// One placement of a mesh in the world.
#[derive(Clone, Copy, Debug)]
pub struct Instance {
    pub mesh: u32,
    pub material: u32,
    pub transform: Mat4,
}

/// Where a viewer should start.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct Viewpoint {
    pub position: [f32; 3],
    /// Rotation around +Y, in radians. Zero looks along -Z.
    pub yaw: f32,
    /// Rotation above the horizon, in radians.
    pub pitch: f32,
}

/// An open sea around a scene: where its surface lies, and what its water is like.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Sea {
    /// Height of the calm surface, in metres.
    pub level: f32,
    /// The name of a built-in ocean, such as `coastal`, or a whole ocean in its text form, as
    /// `solstice_render::Ocean::parse` reads it, so a scene can carry an ocean of its own.
    pub ocean: String,
}

/// Still water over a scene: ponds, pools and basins, each a level sheet over a rectangle of it
/// whose basin's own banks shape its edge, and what their water is like.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StillWater {
    /// Each pond: its least x and z, its greatest x and z, and the water's height, in metres.
    pub ponds: Vec<[f32; 5]>,
    /// The name of a built-in ocean, or a whole ocean in its text form, whose absorption and
    /// scattering colour the water. Empty for clear water.
    pub water: String,
}

/// Which entity of an editor project made which instances (`docs/editor.md`), so the editor can
/// turn a click on an instance into the thing to select, and move what it made together.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Source {
    /// The entity's id in its project, or `terrain` or `circuit` for all that the scene's land
    /// made.
    pub id: String,
    pub instances: std::ops::Range<u32>,
    /// Where the bake placed the entity: each of its instances is this times the instance's
    /// place in the model. It holds what the project doesn't say, such as the height a model
    /// was seated at on the ground, or the turn and scale an actor's file gives its mesh.
    /// Identity for the land.
    pub placement: glam::Mat4,
}

/// One text file of a scene's gameplay: an actor file, a script, the input actions, or the list
/// of placed actors. Carried as it was written, by the name it was written under, so the scene
/// file needs no knowledge of what is in it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GameFile {
    /// Its path relative to the scene's text file, with `/` between folders.
    pub name: String,
    pub text: String,
}

/// A complete, self-contained scene.
#[derive(Clone, Debug, Default)]
pub struct SceneAsset {
    pub meshes: Vec<VirtualMesh>,
    pub materials: Vec<Material>,
    pub instances: Vec<Instance>,
    pub lights: Vec<Light>,
    /// Edge length of the square textures in `textures`.
    pub texture_size: u32,
    pub texture_encoding: TextureEncoding,
    pub textures: Vec<TextureLayer>,
    pub viewpoint: Option<Viewpoint>,
    pub sea: Option<Sea>,
    /// The scene's ponds, if it has any. A scene with a sea draws the sea and not its ponds.
    pub still_water: Option<StillWater>,
    /// The scene's gameplay, if it has any. Empty for a scene that is only looked at.
    pub gameplay: Vec<GameFile>,
    /// Which entity made which instances, for a scene baked from a project. Empty otherwise.
    pub sources: Vec<Source>,
    /// How each instance collides, where the scene says. Instances past its end are
    /// [`Collide::Auto`](crate::collision::Collide::Auto).
    pub collide: Vec<crate::collision::Collide>,
    /// The ground as a grid of heights, for collision, if the scene has terrain.
    pub ground: Option<crate::collision::Heightfield>,
    /// Every skeleton in the scene.
    pub skeletons: Vec<crate::skeleton::Skeleton>,
    /// The poses a scene can hold at once, each the index of its skeleton: one for each placed
    /// character, however many meshes it is made of.
    pub poses: Vec<u32>,
    /// Which pose each instance follows, or [`crate::skeleton::NO_POSE`]. Instances past its end
    /// follow none.
    pub instance_poses: Vec<u32>,
    /// Every animation in the scene.
    pub clips: Vec<crate::skeleton::Clip>,
}

impl SceneAsset {
    /// Triangles drawn if every instance rendered its finest LOD.
    pub fn total_source_triangles(&self) -> u64 {
        self.instances
            .iter()
            .map(|i| self.meshes[i.mesh as usize].source_triangles)
            .sum()
    }

    /// Triangles stored once per mesh, regardless of instancing.
    pub fn unique_source_triangles(&self) -> u64 {
        self.meshes.iter().map(|m| m.source_triangles).sum()
    }
}
