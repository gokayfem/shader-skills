//! Clips: quantised, at 30 Hz, as in the Animation Compression Library.
//!
//! A clip's data lives in [`solstice_asset::Clip`], so that scene files carry it. A clip is made
//! from whole frames by [`compress`], and sampled here by [`sample`]. Rotations are stored as
//! their three smallest components, in 48 bits, and translations relative to their own range;
//! a bone that doesn't move in a clip keeps one sample.

use crate::pose::Pose;
use glam::{Quat, Vec3};
use solstice_asset::{BoneTrack, Clip, CLIP_HZ};

/// A clip as frames of every bone, at [`CLIP_HZ`], before compression.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Frames {
    /// Per frame, per bone.
    pub rotations: Vec<Vec<Quat>>,
    pub translations: Vec<Vec<Vec3>>,
}

/// Rotations differing by less than this, in the quaternion's own units (half the angle), are one
/// sample: about a hundredth of a degree.
const STILL_ROTATION: f32 = 1.0e-4;
/// Translations closer than this are one sample: a tenth of a millimetre.
const STILL_TRANSLATION: f32 = 1.0e-4;

/// Encodes a unit quaternion as its three smallest components: 2 bits for which was dropped, and
/// 14, 16 and 16 bits for the rest. The fourth is rebuilt, positive, from the others.
pub fn encode_rotation(q: Quat) -> [u16; 3] {
    let values = q.to_array();
    let mut largest = 0;
    for i in 1..4 {
        if values[i].abs() > values[largest].abs() {
            largest = i;
        }
    }
    let sign = values[largest].signum();
    let mut parts = [0.0f32; 3];
    let mut at = 0;
    for (i, value) in values.iter().enumerate() {
        if i != largest {
            parts[at] = value * sign;
            at += 1;
        }
    }
    let range = std::f32::consts::FRAC_1_SQRT_2;
    let quantise = |v: f32, max: f32| ((v / range * 0.5 + 0.5) * max).round().clamp(0.0, max);
    let first = quantise(parts[0], 16383.0) as u16;
    [
        first | (largest as u16) << 14,
        quantise(parts[1], 65535.0) as u16,
        quantise(parts[2], 65535.0) as u16,
    ]
}

/// Inverse of [`encode_rotation`].
pub fn decode_rotation(packed: [u16; 3]) -> Quat {
    let range = std::f32::consts::FRAC_1_SQRT_2;
    let unquantise = |v: u16, max: f32| (f32::from(v) / max - 0.5) / 0.5 * range;
    let largest = usize::from(packed[0] >> 14);
    let parts = [
        unquantise(packed[0] & 0x3fff, 16383.0),
        unquantise(packed[1], 65535.0),
        unquantise(packed[2], 65535.0),
    ];
    let w = (1.0 - parts[0] * parts[0] - parts[1] * parts[1] - parts[2] * parts[2])
        .max(0.0)
        .sqrt();
    let mut values = [0.0f32; 4];
    let mut at = 0;
    for (i, slot) in values.iter_mut().enumerate() {
        if i == largest {
            *slot = w;
        } else {
            *slot = parts[at];
            at += 1;
        }
    }
    Quat::from_array(values).normalize()
}

fn encode_translation(v: Vec3, min: Vec3, max: Vec3) -> [u16; 3] {
    let t = (v - min) / (max - min).max(Vec3::splat(1.0e-9));
    (t * 65535.0)
        .round()
        .clamp(Vec3::ZERO, Vec3::splat(65535.0))
        .to_array()
        .map(|c| c as u16)
}

fn decode_translation(packed: [u16; 3], min: Vec3, max: Vec3) -> Vec3 {
    let t = Vec3::new(
        f32::from(packed[0]),
        f32::from(packed[1]),
        f32::from(packed[2]),
    ) / 65535.0;
    min + t * (max - min)
}

/// Quantises whole frames into a clip. Frames must not be empty.
pub fn compress(name: &str, skeleton: u32, frames: &Frames, loops: bool) -> Clip {
    let count = frames.rotations.len();
    assert!(count > 0, "a clip has at least one frame");
    let bones = frames.rotations[0].len();
    let mut clip = Clip {
        name: name.to_owned(),
        skeleton,
        frames: count as u32,
        loops,
        ..Default::default()
    };
    for bone in 0..bones {
        let rotations: Vec<Quat> = frames.rotations.iter().map(|f| f[bone]).collect();
        let translations: Vec<Vec3> = frames.translations.iter().map(|f| f[bone]).collect();
        let still_rotation = rotations.iter().all(|q| {
            let q = if q.dot(rotations[0]) < 0.0 { -*q } else { *q };
            (q - rotations[0]).length() < STILL_ROTATION
        });
        let (min, max) = translations
            .iter()
            .fold((Vec3::MAX, Vec3::MIN), |(lo, hi), v| {
                (lo.min(*v), hi.max(*v))
            });
        let still_translation = (max - min).max_element() < STILL_TRANSLATION;

        let rotations_first = clip.rotations.len() as u32;
        let kept = if still_rotation { 1 } else { rotations.len() };
        clip.rotations
            .extend(rotations[..kept].iter().map(|&q| encode_rotation(q)));
        let translations_first = clip.translations.len() as u32;
        let (kept_t, min, max) = if still_translation {
            (1, translations[0], translations[0])
        } else {
            (translations.len(), min, max)
        };
        clip.translations.extend(
            translations[..kept_t]
                .iter()
                .map(|&v| encode_translation(v, min, max)),
        );
        clip.tracks.push(BoneTrack {
            rotations_first,
            rotations_frames: kept as u32,
            translations_first,
            translations_frames: kept_t as u32,
            translation_min: min.to_array(),
            translation_max: max.to_array(),
        });
    }
    clip
}

/// How long a clip lasts, in seconds. A looping clip wraps from its last frame to its first, so
/// that frame's span counts too.
pub fn duration(clip: &Clip) -> f32 {
    let spans = if clip.loops {
        clip.frames
    } else {
        clip.frames.saturating_sub(1)
    };
    spans as f32 / CLIP_HZ
}

/// The two frames either side of `time`, and how far between them it lies. A looping clip wraps
/// around; one that doesn't holds its ends.
fn frame_span(clip: &Clip, time: f32) -> (usize, usize, f32) {
    let frames = clip.frames as usize;
    if frames <= 1 {
        return (0, 0, 0.0);
    }
    let length = duration(clip);
    let t = if clip.loops {
        time.rem_euclid(length)
    } else {
        time.clamp(0.0, length)
    };
    let position = t * CLIP_HZ;
    let first = position.floor() as usize;
    let fraction = position - first as f32;
    if clip.loops {
        (first % frames, (first + 1) % frames, fraction)
    } else {
        (first.min(frames - 1), (first + 1).min(frames - 1), fraction)
    }
}

/// The pose of `clip` at `time` seconds. `out` must be the size of its skeleton.
pub fn sample(clip: &Clip, time: f32, out: &mut Pose) {
    let (a, b, t) = frame_span(clip, time);
    for (bone, track) in clip.tracks.iter().enumerate() {
        let frames = track.rotations_frames as usize;
        let start = track.rotations_first as usize;
        out.rotations[bone] = if frames == 1 {
            decode_rotation(clip.rotations[start])
        } else {
            let from = decode_rotation(clip.rotations[start + a]);
            let to = decode_rotation(clip.rotations[start + b]);
            let to = if from.dot(to) < 0.0 { -to } else { to };
            from.lerp(to, t).normalize()
        };
        let frames = track.translations_frames as usize;
        let start = track.translations_first as usize;
        let (min, max) = (
            Vec3::from_array(track.translation_min),
            Vec3::from_array(track.translation_max),
        );
        out.translations[bone] = if frames == 1 {
            decode_translation(clip.translations[start], min, max)
        } else {
            let from = decode_translation(clip.translations[start + a], min, max);
            let to = decode_translation(clip.translations[start + b], min, max);
            from.lerp(to, t)
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_quaternion_survives_the_three_smallest_encoding() {
        for (axis, angle) in [
            (Vec3::X, 0.7),
            (Vec3::new(0.3, -0.8, 0.5).normalize(), 2.9),
            (Vec3::Z, -1.2),
            (Vec3::Y, 3.1),
        ] {
            let q = Quat::from_axis_angle(axis, angle);
            let back = decode_rotation(encode_rotation(q));
            assert!(q.dot(back).abs() > 0.99999, "{q:?} came back as {back:?}");
        }
    }

    fn two_bone_frames() -> Frames {
        let frames = 12;
        Frames {
            rotations: (0..frames)
                .map(|i| {
                    vec![
                        Quat::IDENTITY,
                        Quat::from_rotation_x(i as f32 / frames as f32 * std::f32::consts::TAU),
                    ]
                })
                .collect(),
            translations: (0..frames)
                .map(|i| {
                    vec![
                        Vec3::new(0.0, 1.0, 0.0),
                        Vec3::new(0.0, i as f32 * 0.01, 0.0),
                    ]
                })
                .collect(),
        }
    }

    #[test]
    fn a_clip_gives_back_its_frames_and_keeps_one_sample_of_what_is_still() {
        let frames = two_bone_frames();
        let clip = compress("spin", 0, &frames, false);
        assert_eq!(clip.tracks[0].rotations_frames, 1);
        assert_eq!(clip.tracks[0].translations_frames, 1);
        assert_eq!(clip.tracks[1].rotations_frames, 12);
        let mut pose = Pose::with_bones(2);
        for frame in [0usize, 3, 7, 11] {
            sample(&clip, frame as f32 / CLIP_HZ, &mut pose);
            assert!(
                pose.rotations[1].dot(frames.rotations[frame][1]).abs() > 0.9999,
                "frame {frame}"
            );
            let want = frames.translations[frame][1];
            assert!(
                pose.translations[1].distance(want) < 1.0e-3,
                "frame {frame}: {} against {want}",
                pose.translations[1]
            );
        }
        assert!(pose.translations[0].distance(Vec3::Y) < 1.0e-4);
    }

    #[test]
    fn sampling_blends_between_frames() {
        let frames = two_bone_frames();
        let clip = compress("spin", 0, &frames, false);
        let mut pose = Pose::with_bones(2);
        sample(&clip, 0.5 / CLIP_HZ, &mut pose);
        assert!((pose.translations[1].y - 0.005).abs() < 1.0e-3);
    }

    #[test]
    fn a_looping_clip_wraps_and_one_that_doesnt_holds_its_ends() {
        let frames = two_bone_frames();
        let looping = compress("loop", 0, &frames, true);
        let once = compress("once", 0, &frames, false);
        assert!((duration(&looping) - 12.0 / CLIP_HZ).abs() < 1.0e-6);
        assert!((duration(&once) - 11.0 / CLIP_HZ).abs() < 1.0e-6);
        let mut a = Pose::with_bones(2);
        let mut b = Pose::with_bones(2);
        sample(&looping, 12.5 / CLIP_HZ, &mut a);
        sample(&looping, 0.5 / CLIP_HZ, &mut b);
        assert!(a.rotations[1].dot(b.rotations[1]) > 0.99999, "no wrap");
        sample(&once, 100.0, &mut a);
        sample(&once, 11.0 / CLIP_HZ, &mut b);
        assert!(
            a.rotations[1].dot(b.rotations[1]) > 0.99999,
            "doesn't hold its end"
        );

        // Between the last frame of a loop and its first again.
        sample(&looping, 11.5 / CLIP_HZ, &mut a);
        let half = Quat::from_rotation_x(11.5 / 12.0 * std::f32::consts::TAU);
        assert!(
            a.rotations[1].dot(half).abs() > 0.999,
            "{:?}",
            a.rotations[1]
        );
    }

    #[test]
    fn a_slow_bone_is_kept_as_moving() {
        let frames = Frames {
            rotations: (0..30)
                .map(|f| vec![Quat::from_rotation_x((f as f32 * 0.2).sin() * 0.02)])
                .collect(),
            translations: vec![vec![Vec3::ZERO]; 30],
        };
        let clip = compress("breath", 0, &frames, true);
        assert_eq!(clip.tracks[0].rotations_frames, 30);
    }
}
