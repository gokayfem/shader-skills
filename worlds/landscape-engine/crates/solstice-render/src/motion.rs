//! Instances that change while a scene runs: moved, hidden, or glowing more or less.
//!
//! A scene's instances are uploaded once, when it opens. Anything that changes one afterwards
//! goes through [`InstanceTable`]. It keeps the CPU's copy, and each frame works out what to
//! upload, and whether the views kept from frame to frame (shadow cascades, the seabed map, GI
//! voxels) must be drawn again.
//!
//! An instance that moves becomes movable: it is drawn afresh every frame, and the cached views
//! leave it out, so they never keep it where it was. Making an instance movable, or changing a
//! static one, changes what those views hold, so they are drawn again once. Only the views that
//! hold what changed are: a lamp's glow is in the GI voxels, but not in the shadows. Something
//! that changes often is better made movable, so that the cached views needn't be redrawn.
//!
//! Motion vectors need where each instance was drawn last frame. One moved this frame keeps
//! last frame's transform as its previous one. One that moved last frame but not this frame
//! catches up, so that it stops showing motion.

use crate::scene::{GpuInstance, INSTANCE_HIDDEN, INSTANCE_MOVABLE, INSTANCE_STAYS_IN_GI};
use glam::{Mat4, Vec3};
use std::ops::Range;

/// The CPU's copy of a scene's instances, and what has changed in it since the last frame.
///
/// Instances are named by their index in the scene file. Instances of meshes with no geometry
/// have no place on the GPU, and every change to them is refused.
pub struct InstanceTable {
    data: Vec<GpuInstance>,
    /// Where each of the scene's instances is in `data`.
    gpu_index: Vec<Option<u32>>,
    /// Each instance's object-space bounding sphere, by position in `data`.
    spheres: Vec<(Vec3, f32)>,
    /// One pending transform per changed instance, and whether it is a jump rather than a motion.
    moves: Vec<(u32, Mat4, bool)>,
    /// Each instance's entry in `moves`, or `u32::MAX` when it has no pending move.
    pending_move: Vec<u32>,
    /// What moved last frame, whose previous transform must catch up this frame.
    moved_last_frame: Vec<u32>,
    /// What has changed in other ways since the last upload.
    changed: Vec<u32>,
    /// Every movable instance, by position in `data`, in no particular order.
    movable: Vec<u32>,
    shadows_changed: bool,
    gi_changed: bool,
}

/// What a frame changed: the runs of instances to upload, and which cached views must be drawn
/// again.
#[derive(Debug, Default, PartialEq)]
pub struct FrameChanges {
    pub ranges: Vec<Range<usize>>,
    /// What the shadow cascades and the seabed map hold has changed.
    pub shadows_changed: bool,
    /// What the GI voxels hold has changed.
    pub gi_changed: bool,
}

/// Changed instances closer than this to the run before them are uploaded with it, since one
/// larger write costs less than several small ones.
const MERGE_GAP: usize = 8;

impl InstanceTable {
    pub fn new(
        data: Vec<GpuInstance>,
        gpu_index: Vec<Option<u32>>,
        spheres: Vec<(Vec3, f32)>,
    ) -> Self {
        debug_assert_eq!(data.len(), spheres.len());
        let movable = (0..data.len() as u32)
            .filter(|&i| data[i as usize].has_flag(INSTANCE_MOVABLE))
            .collect();
        Self {
            pending_move: vec![u32::MAX; data.len()],
            data,
            gpu_index,
            spheres,
            moves: Vec::new(),
            moved_last_frame: Vec::new(),
            changed: Vec::new(),
            movable,
            shadows_changed: false,
            gi_changed: false,
        }
    }

    /// How many instances the scene has, with or without geometry.
    pub fn len(&self) -> usize {
        self.gpu_index.len()
    }

    pub fn is_empty(&self) -> bool {
        self.gpu_index.is_empty()
    }

    /// Which of the instances on the GPU one of the scene's is, if it is drawn at all.
    pub fn gpu_number(&self, instance: u32) -> Option<u32> {
        self.index(instance).map(|index| index as u32)
    }

    fn index(&self, instance: u32) -> Option<usize> {
        self.gpu_index
            .get(instance as usize)
            .copied()
            .flatten()
            .map(|i| i as usize)
    }

    /// Moves an instance to `world` from the next frame on, making it movable if it was not.
    /// If it is moved more than once before a frame, the last move counts. A `jump` puts it
    /// there with no motion, as for a respawn, rather than moving it there over the frame.
    pub fn set_transform(&mut self, instance: u32, world: Mat4, jump: bool) -> bool {
        let Some(i) = self.index(instance) else {
            return false;
        };
        self.make_movable(i, true);
        let slot = self.pending_move[i];
        if slot == u32::MAX {
            self.pending_move[i] = self.moves.len() as u32;
            self.moves.push((i as u32, world, jump));
        } else {
            self.moves[slot as usize] = (i as u32, world, jump);
        }
        true
    }

    /// Makes an instance movable, or static again. Either way, the cached views are drawn again.
    /// A skinned instance is refused going static: the cached views can't hold a pose.
    pub fn set_movable(&mut self, instance: u32, movable: bool) -> bool {
        let Some(i) = self.index(instance) else {
            return false;
        };
        if !movable && self.data[i].is_skinned() {
            return false;
        }
        self.make_movable(i, movable);
        true
    }

    fn make_movable(&mut self, i: usize, movable: bool) {
        if self.data[i].has_flag(INSTANCE_MOVABLE) == movable {
            return;
        }
        self.data[i].set_flag(INSTANCE_MOVABLE, movable);
        if movable {
            self.movable.push(i as u32);
        } else {
            self.movable.retain(|&m| m as usize != i);
        }
        self.changed.push(i as u32);
        self.shadows_changed = true;
        self.gi_changed = true;
    }

    /// Hides an instance, or shows it again.
    pub fn set_hidden(&mut self, instance: u32, hidden: bool) -> bool {
        let Some(i) = self.index(instance) else {
            return false;
        };
        if self.data[i].has_flag(INSTANCE_HIDDEN) != hidden {
            self.data[i].set_flag(INSTANCE_HIDDEN, hidden);
            self.changed.push(i as u32);
            self.shadows_changed |= !self.data[i].has_flag(INSTANCE_MOVABLE);
            self.gi_changed |= self.in_gi(i);
        }
        true
    }

    /// Keeps a movable instance in the GI voxels, where it stood when they were last drawn, or
    /// leaves it out of them. For things that move only a little. A skinned instance is refused
    /// staying in: the voxels are drawn without poses.
    pub fn set_stays_in_gi(&mut self, instance: u32, stays: bool) -> bool {
        let Some(i) = self.index(instance) else {
            return false;
        };
        if stays && self.data[i].is_skinned() {
            return false;
        }
        if self.data[i].has_flag(INSTANCE_STAYS_IN_GI) != stays {
            self.data[i].set_flag(INSTANCE_STAYS_IN_GI, stays);
            self.changed.push(i as u32);
            // Only a movable instance's place in the voxels changes.
            self.gi_changed |= self.data[i].has_flag(INSTANCE_MOVABLE);
        }
        true
    }

    /// Scales how brightly an instance's material glows: 1 as the material says, 0 dark.
    pub fn set_glow(&mut self, instance: u32, glow: f32) -> bool {
        let Some(i) = self.index(instance) else {
            return false;
        };
        if self.data[i].glow() != glow {
            self.data[i].set_glow(glow);
            self.changed.push(i as u32);
            // Glow is only in the voxels, as light; shadows don't hold it.
            self.gi_changed |= self.in_gi(i);
        }
        true
    }

    /// Sets how far an instance's pose carries anything, by its place on the GPU.
    pub(crate) fn set_lod_margin(&mut self, gpu_index: u32, margin: f32) {
        self.data[gpu_index as usize].set_lod_margin(margin);
        self.changed.push(gpu_index);
    }

    /// Whether the GI voxels hold an instance: if it is static, or movable but stays in them.
    fn in_gi(&self, i: usize) -> bool {
        !self.data[i].has_flag(INSTANCE_MOVABLE) || self.data[i].has_flag(INSTANCE_STAYS_IN_GI)
    }

    /// Where an instance will be drawn next frame: its latest move, or where it is.
    pub fn transform(&self, instance: u32) -> Option<Mat4> {
        let i = self.index(instance)?;
        let pending = self.moves.get(self.pending_move[i] as usize);
        Some(pending.map_or_else(|| self.data[i].worlds().0, |m| m.1))
    }

    /// An instance's bounding sphere in the world, where it is drawn now.
    pub fn world_sphere(&self, instance: u32) -> Option<(Vec3, f32)> {
        self.index(instance).map(|i| self.sphere(i))
    }

    fn sphere(&self, i: usize) -> (Vec3, f32) {
        let (center, radius) = self.spheres[i];
        let world = self.data[i].worlds().0;
        (
            world.transform_point3(center),
            (radius + self.data[i].lod_margin()) * self.data[i].max_scale(),
        )
    }

    /// The world bounding spheres of every movable instance that is shown, where they are drawn
    /// now.
    pub fn moving_spheres(&self) -> Vec<(Vec3, f32)> {
        self.movable
            .iter()
            .map(|&i| i as usize)
            .filter(|&i| !self.data[i].has_flag(INSTANCE_HIDDEN))
            .map(|i| self.sphere(i))
            .collect()
    }

    /// Moves on to the next frame: applies its moves, lets last frame's moves catch up, and
    /// reports what to upload.
    pub fn advance(&mut self) -> FrameChanges {
        let mut touched = std::mem::take(&mut self.changed);
        for i in self.moved_last_frame.drain(..) {
            self.data[i as usize].settle();
            touched.push(i);
        }

        // Repeated writes already replaced the pending entry. Instances are independent, so
        // applying them needs no sort, and a simulation tick cannot grow duplicate work.
        for &(i, world, jump) in &self.moves {
            self.pending_move[i as usize] = u32::MAX;
            let instance = &mut self.data[i as usize];
            instance.settle();
            instance.set_world(world);
            touched.push(i);
            if jump {
                // Already caught up: it shows no motion.
                instance.settle();
            } else {
                self.moved_last_frame.push(i);
            }
        }
        self.moves.clear();
        let ranges = runs(&mut touched);
        touched.clear();
        self.changed = touched;

        FrameChanges {
            ranges,
            shadows_changed: std::mem::take(&mut self.shadows_changed),
            gi_changed: std::mem::take(&mut self.gi_changed),
        }
    }

    /// The GPU's copy of a run of instances, to upload.
    pub fn gpu_data(&self, range: Range<usize>) -> &[GpuInstance] {
        &self.data[range]
    }
}

/// Sorts indices into runs to upload, merging runs separated by small gaps.
fn runs(indices: &mut [u32]) -> Vec<Range<usize>> {
    indices.sort_unstable();
    let mut ranges: Vec<Range<usize>> = Vec::new();
    for i in indices.iter().map(|&i| i as usize) {
        match ranges.last_mut() {
            Some(last) if i <= last.end + MERGE_GAP => last.end = i + 1,
            _ => ranges.push(i..i + 1),
        }
    }
    ranges
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytemuck::Zeroable;

    fn instance_at(x: f32) -> GpuInstance {
        let mut instance = GpuInstance::zeroed();
        instance.set_world(Mat4::from_translation(Vec3::new(x, 0.0, 0.0)));
        instance.settle();
        instance.set_glow(1.0);
        instance.set_bones(crate::skin::NO_BONES);
        instance
    }

    /// Five instances with geometry, at x = 0..5, and a sixth scene instance without any,
    /// between the second and third.
    fn table() -> InstanceTable {
        let data = (0..5).map(|i| instance_at(i as f32)).collect();
        let gpu_index = vec![Some(0), Some(1), None, Some(2), Some(3), Some(4)];
        InstanceTable::new(data, gpu_index, vec![(Vec3::ZERO, 1.0); 5])
    }

    fn worlds(table: &InstanceTable, gpu: usize) -> (Vec3, Vec3) {
        let (now, before) = table.data[gpu].worlds();
        (now.w_axis.truncate(), before.w_axis.truncate())
    }

    fn at(x: f32) -> Mat4 {
        Mat4::from_translation(Vec3::new(x, 0.0, 0.0))
    }

    #[test]
    fn a_moved_instance_remembers_where_it_was_then_catches_up() {
        let mut table = table();
        assert!(table.set_transform(1, at(10.0), false));
        table.advance();
        assert_eq!(worlds(&table, 1), (Vec3::X * 10.0, Vec3::X));
        // Not moved this frame: it must stop showing motion.
        let changes = table.advance();
        assert_eq!(worlds(&table, 1), (Vec3::X * 10.0, Vec3::X * 10.0));
        assert_eq!(changes.ranges, vec![1..2]);
        // And after that, nothing more to upload.
        assert_eq!(table.advance(), FrameChanges::default());
    }

    #[test]
    fn moving_every_frame_keeps_each_frames_place_as_the_previous() {
        let mut table = table();
        for x in [5.0, 6.0, 8.0] {
            let before = worlds(&table, 0).0;
            table.set_transform(0, at(x), false);
            table.advance();
            assert_eq!(worlds(&table, 0), (Vec3::X * x, before));
        }
    }

    #[test]
    fn the_last_move_before_a_frame_counts() {
        let mut table = table();
        table.set_transform(0, at(3.0), false);
        table.set_transform(3, at(7.0), false);
        table.set_transform(0, at(4.0), false);
        assert_eq!(table.transform(0), Some(at(4.0)));
        table.advance();
        assert_eq!(worlds(&table, 0).0, Vec3::X * 4.0);
        // Scene instance 3 is the third on the GPU.
        assert_eq!(worlds(&table, 2).0, Vec3::X * 7.0);
    }

    #[test]
    fn many_interleaved_moves_match_a_frame_by_frame_reference() {
        const COUNT: usize = 513;
        let mut table = InstanceTable::new(
            (0..COUNT).map(|i| instance_at(i as f32)).collect(),
            (0..COUNT as u32).map(Some).collect(),
            vec![(Vec3::ZERO, 1.0); COUNT],
        );
        let mut current: Vec<Mat4> = (0..COUNT).map(|i| at(i as f32)).collect();
        let mut previous = current.clone();
        let mut pending = vec![None; COUNT];
        let mut random = 0x1234_5678u32;
        for frame in 0..160 {
            // Empty frames, sparse updates and several updates per instance, interleaved.
            let changes = match frame % 4 {
                0 => 0,
                1 => 11,
                _ => COUNT * 8,
            };
            for n in 0..changes {
                random = random.wrapping_mul(1664525).wrapping_add(1013904223);
                let i = random as usize % COUNT;
                let world = Mat4::from_scale_rotation_translation(
                    Vec3::new(1.0, 2.0, 0.5),
                    glam::Quat::from_rotation_y(n as f32 * 0.01),
                    Vec3::new(i as f32, frame as f32, n as f32),
                );
                let jump = n % 3 == 0;
                table.set_transform(i as u32, world, jump);
                pending[i] = Some((world, jump));
                assert_eq!(table.transform(i as u32), Some(world));
            }
            let old_previous = previous.clone();
            let old_current = current.clone();
            for i in 0..COUNT {
                previous[i] = current[i];
                if let Some((world, jump)) = pending[i].take() {
                    current[i] = world;
                    if jump {
                        previous[i] = world;
                    }
                }
            }
            let changes = table.advance();
            for i in 0..COUNT {
                let (now, before) = table.data[i].worlds();
                assert_eq!(
                    (now, before),
                    (current[i], previous[i]),
                    "frame {frame}, instance {i}"
                );
                assert_eq!(table.transform(i as u32), Some(current[i]));
                if old_current[i] != current[i] || old_previous[i] != previous[i] {
                    assert!(
                        changes.ranges.iter().any(|range| range.contains(&i)),
                        "changed instance {i} was not uploaded at frame {frame}"
                    );
                }
            }
        }
    }

    #[test]
    fn instances_without_geometry_or_out_of_range_are_refused() {
        let mut table = table();
        assert!(!table.set_transform(2, at(1.0), false));
        assert!(!table.set_hidden(2, true));
        assert!(!table.set_glow(9, 0.5));
        assert!(!table.set_movable(6, true));
        assert_eq!(table.transform(2), None);
        assert_eq!(table.advance(), FrameChanges::default());
    }

    /// Which cached views a frame says to redraw: (shadows, GI).
    fn redraws(table: &mut InstanceTable) -> (bool, bool) {
        let changes = table.advance();
        (changes.shadows_changed, changes.gi_changed)
    }

    #[test]
    fn moving_a_static_instance_redraws_the_cached_views_once() {
        let mut table = table();
        table.set_transform(0, at(1.0), false);
        assert_eq!(redraws(&mut table), (true, true));
        table.set_transform(0, at(2.0), false);
        assert_eq!(redraws(&mut table), (false, false));
        // Making it static again puts it back in them.
        table.set_movable(0, false);
        assert_eq!(redraws(&mut table), (true, true));
    }

    #[test]
    fn each_change_redraws_only_the_views_that_hold_it() {
        let mut table = table();
        // Static: hidden changes both; glow is only in GI.
        table.set_hidden(0, true);
        assert_eq!(redraws(&mut table), (true, true));
        table.set_glow(1, 0.0);
        assert_eq!(redraws(&mut table), (false, true));

        // Movable, and out of GI: in no cached view.
        table.set_movable(4, true);
        table.advance();
        table.set_hidden(4, true);
        table.set_glow(4, 2.0);
        let changes = table.advance();
        assert_eq!(
            (changes.shadows_changed, changes.gi_changed),
            (false, false)
        );
        assert_eq!(changes.ranges, vec![3..4]);

        // Movable but staying in GI: in the voxels only.
        table.set_stays_in_gi(4, true);
        assert_eq!(redraws(&mut table), (false, true));
        table.set_hidden(4, false);
        assert_eq!(redraws(&mut table), (false, true));
        table.set_glow(4, 1.0);
        assert_eq!(redraws(&mut table), (false, true));
    }

    #[test]
    fn staying_in_gi_redraws_the_voxels_only_for_movable_instances() {
        let mut table = table();
        assert!(table.set_stays_in_gi(0, true));
        let changes = table.advance();
        assert_eq!(
            (changes.shadows_changed, changes.gi_changed),
            (false, false)
        );
        assert_eq!(changes.ranges, vec![0..1]);
        table.set_movable(1, true);
        table.advance();
        table.set_stays_in_gi(1, true);
        assert_eq!(redraws(&mut table), (false, true));
        table.set_stays_in_gi(1, true);
        assert_eq!(table.advance(), FrameChanges::default());
        assert!(!table.set_stays_in_gi(2, true));
    }

    #[test]
    fn a_jump_shows_no_motion() {
        let mut table = table();
        table.set_transform(0, at(50.0), true);
        let changes = table.advance();
        assert_eq!(worlds(&table, 0), (Vec3::X * 50.0, Vec3::X * 50.0));
        assert_eq!(changes.ranges, vec![0..1]);
        // Nothing to catch up afterwards.
        assert_eq!(table.advance(), FrameChanges::default());
        // A motion after the jump starts from where it jumped to.
        table.set_transform(0, at(51.0), false);
        table.advance();
        assert_eq!(worlds(&table, 0), (Vec3::X * 51.0, Vec3::X * 50.0));
    }

    #[test]
    fn setting_what_is_already_so_changes_nothing() {
        let mut table = table();
        table.set_hidden(0, false);
        table.set_glow(0, 1.0);
        table.set_movable(0, false);
        assert_eq!(table.advance(), FrameChanges::default());
    }

    #[test]
    fn a_skinned_instance_stays_out_of_the_views_that_cant_hold_a_pose() {
        let mut table = table();
        table.data[0].set_bones(0);
        assert!(table.set_movable(0, true));
        table.advance();
        assert!(!table.set_movable(0, false));
        assert!(!table.set_stays_in_gi(0, true));
        assert!(table.set_stays_in_gi(0, false));
        assert!(table.data[0].has_flag(INSTANCE_MOVABLE));
        assert!(!table.data[0].has_flag(INSTANCE_STAYS_IN_GI));
    }

    #[test]
    fn a_margin_follows_the_instances_scale() {
        let mut table = table();
        table.data[0].set_lod_margin(0.5);
        table.set_transform(0, Mat4::from_scale(Vec3::splat(4.0)), false);
        table.advance();
        assert_eq!(table.world_sphere(0).unwrap().1, (1.0 + 0.5) * 4.0);
    }

    #[test]
    fn glow_below_zero_is_kept_for_a_lamp_that_glows_red() {
        let mut table = table();
        table.set_glow(0, -3.0);
        assert_eq!(table.data[0].glow(), -3.0);
    }

    #[test]
    fn nearby_changes_upload_together_and_distant_ones_apart() {
        assert_eq!(runs(&mut [4, 1, 2, 2]), vec![1..5]);
        assert_eq!(runs(&mut [0, 20]), vec![0..1, 20..21]);
        assert_eq!(runs(&mut [0, 9]), vec![0..10]);
        assert_eq!(runs(&mut [0, 10]), vec![0..1, 10..11]);
    }

    #[test]
    fn moving_spheres_cover_shown_movable_instances_where_they_are() {
        let mut table = table();
        let mut scaled = Mat4::from_scale(Vec3::splat(3.0));
        scaled.w_axis = Vec3::new(1.0, 2.0, 3.0).extend(1.0);
        table.set_transform(0, scaled, false);
        table.set_transform(1, at(5.0), false);
        table.set_hidden(1, true);
        table.advance();
        assert_eq!(
            table.moving_spheres(),
            vec![(Vec3::new(1.0, 2.0, 3.0), 3.0)]
        );
        table.set_movable(0, false);
        table.advance();
        assert!(table.moving_spheres().is_empty());
    }
}
