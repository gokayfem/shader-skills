//! A demonstration of things that move: a group of instances rocking together on the swell, as
//! a ship riding at anchor does. Plain Rust drives it for now; gameplay and the sea's own waves
//! take over later (docs/gameplay.md).

use glam::{Mat4, Quat, Vec3};
use solstice_render::Renderer;

/// Instances that bob together about a pivot.
pub struct Bobbing {
    pivot: Vec3,
    /// How far it rocks: 1 as on a gentle swell, 0 not at all (movable, but still).
    amplitude: f32,
    /// Each instance, with the transform it had when the scene loaded.
    instances: Vec<(u32, Mat4)>,
    seconds: f32,
}

impl Bobbing {
    /// Takes every instance whose bounds are centred within `radius` metres of `(x, z)` across
    /// the ground, at any height, and makes them movable, keeping them in the GI voxels. They
    /// rock about `(x, height, z)`, by `amplitude` times a gentle swell's motion.
    pub fn around(
        renderer: &mut Renderer,
        [x, z]: [f32; 2],
        radius: f32,
        height: f32,
        amplitude: f32,
    ) -> Self {
        let instances: Vec<(u32, Mat4)> = (0..renderer.instance_count())
            .filter(|&i| {
                renderer
                    .instance_bounds(i)
                    .is_some_and(|(center, _)| (center.x - x).hypot(center.z - z) <= radius)
            })
            .filter_map(|i| Some((i, renderer.instance_transform(i)?)))
            .collect();
        // They move only a little, so they keep their place in the GI voxels: their shade, the
        // light they bounce, and their reflection in the water.
        for &(i, _) in &instances {
            renderer.set_instance_movable(i, true);
            renderer.set_instance_stays_in_gi(i, true);
        }
        Self {
            pivot: Vec3::new(x, height, z),
            amplitude,
            instances,
            seconds: 0.0,
        }
    }

    /// How many instances move.
    pub fn len(&self) -> usize {
        self.instances.len()
    }

    pub fn is_empty(&self) -> bool {
        self.instances.is_empty()
    }

    /// Moves them on by `seconds`.
    pub fn advance(&mut self, renderer: &mut Renderer, seconds: f32) {
        self.seconds += seconds;
        let motion = rocking(self.pivot, self.seconds, self.amplitude);
        for &(i, start) in &self.instances {
            renderer.move_instance(i, motion * start);
        }
    }
}

/// How a hull rides a gentle swell at `seconds`, scaled by `amplitude`: rising and falling,
/// rolling and pitching a little, each with its own period so the motion never quite repeats.
/// It starts level, where the scene put it.
pub fn rocking(pivot: Vec3, seconds: f32, amplitude: f32) -> Mat4 {
    use std::f32::consts::TAU;
    let heave = amplitude * 0.25 * (seconds * TAU / 7.0).sin();
    let roll = amplitude * 3.0_f32.to_radians() * (seconds * TAU / 9.0).sin();
    let pitch = amplitude * 1.5_f32.to_radians() * (seconds * TAU / 6.0).sin();
    Mat4::from_translation(pivot + Vec3::Y * heave)
        * Mat4::from_quat(Quat::from_rotation_z(roll) * Quat::from_rotation_x(pitch))
        * Mat4::from_translation(-pivot)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pivot_only_rises_and_falls() {
        let pivot = Vec3::new(140.0, -0.6, -8.0);
        for seconds in [0.0, 1.3, 4.0, 11.0] {
            let moved = rocking(pivot, seconds, 1.0).transform_point3(pivot);
            assert!((moved.x - pivot.x).abs() < 1.0e-3 && (moved.z - pivot.z).abs() < 1.0e-3);
            assert!((moved.y - pivot.y).abs() <= 0.25 + 1.0e-3);
        }
    }

    #[test]
    fn the_hull_tilts_but_only_a_little() {
        let pivot = Vec3::ZERO;
        let up = |s| rocking(pivot, s, 1.0).transform_vector3(Vec3::Y);
        let tilts: Vec<f32> = (0..40)
            .map(|i| up(i as f32 * 0.5).angle_between(Vec3::Y))
            .collect();
        assert!(
            tilts.iter().any(|&t| t > 1.0_f32.to_radians()),
            "it never rocks"
        );
        assert!(
            tilts.iter().all(|&t| t < 4.0_f32.to_radians()),
            "it rocks too far"
        );
    }

    #[test]
    fn it_starts_where_the_scene_put_it() {
        assert!(rocking(Vec3::new(3.0, 1.0, 2.0), 0.0, 1.0).abs_diff_eq(Mat4::IDENTITY, 1.0e-6));
    }

    #[test]
    fn no_amplitude_holds_it_still() {
        assert_eq!(rocking(Vec3::new(3.0, 1.0, 2.0), 5.5, 0.0), Mat4::IDENTITY);
    }
}
