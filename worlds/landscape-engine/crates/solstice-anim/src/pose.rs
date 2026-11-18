//! A skeleton's pose, as each bone's place relative to its parent, and what can be done with it.

use glam::{Mat4, Quat, Vec3};
use solstice_asset::Skeleton;

/// Where every bone is, relative to its parent. Bones never scale.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Pose {
    pub rotations: Vec<Quat>,
    pub translations: Vec<Vec3>,
}

impl Pose {
    /// A pose for this many bones, all at their parents' origins.
    pub fn with_bones(bones: usize) -> Self {
        Self {
            rotations: vec![Quat::IDENTITY; bones],
            translations: vec![Vec3::ZERO; bones],
        }
    }

    /// The rest pose of a skeleton.
    pub fn rest(skeleton: &Skeleton) -> Self {
        Self {
            rotations: skeleton.bones.iter().map(|b| b.rotation).collect(),
            translations: skeleton.bones.iter().map(|b| b.translation).collect(),
        }
    }

    pub fn copy_from(&mut self, other: &Pose) {
        self.rotations.copy_from_slice(&other.rotations);
        self.translations.copy_from_slice(&other.translations);
    }

    /// Each bone's place in the mesh's own space, what the renderer poses a character with.
    pub fn to_model(&self, skeleton: &Skeleton, out: &mut Vec<Mat4>) {
        out.clear();
        for (index, bone) in skeleton.bones.iter().enumerate() {
            let local =
                Mat4::from_rotation_translation(self.rotations[index], self.translations[index]);
            let parent = bone
                .parent
                .and_then(|p| out.get(p as usize).copied())
                .unwrap_or(Mat4::IDENTITY);
            out.push(parent * local);
        }
    }
}

/// Which bones a blend reaches, with how much weight each: 1 fully, 0 not at all.
#[derive(Clone, Debug, PartialEq)]
pub struct BoneMask {
    pub weights: Vec<f32>,
}

impl BoneMask {
    pub fn all(bones: usize) -> Self {
        Self {
            weights: vec![1.0; bones],
        }
    }

    /// The bones named and everything hanging from them, wholly.
    pub fn branches(skeleton: &Skeleton, roots: &[&str]) -> Self {
        let mut weights = vec![0.0; skeleton.bones.len()];
        for (index, bone) in skeleton.bones.iter().enumerate() {
            let parent_masked = bone.parent.is_some_and(|p| weights[p as usize] > 0.0);
            if parent_masked || roots.iter().any(|name| *name == bone.name) {
                weights[index] = 1.0;
            }
        }
        Self { weights }
    }
}

/// Blends `a` towards `b` by `weight`, into `a`. A mask says how much each bone takes part.
pub fn blend(a: &mut Pose, b: &Pose, weight: f32, mask: Option<&BoneMask>) {
    if weight <= 0.0 {
        return;
    }
    for bone in 0..a.rotations.len() {
        let w = weight * mask.map_or(1.0, |m| m.weights[bone]);
        if w <= 0.0 {
            continue;
        }
        let target = if a.rotations[bone].dot(b.rotations[bone]) < 0.0 {
            -b.rotations[bone]
        } else {
            b.rotations[bone]
        };
        a.rotations[bone] = a.rotations[bone].lerp(target, w).normalize();
        a.translations[bone] = a.translations[bone].lerp(b.translations[bone], w);
    }
}

/// Adds the change that takes `reference` to `delta` onto `base`, scaled by `weight`.
pub fn add(base: &mut Pose, delta: &Pose, reference: &Pose, weight: f32) {
    for bone in 0..base.rotations.len() {
        let change = reference.rotations[bone].inverse() * delta.rotations[bone];
        let change = Quat::IDENTITY.slerp(change, weight);
        base.rotations[bone] = (base.rotations[bone] * change).normalize();
        base.translations[bone] +=
            (delta.translations[bone] - reference.translations[bone]) * weight;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use solstice_asset::Bone;

    fn chain() -> Skeleton {
        let bone = |name: &str, parent| Bone {
            name: name.to_owned(),
            parent,
            translation: Vec3::Y,
            ..Default::default()
        };
        Skeleton {
            bones: vec![
                bone("hips", None),
                bone("spine", Some(0)),
                bone("arm", Some(1)),
                bone("leg", Some(0)),
            ],
        }
    }

    #[test]
    fn a_pose_turns_into_places_down_the_chain() {
        let skeleton = chain();
        let mut pose = Pose::rest(&skeleton);
        pose.rotations[1] = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);
        let mut places = Vec::new();
        pose.to_model(&skeleton, &mut places);
        assert!(places[0].w_axis.truncate().abs_diff_eq(Vec3::Y, 1.0e-5));
        // The spine turns, so the arm, one up in its space, ends up beside it.
        let arm = places[2].w_axis.truncate();
        assert!(arm.abs_diff_eq(Vec3::new(-1.0, 2.0, 0.0), 1.0e-5), "{arm}");
    }

    #[test]
    fn half_a_blend_is_halfway_and_a_mask_keeps_bones_out() {
        let skeleton = chain();
        let mut a = Pose::rest(&skeleton);
        let mut b = Pose::rest(&skeleton);
        b.translations[1] = Vec3::Y * 3.0;
        b.translations[3] = Vec3::Y * 3.0;
        let mask = BoneMask::branches(&skeleton, &["spine"]);
        assert_eq!(mask.weights, [0.0, 1.0, 1.0, 0.0]);
        blend(&mut a, &b, 0.5, Some(&mask));
        assert_eq!(a.translations[1], Vec3::Y * 2.0);
        assert_eq!(a.translations[3], Vec3::Y);
    }

    #[test]
    fn an_additive_adds_only_the_difference() {
        let skeleton = chain();
        let reference = Pose::rest(&skeleton);
        let mut delta = Pose::rest(&skeleton);
        delta.rotations[0] = Quat::from_rotation_y(0.6);
        delta.translations[0] += Vec3::X;
        let mut base = Pose::rest(&skeleton);
        base.rotations[0] = Quat::from_rotation_x(0.3);
        add(&mut base, &delta, &reference, 1.0);
        let want = Quat::from_rotation_x(0.3) * Quat::from_rotation_y(0.6);
        assert!(base.rotations[0].dot(want).abs() > 0.99999);
        assert!(base.translations[0].abs_diff_eq(Vec3::new(1.0, 1.0, 0.0), 1.0e-5));
    }
}
