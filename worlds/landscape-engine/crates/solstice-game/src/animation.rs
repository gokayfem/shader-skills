//! Animated actors: what plays on each actor's skeleton, frame by frame, and what it tells the
//! renderer and its script.
//!
//! An actor's `.anim` file names its state machine. If the graph has a `speed` parameter, how
//! fast the actor is walking sets it each step, so idling and walking follow what it is doing.
//!
//! Animations move on in fixed steps, as gameplay does, so that what depends on them (transitions,
//! notifies) replays exactly at any frame rate. Only the working out of what to draw follows the
//! frame rate and the camera's distance.

use glam::{Mat4, Vec3};
use solstice_anim::{AnimGraph, Animator, ClipSet, Pose};
use solstice_asset::{Clip, GameFile, Skeleton, NO_POSE};
use solstice_script::AnimCommand;
use solstice_world::{Entity, GlobalTransform, Walk, World, STEP_SECONDS};
use std::collections::HashMap;
use std::rc::Rc;

/// The parts of a scene that animation needs.
#[derive(Clone, Copy, Default)]
pub struct AnimationAssets<'a> {
    pub skeletons: &'a [Skeleton],
    pub clips: &'a [Clip],
    pub poses: &'a [u32],
    pub instance_poses: &'a [u32],
}

/// The name a scene gives a clip: its graph's file, and its name there.
pub fn clip_key(graph_file: &str, clip: &str) -> String {
    format!("{graph_file}:{clip}")
}

/// One animated actor.
struct Animated {
    entity: Entity,
    pose_id: u32,
    skeleton: Rc<Skeleton>,
    clips: Rc<ClipSet>,
    rest: Pose,
    pose: Pose,
    animator: Animator,
    bones: Vec<Mat4>,
    /// Frames since its pose was worked out.
    waited: u32,
    uses_speed: bool,
}

/// Every animated actor in a game.
#[derive(Default)]
pub struct AnimatedActors {
    actors: Vec<Animated>,
    by_entity: HashMap<Entity, usize>,
}

impl AnimatedActors {
    /// Builds an animation for each of `actors`: its entity, its `.anim` file, and one of its
    /// renderer instances, which says which pose it is.
    pub fn load(
        files: &[GameFile],
        assets: &AnimationAssets<'_>,
        actors: &[(Entity, String, u32)],
    ) -> Result<Self, Vec<String>> {
        let mut errors = Vec::new();
        let mut skeletons: HashMap<u32, Rc<Skeleton>> = HashMap::new();
        let mut sets: HashMap<(String, u32), Rc<ClipSet>> = HashMap::new();
        let mut out = Self::default();
        for (entity, file, instance) in actors {
            let Some(text) = files.iter().find(|f| f.name == *file).map(|f| &f.text) else {
                errors.push(format!("{file}: missing from the scene's gameplay"));
                continue;
            };
            let graph = match AnimGraph::parse(text) {
                Ok(graph) => graph,
                Err(e) => {
                    errors.push(format!("{file}: {e}"));
                    continue;
                }
            };
            let pose_id = assets
                .instance_poses
                .get(*instance as usize)
                .copied()
                .unwrap_or(NO_POSE);
            if pose_id == NO_POSE {
                errors.push(format!(
                    "{file}: the actor's mesh has no skeleton: is it animated in the scene?"
                ));
                continue;
            }
            let skeleton_index = assets.poses[pose_id as usize];
            let skeleton = skeletons
                .entry(skeleton_index)
                .or_insert_with(|| Rc::new(assets.skeletons[skeleton_index as usize].clone()))
                .clone();
            let key = (file.clone(), skeleton_index);
            let clips = match sets.get(&key) {
                Some(clips) => clips.clone(),
                None => {
                    let mut named = Vec::new();
                    for line in &graph.clips {
                        let wanted = clip_key(file, &line.name);
                        match assets
                            .clips
                            .iter()
                            .find(|c| c.name == wanted && c.skeleton == skeleton_index)
                        {
                            Some(clip) => named.push((line.name.clone(), clip.clone())),
                            None => errors.push(format!(
                                "{file}: the scene has no clip `{}` for this skeleton",
                                line.name
                            )),
                        }
                    }
                    let clips = Rc::new(ClipSet::new(named));
                    sets.insert(key, clips.clone());
                    clips
                }
            };
            if graph.clips.iter().any(|c| !clips.contains(&c.name)) {
                continue;
            }
            let bones = skeleton.bones.len();
            let uses_speed = graph.params.iter().any(|(name, _)| name == "speed");
            let rest = Pose::rest(&skeleton);
            out.by_entity.insert(*entity, out.actors.len());
            out.actors.push(Animated {
                entity: *entity,
                pose_id,
                pose: rest.clone(),
                rest,
                clips,
                animator: Animator::new(graph, bones),
                skeleton,
                bones: Vec::with_capacity(bones),
                waited: u32::MAX,
                uses_speed,
            });
        }
        if errors.is_empty() {
            Ok(out)
        } else {
            Err(errors)
        }
    }

    /// Carries out what scripts asked.
    pub fn apply(&mut self, commands: &[AnimCommand]) {
        for command in commands {
            let Some(&index) = self.by_entity.get(&command.actor) else {
                continue;
            };
            let actor = &mut self.actors[index];
            if let Some((param, value)) = &command.param {
                actor.animator.set(param, *value);
            }
            if let Some((state, fade)) = &command.state {
                actor.animator.go_to(state, *fade);
            }
        }
    }

    /// Moves every animation on by one fixed step. Notifies come back with who they are for.
    pub fn step(&mut self, world: &World, notifies: &mut Vec<(Entity, String)>) {
        for actor in &mut self.actors {
            // Only an actor that has been told to walk: otherwise its script may set this itself.
            if actor.uses_speed {
                if let Ok(walk) = world.entities.get::<&Walk>(actor.entity) {
                    actor.animator.set("speed", walk.0.length());
                }
            }
            notifies.extend(
                actor
                    .animator
                    .advance(STEP_SECONDS as f32, &actor.clips)
                    .into_iter()
                    .map(|name| (actor.entity, name)),
            );
        }
    }

    /// Works out the poses to draw this frame, `alpha` of the way through the step in progress.
    /// Faraway characters update less often, by [`solstice_anim::update_interval`].
    pub fn frame(
        &mut self,
        alpha: f32,
        world: &World,
        viewer: Vec3,
        poses: &mut Vec<(u32, Vec<Mat4>)>,
    ) {
        for actor in &mut self.actors {
            let place = world
                .entities
                .get::<&GlobalTransform>(actor.entity)
                .ok()
                .map(|g| Vec3::from(g.now.translation));
            let distance = place.map_or(0.0, |p| p.distance(viewer));
            let interval = solstice_anim::update_interval(distance);
            actor.waited = actor.waited.saturating_add(1);
            if actor.waited < interval {
                continue;
            }
            actor.waited = 0;
            actor.animator.evaluate_ahead(
                &actor.clips,
                &actor.rest,
                &mut actor.pose,
                alpha * STEP_SECONDS as f32,
            );
            actor.pose.to_model(&actor.skeleton, &mut actor.bones);
            poses.push((actor.pose_id, actor.bones.clone()));
        }
    }

    #[cfg(test)]
    fn state_of(&self, entity: Entity) -> Option<&str> {
        let index = *self.by_entity.get(&entity)?;
        Some(self.actors[index].animator.state_name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Quat;
    use solstice_anim::{compress, Frames};
    use solstice_asset::Bone;

    const GRAPH: &str = "
        clip idle I
        clip walk W
        param speed 0
        notify walk 0.0 step
        state idle clip idle
        state walking clip walk
        start idle
        transition idle walking speed > 0.1 fade 0
        transition walking idle speed < 0.1 fade 0
    ";

    fn assets() -> (Vec<Skeleton>, Vec<Clip>) {
        let skeleton = Skeleton {
            bones: vec![Bone {
                name: "root".to_owned(),
                ..Default::default()
            }],
        };
        let clip = |name: &str, y: f32| {
            let mut made = compress(
                name,
                0,
                &Frames {
                    rotations: vec![vec![Quat::IDENTITY]; 2],
                    translations: vec![vec![Vec3::Y * y]; 2],
                },
                true,
            );
            made.name = clip_key("a.anim", name);
            made
        };
        (vec![skeleton], vec![clip("idle", 1.0), clip("walk", 2.0)])
    }

    fn files() -> Vec<GameFile> {
        vec![GameFile {
            name: "a.anim".to_owned(),
            text: GRAPH.to_owned(),
        }]
    }

    fn world_with_actor() -> (World, Entity) {
        let mut world = World::new();
        let entity = world.spawn((GlobalTransform {
            now: glam::Affine3A::IDENTITY,
            before: glam::Affine3A::IDENTITY,
        },));
        (world, entity)
    }

    #[test]
    fn walking_moves_it_out_of_idle_and_stopping_brings_it_back() {
        let (skeletons, clips) = assets();
        let (mut world, entity) = world_with_actor();
        let assets = AnimationAssets {
            skeletons: &skeletons,
            clips: &clips,
            poses: &[0],
            instance_poses: &[0],
        };
        let mut actors =
            AnimatedActors::load(&files(), &assets, &[(entity, "a.anim".to_owned(), 0)]).unwrap();
        let mut poses = Vec::new();
        let mut notifies = Vec::new();
        actors.step(&world, &mut notifies);
        actors.frame(0.0, &world, Vec3::ZERO, &mut poses);
        assert_eq!(actors.state_of(entity), Some("idle"));
        assert_eq!(poses.len(), 1, "the idle pose is drawn");
        assert_eq!(poses[0].0, 0);
        assert!((poses[0].1[0].w_axis.y - 1.0).abs() < 1.0e-3);

        world.entities.insert_one(entity, Walk(Vec3::X)).unwrap();
        actors.step(&world, &mut notifies);
        actors.frame(0.0, &world, Vec3::ZERO, &mut poses);
        assert_eq!(actors.state_of(entity), Some("walking"));
        assert!((poses[1].1[0].w_axis.y - 2.0).abs() < 1.0e-3);

        world.entities.insert_one(entity, Walk(Vec3::ZERO)).unwrap();
        actors.step(&world, &mut notifies);
        assert_eq!(actors.state_of(entity), Some("idle"));
    }

    #[test]
    fn a_far_character_is_posed_less_often() {
        let (skeletons, clips) = assets();
        let (world, entity) = world_with_actor();
        let assets = AnimationAssets {
            skeletons: &skeletons,
            clips: &clips,
            poses: &[0],
            instance_poses: &[0],
        };
        let mut actors =
            AnimatedActors::load(&files(), &assets, &[(entity, "a.anim".to_owned(), 0)]).unwrap();
        let mut poses = Vec::new();
        let far = Vec3::X * 500.0;
        for _ in 0..16 {
            actors.frame(0.0, &world, far, &mut poses);
        }
        assert!(
            (2..=4).contains(&poses.len()),
            "{} poses in 16 frames",
            poses.len()
        );
    }

    #[test]
    fn scripts_can_set_parameters_and_force_a_state() {
        let (skeletons, clips) = assets();
        let (world, entity) = world_with_actor();
        let assets = AnimationAssets {
            skeletons: &skeletons,
            clips: &clips,
            poses: &[0],
            instance_poses: &[0],
        };
        let mut actors =
            AnimatedActors::load(&files(), &assets, &[(entity, "a.anim".to_owned(), 0)]).unwrap();
        actors.apply(&[AnimCommand {
            actor: entity,
            param: None,
            state: Some(("walking".to_owned(), 0.0)),
        }]);
        assert_eq!(actors.state_of(entity), Some("walking"));
        let mut notifies = Vec::new();
        actors.step(&world, &mut notifies);
        // The `speed` stays 0, so the transition back takes it to idle.
        assert_eq!(actors.state_of(entity), Some("idle"));
    }

    #[test]
    fn an_actor_with_no_skeleton_or_no_clip_is_explained() {
        let (skeletons, _) = assets();
        let (_, entity) = world_with_actor();
        let empty = AnimationAssets {
            skeletons: &skeletons,
            clips: &[],
            poses: &[0],
            instance_poses: &[NO_POSE],
        };
        let errors = AnimatedActors::load(&files(), &empty, &[(entity, "a.anim".to_owned(), 0)])
            .err()
            .unwrap();
        assert!(errors[0].contains("no skeleton"), "{errors:?}");
        let no_clips = AnimationAssets {
            instance_poses: &[0],
            ..empty
        };
        let errors = AnimatedActors::load(&files(), &no_clips, &[(entity, "a.anim".to_owned(), 0)])
            .err()
            .unwrap();
        assert!(errors[0].contains("no clip `idle`"), "{errors:?}");
    }
}
