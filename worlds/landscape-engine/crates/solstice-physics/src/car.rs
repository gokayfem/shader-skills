//! Cars: `solstice-vehicle`'s dynamics, on the scene's collision, among everything else that
//! collides.
//!
//! A car is two things at once. Its wheels, springs, engine and tyres are `solstice-vehicle`'s,
//! which steps them many times a step against rays cast at the ground. Its body is also a rigid
//! body here, a box, so that it hits walls and other cars as anything else does.
//!
//! Each step, the car's own dynamics run first, from where the body is, and say where the step
//! ends. The body is then given the velocity that carries it exactly there, and the physics
//! step moves it and deals with whatever it hits. What that changed in its velocity, a wall
//! pushing back or another car, is added to the velocity the car's dynamics ended with.

use crate::{from_vector, to_pose, to_vector, Physics, GRAVITY};
use glam::{Quat, Vec3};
use rapier3d::prelude::*;
use solstice_vehicle::{Asked, Contact, Controls, Driver, Ground, Press, Ruts, Surface, Vehicle};
use solstice_world::{Entity, Transform, World, STEP_SECONDS};
use std::cell::RefCell;

/// Makes an actor a car. Its `Transform` is the car's origin: on the ground under its middle.
#[derive(Clone, Debug)]
pub struct Car {
    pub vehicle: Vehicle,
    /// What turns what is asked into the wheel and pedals: easing, and the automatic gearbox.
    pub driver: Driver,
    /// What its driver is asking for, set each step by whatever drives it.
    pub asked: Asked,
    /// What the wheel and pedals did in the latest step, for instruments to show.
    pub controls: Controls,
}

impl Car {
    pub fn new(vehicle: Vehicle, automatic: bool) -> Self {
        Self {
            vehicle,
            driver: Driver::new(automatic),
            asked: Asked::default(),
            controls: Controls::default(),
        }
    }
}

/// A car's step, between its own dynamics and the physics step.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct CarStep {
    /// The velocity and spin its dynamics ended the step with.
    velocity: Vec3,
    spin: Vec3,
    /// The velocity and spin its body was given, to carry it to where the step ends.
    carried: Vec3,
    turned: Vec3,
    /// Whether its dynamics have once failed, and been said to.
    failed: bool,
    /// What hitting things did to it in the step: see [`Knock`].
    knock: Knock,
}

/// What hitting things did to a car in one step, in the world: how much it changed the
/// velocity of its centre of mass, in metres a second, and its spin, in radians a second.
/// Nothing of it is the car's own doing: its wheels, springs and engine, and gravity, are
/// its dynamics', and this is what the walls, the ground under its body and whatever else
/// it met added to them. Times its mass and its inertia, it is the blow's impulse.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Knock {
    pub velocity: Vec3,
    pub spin: Vec3,
}

/// The scene, as a wheel finds it.
struct SceneGround<'a> {
    queries: QueryPipeline<'a>,
    surfaces: Option<&'a dyn Fn(Vec3) -> Surface>,
    /// What has been pressed into the ground: a tyre stands in the ruts kept here, and
    /// presses them deeper.
    ruts: &'a RefCell<Ruts>,
}

/// How near the ground a car's belly must be to drag in it, in metres, and how far down it
/// presses what it drags through.
const BELLY_NEAR: f32 = 0.02;
const BELLY_PRESSES: f32 = 0.025;

/// Where a car's belly is down on soft ground, between its ruts, it drags the crown flat:
/// a broad, shallow smear of churned soil under its middle, front to back.
fn drag_belly(ground: &SceneGround<'_>, vehicle: &Vehicle, body: &solstice_vehicle::Body) {
    let spec = vehicle.spec();
    let (up, forward) = (body.up(), body.forward());
    let along = glam::Vec2::new(forward.x, forward.z).normalize_or_zero();
    if up.y < 0.5 || along == glam::Vec2::ZERO {
        return;
    }
    // The underside of its body, at three places along its middle.
    let under = spec.body_at.y - spec.body_size.y * 0.5;
    const ABOVE: f32 = 0.4;
    for share in [-0.3, 0.0, 0.3] {
        let at = body.position
            + body.rotation * Vec3::new(0.0, under, spec.body_at.z + share * spec.body_size.z);
        let from = at + up * ABOVE;
        let Some(contact) = ground.cast(from, -up, ABOVE + BELLY_NEAR) else {
            continue;
        };
        if !contact.surface.is_soft() {
            continue;
        }
        let point = from - up * contact.distance;
        // How far down the belly is against the ground as it was made, and whether the
        // ground as it now is has already been pressed out of its way.
        let down = ABOVE - contact.distance + BELLY_PRESSES;
        if down <= ground.rut(point) {
            continue;
        }
        let soil = contact.surface.soil(vehicle.wetness());
        let place = glam::Vec2::new(point.x, point.z);
        ground.press(&Press {
            from: place,
            to: place,
            along,
            half_width: spec.body_size.x * 0.3,
            half_length: spec.body_size.z * 0.2,
            depth: down,
            berm: 0.0,
            phase: (0.0, 0.0),
            print: 0.0,
            wet: soil.wet,
            stays: soil.stays,
            churn: soil.churn,
        });
    }
}

impl Ground for SceneGround<'_> {
    fn cast(&self, from: Vec3, direction: Vec3, reach: f32) -> Option<Contact> {
        let ray = Ray::new(to_vector(from), to_vector(direction));
        let (_, hit) = self.queries.cast_ray_and_get_normal(&ray, reach, true)?;
        let mut normal = from_vector(hit.normal);
        // A mesh is met from either side: the ground faces the wheel.
        if normal.dot(direction) > 0.0 {
            normal = -normal;
        }
        let point = from + direction * hit.time_of_impact;
        Some(Contact {
            distance: hit.time_of_impact,
            normal,
            surface: self
                .surfaces
                .map_or(Surface::Asphalt, |surface| surface(point)),
        })
    }

    fn rut(&self, at: Vec3) -> f32 {
        self.ruts.borrow().depth(at.x, at.z)
    }

    fn press(&self, press: &Press) {
        self.ruts.borrow_mut().press(press);
    }
}

/// A box with a hollow cut out of it: the boxes that are left, under the hollow and over
/// it, to each side of it, before it and behind. A box the hollow does not reach is left
/// whole. Each box and the hollow are their least corner and their greatest.
pub(crate) fn hollowed(solid: (Vec3, Vec3), hollow: (Vec3, Vec3)) -> Vec<(Vec3, Vec3)> {
    let (low, high) = solid;
    let (from, to) = (hollow.0.max(low), hollow.1.min(high));
    if from.cmpge(to).any() {
        return vec![solid];
    }
    let left = [
        // Under it and over it, the whole of the box across and along.
        (low, Vec3::new(high.x, from.y, high.z)),
        (Vec3::new(low.x, to.y, low.z), high),
        // To each side of it, as high as it is.
        (
            Vec3::new(low.x, from.y, low.z),
            Vec3::new(from.x, to.y, high.z),
        ),
        (
            Vec3::new(to.x, from.y, low.z),
            Vec3::new(high.x, to.y, high.z),
        ),
        // Before it and behind, as high and as wide as it is.
        (
            Vec3::new(from.x, from.y, low.z),
            Vec3::new(to.x, to.y, from.z),
        ),
        (
            Vec3::new(from.x, from.y, to.z),
            Vec3::new(to.x, to.y, high.z),
        ),
    ];
    left.into_iter()
        .filter(|(low, high)| (*high - *low).min_element() > 1e-3)
        .collect()
}

impl Physics {
    /// Says what the ground is made of at each place, for tyres. Without it, all is asphalt.
    pub fn set_surfaces(&mut self, surfaces: impl Fn(Vec3) -> Surface + 'static) {
        self.surfaces = Some(Box::new(surfaces));
    }

    /// Gives a body to each new car.
    pub(crate) fn take_in_cars(&mut self, world: &mut World) {
        let new: Vec<(Entity, Transform)> = world
            .entities
            .query::<(Entity, &Transform, &Car)>()
            .iter()
            .filter(|(entity, _, _)| !self.actors.contains_key(entity))
            .map(|(entity, transform, _)| (entity, *transform))
            .collect();
        for (entity, transform) in new {
            let sound = crate::reachable(transform.translation) && transform.rotation.is_finite();
            if !sound {
                self.warnings.push(format!(
                    "a car at {} is out of bounds, and was left out",
                    transform.translation
                ));
                let _ = world.entities.remove_one::<Car>(entity);
                continue;
            }
            let Ok(car) = world.entities.get::<&Car>(entity) else {
                continue;
            };
            let spec = car.vehicle.spec();
            // Its mass and inertia are the car's own, not a box's: the collider weighs nothing.
            let mass =
                MassProperties::new(to_vector(spec.centre), spec.mass, to_vector(spec.inertia));
            let handle = self.bodies.insert(
                RigidBodyBuilder::dynamic()
                    .pose(to_pose(transform.translation, transform.rotation))
                    .additional_mass_properties(mass)
                    // Gravity is among the forces the car's own dynamics sum.
                    .gravity_scale(0.0)
                    .linear_damping(0.0)
                    .angular_damping(0.0)
                    .can_sleep(false)
                    // At 80 metres a second a car crosses a barrier's thickness in a step.
                    .ccd_enabled(true)
                    .user_data(crate::entity_data(entity)),
            );
            // Rounded, so that it slides along a wall's joins rather than catching on them.
            const ROUNDING: f32 = 0.08;
            // Its boxes: one, or, with a tray, the cab and what is under and round the tray.
            // Each its middle, its half size, and whether it is rounded.
            let (low, high) = (
                spec.body_at - spec.body_size * 0.5,
                spec.body_at + spec.body_size * 0.5,
            );
            let boxed = |low: Vec3, high: Vec3, round: bool| {
                ((low + high) * 0.5, (high - low) * 0.5, round)
            };
            let boxes = match spec.tray {
                None => vec![boxed(low, high, true)],
                Some((size, floor)) => {
                    // The tray's inside: from, and to. Its sides are the body's own, its
                    // front is the cab's back, and its tailgate is as thick as a hand.
                    const GATE: f32 = 0.1;
                    let inside_low = floor - Vec3::new(size.x, 0.0, size.z) * 0.5;
                    let inside_high = floor + Vec3::new(size.x * 0.5, size.y, size.z * 0.5);
                    let back = inside_high.z + GATE;
                    let (left, right) = (
                        low.x.min(inside_low.x - GATE),
                        high.x.max(inside_high.x + GATE),
                    );
                    vec![
                        boxed(low, Vec3::new(high.x, high.y, inside_low.z), true),
                        // Under its floor.
                        boxed(
                            Vec3::new(low.x, low.y.min(floor.y - GATE), inside_low.z),
                            Vec3::new(high.x, floor.y, back),
                            false,
                        ),
                        // Its two sides, and its tailgate.
                        boxed(
                            Vec3::new(left, floor.y, inside_low.z),
                            Vec3::new(inside_low.x, inside_high.y, back),
                            false,
                        ),
                        boxed(
                            Vec3::new(inside_high.x, floor.y, inside_low.z),
                            Vec3::new(right, inside_high.y, back),
                            false,
                        ),
                        boxed(
                            Vec3::new(inside_low.x, floor.y, inside_high.z),
                            Vec3::new(inside_high.x, inside_high.y, back),
                            false,
                        ),
                    ]
                }
            };
            // Where it has holds it is hollow: each is cut out of whatever boxes it is in,
            // with a wall left round it, so that what is put in one is shut in.
            let boxes = if spec.holds.is_empty() {
                boxes
            } else {
                const WALL: f32 = 0.04;
                let mut solid: Vec<(Vec3, Vec3)> = boxes
                    .iter()
                    .map(|(middle, half, _)| (*middle - *half, *middle + *half))
                    .collect();
                for (size, floor) in &spec.holds {
                    let hollow = (
                        (*floor - Vec3::new(size.x, 0.0, size.z) * 0.5).max(low + WALL),
                        (*floor + Vec3::new(size.x * 0.5, size.y, size.z * 0.5)).min(high - WALL),
                    );
                    solid = solid
                        .into_iter()
                        .flat_map(|solid| hollowed(solid, hollow))
                        .collect();
                }
                solid
                    .into_iter()
                    .map(|(low, high)| boxed(low, high, false))
                    .collect()
            };
            let mut colliders = Vec::new();
            for (middle, half, round) in boxes {
                let shape = if round {
                    let half = (half - Vec3::splat(ROUNDING)).max(Vec3::splat(0.01));
                    ColliderBuilder::round_cuboid(half.x, half.y, half.z, ROUNDING)
                } else {
                    ColliderBuilder::cuboid(half.x, half.y, half.z)
                };
                let collider = self.colliders.insert_with_parent(
                    shape
                        .translation(to_vector(middle))
                        .density(0.0)
                        // Two tonnes and more of truck lands and stays; a racing car's shell
                        // skips off what it touches.
                        .friction(if spec.mass > 2000.0 { 0.55 } else { 0.35 })
                        .restitution(if spec.mass > 2000.0 { 0.02 } else { 0.15 })
                        .user_data(crate::entity_data(entity)),
                    handle,
                    &mut self.bodies,
                );
                self.owners.insert(collider, (entity, false));
                colliders.push(collider);
            }
            self.actors.insert(
                entity,
                crate::Body {
                    handle,
                    kind: crate::BodyKind::Dynamic(spec.mass),
                    character: None,
                    at: transform.translation,
                    rotation: transform.rotation,
                    colliders,
                },
            );
            self.cars.insert(entity, CarStep::default());
        }
        self.cars
            .retain(|entity, _| self.actors.contains_key(entity));
    }

    /// Runs each car's own dynamics for the step, and gives its body the velocity that carries
    /// it to where they end.
    pub(crate) fn drive_cars(&mut self, world: &mut World) {
        let dt = STEP_SECONDS as f32;
        let entities: Vec<Entity> = self.cars.keys().copied().collect();
        for entity in entities {
            let Some(handle) = self.actors.get(&entity).map(|actor| actor.handle) else {
                continue;
            };
            let Ok(mut car) = world.entities.get::<&mut Car>(entity) else {
                continue;
            };
            let car = &mut *car;
            let Some(rigid) = self.bodies.get(handle) else {
                continue;
            };
            let rotation = rigid.rotation();
            let before = solstice_vehicle::Body {
                position: from_vector(rigid.translation()),
                rotation: Quat::from_xyzw(rotation.x, rotation.y, rotation.z, rotation.w),
                velocity: from_vector(rigid.linvel()),
                spin: from_vector(rigid.angvel()),
            };
            let mut body = before;
            {
                let filter = QueryFilter::new()
                    .exclude_sensors()
                    .exclude_rigid_body(handle);
                let ground = SceneGround {
                    queries: self.broad_phase.as_query_pipeline(
                        self.narrow_phase.query_dispatcher(),
                        &self.bodies,
                        &self.colliders,
                        filter,
                    ),
                    surfaces: self.surfaces.as_deref(),
                    ruts: &self.ruts,
                };
                let controls = car
                    .driver
                    .drive(&car.asked, &car.vehicle, &body, GRAVITY, dt);
                car.vehicle.step(&mut body, &controls, &ground, dt);
                car.controls = controls;
                drag_belly(&ground, &car.vehicle, &body);
            }
            if !(body.position.is_finite() && body.velocity.is_finite() && body.spin.is_finite()) {
                // Its wheels and engine may hold the same nonsense, so the car starts again from
                // rest where it is, and is said to have once rather than every step.
                let failed = self.cars.get(&entity).is_some_and(|step| step.failed);
                if !failed {
                    self.warnings.push(
                        "a car's dynamics gave no answer for a step, so it was stopped".into(),
                    );
                }
                car.vehicle = Vehicle::new(car.vehicle.spec().clone(), GRAVITY);
                let rigid = self.bodies.get_mut(handle).expect("found above");
                rigid.set_linvel(Vector::ZERO, true);
                rigid.set_angvel(Vector::ZERO, true);
                let step = CarStep {
                    failed: true,
                    ..CarStep::default()
                };
                self.cars.insert(entity, step);
                continue;
            }
            let centre = car.vehicle.spec().centre;
            let moved = (body.position + body.rotation * centre)
                - (before.position + before.rotation * centre);
            let step = CarStep {
                velocity: body.velocity,
                spin: body.spin,
                carried: moved / dt,
                turned: (body.rotation * before.rotation.inverse()).to_scaled_axis() / dt,
                failed: self.cars.get(&entity).is_some_and(|step| step.failed),
                knock: Knock::default(),
            };
            let rigid = self.bodies.get_mut(handle).expect("found above");
            rigid.set_linvel(to_vector(step.carried), true);
            rigid.set_angvel(to_vector(step.turned), true);
            self.cars.insert(entity, step);
        }
    }

    /// After the physics step: each car's velocity is what its dynamics ended with, and what
    /// hitting things did to it.
    pub(crate) fn settle_cars(&mut self) {
        for (entity, step) in &mut self.cars {
            let Some(rigid) = self
                .actors
                .get(entity)
                .and_then(|actor| self.bodies.get_mut(actor.handle))
            else {
                continue;
            };
            let knocked = from_vector(rigid.linvel()) - step.carried;
            let twisted = from_vector(rigid.angvel()) - step.turned;
            rigid.set_linvel(to_vector(step.velocity + knocked), true);
            rigid.set_angvel(to_vector(step.spin + twisted), true);
            step.knock = Knock {
                velocity: knocked,
                spin: twisted,
            };
        }
    }

    /// What hitting things did to a car in the latest step: for what a blow does to it.
    pub fn car_knock(&self, entity: Entity) -> Option<Knock> {
        self.cars.get(&entity).map(|step| step.knock)
    }

    /// Where a car's body is touching things, as of the latest step: each place on its
    /// body, in the car's own space. Between them they are the patch of it a blow lands on:
    /// its whole nose against a wall, one corner of it against a post.
    pub fn car_touches(&self, entity: Entity) -> Vec<Vec3> {
        let mut touches = Vec::new();
        let Some(actor) = self.actors.get(&entity) else {
            return touches;
        };
        for &handle in &actor.colliders {
            let Some(collider) = self.colliders.get(handle) else {
                continue;
            };
            let within = collider
                .position_wrt_parent()
                .map_or(Vec3::ZERO, |pose| from_vector(pose.translation));
            for pair in self.narrow_phase.contact_pairs_with(handle) {
                let first = pair.collider1 == handle;
                // As the solver had them: those are the ones that say what each pushed with.
                for manifold in pair.solver_manifolds() {
                    for point in &manifold.points {
                        // What pushed the car this step, or is touching it, or all but: not
                        // what is only near enough to be watched. (A car that arrives fast
                        // is pushed by places it had not yet reached when they were found.)
                        if point.data.impulse <= 0.0 && point.dist > 0.03 {
                            continue;
                        }
                        let on = if first {
                            point.local_p1
                        } else {
                            point.local_p2
                        };
                        touches.push(within + from_vector(on));
                    }
                }
            }
        }
        touches
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// How much room boxes take up, between them.
    fn volume(boxes: &[(Vec3, Vec3)]) -> f32 {
        boxes
            .iter()
            .map(|(low, high)| {
                let size = *high - *low;
                size.x * size.y * size.z
            })
            .sum()
    }

    #[test]
    fn a_box_hollowed_is_what_is_left_round_the_hollow() {
        let solid = (Vec3::new(-1.0, 0.0, -2.0), Vec3::new(1.0, 1.5, 2.0));
        // A hollow in the middle of it leaves six boxes, which with it make up the whole.
        let hollow = (Vec3::new(-0.5, 0.4, -0.5), Vec3::new(0.2, 1.0, 0.6));
        let left = hollowed(solid, hollow);
        assert_eq!(left.len(), 6);
        let hole = (hollow.1 - hollow.0).to_array().iter().product::<f32>();
        assert!((volume(&left) + hole - volume(&[solid])).abs() < 1e-4);
        // Nothing left is in the hollow: its middle is in none of them.
        let middle = (hollow.0 + hollow.1) * 0.5;
        assert!(left
            .iter()
            .all(|(low, high)| !(middle.cmpgt(*low).all() && middle.cmplt(*high).all())));
        // One that reaches the top of the box leaves nothing over it; one outside the box
        // leaves the box as it was.
        let open = (Vec3::new(-0.5, 0.4, -0.5), Vec3::new(0.2, 2.0, 0.6));
        assert_eq!(hollowed(solid, open).len(), 5);
        let away = (Vec3::new(3.0, 0.0, 0.0), Vec3::new(4.0, 1.0, 1.0));
        assert_eq!(hollowed(solid, away), [solid]);
    }
}
