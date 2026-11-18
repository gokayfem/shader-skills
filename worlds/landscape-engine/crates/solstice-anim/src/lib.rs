//! Solstice's animation runtime (docs/gameplay.md, The animation runtime): clips, poses,
//! blends, blend spaces and state machines, on the CPU, with no GPU or world of its own.
//!
//! * [`clip`]: compressed clips, and sampling them.
//! * [`pose`]: a skeleton's pose, blends between poses, and bone masks.
//! * [`graph`]: state machines in a small text format, cross-fades and notifies.

pub mod clip;
pub mod graph;
pub mod pose;

pub use clip::{compress, duration, sample, Frames};
pub use graph::{AnimGraph, Animator, ClipSet, Motion};
pub use pose::{add, blend, BoneMask, Pose};

/// How many frames apart a character at this distance from the camera updates its pose, as
/// Unreal's animation budget allocator does: near ones every frame, far ones rarely.
pub fn update_interval(distance: f32) -> u32 {
    match distance {
        d if d < 15.0 => 1,
        d if d < 40.0 => 2,
        d if d < 90.0 => 4,
        _ => 8,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distant_characters_update_less_often() {
        assert_eq!(update_interval(3.0), 1);
        assert_eq!(update_interval(30.0), 2);
        assert!(update_interval(500.0) > update_interval(50.0));
    }
}
