//! Data shared between the offline builder and the runtime renderer.
//!
//! * [`gpu`] defines the exact memory layout of everything the shaders read.
//! * [`scene`] defines the CPU-side description of a scene.
//! * [`paged`] divides a scene into what stays resident and what loads on demand.
//! * [`file`] stores a paged scene so that its pages can be read one at a time.

pub mod collision;
pub mod file;
pub mod gpu;
pub mod join;
pub mod pack;
pub mod paged;
pub mod scene;
pub mod skeleton;

pub use skeleton::{Bone, BoneTrack, Clip, Skeleton, CLIP_HZ, NO_POSE};

pub use collision::{
    Collide, Collision, CollisionIndex, CollisionShape, Heightfield, MAX_COORDINATE,
    MAX_HULL_POINTS, NO_SHAPE,
};
pub use gpu::{unpack_triangle, PackedCluster, PackedNode, PackedSkin, PackedVertex};
pub use paged::{LoadedScene, PagedScene};
pub use scene::{
    Card, CardImage, CardVertex, Cards, GameFile, GlowMaps, Instance, Light, Material, SceneAsset,
    Sea, Source, StillWater, TextureEncoding, TextureLayer, Viewpoint, VirtualMesh,
};
