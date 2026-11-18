include!("../../common.rs");
use glam::{Mat4, Quat, Vec3};
use solstice_anim::{compress, AnimGraph, Animator, ClipSet, Frames, Pose};
use solstice_asset::{Bone, Skeleton};

/// A skeleton of `bones` bones in a chain, as long as the keeper's.
fn skeleton(bones: usize) -> Skeleton {
    Skeleton {
        bones: (0..bones)
            .map(|i| Bone {
                name: format!("bone{i}"),
                parent: i.checked_sub(1).map(|p| p as u32),
                translation: Vec3::Y * 0.1,
                rotation: Quat::IDENTITY,
                inverse_bind: Mat4::IDENTITY,
                center: Vec3::ZERO,
                radius: 0.1,
            })
            .collect(),
    }
}

/// A clip in which every bone moves every frame: the worst case.
fn clip(name: &str, bones: usize, frames: usize, rate: f32) -> solstice_asset::Clip {
    let data = Frames {
        rotations: (0..frames)
            .map(|f| {
                (0..bones)
                    .map(|b| Quat::from_rotation_x((f as f32 * rate + b as f32).sin() * 0.3))
                    .collect()
            })
            .collect(),
        translations: (0..frames)
            .map(|f| vec![Vec3::Y * 0.1 + Vec3::X * (f as f32 * 0.1).sin() * 0.01; bones])
            .collect(),
    };
    compress(name, 0, &data, true)
}

pub fn bench() -> String {
    const GRAPH: &str = "
        clip idle I
        clip walk W
        clip run R
        param speed 0
        state move blend1d speed 0 idle 1.4 walk 4 run
        start move
    ";
    let mut out = String::new();
    let bones = 53;
    let skeleton = skeleton(bones);
    let rest = Pose::rest(&skeleton);
    let clips = ClipSet::new(vec![
        ("idle".to_owned(), clip("idle", bones, 90, 0.1)),
        ("walk".to_owned(), clip("walk", bones, 34, 0.2)),
        ("run".to_owned(), clip("run", bones, 22, 0.3)),
    ]);
    for characters in [1usize, 20, 100] {
        let mut animators: Vec<Animator> = (0..characters)
            .map(|i| {
                let mut a = Animator::new(AnimGraph::parse(GRAPH).unwrap(), bones);
                a.set("speed", 0.7 + i as f32 * 0.02);
                a
            })
            .collect();
        let mut pose = Pose::with_bones(bones);
        let mut model = Vec::new();
        let frames = 300;
        let start = now_ms();
        for _ in 0..frames {
            for a in &mut animators {
                a.advance(1.0 / 60.0, &clips);
                a.evaluate(&clips, &rest, &mut pose);
                pose.to_model(&skeleton, &mut model);
                std::hint::black_box(&model);
            }
        }
        let ms = (now_ms() - start) / frames as f64;
        out += &format!("{characters} characters of {bones} bones, each blending two clips: {ms:.3} ms a frame\n");
    }
    out
}
