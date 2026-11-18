//! Where things are: each entity's place relative to its parent, and the world's view of it.
//!
//! An entity's [`Transform`] is relative to its [`Parent`], or to the world if it has none. After
//! every step the world works out each one's [`GlobalTransform`], parents before children, and
//! keeps the one before, so a frame can draw things between the two steps it falls between.
//!
//! The physics group sees the places worked out after the step before; anything moved earlier in
//! the step is placed when the step ends.

use glam::{Affine3A, Mat4, Quat, Vec3};
use hecs::Entity;

/// A place, turn and size, relative to the parent.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transform {
    pub translation: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
}

impl Default for Transform {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Transform {
    pub const IDENTITY: Self = Self {
        translation: Vec3::ZERO,
        rotation: Quat::IDENTITY,
        scale: Vec3::ONE,
    };

    pub fn from_translation(translation: Vec3) -> Self {
        Self {
            translation,
            ..Self::IDENTITY
        }
    }

    pub fn with_rotation(mut self, rotation: Quat) -> Self {
        self.rotation = rotation;
        self
    }

    pub fn with_scale(mut self, scale: Vec3) -> Self {
        self.scale = scale;
        self
    }

    pub fn affine(&self) -> Affine3A {
        Affine3A::from_scale_rotation_translation(self.scale, self.rotation, self.translation)
    }
}

/// The entity this one is attached to. Its transform is then relative to the parent's.
///
/// A parent that no longer exists counts as none, so its children are then placed relative to
/// the world. Attaching in a loop counts as none at the link that closes the loop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Parent(pub Entity);

/// Marks an entity whose next move is a jump, such as a respawn: when the step ends, it is drawn
/// at its new place from the start, rather than sliding there over a frame. The mark is then
/// removed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Snap;

/// Where an entity is in the world after the latest step, and after the step before.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlobalTransform {
    pub now: Affine3A,
    pub before: Affine3A,
}

impl GlobalTransform {
    /// Where to draw it, `alpha` of the way from the step before to the latest: the frame's
    /// place between the two steps it falls between. Positions and sizes are blended, turns are
    /// slerped. Sizes are taken apart from turns, so a shear (a stretched child of a turned
    /// parent) or a mirroring scale doesn't blend exactly; the two ends are always exact.
    pub fn interpolated(&self, alpha: f32) -> Mat4 {
        if alpha >= 1.0 || self.now == self.before {
            return Mat4::from(self.now);
        }
        if alpha <= 0.0 {
            return Mat4::from(self.before);
        }
        let (s0, r0, t0) = self.before.to_scale_rotation_translation();
        let (s1, r1, t1) = self.now.to_scale_rotation_translation();
        Mat4::from_scale_rotation_translation(
            s0.lerp(s1, alpha),
            r0.slerp(r1, alpha),
            t0.lerp(t1, alpha),
        )
    }
}

/// How a walking character wants to move, in metres a second across the ground. Set by its script
/// each step, carried out by physics, and held until set again. Its `y` is a jump: the speed
/// to leave the ground with, heeded only while the character stands on something.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Walk(pub Vec3);

/// Marks an entity id with no transform in [`Propagation::slots`].
const NONE: u32 = u32::MAX;

/// How far through working out an entity's place is.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Progress {
    Waiting,
    /// Its parents are being worked out: reaching it again means they loop back to it.
    Climbing,
    Done,
}

/// The working arrays of an update, indexed densely rather than hashed: entity ids are small
/// and packed, so an array as long as the largest id costs little and finds each in one step.
/// Kept from step to step, so they are allocated once.
#[derive(Default)]
pub(crate) struct Propagation {
    /// Where each entity id's transform is in the arrays below, or [`NONE`].
    slots: Vec<u32>,
    found: Vec<(Entity, Affine3A, Option<Entity>)>,
    /// Each one's parent's slot, or [`NONE`].
    parents: Vec<u32>,
    globals: Vec<Affine3A>,
    progress: Vec<Progress>,
    chain: Vec<u32>,
    snapped: Vec<Entity>,
    stale: Vec<Entity>,
    new: Vec<(Entity, Affine3A)>,
}

impl Propagation {
    /// Works out the place of the entity in `slot`, and of every parent it waits on: climbs the
    /// chain of parents to one already done, or to a root, then works back down it.
    fn resolve(&mut self, slot: u32) {
        let mut at = slot;
        while self.progress[at as usize] == Progress::Waiting {
            self.progress[at as usize] = Progress::Climbing;
            self.chain.push(at);
            let parent = self.parents[at as usize];
            if parent == NONE {
                break;
            }
            if self.progress[parent as usize] == Progress::Climbing {
                // The chain loops back on itself here: cut it, and place this one at the root.
                self.parents[at as usize] = NONE;
                break;
            }
            at = parent;
        }
        while let Some(at) = self.chain.pop() {
            let i = at as usize;
            let local = self.found[i].1;
            let parent = self.parents[i];
            self.globals[i] = if parent == NONE {
                local
            } else {
                self.globals[parent as usize] * local
            };
            self.progress[i] = Progress::Done;
        }
    }
}

/// Works out every entity's [`GlobalTransform`] from its [`Transform`] and its parents', keeping
/// last step's as the one before. Entities without one yet get one, with no motion, as do those
/// marked [`Snap`]. Entities that no longer have a transform lose theirs.
pub(crate) fn update_global_transforms(world: &mut hecs::World, work: &mut Propagation) {
    work.found.clear();
    work.found.extend(
        world
            .query_mut::<(Entity, &Transform, Option<&Parent>)>()
            .into_iter()
            .map(|(entity, transform, parent)| (entity, transform.affine(), parent.map(|p| p.0))),
    );

    let longest = work
        .found
        .iter()
        .map(|f| f.0.id() as usize + 1)
        .max()
        .unwrap_or(0);
    work.slots.clear();
    work.slots.resize(longest, NONE);
    for (slot, found) in work.found.iter().enumerate() {
        work.slots[found.0.id() as usize] = slot as u32;
    }
    // A parent counts only if it is the very entity named, still alive (an id is reused, with a
    // new generation, once its entity is gone), and has a transform.
    work.parents.clear();
    for found in &work.found {
        let parent = found
            .2
            .filter(|&p| world.contains(p))
            .and_then(|p| work.slots.get(p.id() as usize).copied())
            .unwrap_or(NONE);
        work.parents.push(parent);
    }
    let count = work.found.len();
    work.globals.clear();
    work.globals.resize(count, Affine3A::IDENTITY);
    work.progress.clear();
    work.progress.resize(count, Progress::Waiting);
    for slot in 0..count as u32 {
        work.resolve(slot);
    }

    work.snapped.clear();
    work.stale.clear();
    work.new.clear();
    for (entity, global, has_transform, snap) in world.query_mut::<(
        Entity,
        Option<&mut GlobalTransform>,
        hecs::Satisfies<&Transform>,
        hecs::Satisfies<&Snap>,
    )>() {
        if !has_transform {
            if global.is_some() {
                work.stale.push(entity);
            }
            continue;
        }
        let now = work.globals[work.slots[entity.id() as usize] as usize];
        match global {
            Some(global) => {
                global.before = if snap { now } else { global.now };
                global.now = now;
            }
            None => work.new.push((entity, now)),
        }
        if snap {
            work.snapped.push(entity);
        }
    }
    for &(entity, now) in &work.new {
        world
            .insert_one(entity, GlobalTransform { now, before: now })
            .expect("the entity was just queried");
    }
    for &entity in &work.snapped {
        world
            .remove_one::<Snap>(entity)
            .expect("the entity was just queried");
    }
    for &entity in &work.stale {
        world
            .remove_one::<GlobalTransform>(entity)
            .expect("the entity was just queried");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(x: f32) -> Transform {
        Transform::from_translation(Vec3::new(x, 0.0, 0.0))
    }

    fn update(world: &mut hecs::World) {
        update_global_transforms(world, &mut Propagation::default());
    }

    fn position(world: &hecs::World, entity: Entity) -> Vec3 {
        world
            .get::<&GlobalTransform>(entity)
            .unwrap()
            .now
            .translation
            .into()
    }

    #[test]
    fn children_follow_their_parents_however_deep() {
        let mut world = hecs::World::new();
        // Children spawned before their parents, so they are worked out first.
        let great = world.reserve_entity();
        let grandchild = world.reserve_entity();
        let child = world.reserve_entity();
        let root = world
            .spawn((at(1.0).with_rotation(Quat::from_rotation_y(std::f32::consts::FRAC_PI_2)),));
        world.insert(great, (at(1.0), Parent(grandchild))).unwrap();
        world
            .insert(
                grandchild,
                (at(3.0).with_scale(Vec3::splat(2.0)), Parent(child)),
            )
            .unwrap();
        world.insert(child, (at(2.0), Parent(root))).unwrap();
        update(&mut world);
        // The root turns everything a quarter turn: +X becomes -Z.
        assert!(position(&world, child).abs_diff_eq(Vec3::new(1.0, 0.0, -2.0), 1.0e-5));
        assert!(position(&world, grandchild).abs_diff_eq(Vec3::new(1.0, 0.0, -5.0), 1.0e-5));
        // The grandchild's scale doubles the step to its own child.
        assert!(position(&world, great).abs_diff_eq(Vec3::new(1.0, 0.0, -7.0), 1.0e-5));
    }

    #[test]
    fn a_long_chain_is_placed_exactly_whichever_end_comes_first() {
        let mut world = hecs::World::new();
        // A thousand links, each spawned before the one it hangs from.
        let links: Vec<Entity> = (0..1000).map(|_| world.reserve_entity()).collect();
        for (i, &link) in links.iter().enumerate() {
            match links.get(i + 1) {
                Some(&parent) => world.insert(link, (at(1.0), Parent(parent))).unwrap(),
                None => world.insert(link, (at(1.0),)).unwrap(),
            }
        }
        update(&mut world);
        assert_eq!(position(&world, links[0]).x, 1000.0);
        assert_eq!(position(&world, links[500]).x, 500.0);
    }

    #[test]
    fn the_step_before_is_kept_and_new_entities_start_still() {
        let mut world = hecs::World::new();
        let ball = world.spawn((at(1.0),));
        let mut work = Propagation::default();
        update_global_transforms(&mut world, &mut work);
        let global = *world.get::<&GlobalTransform>(ball).unwrap();
        assert_eq!(global.now, global.before);
        world.get::<&mut Transform>(ball).unwrap().translation.x = 4.0;
        update_global_transforms(&mut world, &mut work);
        let global = *world.get::<&GlobalTransform>(ball).unwrap();
        assert_eq!(Vec3::from(global.before.translation).x, 1.0);
        assert_eq!(Vec3::from(global.now.translation).x, 4.0);
        // Halfway between the two steps.
        let between = global.interpolated(0.5).w_axis.truncate();
        assert!(between.abs_diff_eq(Vec3::new(2.5, 0.0, 0.0), 1.0e-5));
        // Each step's place becomes the next step's place before.
        world.get::<&mut Transform>(ball).unwrap().translation.x = 6.0;
        update_global_transforms(&mut world, &mut work);
        let global = *world.get::<&GlobalTransform>(ball).unwrap();
        assert_eq!(Vec3::from(global.before.translation).x, 4.0);
        assert_eq!(Vec3::from(global.now.translation).x, 6.0);
    }

    #[test]
    fn a_snap_jumps_with_no_motion_once() {
        let mut world = hecs::World::new();
        let ball = world.spawn((at(1.0),));
        update(&mut world);
        world.get::<&mut Transform>(ball).unwrap().translation.x = 50.0;
        world.insert_one(ball, Snap).unwrap();
        update(&mut world);
        let global = *world.get::<&GlobalTransform>(ball).unwrap();
        assert_eq!(global.now, global.before);
        assert!(world.get::<&Snap>(ball).is_err(), "the mark is used up");
    }

    #[test]
    fn interpolation_turns_the_short_way_at_a_steady_rate() {
        let global = GlobalTransform {
            before: Affine3A::from_rotation_y(0.0),
            now: Affine3A::from_rotation_y(1.0),
        };
        let quarter = global.interpolated(0.25);
        let angle = Quat::from_mat4(&quarter).angle_between(Quat::IDENTITY);
        assert!((angle - 0.25).abs() < 1.0e-4, "{angle}");
        assert_eq!(global.interpolated(1.0), Mat4::from(global.now));
        assert_eq!(global.interpolated(0.0), Mat4::from(global.before));
    }

    #[test]
    fn a_parent_that_is_gone_counts_as_none_even_when_its_id_is_reused() {
        let mut world = hecs::World::new();
        let gone = world.spawn((at(100.0),));
        let orphan = world.spawn((at(2.0), Parent(gone)));
        world.despawn(gone).unwrap();
        // The next entity takes the gone one's id, with a new generation.
        let newcomer = world.spawn((at(7.0),));
        assert_eq!(newcomer.id(), gone.id());
        update(&mut world);
        assert_eq!(position(&world, orphan), Vec3::new(2.0, 0.0, 0.0));
    }

    #[test]
    fn a_loop_is_cut_where_it_closes_and_stays_finite() {
        let mut world = hecs::World::new();
        let grow = at(1.0).with_scale(Vec3::splat(2.0));
        let a = world.spawn((grow,));
        let b = world.spawn((grow, Parent(a)));
        world.insert_one(a, Parent(b)).unwrap();
        update(&mut world);
        // One of the two becomes the root; the other hangs from it once, not 256 times.
        let (pa, pb) = (position(&world, a), position(&world, b));
        assert!(pa.is_finite() && pb.is_finite());
        let mut xs = [pa.x, pb.x];
        xs.sort_by(f32::total_cmp);
        assert_eq!(xs, [1.0, 3.0]);
    }

    #[test]
    fn without_a_transform_there_is_no_place_in_the_world() {
        let mut world = hecs::World::new();
        let thing = world.spawn((42_u32,));
        let ball = world.spawn((at(1.0),));
        update(&mut world);
        assert!(world.get::<&GlobalTransform>(thing).is_err());
        world.remove_one::<Transform>(ball).unwrap();
        update(&mut world);
        assert!(world.get::<&GlobalTransform>(ball).is_err());
    }
}
