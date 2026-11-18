//! The Universal Animation Library's clips on the real character. Both are kept out of git, so
//! this says so and passes without them: `python -I scripts/fetch-ual.py --zips FOLDER`, then
//! `tools/blender/make_character.py`.

use glam::{Quat, Vec3};
use solstice_anim::Frames;
use solstice_asset::{Skeleton, CLIP_HZ};
use solstice_import::animation::{read_clips, retarget, ClipFile};
use solstice_import::character::Roles;
use std::path::{Path, PathBuf};

fn repo(path: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(path)
}

fn roles(file: &str) -> Roles {
    Roles::parse(&std::fs::read_to_string(repo(file)).unwrap()).unwrap()
}

/// The library's first file and the keeper's rig, if both are here.
fn setup() -> Option<(ClipFile, Skeleton)> {
    let (ual, keeper) = (
        repo("target/ual/ual1/UAL1_Standard.glb"),
        repo("target/characters/keeper.glb"),
    );
    if !ual.exists() || !keeper.exists() {
        eprintln!("the library or the keeper isn't here: skipping");
        return None;
    }
    let library = read_clips(ual, |name| name.ends_with("_Loop")).unwrap();
    let rig = read_clips(keeper, |_| true).unwrap().rig;
    Some((library, rig))
}

fn bone(rig: &Skeleton, name: &str) -> usize {
    rig.bones.iter().position(|b| b.name == name).unwrap()
}

/// Every bone's place in the rig's whole space, in one frame.
fn world(rig: &Skeleton, rotations: &[Quat], translations: &[Vec3]) -> Vec<(Vec3, Quat)> {
    let mut out: Vec<(Vec3, Quat)> = Vec::with_capacity(rig.bones.len());
    for (i, bone) in rig.bones.iter().enumerate() {
        let (p, r) = bone
            .parent
            .map_or((Vec3::ZERO, Quat::IDENTITY), |p| out[p as usize]);
        out.push((p + r * translations[i], (r * rotations[i]).normalize()));
    }
    out
}

fn keeper_clip(library: &ClipFile, keeper: &Skeleton, name: &str) -> Frames {
    let (_, frames) = library.clips.iter().find(|(n, _)| n == name).unwrap();
    retarget(
        &library.rig,
        &roles("characters/rigs/ual.roles"),
        keeper,
        &roles("characters/rigs/game_engine.roles"),
        frames,
    )
}

/// Each frame's height and place along the walking direction of a bone.
fn track(rig: &Skeleton, frames: &Frames, name: &str) -> Vec<Vec3> {
    let at = bone(rig, name);
    frames
        .rotations
        .iter()
        .zip(&frames.translations)
        .map(|(r, t)| world(rig, r, t)[at].0)
        .collect()
}

/// What a walk does with its feet: how fast the foot on the ground goes back, which is the speed
/// the clip wants to be walked at, and how far it strays from that while it bears weight.
fn stance(rig: &Skeleton, frames: &Frames, name: &str) -> (f32, f32, usize) {
    let ball = track(rig, frames, name);
    let n = ball.len();
    let lowest = ball.iter().map(|p| p.y).fold(f32::MAX, f32::min);
    // On the ground: within 3 cm of the lowest it gets.
    let planted: Vec<usize> = (0..n).filter(|&i| ball[i].y < lowest + 0.03).collect();
    let speeds: Vec<f32> = planted
        .iter()
        .filter(|&&i| ball[(i + 1) % n].y < lowest + 0.03)
        .map(|&i| (ball[i].z - ball[(i + 1) % n].z) * CLIP_HZ)
        .collect();
    let mut sorted = speeds.clone();
    sorted.sort_by(f32::total_cmp);
    let speed = sorted[sorted.len() / 2];
    let slide = speeds
        .iter()
        .map(|s| (s - speed).abs() / CLIP_HZ)
        .sum::<f32>();
    (speed, slide, planted.len())
}

#[test]
fn a_retargeted_walk_wants_its_own_speed_and_its_feet_hardly_slide() {
    let Some((library, keeper)) = setup() else {
        return;
    };
    let walk = keeper_clip(&library, &keeper, "Walk_Loop");
    for foot in ["ball_l", "ball_r"] {
        let (speed, slide, frames) = stance(&keeper, &walk, foot);
        assert!((speed - 1.0).abs() < 0.1, "{foot} wants {speed} m/s");
        assert!(frames > 12, "{foot} is on the ground for {frames} frames");
        // Played at the 1.0 m/s the demo walks at, what the foot does over its stance.
        let slip = (speed - 1.0).abs() * frames as f32 / CLIP_HZ;
        assert!(slip < 0.05, "{foot} slips {slip} m against the ground");
        assert!(slide < 0.1, "{foot} slides {slide} m while it bears weight");
    }
}

#[test]
fn an_idle_keeps_its_feet_where_they_are() {
    let Some((library, keeper)) = setup() else {
        return;
    };
    let idle = keeper_clip(&library, &keeper, "Idle_Loop");
    for foot in ["ball_l", "ball_r"] {
        let track = track(&keeper, &idle, foot);
        let moved = track
            .iter()
            .map(|p| p.distance(track[0]))
            .fold(0.0, f32::max);
        assert!(moved < 0.02, "{foot} moves {moved} m standing still");
    }
}

#[test]
fn feet_stay_on_the_ground_the_character_stands_on() {
    let Some((library, keeper)) = setup() else {
        return;
    };
    let rest = solstice_anim::Pose::rest(&keeper);
    let rest_ball = world(&keeper, &rest.rotations, &rest.translations)[bone(&keeper, "ball_l")].0;
    for clip in ["Idle_Loop", "Walk_Loop"] {
        let frames = keeper_clip(&library, &keeper, clip);
        for foot in ["ball_l", "ball_r"] {
            let lowest = track(&keeper, &frames, foot)
                .iter()
                .map(|p| p.y)
                .fold(f32::MAX, f32::min);
            assert!(
                lowest > rest_ball.y - 0.01 && lowest < rest_ball.y + 0.04,
                "in {clip} {foot} gets down to {lowest}, against {} at rest",
                rest_ball.y
            );
        }
    }
}

/// The library rests in a T and the keeper with its arms down and elbows bent. Its T-pose clip
/// should stand the keeper in a T.
#[test]
fn the_librarys_t_pose_stands_the_character_in_a_t() {
    let Some((library, keeper)) = setup() else {
        return;
    };
    let frames = keeper_clip(&library, &keeper, "A_TPose");
    let place = world(&keeper, &frames.rotations[0], &frames.translations[0]);
    let (_, source_frames) = library.clips.iter().find(|(n, _)| n == "A_TPose").unwrap();
    let source = world(
        &library.rig,
        &source_frames.rotations[0],
        &source_frames.translations[0],
    );
    for side in ["l", "r"] {
        for (a, b) in [("upperarm", "lowerarm"), ("lowerarm", "hand")] {
            let (from, to) = (format!("{a}_{side}"), format!("{b}_{side}"));
            let on = |rig: &Skeleton, p: &[(Vec3, Quat)]| {
                (p[bone(rig, &to)].0 - p[bone(rig, &from)].0).normalize()
            };
            let off = on(&keeper, &place)
                .dot(on(&library.rig, &source))
                .clamp(-1.0, 1.0)
                .acos()
                .to_degrees();
            assert!(off < 3.0, "{from} is {off} degrees off the library's");
        }
        // Its palm faces the way the library's does.
        let palm = |rig: &Skeleton, p: &[(Vec3, Quat)]| {
            let at = |name: String| p[bone(rig, &name)].0;
            let along = at(format!("middle_01_{side}")) - at(format!("hand_{side}"));
            let across = at(format!("pinky_01_{side}")) - at(format!("index_01_{side}"));
            along.cross(across).normalize()
        };
        let off = palm(&keeper, &place)
            .dot(palm(&library.rig, &source))
            .clamp(-1.0, 1.0)
            .acos()
            .to_degrees();
        assert!(
            off < 0.2,
            "the {side} palm is {off} degrees off the library's"
        );
    }
}

/// Standing still, the feet are about as far apart as the library's are, for the hips they have.
#[test]
fn an_idle_stands_with_its_feet_under_its_hips() {
    let Some((library, keeper)) = setup() else {
        return;
    };
    let idle = keeper_clip(&library, &keeper, "Idle_Loop");
    let (_, source) = library
        .clips
        .iter()
        .find(|(n, _)| n == "Idle_Loop")
        .unwrap();
    let wide = |rig: &Skeleton, r: &[Quat], t: &[Vec3]| {
        let at = world(rig, r, t);
        (at[bone(rig, "ball_l")].0.x - at[bone(rig, "ball_r")].0.x).abs()
    };
    let hips = |rig: &Skeleton| {
        let rest = solstice_anim::Pose::rest(rig);
        let at = world(rig, &rest.rotations, &rest.translations);
        (at[bone(rig, "thigh_l")].0.x - at[bone(rig, "thigh_r")].0.x).abs()
    };
    let theirs = wide(&library.rig, &source.rotations[0], &source.translations[0]);
    let ours = wide(&keeper, &idle.rotations[0], &idle.translations[0]);
    let allowed = theirs + (hips(&keeper) - hips(&library.rig)) + 0.03;
    assert!(ours < allowed, "feet {ours} m apart, allowed {allowed}");
}
