//! Reads animation clips from a glTF, and fits them onto the character that will play them.
//!
//! A clip's file need not hold a mesh or a skin: a Blender armature exported on its own is enough.
//! Every node in it is a bone. Where a file's rig differs from the character's, a role file on
//! each side says which bones play each part, and [`retarget`] lines up the two rigs' limbs at
//! rest, then carries each bone's rotations across in its rest frame, and scales the pelvis's
//! travel by the ratio of leg lengths, so feet don't slide (docs/gameplay.md, Retargeting).

use crate::character::Roles;
use crate::ImportError;
use glam::{Quat, Vec3};
use gltf::animation::util::ReadOutputs;
use gltf::animation::Interpolation;
use solstice_anim::{Frames, Pose};
use solstice_asset::{Bone, Skeleton, CLIP_HZ};
use std::path::Path;

/// A file of clips, in the file's own rig.
#[derive(Clone, Debug, Default)]
pub struct ClipFile {
    pub rig: Skeleton,
    /// Each clip by its name in the file, as a full frame of every bone.
    pub clips: Vec<(String, Frames)>,
}

/// One animated value of a bone over time.
struct Channel<T> {
    times: Vec<f32>,
    values: Vec<T>,
    step: bool,
}

impl<T: Copy> Channel<T> {
    /// The keys around `time`, and how far between them it lies.
    fn at(&self, time: f32, blend: impl Fn(T, T, f32) -> T) -> T {
        if self.times.len() < 2 || time <= self.times[0] {
            return self.values[0];
        }
        if time >= *self.times.last().expect("not empty") {
            return *self.values.last().expect("not empty");
        }
        let next = self.times.partition_point(|&t| t <= time);
        let (a, b) = (next - 1, next);
        if self.step {
            return self.values[a];
        }
        let t = (time - self.times[a]) / (self.times[b] - self.times[a]).max(1.0e-6);
        blend(self.values[a], self.values[b], t)
    }
}

/// Every node of a file's default scene, parents first, as a rig of bones standing at rest.
fn read_rig(document: &gltf::Document) -> (Skeleton, Vec<Option<u32>>) {
    fn walk(
        node: gltf::Node<'_>,
        parent: Option<u32>,
        bones: &mut Vec<Bone>,
        numbers: &mut [Option<u32>],
    ) {
        let (translation, rotation, _) = node.transform().decomposed();
        let number = bones.len() as u32;
        numbers[node.index()] = Some(number);
        bones.push(Bone {
            name: node.name().unwrap_or("").to_owned(),
            parent,
            translation: Vec3::from(translation),
            rotation: Quat::from_array(rotation),
            inverse_bind: glam::Mat4::IDENTITY,
            center: Vec3::ZERO,
            radius: 0.0,
        });
        for child in node.children() {
            walk(child, Some(number), bones, numbers);
        }
    }
    let mut numbers = vec![None; document.nodes().len()];
    let mut bones = Vec::new();
    if let Some(scene) = document
        .default_scene()
        .or_else(|| document.scenes().next())
    {
        for node in scene.nodes() {
            walk(node, None, &mut bones, &mut numbers);
        }
    }
    (Skeleton { bones }, numbers)
}

/// Reads every animation in a glTF, with `loops` saying which of them repeat. A repeating clip's
/// last key is its first over again, so it has one frame fewer than one that doesn't repeat.
pub fn read_clips(
    path: impl AsRef<Path>,
    loops: impl Fn(&str) -> bool,
) -> Result<ClipFile, ImportError> {
    let (document, buffers, _) = gltf::import(path).map_err(ImportError::Gltf)?;
    let (rig, numbers) = read_rig(&document);
    let mut clips = Vec::new();
    for animation in document.animations() {
        let name = animation.name().unwrap_or("").to_owned();
        let bones = rig.bones.len();
        let mut rotations: Vec<Option<Channel<Quat>>> = (0..bones).map(|_| None).collect();
        let mut translations: Vec<Option<Channel<Vec3>>> = (0..bones).map(|_| None).collect();
        let mut seconds = 0.0f32;
        let mut start = None;
        for channel in animation.channels() {
            let Some(bone) = numbers[channel.target().node().index()] else {
                continue;
            };
            let reader = channel.reader(|b| buffers.get(b.index()).map(|d| &d.0[..]));
            let Some(times) = reader.read_inputs() else {
                continue;
            };
            // Blender writes its first frame at a frame's length in, not at zero.
            let mut times: Vec<f32> = times.collect();
            let first = *start.get_or_insert(times.first().copied().unwrap_or(0.0));
            for time in &mut times {
                *time -= first;
            }
            seconds = seconds.max(times.last().copied().unwrap_or(0.0));
            let interpolation = channel.sampler().interpolation();
            let step = interpolation == Interpolation::Step;
            // A cubic spline stores an in tangent, a value and an out tangent for each key.
            let keep = |values: Vec<[f32; 3]>| -> Vec<[f32; 3]> {
                if interpolation == Interpolation::CubicSpline {
                    values.chunks_exact(3).map(|c| c[1]).collect()
                } else {
                    values
                }
            };
            match reader.read_outputs() {
                Some(ReadOutputs::Translations(values)) => {
                    translations[bone as usize] = Some(Channel {
                        times,
                        values: keep(values.collect()).into_iter().map(Vec3::from).collect(),
                        step,
                    });
                }
                Some(ReadOutputs::Rotations(values)) => {
                    let values: Vec<[f32; 4]> = values.into_f32().collect();
                    let values = if interpolation == Interpolation::CubicSpline {
                        values.chunks_exact(3).map(|c| c[1]).collect()
                    } else {
                        values
                    };
                    rotations[bone as usize] = Some(Channel {
                        times,
                        values: values.into_iter().map(Quat::from_array).collect(),
                        step,
                    });
                }
                _ => {}
            }
        }

        let looping = loops(&name);
        let spans = (seconds * CLIP_HZ).round() as usize;
        let count = (if looping { spans } else { spans + 1 }).max(1);
        let mut frames = Frames::default();
        for frame in 0..count {
            let time = frame as f32 / CLIP_HZ;
            let mut r = Vec::with_capacity(bones);
            let mut t = Vec::with_capacity(bones);
            for (bone, rest) in rig.bones.iter().enumerate() {
                r.push(rotations[bone].as_ref().map_or(rest.rotation, |channel| {
                    channel.at(time, |a, b, f| {
                        let b = if a.dot(b) < 0.0 { -b } else { b };
                        a.lerp(b, f).normalize()
                    })
                }));
                t.push(
                    translations[bone]
                        .as_ref()
                        .map_or(rest.translation, |channel| {
                            channel.at(time, |a, b, f| a.lerp(b, f))
                        }),
                );
            }
            frames.rotations.push(r);
            frames.translations.push(t);
        }
        clips.push((name, frames));
    }
    Ok(ClipFile { rig, clips })
}

/// Which source bone plays each target bone, for bones in roles both rigs name: pairs of bones
/// by where they lie along their chain, however long each chain is.
fn role_map(
    source: &Skeleton,
    source_roles: &Roles,
    target: &Skeleton,
    target_roles: &Roles,
) -> Vec<Option<usize>> {
    let number =
        |skeleton: &Skeleton, name: &str| skeleton.bones.iter().position(|bone| bone.name == name);
    let mut map = vec![None; target.bones.len()];
    for (role, target_chain) in &target_roles.roles {
        let Some((_, source_chain)) = source_roles.roles.iter().find(|(r, _)| r == role) else {
            continue;
        };
        for (index, bone) in target_chain.iter().enumerate() {
            let along = if target_chain.len() == 1 {
                0
            } else {
                (index * (source_chain.len() - 1) + (target_chain.len() - 1) / 2)
                    / (target_chain.len() - 1)
            };
            if let (Some(t), Some(s)) = (number(target, bone), number(source, &source_chain[along]))
            {
                map[t] = Some(s);
            }
        }
    }
    map
}

/// A bone and its parents, from the root down to it.
fn chain_to_root(skeleton: &Skeleton, bone: usize) -> Vec<usize> {
    let mut chain = vec![bone];
    let mut at = bone;
    while let Some(parent) = skeleton.bones[at].parent {
        at = parent as usize;
        chain.push(at);
    }
    chain.reverse();
    chain
}

/// How far from the pelvis the feet stand at rest, for the ratio of leg lengths.
fn leg_length(skeleton: &Skeleton, roles: &Roles) -> Option<f32> {
    let rest = skeleton.rest();
    let at = |name: &str| {
        skeleton
            .bones
            .iter()
            .position(|bone| bone.name == name)
            .map(|index| rest[index].w_axis.truncate())
    };
    let (_, pelvis) = roles.roles.iter().find(|(r, _)| r == "pelvis")?;
    let (_, leg) = roles
        .roles
        .iter()
        .find(|(r, _)| r == "leg_l" || r == "leg_r")?;
    Some(at(pelvis.first()?)?.distance(at(leg.last()?)?))
}

/// Turns `a` onto `to`, then rolls about `to` until `across`, turned the same way, lies as nearly
/// along `to_across` as it can.
fn frame_between(a: Vec3, across: Vec3, to: Vec3, to_across: Vec3) -> Quat {
    let swing = Quat::from_rotation_arc(a, to);
    let flat = |v: Vec3| (v - to * v.dot(to)).normalize_or_zero();
    let (now, wanted) = (flat(swing * across), flat(to_across));
    let roll = now.cross(wanted).dot(to).atan2(now.dot(wanted));
    (Quat::from_axis_angle(to, roll) * swing).normalize()
}

/// How far `rotation` turns about `axis`, with the rest of its turn left out.
fn twist_about(rotation: Quat, axis: Vec3) -> f32 {
    let along = Vec3::new(rotation.x, rotation.y, rotation.z).dot(axis);
    let angle = 2.0 * along.atan2(rotation.w);
    // The short way round.
    if angle > std::f32::consts::PI {
        angle - std::f32::consts::TAU
    } else if angle < -std::f32::consts::PI {
        angle + std::f32::consts::TAU
    } else {
        angle
    }
}

/// What turns each of the target's limbs, at rest, to lie along the source's. Rigs differ most in
/// how they rest their limbs, one with its arms in a T and one with them down and the elbows bent,
/// and carrying a bone's turn from rest to rest would keep that difference in every pose: a slight
/// outward lean of the legs at rest stands a character with its feet far apart. So each limb's
/// bones are lined up with the source's first, each from where its parent's lining-up left it, so
/// that no bone rolls against the one before. The hand is lined up by its palm as well, and its
/// fingers go with it, with the forearm taking half of the hand's roll, as a forearm does. A
/// collarbone isn't lined up: it would drag the shoulder away from where the mesh has it.
fn limb_alignment(
    map: &[Option<usize>],
    target: &Skeleton,
    target_roles: &Roles,
    target_rest: &[Vec3],
    source_rest: &[Vec3],
) -> Vec<Quat> {
    let number = |name: &String| target.bones.iter().position(|bone| bone.name == *name);
    let chain_of = |role: &str| {
        target_roles
            .roles
            .iter()
            .find(|(r, _)| r == role)
            .map(|c| &c.1)
    };
    // The way from one target bone to another, at rest, in the target and in the source.
    let between = |from: usize, to: usize| -> Option<(Vec3, Vec3)> {
        let target = (target_rest[to] - target_rest[from]).normalize_or_zero();
        let source = (source_rest[map[to]?] - source_rest[map[from]?]).normalize_or_zero();
        (target != Vec3::ZERO && source != Vec3::ZERO).then_some((target, source))
    };
    let mut out = vec![Quat::IDENTITY; target.bones.len()];
    // Lines up a chain's bones, and gives its last bone what the one before it got.
    let mut line_up = |chain: &[String]| {
        let mut carried = Quat::IDENTITY;
        for pair in chain.windows(2) {
            let (Some(bone), Some(next)) = (number(&pair[0]), number(&pair[1])) else {
                continue;
            };
            if let Some((from, to)) = between(bone, next) {
                carried = (Quat::from_rotation_arc(carried * from, to) * carried).normalize();
                out[bone] = carried;
            }
        }
        if let Some(bone) = chain.last().and_then(&number) {
            out[bone] = carried;
        }
    };
    for side in ["l", "r"] {
        if let Some(arm) = chain_of(&format!("arm_{side}")) {
            // The upper arm, the lower arm and the hand, with or without a collarbone before them.
            if let Some(first) = arm.len().checked_sub(3) {
                line_up(&arm[first..]);
            }
        }
        if let Some(leg) = chain_of(&format!("leg_{side}")) {
            line_up(leg);
        }
    }
    for side in ["l", "r"] {
        let Some(arm) = chain_of(&format!("arm_{side}")) else {
            continue;
        };
        let (Some(hand), Some(lower)) = (
            arm.last().and_then(&number),
            arm.len().checked_sub(2).and_then(|i| number(&arm[i])),
        ) else {
            continue;
        };
        let start = |finger: &str| {
            chain_of(&format!("{finger}_{side}"))
                .and_then(|chain| chain.first())
                .and_then(&number)
        };
        let palm = (|| {
            let (along, along_to) = between(hand, start("middle")?)?;
            let (across, across_to) = between(start("index")?, start("pinky")?)?;
            let before = out[hand];
            Some(frame_between(before * along, before * across, along_to, across_to) * before)
        })();
        if let (Some(whole), Some((_, axis))) = (palm, between(lower, hand)) {
            let roll = twist_about(whole * out[hand].inverse(), axis);
            out[lower] = (Quat::from_axis_angle(axis, roll * 0.5) * out[lower]).normalize();
            out[hand] = whole;
        }
        let fingers = ["thumb", "index", "middle", "ring", "pinky"];
        let chains = fingers
            .iter()
            .filter_map(|f| chain_of(&format!("{f}_{side}")));
        for name in chains.flatten() {
            if let Some(bone) = number(name) {
                out[bone] = out[hand];
            }
        }
    }
    out
}

/// How far lining up the legs moved the feet down at rest, which the pelvis rises by so that they
/// stay on the ground.
fn leg_lift(target: &Skeleton, roles: &Roles, rest: &[Vec3], aligned: &[Quat]) -> f32 {
    let number = |name: &String| target.bones.iter().position(|bone| bone.name == *name);
    let mut lifts = Vec::new();
    for leg in roles
        .roles
        .iter()
        .filter(|(role, _)| role == "leg_l" || role == "leg_r")
        .map(|(_, chain)| chain)
    {
        let bones: Option<Vec<usize>> = leg.iter().map(number).collect();
        let Some(bones) = bones.filter(|bones| !bones.is_empty()) else {
            continue;
        };
        let foot = bones.windows(2).fold(rest[bones[0]], |at, pair| {
            at + aligned[pair[0]] * (rest[pair[1]] - rest[pair[0]])
        });
        lifts.push(rest[bones[bones.len() - 1]].y - foot.y);
    }
    if lifts.is_empty() {
        0.0
    } else {
        lifts.iter().sum::<f32>() / lifts.len() as f32
    }
}

/// Carries a clip from one rig onto another, by the two rigs' role files.
pub fn retarget(
    source: &Skeleton,
    source_roles: &Roles,
    target: &Skeleton,
    target_roles: &Roles,
    frames: &Frames,
) -> Frames {
    let map = role_map(source, source_roles, target, target_roles);
    let scale = match (
        leg_length(target, target_roles),
        leg_length(source, source_roles),
    ) {
        (Some(t), Some(s)) if s > 1.0e-3 => t / s,
        _ => 1.0,
    };
    let pelvis = |skeleton: &Skeleton, roles: &Roles| {
        roles
            .roles
            .iter()
            .find(|(r, _)| r == "pelvis")
            .and_then(|(_, chain)| chain.first())
            .and_then(|name| skeleton.bones.iter().position(|bone| bone.name == *name))
    };
    let source_pelvis = pelvis(source, source_roles);
    let target_pelvis = pelvis(target, target_roles);
    let root = |skeleton: &Skeleton, roles: &Roles| {
        roles
            .roles
            .iter()
            .find(|(r, _)| r == "root")
            .and_then(|(_, chain)| chain.first())
            .and_then(|name| skeleton.bones.iter().position(|bone| bone.name == *name))
    };
    let target_root = root(target, target_roles);

    // Each bone's place at rest, in its rig's whole space.
    let world_rotations = |skeleton: &Skeleton, rotations: &[Quat]| -> Vec<Quat> {
        let mut out: Vec<Quat> = Vec::with_capacity(skeleton.bones.len());
        for (index, bone) in skeleton.bones.iter().enumerate() {
            let parent = bone.parent.map_or(Quat::IDENTITY, |p| out[p as usize]);
            out.push((parent * rotations[index]).normalize());
        }
        out
    };
    let world_positions =
        |skeleton: &Skeleton, rotations: &[Quat], translations: &[Vec3]| -> Vec<Vec3> {
            let mut rot: Vec<Quat> = Vec::with_capacity(skeleton.bones.len());
            let mut pos: Vec<Vec3> = Vec::with_capacity(skeleton.bones.len());
            for (index, bone) in skeleton.bones.iter().enumerate() {
                let (p_r, p_p) = bone.parent.map_or((Quat::IDENTITY, Vec3::ZERO), |p| {
                    (rot[p as usize], pos[p as usize])
                });
                rot.push((p_r * rotations[index]).normalize());
                pos.push(p_p + p_r * translations[index]);
            }
            pos
        };
    let source_rest = Pose::rest(source);
    let source_rest_world = world_rotations(source, &source_rest.rotations);
    let target_rest = Pose::rest(target);
    let target_rest_world = world_rotations(target, &target_rest.rotations);
    let source_rest_positions =
        world_positions(source, &source_rest.rotations, &source_rest.translations);
    let target_rest_positions =
        world_positions(target, &target_rest.rotations, &target_rest.translations);
    let aligned = limb_alignment(
        &map,
        target,
        target_roles,
        &target_rest_positions,
        &source_rest_positions,
    );
    let lift = leg_lift(target, target_roles, &target_rest_positions, &aligned);

    let mut out = Frames::default();
    for (rotations, translations) in frames.rotations.iter().zip(&frames.translations) {
        let world = world_rotations(source, rotations);
        let positions = world_positions(source, rotations, translations);
        let mut target_world: Vec<Quat> = Vec::with_capacity(target.bones.len());
        let mut target_rotations = Vec::with_capacity(target.bones.len());
        let mut target_translations = target_rest.translations.clone();
        for (bone_index, bone) in target.bones.iter().enumerate() {
            let parent = bone
                .parent
                .map_or(Quat::IDENTITY, |p| target_world[p as usize]);
            let world_rotation = match map[bone_index] {
                // What the source bone did, relative to its own rest, applied to this bone's rest.
                Some(s) => ((world[s] * source_rest_world[s].inverse())
                    * aligned[bone_index]
                    * target_rest_world[bone_index])
                    .normalize(),
                None => (parent * target_rest.rotations[bone_index]).normalize(),
            };
            target_rotations.push((parent.inverse() * world_rotation).normalize());
            target_world.push(world_rotation);
        }
        if let Some(root) = target_root {
            // The root follows the mapped root's own travel, if any.
            if let Some(s) = map[root] {
                target_translations[root] = target_rest.translations[root]
                    + (positions[s] - source_rest_positions[s]) * scale;
            }
        }
        // The pelvis travels by the ratio of leg lengths, so feet keep their place, whatever
        // its parents did on the way.
        if let (Some(source_p), Some(target_p)) = (source_pelvis, target_pelvis) {
            let moved = (positions[source_p] - source_rest_positions[source_p]) * scale;
            let wanted = target_rest_positions[target_p] + moved + Vec3::Y * lift;
            let mut parent_position = Vec3::ZERO;
            let mut parent_rotation = Quat::IDENTITY;
            let chain = chain_to_root(target, target_p);
            for &bone in chain.iter().rev().skip(1).rev() {
                parent_position += parent_rotation * target_translations[bone];
                parent_rotation = target_world[bone];
            }
            target_translations[target_p] = parent_rotation.inverse() * (wanted - parent_position);
        }
        out.rotations.push(target_rotations);
        out.translations.push(target_translations);
    }
    out
}

/// Fits frames from the rig that has them onto the character's, bones matched by name. Bones the
/// source doesn't have stand at the character's rest.
pub fn by_name(source: &Skeleton, target: &Skeleton, frames: &Frames) -> Frames {
    let map: Vec<Option<usize>> = target
        .bones
        .iter()
        .map(|bone| source.bones.iter().position(|b| b.name == bone.name))
        .collect();
    let mut out = Frames::default();
    let rest = Pose::rest(target);
    for (rotations, translations) in frames.rotations.iter().zip(&frames.translations) {
        out.rotations.push(
            map.iter()
                .enumerate()
                .map(|(bone, s)| s.map_or(rest.rotations[bone], |s| rotations[s]))
                .collect(),
        );
        out.translations.push(
            map.iter()
                .enumerate()
                .map(|(bone, s)| s.map_or(rest.translations[bone], |s| translations[s]))
                .collect(),
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A biped: root, pelvis, one leg of thigh, calf and foot straight below it.
    fn biped(leg: f32, turn: f32) -> Skeleton {
        let bone = |name: &str, parent, y: f32| Bone {
            name: name.to_owned(),
            parent,
            translation: Vec3::new(0.0, y, 0.0),
            rotation: Quat::from_rotation_y(turn),
            inverse_bind: glam::Mat4::IDENTITY,
            center: Vec3::ZERO,
            radius: 0.0,
        };
        Skeleton {
            bones: vec![
                bone("root", None, 0.0),
                bone("pelvis", Some(0), leg),
                bone("thigh", Some(1), 0.0),
                bone("calf", Some(2), -leg * 0.5),
                bone("foot", Some(3), -leg * 0.5),
            ],
        }
    }

    fn roles() -> Roles {
        Roles::parse("root root\npelvis pelvis\nleg_l thigh calf foot\n").unwrap()
    }

    fn step_frames(rig: &Skeleton) -> Frames {
        let mut frames = Frames::default();
        for f in 0..8 {
            let swing = (f as f32 / 8.0 * std::f32::consts::TAU).sin() * 0.4;
            let rest = Pose::rest(rig);
            let mut rotations = rest.rotations.clone();
            let mut translations = rest.translations.clone();
            // The root and the pelvis both travel, as a walk's root motion would.
            translations[0] += Vec3::Z * f as f32 * 0.1;
            translations[1] += Vec3::X * swing * 0.05;
            rotations[2] *= Quat::from_rotation_x(swing);
            rotations[3] *= Quat::from_rotation_x(swing * 0.5);
            frames.rotations.push(rotations);
            frames.translations.push(translations);
        }
        frames
    }

    /// A glTF of one bone and one animation that moves it up a metre in 0.2 s, keys at 0.1 and
    /// 0.3 s, as Blender writes them, its first frame a frame in.
    fn write_clip(folder: &std::path::Path) -> std::path::PathBuf {
        let mut bin = Vec::new();
        for time in [0.1f32, 0.3] {
            bin.extend_from_slice(&time.to_le_bytes());
        }
        for y in [0.0f32, 1.0] {
            for v in [0.0f32, y, 0.0] {
                bin.extend_from_slice(&v.to_le_bytes());
            }
        }
        std::fs::write(folder.join("c.bin"), &bin).unwrap();
        let json = format!(
            r#"{{
            "asset": {{"version": "2.0"}},
            "scene": 0, "scenes": [{{"nodes": [0]}}],
            "nodes": [{{"name": "root"}}],
            "animations": [{{
                "name": "up",
                "samplers": [{{"input": 0, "output": 1}}],
                "channels": [{{"sampler": 0, "target": {{"node": 0, "path": "translation"}}}}]
            }}],
            "buffers": [{{"uri": "c.bin", "byteLength": {}}}],
            "bufferViews": [
                {{"buffer": 0, "byteOffset": 0, "byteLength": 8}},
                {{"buffer": 0, "byteOffset": 8, "byteLength": 24}}
            ],
            "accessors": [
                {{"bufferView": 0, "componentType": 5126, "count": 2, "type": "SCALAR", "min": [0.1], "max": [0.3]}},
                {{"bufferView": 1, "componentType": 5126, "count": 2, "type": "VEC3"}}
            ]
        }}"#,
            bin.len()
        );
        let path = folder.join("c.gltf");
        std::fs::write(&path, json).unwrap();
        path
    }

    /// A collarbone, an arm of upper, lower and hand, and a finger, the arm held out along +X or
    /// hanging at an angle, its elbow bent.
    fn armed(hanging: bool) -> Skeleton {
        let bone = |name: &str, parent, translation: Vec3, rotation: Quat| Bone {
            name: name.to_owned(),
            parent,
            translation,
            rotation,
            inverse_bind: glam::Mat4::IDENTITY,
            center: Vec3::ZERO,
            radius: 0.0,
        };
        let down = if hanging {
            Quat::from_rotation_z(-0.9)
        } else {
            Quat::IDENTITY
        };
        let bend = if hanging {
            Quat::from_rotation_y(-0.7)
        } else {
            Quat::IDENTITY
        };
        let roll = if hanging {
            Quat::from_rotation_x(0.6)
        } else {
            Quat::IDENTITY
        };
        let along = Vec3::X * 0.3;
        Skeleton {
            bones: vec![
                bone("root", None, Vec3::ZERO, Quat::IDENTITY),
                bone("clavicle", Some(0), Vec3::Y, Quat::IDENTITY),
                bone("upper", Some(1), Vec3::X * 0.1, down),
                bone("lower", Some(2), along, bend),
                bone("hand", Some(3), along, roll),
                bone("index", Some(4), Vec3::new(0.1, 0.0, 0.03), Quat::IDENTITY),
                bone("middle", Some(4), Vec3::X * 0.1, Quat::IDENTITY),
                bone("pinky", Some(4), Vec3::new(0.1, 0.0, -0.03), Quat::IDENTITY),
            ],
        }
    }

    #[test]
    fn a_rig_resting_its_arms_differently_is_lined_up_with_the_clips() {
        let roles = Roles::parse("root root\narm_l clavicle upper lower hand\nindex_l index\nmiddle_l middle\npinky_l pinky\n").unwrap();
        let (source, target) = (armed(false), armed(true));
        // The source standing at its rest: arms straight out along +X.
        let rest = Pose::rest(&source);
        let frames = Frames {
            rotations: vec![rest.rotations],
            translations: vec![rest.translations],
        };
        let out = retarget(&source, &roles, &target, &roles, &frames);
        let mut places: Vec<(Vec3, Quat)> = Vec::new();
        for (i, bone) in target.bones.iter().enumerate() {
            let (p, r) = bone
                .parent
                .map_or((Vec3::ZERO, Quat::IDENTITY), |p| places[p as usize]);
            places.push((p + r * out.translations[0][i], r * out.rotations[0][i]));
        }
        // Each arm bone lies along the source's, and so does the hand, to its middle finger.
        for (bone, next) in [(2, 3), (3, 4), (4, 6)] {
            let along = (places[next].0 - places[bone].0).normalize();
            assert!(
                along.dot(Vec3::X) > 0.9999,
                "bone {bone} points along {along}"
            );
        }
        // The hand's palm faces as the source's does, though it rests rolled.
        let across = (places[7].0 - places[5].0).normalize();
        assert!(
            across.dot(Vec3::NEG_Z) > 0.9999,
            "the palm lies across {across}"
        );
        // The collarbone is left where it was: the shoulder stays put.
        assert!((places[2].0 - Vec3::new(0.1, 1.0, 0.0)).length() < 1.0e-5);
    }

    #[test]
    fn a_rig_with_arms_onto_itself_comes_back_unchanged() {
        let roles = Roles::parse("root root\narm_l clavicle upper lower hand\nindex_l index\nmiddle_l middle\npinky_l pinky\n").unwrap();
        let rig = armed(true);
        let rest = Pose::rest(&rig);
        let frames = Frames {
            rotations: vec![rest.rotations.clone()],
            translations: vec![rest.translations.clone()],
        };
        let out = retarget(&rig, &roles, &rig, &roles, &frames);
        for (bone, (a, b)) in out.rotations[0].iter().zip(&rest.rotations).enumerate() {
            assert!(
                a.dot(*b).abs() > 0.99999,
                "bone {bone} turned: {a} against {b}"
            );
        }
    }

    #[test]
    fn a_clip_file_is_sampled_from_its_first_key() {
        let folder = std::env::temp_dir().join(format!("solstice-clip-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let once = read_clips(write_clip(&folder), |_| false).unwrap();
        let looping = read_clips(write_clip(&folder), |_| true).unwrap();
        std::fs::remove_dir_all(&folder).ok();
        assert_eq!(once.rig.bones[0].name, "root");
        let (name, frames) = &once.clips[0];
        assert_eq!(name, "up");
        // 0.2 s at 30 Hz is 6 spans: 7 samples, or 6 if it loops.
        assert_eq!(frames.translations.len(), 7);
        assert_eq!(looping.clips[0].1.translations.len(), 6);
        assert!(frames.translations[0][0].y.abs() < 1.0e-5);
        assert!((frames.translations[3][0].y - 0.5).abs() < 1.0e-3);
        assert!((frames.translations[6][0].y - 1.0).abs() < 1.0e-3);
    }

    #[test]
    fn a_clip_retargeted_onto_its_own_rig_comes_back_unchanged() {
        let rig = biped(1.0, 0.0);
        let frames = step_frames(&rig);
        let out = retarget(&rig, &roles(), &rig, &roles(), &frames);
        for (a, b) in frames
            .rotations
            .iter()
            .flatten()
            .zip(out.rotations.iter().flatten())
        {
            assert!(a.dot(*b).abs() > 0.9999, "{a:?} against {b:?}");
        }
        for (a, b) in frames
            .translations
            .iter()
            .flatten()
            .zip(out.translations.iter().flatten())
        {
            assert!(a.distance(*b) < 1.0e-4, "{a} against {b}");
        }
    }

    #[test]
    fn a_different_rest_pose_cancels_out() {
        let source = biped(1.0, 0.7);
        let target = biped(1.0, -0.4);
        let frames = step_frames(&source);
        let out = retarget(&source, &roles(), &target, &roles(), &frames);
        // The world rotation change of every mapped bone is the same as the source's.
        let change = |skeleton: &Skeleton, rotations: &[Quat]| {
            let mut world: Vec<Quat> = Vec::new();
            for (index, bone) in skeleton.bones.iter().enumerate() {
                let parent = bone.parent.map_or(Quat::IDENTITY, |p| world[p as usize]);
                world.push(parent * rotations[index]);
            }
            world
        };
        let rest_s = change(&source, &Pose::rest(&source).rotations);
        let rest_t = change(&target, &Pose::rest(&target).rotations);
        let now_s = change(&source, &frames.rotations[2]);
        let now_t = change(&target, &out.rotations[2]);
        for bone in 0..source.bones.len() {
            let a = now_s[bone] * rest_s[bone].inverse();
            let b = now_t[bone] * rest_t[bone].inverse();
            assert!(a.dot(b).abs() > 0.9999, "bone {bone}");
        }
    }

    /// A pelvis gliding along with one foot planted: on a rig twice as long the foot must stay
    /// planted too, when the pelvis's travel is scaled by leg length.
    #[test]
    fn a_planted_foot_stays_planted_on_a_longer_rig() {
        let source = biped(1.0, 0.0);
        let target = biped(2.0, 0.0);
        let mut frames = Frames::default();
        for f in 0..8 {
            let rest = Pose::rest(&source);
            let mut rotations = rest.rotations.clone();
            let mut translations = rest.translations.clone();
            let travel = f as f32 * 0.05;
            let forward = Vec3::Z * travel;
            // The pelvis has moved forward and is above a foot left behind, so the leg leans back.
            translations[1] += forward;
            let lean = travel.asin();
            rotations[2] = Quat::from_rotation_x(lean);
            frames.rotations.push(rotations);
            frames.translations.push(translations);
        }
        let out = retarget(&source, &roles(), &target, &roles(), &frames);
        let sideways = |a: Vec3, b: Vec3| ((a.x - b.x).powi(2) + (a.z - b.z).powi(2)).sqrt();
        let foot_of = |skeleton: &Skeleton,
                       frame: usize,
                       rotations: &[Vec<Quat>],
                       translations: &[Vec<Vec3>]| {
            let mut rot: Vec<Quat> = Vec::new();
            let mut pos: Vec<Vec3> = Vec::new();
            for (index, bone) in skeleton.bones.iter().enumerate() {
                let (r, p) = bone.parent.map_or((Quat::IDENTITY, Vec3::ZERO), |p| {
                    (rot[p as usize], pos[p as usize])
                });
                pos.push(p + r * translations[frame][index]);
                rot.push(r * rotations[frame][index]);
            }
            pos[4]
        };
        let source_slide = sideways(
            foot_of(&source, 7, &frames.rotations, &frames.translations),
            foot_of(&source, 0, &frames.rotations, &frames.translations),
        );
        let target_slide = sideways(
            foot_of(&target, 7, &out.rotations, &out.translations),
            foot_of(&target, 0, &out.rotations, &out.translations),
        );
        // Only the foot's small offset from the rig's own ratio is left over.
        assert!(
            source_slide < 0.02,
            "the source's foot slides {source_slide}"
        );
        assert!(
            target_slide < 0.04,
            "the retargeted foot slides {target_slide}"
        );
    }

    #[test]
    fn bones_a_source_lacks_stand_at_rest() {
        let source = biped(1.0, 0.0);
        let mut target = biped(1.0, 0.0);
        target.bones.push(Bone {
            name: "tail".to_owned(),
            parent: Some(1),
            translation: Vec3::X,
            rotation: Quat::IDENTITY,
            ..Default::default()
        });
        let frames = step_frames(&source);
        let out = by_name(&source, &target, &frames);
        assert_eq!(out.rotations[3].len(), 6);
        assert_eq!(out.translations[3][5], Vec3::X);
    }
}
