//! A character's bones, as baked into a scene.

use glam::{Mat4, Quat, Vec3};

/// Instances past the end of [`crate::SceneAsset::instance_poses`], or holding this, follow no
/// pose. A skinned mesh among them draws as it was bound.
pub const NO_POSE: u32 = u32::MAX;

/// The most bones a skeleton may have: [`crate::gpu::PackedSkin`] names them in eight bits.
pub const MAX_BONES: usize = 256;

/// Clips are sampled this many times a second.
pub const CLIP_HZ: f32 = 30.0;

/// Where one bone's samples are in a [`Clip`]. A bone that doesn't move keeps one sample.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct BoneTrack {
    pub rotations_first: u32,
    pub rotations_frames: u32,
    pub translations_first: u32,
    pub translations_frames: u32,
    /// The range translations are quantised over.
    pub translation_min: [f32; 3],
    pub translation_max: [f32; 3],
}

/// An animation, retargeted onto one skeleton and quantised (solstice-anim samples it).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Clip {
    pub name: String,
    /// The index of its skeleton in the scene.
    pub skeleton: u32,
    /// Samples, at [`CLIP_HZ`].
    pub frames: u32,
    /// A looping clip's last sample wraps round to its first.
    pub loops: bool,
    pub tracks: Vec<BoneTrack>,
    /// Three smallest components, the index of the dropped one in the first's top two bits.
    pub rotations: Vec<[u16; 3]>,
    /// Quantised over each track's range.
    pub translations: Vec<[u16; 3]>,
}

/// One bone of a [`Skeleton`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Bone {
    pub name: String,
    /// The bone it hangs from, which has a lower index, or `None` for the root.
    pub parent: Option<u32>,
    /// Where it stands in its parent's space, in the rest pose. Bones never scale.
    pub translation: Vec3,
    pub rotation: Quat,
    /// Takes the mesh's bind space into the bone's own.
    pub inverse_bind: Mat4,
    /// A sphere in bind space around every vertex this bone moves, which tells how far a pose can
    /// carry them. Radius 0 for a bone that moves nothing.
    pub center: Vec3,
    pub radius: f32,
}

/// A character's bones, parents before children.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Skeleton {
    pub bones: Vec<Bone>,
}

impl Skeleton {
    /// The rest pose: each bone's place in the mesh's space.
    pub fn rest(&self) -> Vec<Mat4> {
        let mut world: Vec<Mat4> = Vec::with_capacity(self.bones.len());
        for bone in &self.bones {
            let local = Mat4::from_rotation_translation(bone.rotation, bone.translation);
            let parent = bone
                .parent
                .and_then(|p| world.get(p as usize).copied())
                .unwrap_or(Mat4::IDENTITY);
            world.push(parent * local);
        }
        world
    }

    /// Whether the bones are in the order everything relies on, and there are not too many.
    pub fn is_well_formed(&self) -> bool {
        self.bones.len() <= MAX_BONES
            && self
                .bones
                .iter()
                .enumerate()
                .all(|(index, bone)| bone.parent.is_none_or(|p| (p as usize) < index))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chain() -> Skeleton {
        let bone = |parent, y: f32| Bone {
            parent,
            translation: Vec3::new(0.0, y, 0.0),
            rotation: Quat::IDENTITY,
            ..Default::default()
        };
        Skeleton {
            bones: vec![bone(None, 1.0), bone(Some(0), 0.5), bone(Some(1), 0.25)],
        }
    }

    #[test]
    fn the_rest_pose_follows_the_chain() {
        let rest = chain().rest();
        assert_eq!(rest[2].w_axis.truncate(), Vec3::new(0.0, 1.75, 0.0));
    }

    #[test]
    fn a_parent_after_its_child_is_refused() {
        let mut skeleton = chain();
        assert!(skeleton.is_well_formed());
        skeleton.bones[1].parent = Some(2);
        assert!(!skeleton.is_well_formed());
    }
}
