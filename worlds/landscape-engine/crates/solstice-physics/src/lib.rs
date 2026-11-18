//! Physics for gameplay (docs/gameplay.md, Physics): Rapier, in a [`solstice_world::World`].
//!
//! * The scene's collision, baked into its file, is the fixed world: see [`Physics::add_scene`].
//! * An actor collides with an [`ActorCollider`]. By default it is kinematic: it goes where its
//!   `Transform` says, pushing what is in the way. It can instead be dynamic, moved by physics, or
//!   a sensor, which only notices what enters and leaves it, as [`Overlap`] events.
//! * A [`Character`] is walked by a kinematic character controller: it follows the ground, climbs
//!   steps and gentle slopes, and is stopped by walls. It goes where [`Walk`] says.
//!
//! * A [`Car`] is driven by `solstice-vehicle`'s dynamics on the scene's collision, with a body
//!   here that hits things: see [`car`].
//!
//! Each runs in the fixed steps, so a recorded run replays the same.

pub mod car;

pub use car::{Car, Knock};
pub use solstice_vehicle as vehicle;

use glam::{Mat4, Quat, Vec3};
use rapier3d::control::{CharacterAutostep, CharacterLength, KinematicCharacterController};
use rapier3d::parry::shape::TriMeshFlags;
use rapier3d::parry::utils::Array2;
use rapier3d::prelude::*;
use solstice_asset::collision::MAX_COORDINATE;
use solstice_asset::{Collision, CollisionShape, Heightfield, NO_SHAPE};
use solstice_world::{Entity, Schedule, TickGroup, Transform, Walk, World, STEP_SECONDS};
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;
use std::sync::mpsc::{channel, Receiver};

/// Metres a second squared.
pub const GRAVITY: f32 = 9.81;
/// The steepest slope, in degrees, a character can walk up.
pub const MAX_SLOPE_DEGREES: f32 = 45.0;
/// The highest step, in metres, a character climbs without jumping: a kerb, a verandah's
/// edge, a ledge of rock on a track.
pub const MAX_STEP: f32 = 0.6;
/// How fast a character may fall, in metres a second.
pub const TERMINAL_SPEED: f32 = 50.0;

/// The shape of an actor's collider, centred on the actor unless a capsule, which stands on it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shape {
    /// A box, by its full size in metres.
    Box(Vec3),
    /// A capsule, standing, by its radius and whole height.
    Capsule { radius: f32, height: f32 },
}

/// How an actor's collider behaves.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BodyKind {
    /// Goes where its `Transform` says, pushing what is in the way.
    Kinematic,
    /// Falls and is pushed by physics, weighing this many kilograms.
    Dynamic(f32),
    /// Blocks nothing; reports what enters and leaves it.
    Sensor,
}

/// Gives an actor a collider.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ActorCollider {
    pub shape: Shape,
    pub kind: BodyKind,
    /// Where the shape is, relative to the actor: a door hinged at its origin has its box out
    /// along the door.
    pub offset: Vec3,
}

/// Makes an actor walk: a capsule moved by the character controller. Its `Transform` is at its
/// feet.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Character {
    pub radius: f32,
    pub height: f32,
}

/// Something entered or left a sensor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Overlap {
    pub sensor: Entity,
    pub other: Entity,
    pub entered: bool,
}

/// What physics knows of an actor it has taken in.
struct Body {
    handle: RigidBodyHandle,
    kind: BodyKind,
    character: Option<CharacterState>,
    /// Where physics last put it, to tell when something else has moved it.
    at: Vec3,
    rotation: Quat,
    /// Its colliders, to forget when it goes.
    colliders: Vec<ColliderHandle>,
}

struct CharacterState {
    collider: ColliderHandle,
    /// Falling speed, negative downward.
    vertical: f32,
    /// Whether it stood on something at the end of the last step: only then can it jump.
    grounded: bool,
}

/// The simulation of one world.
pub struct Physics {
    bodies: RigidBodySet,
    colliders: ColliderSet,
    impulse_joints: ImpulseJointSet,
    multibody_joints: MultibodyJointSet,
    soft_bodies: SoftBodySet,
    pipeline: PhysicsPipeline,
    islands: IslandManager,
    broad_phase: DefaultBroadPhase,
    narrow_phase: NarrowPhase,
    ccd: CCDSolver,
    params: IntegrationParameters,
    controller: KinematicCharacterController,
    /// In a fixed order, so that a run comes out the same every time.
    actors: BTreeMap<Entity, Body>,
    /// Each actor's collider: whose it is, and whether it is a sensor. Kept until the step after
    /// its actor goes, as a collider that has gone can still report that it stopped touching.
    owners: HashMap<ColliderHandle, (Entity, bool)>,
    /// The colliders of actors gone this step, to forget from `owners` once reported.
    forgotten: Vec<ColliderHandle>,
    events: ChannelEventCollector,
    collisions: Receiver<CollisionEvent>,
    _forces: Receiver<ContactForceEvent>,
    _tears: Receiver<SoftBodyTearEvent>,
    /// Each car's step, in a fixed order.
    cars: BTreeMap<Entity, car::CarStep>,
    /// What the ground is made of at a place, for tyres.
    surfaces: Option<Box<dyn Fn(Vec3) -> solstice_vehicle::Surface>>,
    /// What tyres have pressed into the ground: the ruts they stand in, which stay.
    ruts: std::cell::RefCell<solstice_vehicle::Ruts>,
    /// Problems with the scene's shapes, for the viewer to show.
    pub warnings: Vec<String>,
}

fn to_vector(v: Vec3) -> Vector {
    Vector::new(v.x, v.y, v.z)
}

fn from_vector(v: Vector) -> Vec3 {
    Vec3::new(v.x, v.y, v.z)
}

fn to_pose(position: Vec3, rotation: Quat) -> Pose {
    let mut pose = Pose::from_translation(to_vector(position));
    pose.rotation = Rotation::from_xyzw(rotation.x, rotation.y, rotation.z, rotation.w);
    pose
}

fn entity_data(entity: Entity) -> u128 {
    u128::from(entity.to_bits().get())
}

impl Default for Physics {
    fn default() -> Self {
        Self::new()
    }
}

impl Physics {
    pub fn new() -> Self {
        let (collision_sender, collisions) = channel();
        let (force_sender, forces) = channel();
        let (tear_sender, tears) = channel();
        let params = IntegrationParameters {
            dt: STEP_SECONDS as f32,
            ..IntegrationParameters::default()
        };
        let controller = KinematicCharacterController {
            up: Vector::Y,
            offset: CharacterLength::Absolute(0.02),
            slide: true,
            autostep: Some(CharacterAutostep {
                max_height: CharacterLength::Absolute(MAX_STEP),
                min_width: CharacterLength::Absolute(0.1),
                include_dynamic_bodies: false,
            }),
            max_slope_climb_angle: MAX_SLOPE_DEGREES.to_radians(),
            min_slope_slide_angle: (MAX_SLOPE_DEGREES + 5.0).to_radians(),
            snap_to_ground: Some(CharacterLength::Absolute(0.2)),
            ..KinematicCharacterController::default()
        };
        Self {
            bodies: RigidBodySet::new(),
            colliders: ColliderSet::new(),
            impulse_joints: ImpulseJointSet::new(),
            multibody_joints: MultibodyJointSet::new(),
            soft_bodies: SoftBodySet::new(),
            pipeline: PhysicsPipeline::new(),
            islands: IslandManager::new(),
            broad_phase: DefaultBroadPhase::new(),
            narrow_phase: NarrowPhase::new(),
            ccd: CCDSolver::new(),
            params,
            controller,
            actors: BTreeMap::new(),
            owners: HashMap::new(),
            forgotten: Vec::new(),
            events: ChannelEventCollector::new(collision_sender, force_sender, tear_sender),
            collisions,
            _forces: forces,
            _tears: tears,
            cars: BTreeMap::new(),
            surfaces: None,
            ruts: Default::default(),
            warnings: Vec::new(),
        }
    }

    /// Adds a scene's collision as the fixed world. `instance` gives where each of its instances
    /// is. A shape is built once for each size it is used at, and shared.
    pub fn add_scene(&mut self, collision: &Collision, instance: impl Fn(u32) -> Option<Mat4>) {
        let mut built: HashMap<(u32, [u32; 3]), Option<SharedShape>> = HashMap::new();
        for (index, &shape) in collision.instance_shapes.iter().enumerate() {
            if shape == NO_SHAPE {
                continue;
            }
            let Some(transform) = instance(index as u32) else {
                self.warnings.push(format!(
                    "collision names instance {index}, which isn't there"
                ));
                continue;
            };
            let (scale, rotation, translation) = transform.to_scale_rotation_translation();
            let sound = scale.is_finite()
                && scale.abs().min_element() > 1.0e-6
                && rotation.is_finite()
                && translation.is_finite();
            if !sound {
                self.warnings.push(format!(
                    "instance {index} can't collide: its transform is degenerate"
                ));
                continue;
            }
            let key = (shape, scale_key(scale));
            let warnings = &mut self.warnings;
            let built = built
                .entry(key)
                .or_insert_with(|| build_shape(&collision.shapes[shape as usize], scale, warnings));
            if let Some(shape) = built {
                self.colliders.insert(
                    ColliderBuilder::new(shape.clone()).position(to_pose(translation, rotation)),
                );
            }
        }
        if let Some(ground) = &collision.heightfield {
            self.add_ground(ground);
        }
    }

    fn add_ground(&mut self, ground: &Heightfield) {
        let (rows, columns) = (ground.rows as usize, ground.columns as usize);
        let mut heights = Array2::new(rows, columns, vec![0.0; rows * columns]);
        for row in 0..rows {
            for column in 0..columns {
                heights[(row, column)] = ground.heights[row * columns + column];
            }
        }
        let size = Vec3::new(
            ground.spacing[0] * (columns - 1) as f32,
            1.0,
            ground.spacing[1] * (rows - 1) as f32,
        );
        // Rapier's heightfield is centred on its position.
        let centre = Vec3::new(
            ground.origin[0] + size.x * 0.5,
            0.0,
            ground.origin[1] + size.z * 0.5,
        );
        self.colliders.insert(
            ColliderBuilder::new(SharedShape::heightfield(heights, to_vector(size)))
                .translation(to_vector(centre)),
        );
    }

    /// Adds an invisible wall: a box of this full size, centred here, turned by `yaw` radians.
    pub fn add_blocker(&mut self, centre: Vec3, size: Vec3, yaw: f32) {
        if !reachable(centre) || !sound_size(size) || !yaw.is_finite() {
            self.warnings.push(format!(
                "a blocker at {centre} of size {size} is out of bounds, and was left out"
            ));
            return;
        }
        let half = size * 0.5;
        self.colliders.insert(
            ColliderBuilder::cuboid(half.x, half.y, half.z)
                .position(to_pose(centre, Quat::from_rotation_y(yaw))),
        );
    }

    /// Adds the system that runs physics each step, in the `Physics` tick group.
    pub fn install(physics: &Rc<RefCell<Self>>, schedule: &mut Schedule) {
        let physics = physics.clone();
        schedule.add(TickGroup::Physics, "physics", move |world| {
            physics.borrow_mut().step(world);
        });
    }

    /// How many distinct shapes the scene's colliders use: for tools and tests.
    pub fn distinct_shapes(&self) -> usize {
        let mut shapes: Vec<*const ()> = self
            .colliders
            .iter()
            .map(|(_, c)| std::sync::Arc::as_ptr(&c.shared_shape().0).cast())
            .collect();
        shapes.sort_unstable();
        shapes.dedup();
        shapes.len()
    }

    /// How many colliders of actors physics is keeping track of: for tools and tests.
    pub fn tracked_colliders(&self) -> usize {
        self.owners.len()
    }

    /// How many colliders there are: for tools and tests.
    pub fn collider_count(&self) -> usize {
        self.colliders.len()
    }

    /// The height of what a ray straight down from `from` hits first: for tools and tests.
    pub fn ground_below(&self, from: Vec3) -> Option<f32> {
        let queries = self.broad_phase.as_query_pipeline(
            self.narrow_phase.query_dispatcher(),
            &self.bodies,
            &self.colliders,
            QueryFilter::new().exclude_sensors(),
        );
        let ray = Ray::new(to_vector(from), -Vector::Y);
        queries
            .cast_ray(&ray, 10_000.0, true)
            .map(|(_, distance)| from.y - distance)
    }

    /// The height of whatever of the scene itself is under a place: its land, its floors and
    /// its walls, and nothing that moves, a car or an actor or a thing lying about. For what
    /// falls off a car to come to rest on. From inside something solid, as what comes off
    /// against a wall is, it is what that stands on.
    pub fn fixed_below(&self, from: Vec3) -> Option<f32> {
        let queries = self.broad_phase.as_query_pipeline(
            self.narrow_phase.query_dispatcher(),
            &self.bodies,
            &self.colliders,
            QueryFilter::only_fixed().exclude_sensors(),
        );
        let ray = Ray::new(to_vector(from), -Vector::Y);
        queries
            .cast_ray(&ray, 10_000.0, false)
            .map(|(_, distance)| from.y - distance)
    }

    /// The height of the land itself under a place, if the scene has a grid of heights:
    /// not of a wall, a tree or a car that stands on it. For a camera to keep above.
    pub fn land_below(&self, from: Vec3) -> Option<f32> {
        let land =
            |_: ColliderHandle, collider: &Collider| collider.shape().as_heightfield().is_some();
        let queries = self.broad_phase.as_query_pipeline(
            self.narrow_phase.query_dispatcher(),
            &self.bodies,
            &self.colliders,
            QueryFilter::new().exclude_sensors().predicate(&land),
        );
        let ray = Ray::new(to_vector(from), -Vector::Y);
        queries
            .cast_ray(&ray, 10_000.0, true)
            .map(|(_, distance)| from.y - distance)
    }

    /// What tyres have pressed into the ground, which every car's wheels read and press
    /// further (`solstice_vehicle::rut`): for whoever draws it.
    pub fn ruts(&self) -> std::cell::Ref<'_, solstice_vehicle::Ruts> {
        self.ruts.borrow()
    }

    /// The same, to change: to forget every rut, or to lay what was thrown where it landed.
    pub fn ruts_mut(&mut self) -> &mut solstice_vehicle::Ruts {
        self.ruts.get_mut()
    }

    /// One fixed step: takes in new actors, moves kinematic ones to where their transforms say,
    /// steps the simulation, walks the characters, and writes back what physics moved.
    pub fn step(&mut self, world: &mut World) {
        self.take_in_actors(world);
        self.take_in_cars(world);
        self.follow_transforms(world);
        self.drive_cars(world);
        self.ruts
            .get_mut()
            .advance(solstice_world::STEP_SECONDS as f32);
        self.pipeline.step(
            to_vector(Vec3::new(0.0, -GRAVITY, 0.0)),
            &self.params,
            &mut self.islands,
            &mut self.broad_phase,
            &mut self.narrow_phase,
            &mut self.bodies,
            &mut self.colliders,
            &mut self.impulse_joints,
            &mut self.multibody_joints,
            &mut self.soft_bodies,
            &mut self.ccd,
            &(),
            &self.events,
        );
        self.settle_cars();
        self.walk_characters(world);
        self.write_back(world);
        self.report_overlaps(world);
    }

    /// Gives a body to each new actor with a collider or a character, and forgets those gone.
    fn take_in_actors(&mut self, world: &mut World) {
        let gone: Vec<Entity> = self
            .actors
            .keys()
            .filter(|&&e| !world.entities.contains(e))
            .copied()
            .collect();
        for entity in gone {
            if let Some(body) = self.actors.remove(&entity) {
                self.forgotten.extend(body.colliders.iter().copied());
                self.bodies.remove(
                    body.handle,
                    &mut self.islands,
                    &mut self.colliders,
                    &mut self.impulse_joints,
                    &mut self.multibody_joints,
                    &mut self.soft_bodies,
                    true,
                );
            }
        }
        let new: Vec<(Entity, Transform, Option<ActorCollider>, Option<Character>)> = world
            .entities
            .query::<(
                Entity,
                &Transform,
                Option<&ActorCollider>,
                Option<&Character>,
            )>()
            .iter()
            .filter(|(e, _, collider, character)| {
                (collider.is_some() || character.is_some()) && !self.actors.contains_key(e)
            })
            .map(|(e, t, collider, character)| (e, *t, collider.copied(), character.copied()))
            .collect();
        for (entity, transform, collider, character) in new {
            if sound_actor(&transform, collider.as_ref(), character.as_ref()) {
                self.add_actor(entity, &transform, collider, character);
            } else {
                self.warnings.push(format!(
                    "an actor at {} is out of bounds, and collides with nothing",
                    transform.translation
                ));
                // So that it isn't looked at again.
                let _ = world.entities.remove_one::<ActorCollider>(entity);
                let _ = world.entities.remove_one::<Character>(entity);
            }
        }
    }

    fn add_actor(
        &mut self,
        entity: Entity,
        transform: &Transform,
        collider: Option<ActorCollider>,
        character: Option<Character>,
    ) {
        let kind = collider.map_or(BodyKind::Kinematic, |c| c.kind);
        let builder = match kind {
            // What is loose may be small and heavy, and is carried on trucks: it is not to
            // pass through a floor in a step, and it comes to rest where it is put.
            BodyKind::Dynamic(_) => RigidBodyBuilder::dynamic()
                .ccd_enabled(true)
                .linear_damping(0.05)
                .angular_damping(0.6),
            _ => RigidBodyBuilder::kinematic_position_based(),
        };
        let handle = self.bodies.insert(
            builder
                .pose(to_pose(transform.translation, transform.rotation))
                .user_data(entity_data(entity)),
        );
        let attach = |shape: ColliderBuilder, kind: BodyKind| {
            let mut shape = shape.user_data(entity_data(entity));
            if kind == BodyKind::Sensor {
                // Noticing what moves: not the fixed scene it may stand in.
                shape = shape
                    .sensor(true)
                    .active_events(ActiveEvents::COLLISION_EVENTS)
                    .active_collision_types(
                        ActiveCollisionTypes::KINEMATIC_KINEMATIC
                            | ActiveCollisionTypes::DYNAMIC_KINEMATIC,
                    );
            }
            if let BodyKind::Dynamic(mass) = kind {
                shape = shape.mass(mass).friction(0.9).restitution(0.0);
            }
            shape
        };
        let mut colliders = Vec::new();
        if let Some(collider) = collider {
            let shape = match collider.shape {
                Shape::Box(size) => {
                    let half = size * 0.5;
                    ColliderBuilder::cuboid(half.x, half.y, half.z)
                        .translation(to_vector(collider.offset))
                }
                Shape::Capsule { radius, height } => {
                    // Stands on the actor's origin.
                    let half = (height * 0.5 - radius).max(0.0);
                    ColliderBuilder::capsule_y(half, radius).translation(to_vector(
                        collider.offset + Vec3::new(0.0, half + radius, 0.0),
                    ))
                }
            };
            let inserted =
                self.colliders
                    .insert_with_parent(attach(shape, kind), handle, &mut self.bodies);
            self.owners
                .insert(inserted, (entity, kind == BodyKind::Sensor));
            colliders.push(inserted);
        }
        let character = character.map(|character| {
            let half = (character.height * 0.5 - character.radius).max(0.0);
            let centre = Vector::new(0.0, half + character.radius, 0.0);
            let collider = self.colliders.insert_with_parent(
                ColliderBuilder::capsule_y(half, character.radius)
                    .translation(centre)
                    .user_data(entity_data(entity)),
                handle,
                &mut self.bodies,
            );
            self.owners.insert(collider, (entity, false));
            colliders.push(collider);
            CharacterState {
                collider,
                vertical: 0.0,
                grounded: false,
            }
        });
        self.actors.insert(
            entity,
            Body {
                handle,
                kind,
                character,
                at: transform.translation,
                rotation: transform.rotation,
                colliders,
            },
        );
    }

    /// Moves kinematic actors to where their transforms say. A character or dynamic actor moved
    /// by something other than physics, such as a respawn, is put there at once. Those that
    /// haven't moved cost nothing.
    fn follow_transforms(&mut self, world: &World) {
        for (&entity, body) in &mut self.actors {
            let Ok(transform) = world.entities.get::<&Transform>(entity) else {
                continue;
            };
            let moved = transform.translation != body.at;
            let turned = transform.rotation != body.rotation;
            if !moved && !turned {
                continue;
            }
            let Some(rigid) = self.bodies.get_mut(body.handle) else {
                continue;
            };
            body.at = transform.translation;
            body.rotation = transform.rotation;
            if let Some(character) = &mut body.character {
                if moved {
                    rigid.set_translation(to_vector(transform.translation), true);
                    character.vertical = 0.0;
                }
                continue;
            }
            let pose = to_pose(transform.translation, transform.rotation);
            match body.kind {
                BodyKind::Dynamic(_) => {
                    rigid.set_position(pose, true);
                    rigid.set_linvel(Vector::ZERO, true);
                    rigid.set_angvel(Vector::ZERO, true);
                }
                BodyKind::Kinematic | BodyKind::Sensor => rigid.set_next_kinematic_position(pose),
            }
        }
    }

    fn walk_characters(&mut self, world: &World) {
        let dt = STEP_SECONDS as f32;
        let walkers: Vec<Entity> = self
            .actors
            .iter()
            .filter(|(_, b)| b.character.is_some())
            .map(|(e, _)| *e)
            .collect();
        for entity in walkers {
            let body = self.actors.get_mut(&entity).expect("listed just now");
            let state = body.character.as_mut().expect("filtered on it");
            let Some(rigid) = self.bodies.get(body.handle) else {
                continue;
            };
            let collider = &self.colliders[state.collider];
            let shape = collider.shared_shape().clone();
            let pose = *collider.position();
            let wanted = world
                .entities
                .get::<&Walk>(entity)
                .map_or(Vec3::ZERO, |w| w.0);
            let walk = Vec3::new(wanted.x, 0.0, wanted.z);
            // A jump: what a walk says of going up is the speed to leave the ground with,
            // and is heeded only on the ground.
            if wanted.y > 0.0 && state.grounded {
                state.vertical = wanted.y;
            }
            let falling = (state.vertical - GRAVITY * dt).max(-TERMINAL_SPEED);
            // Going up, it is not held down to the ground it has just left.
            let mut controller = self.controller;
            if falling > 0.0 {
                controller.snap_to_ground = None;
            }
            let desired = walk * dt + Vec3::Y * falling * dt;
            let filter = QueryFilter::new()
                .exclude_sensors()
                .exclude_rigid_body(body.handle);
            let queries = self.broad_phase.as_query_pipeline(
                self.narrow_phase.query_dispatcher(),
                &self.bodies,
                &self.colliders,
                filter,
            );
            let moved =
                controller.move_shape(dt, &queries, &*shape, &pose, to_vector(desired), |_| {});
            state.grounded = moved.grounded && falling <= 0.0;
            state.vertical = if state.grounded { 0.0 } else { falling };
            let next = rigid.translation() + moved.translation;
            self.bodies
                .get_mut(body.handle)
                .expect("found above")
                .set_next_kinematic_translation(next);
            body.at = from_vector(next);
        }
    }

    /// Puts a walking character somewhere else: it is not walked there, it is there.
    pub fn place(&mut self, entity: Entity, at: Vec3) {
        let Some(body) = self.actors.get_mut(&entity) else {
            return;
        };
        if let Some(rigid) = self.bodies.get_mut(body.handle) {
            rigid.set_translation(to_vector(at), true);
            rigid.set_next_kinematic_translation(to_vector(at));
        }
        body.at = at;
        if let Some(state) = body.character.as_mut() {
            state.vertical = 0.0;
        }
    }

    /// Writes back to their transforms what physics moved.
    fn write_back(&mut self, world: &mut World) {
        for (&entity, body) in &mut self.actors {
            let Ok(mut transform) = world.entities.get::<&mut Transform>(entity) else {
                continue;
            };
            let Some(rigid) = self.bodies.get(body.handle) else {
                continue;
            };
            if body.character.is_some() {
                transform.translation = body.at;
            } else if matches!(body.kind, BodyKind::Dynamic(_)) {
                let translation = from_vector(rigid.translation());
                let rotation = rigid.rotation();
                transform.translation = translation;
                transform.rotation =
                    Quat::from_xyzw(rotation.x, rotation.y, rotation.z, rotation.w);
                body.at = translation;
                body.rotation = transform.rotation;
            }
        }
    }

    /// Reports what entered and left each sensor, as [`Overlap`] events.
    fn report_overlaps(&mut self, world: &mut World) {
        while let Ok(event) = self.collisions.try_recv() {
            let (first, second, entered) = match event {
                CollisionEvent::Started(a, b, _) => (a, b, true),
                CollisionEvent::Stopped(a, b, _) => (a, b, false),
            };
            // Through `owners`, as a collider that has just gone can still report this.
            let (Some(&first), Some(&second)) = (self.owners.get(&first), self.owners.get(&second))
            else {
                continue;
            };
            for (sensor, other) in [(first, second), (second, first)] {
                if sensor.1 {
                    world.send(Overlap {
                        sensor: sensor.0,
                        other: other.0,
                        entered,
                    });
                }
            }
        }
        for collider in self.forgotten.drain(..) {
            self.owners.remove(&collider);
        }
    }
}

/// Whether physics can work with a position this far out.
fn reachable(position: Vec3) -> bool {
    position.is_finite() && position.abs().max_element() <= MAX_COORDINATE
}

/// Whether physics can build something this size.
fn sound_size(size: Vec3) -> bool {
    reachable(size) && size.min_element() > 0.0
}

/// Whether physics can build this actor. A big enough number panics inside Rapier, and these come
/// from files.
fn sound_actor(
    transform: &Transform,
    collider: Option<&ActorCollider>,
    character: Option<&Character>,
) -> bool {
    let collider_sound = collider.is_none_or(|collider| {
        let shape = match collider.shape {
            Shape::Box(size) => sound_size(size),
            Shape::Capsule { radius, height } => sound_size(Vec3::new(radius, height, radius)),
        };
        let mass = match collider.kind {
            BodyKind::Dynamic(mass) => mass.is_finite() && mass > 0.0 && mass <= MAX_COORDINATE,
            _ => true,
        };
        shape && mass && reachable(collider.offset)
    });
    let character_sound = character.is_none_or(|character| {
        sound_size(Vec3::new(
            character.radius,
            character.height,
            character.radius,
        ))
    });
    reachable(transform.translation)
        && transform.rotation.is_finite()
        && collider_sound
        && character_sound
}

/// The same for a scale at any turn, as it differs in its last bits from one to another, and
/// different for sizes that differ by more than a part in a few thousand, however small they are.
fn scale_key(scale: Vec3) -> [u32; 3] {
    /// How many low bits of a float's 23 to ignore, rounding to the nearest.
    const IGNORED: u32 = 11;
    scale
        .to_array()
        .map(|s| s.to_bits().wrapping_add(1 << (IGNORED - 1)) >> IGNORED)
}

/// Builds a shape at a size, or says why it can't be.
fn build_shape(
    shape: &CollisionShape,
    scale: Vec3,
    warnings: &mut Vec<String>,
) -> Option<SharedShape> {
    let vertices: Vec<Vector> = shape
        .vertices
        .iter()
        .map(|v| to_vector(Vec3::from_array(*v) * scale))
        .collect();
    let out_of_range = vertices
        .iter()
        .any(|v| !v.is_finite() || v.abs().max_element() > MAX_COORDINATE);
    if out_of_range {
        warnings.push("a collision shape reaches too far out, and was left out".to_owned());
        return None;
    }
    if shape.hull {
        let built = SharedShape::convex_hull(&vertices);
        if built.is_none() {
            warnings.push(format!("no convex hull of {} points", vertices.len()));
        }
        return built;
    }
    // Two-sided: scanned meshes are often open behind, a body can touch them from there, and a
    // mirrored instance turns its triangles inside out.
    match SharedShape::trimesh_with_flags(
        vertices,
        shape.triangles.clone(),
        TriMeshFlags::FIX_INTERNAL_EDGES_TWO_SIDED,
    ) {
        Ok(built) => Some(built),
        Err(error) => {
            warnings.push(format!("a collision mesh was refused: {error:?}"));
            None
        }
    }
}
