//! Play mode: a scene's game, running.
//!
//! [`Play::load`] builds a world from a baked scene's gameplay files: an entity for each placed
//! actor, with its name, its place, and its script with the placement's property values. Each
//! frame, [`Play::frame`] runs the game, then says where to draw each actor's instances, between
//! the last two steps, and where the player's camera is.

use crate::actor::ActorFile;
use crate::animation::{AnimatedActors, AnimationAssets};
use crate::drive::{self, Chase, DrawnWheel, Hud, Wheels, HUB_REACH, SPIN_REACH};
use crate::input::{Devices, InputMap};
use crate::race::{format_time, Lap, LapEvent};
use crate::resolve;
use crate::scene::{SceneGame, GAME_FILE};
use crate::track::Track;
use glam::{EulerRot, Mat4, Quat, Vec3};
use solstice_asset::NO_POSE;
use solstice_asset::{Collision, GameFile};
use solstice_physics::vehicle::{Surface, Vehicle, VehicleSpec};
use solstice_physics::{Car, Overlap, Physics, GRAVITY};
use solstice_script::{Script, ScriptEvent, ScriptHost};
use solstice_world::{
    Entity, Game, GlobalTransform, Idle, Name, Played, Schedule, TickGroup, Transform, World,
    STEP_SECONDS,
};
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::Rc;

/// How many messages are kept for the viewer to show, newest last.
pub const MAX_MESSAGES: usize = 200;
/// How far the player's camera may look up or down, in radians: not quite straight.
pub const MAX_PITCH: f32 = 1.5;

/// A line for the viewer to show: something a script logged, or an error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    pub error: bool,
    pub text: String,
}

/// Where the player's camera is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    pub position: Vec3,
    /// Around +Y, in radians; zero looks along -Z, as the renderer's camera does.
    pub yaw: f32,
    pub pitch: f32,
}

/// A placed actor with instances to move.
struct Drawn {
    entity: Entity,
    /// Its `.anim` file, if it is animated, by its path in the scene's gameplay.
    animation: Option<String>,
    /// Each instance, and where it is relative to the actor: or, for a car's wheel, relative
    /// to the wheel's own middle.
    instances: Vec<(u32, Mat4)>,
    /// For a car, which wheel each instance is carried by, if any, and whether it turns with
    /// it: a brake caliper is carried, and doesn't.
    wheels: Vec<Option<(usize, bool)>>,
    /// Where the actor was last drawn, to move its instances only when it moves.
    drawn: Option<Mat4>,
    /// Where a car's wheels were last drawn, relative to it: rolled, and as their uprights.
    rolled: Option<[(Mat4, Mat4); 4]>,
}

/// The actor the player plays, and where its eye is.
struct Player {
    entity: Entity,
    eye: Vec3,
    /// Whether it is a car, followed by the chase camera.
    car: bool,
}

const WHEEL_NAMES: [&str; 4] = ["front left", "front right", "rear left", "rear right"];

/// A scene's game, running.
pub struct Play {
    game: Game,
    host: Rc<RefCell<ScriptHost>>,
    drawn: Vec<Drawn>,
    player: Option<Player>,
    /// Whoever drives the player's car, who can get out of it and walk: an actor of their
    /// own with a camera, when the scene has one beside the car. And whether they are out.
    walker: Option<Player>,
    afoot: bool,
    /// What the driver carries on foot, and where it is: in their hand, lying where it was
    /// put down, or put away. Whoever runs the game says ([`Play::carry`]).
    carried: Vec<Entity>,
    carry: Option<(Vec3, Quat)>,
    map: InputMap,
    /// How far the camera looks up, as of the last step.
    pitch: f32,
    /// Seconds since the last step, over which the devices' movement has built up.
    unstepped: f64,
    /// Actions to press in the next frame, from [`Play::press`].
    presses: Vec<String>,
    scripts: Vec<String>,
    messages: VecDeque<Message>,
    physics: Rc<RefCell<Physics>>,
    /// Everything animated, once [`Play::attach_animation`] has been called.
    animated: Rc<RefCell<AnimatedActors>>,
    /// Bones to pose, for the viewer to hand to the renderer.
    poses: Vec<(u32, Vec<Mat4>)>,
    viewer: Vec3,
    /// Whether the player has left whoever they play, to fly the camera.
    flying: bool,
    /// The circuit, if the scene is one.
    track: Option<Rc<Track>>,
    /// How wet the ground is, from 0 to 1, as whoever runs the weather last said.
    wetness: Rc<Cell<f32>>,
    /// The pace the game drives the player's car at, while it does: see
    /// [`Play::set_autopilot`].
    chauffeur: Rc<Cell<Option<drive::Pace>>>,
    /// The camera that follows the player's car.
    chase: Chase,
    /// The player's car's instruments, as of the latest frame.
    hud: Option<Hud>,
    /// What the timing of laps has to say, for the viewer to show.
    race_log: Rc<RefCell<Vec<String>>>,
    /// The blows the player's car has taken since they were last asked for (`impact.rs`),
    /// and what is wrong with it, as whoever keeps its damage last said.
    impacts: Rc<RefCell<Vec<crate::Impact>>>,
    handicap: Rc<Cell<drive::Handicap>>,
}

/// How fast a stick held right over swings the view round a car, in radians a second.
const STICK_ORBIT: f32 = 3.4;

impl Play {
    /// Builds the game in a baked scene's gameplay `files`. `instance` gives where the bake
    /// placed a renderer instance. Every mistake found is reported, each with its file and line.
    ///
    /// `collision` is what the scene's static world is made of, if it has been fetched: without
    /// it, characters have nothing to stand on.
    pub fn load(
        files: &[GameFile],
        instance: impl Fn(u32) -> Option<Mat4>,
        collision: Option<&Collision>,
    ) -> Result<Self, Vec<String>> {
        let file = |name: &str| files.iter().find(|f| f.name == name).map(|f| &f.text);
        let mut errors = Vec::new();
        let scene = file(GAME_FILE)
            .ok_or_else(|| vec![format!("the scene's gameplay has no {GAME_FILE}")])?;
        let scene = SceneGame::parse(scene).map_err(|e| vec![format!("{GAME_FILE}: {e}")])?;
        // Whether any actor is a car: only then are the built-in driving actions read.
        let cars = scene.actors.iter().any(|line| {
            file(&line.file)
                .and_then(|text| ActorFile::parse(text).ok())
                .is_some_and(|actor| actor.vehicle.is_some())
        });
        let map = match &scene.input {
            None => InputMap::built_in(cars),
            Some(name) => match file(name).map(|text| InputMap::parse(text)) {
                Some(Ok(map)) => map,
                Some(Err(e)) => {
                    errors.push(format!("{name}: {e}"));
                    InputMap::built_in(cars)
                }
                None => {
                    errors.push(format!("{name}: missing from the scene's gameplay"));
                    InputMap::built_in(cars)
                }
            },
        };

        let track = match &scene.track {
            None => None,
            Some(name) => match file(name).map(|text| Track::parse(text)) {
                Some(Ok(track)) => Some(Rc::new(track)),
                Some(Err(e)) => {
                    errors.push(format!("{name}: {e}"));
                    None
                }
                None => {
                    errors.push(format!("{name}: missing from the scene's gameplay"));
                    None
                }
            },
        };

        let host = Rc::new(RefCell::new(ScriptHost::new()));
        let physics = Rc::new(RefCell::new(Physics::new()));
        if let Some(track) = &track {
            // What each tyre is on. A car's wheels are near each other, and near where they
            // were, so the search starts from the last place found.
            let (track, near) = (track.clone(), Cell::new(None));
            physics.borrow_mut().set_surfaces(move |point| {
                let place = track.locate(point, near.get());
                near.set(Some(place.sample));
                place.surface
            });
        }
        // Open country has a map of its ground, which says more than a track's edges can.
        let ground: Option<Rc<crate::SurfaceMap>> = match &scene.surfaces {
            None => None,
            Some(name) => match file(name).map(|text| crate::SurfaceMap::parse(text)) {
                Some(Ok(map)) => {
                    let map = Rc::new(map);
                    let tyres = map.clone();
                    physics
                        .borrow_mut()
                        .set_surfaces(move |point| tyres.at(point));
                    Some(map)
                }
                Some(Err(e)) => {
                    errors.push(format!("{name}: {e}"));
                    None
                }
                None => {
                    errors.push(format!("{name}: missing from the scene's gameplay"));
                    None
                }
            },
        };
        let mapped = ground.is_some();
        if let Some(collision) = collision {
            physics.borrow_mut().add_scene(collision, &instance);
        }
        for blocker in &scene.blockers {
            physics.borrow_mut().add_blocker(
                blocker.centre,
                blocker.size,
                blocker.yaw_degrees.to_radians(),
            );
        }
        let mut world = World::new();
        let mut drawn = Vec::new();
        let mut player: Option<Player> = None;
        let mut walker: Option<Player> = None;
        let mut carried = Vec::new();
        let mut scripts: Vec<String> = Vec::new();
        for (index, line) in scene.actors.iter().enumerate() {
            let at = format!("{GAME_FILE}: actor {} ({})", index + 1, line.file);
            let Some(text) = file(&line.file) else {
                errors.push(format!("{at}: missing from the scene's gameplay"));
                continue;
            };
            let actor = match ActorFile::parse(text) {
                Ok(actor) => actor,
                Err(e) => {
                    errors.push(format!("{}: {e}", line.file));
                    continue;
                }
            };
            let rotation = Quat::from_rotation_y(line.yaw_degrees.to_radians());
            let mut components = hecs_builder();
            components
                .add(Name::new(&actor.name))
                .add(Transform::from_translation(line.position).with_rotation(rotation));
            if let Some(relative) = &actor.script {
                let name = resolve(&line.file, relative);
                let mut script = Script::new(&name);
                for property in &actor.properties {
                    script
                        .properties
                        .insert(property.name.clone(), property.default.clone());
                }
                for (setting, value) in &line.settings {
                    match actor.property(setting) {
                        None => {
                            errors.push(format!("{at}: {} has no property `{setting}`", line.file))
                        }
                        Some(property) => match property.kind.parse_setting(value) {
                            Ok(value) => {
                                script.properties.insert(setting.clone(), value);
                            }
                            Err(e) => errors.push(format!("{at}: `{setting}`: {e}")),
                        },
                    }
                }
                if !scripts.contains(&name) {
                    match file(&name) {
                        None => errors.push(format!(
                            "{}: its script {name} is missing from the scene's gameplay",
                            line.file
                        )),
                        Some(source) => {
                            if let Err(e) = host.borrow_mut().load(&name, source) {
                                errors.push(e.message);
                            }
                        }
                    }
                    scripts.push(name);
                }
                components.add(script);
            } else if !line.settings.is_empty() {
                errors.push(format!(
                    "{at}: {} has no script, so no properties",
                    line.file
                ));
            }
            if let Some(collider) = actor.collider {
                components.add(collider);
            }
            if let Some(character) = actor.character {
                components.add(character);
            }
            if actor.camera.is_some() {
                components.add(Played);
            }
            // A car: its numbers, its wheels as they are drawn, and its laps.
            let mut hubs = None;
            if let Some(vehicle) = &actor.vehicle {
                let name = resolve(&line.file, &vehicle.path);
                match file(&name).map(|text| VehicleSpec::parse(text)) {
                    Some(Ok(spec)) => {
                        let car = Car::new(Vehicle::new(spec, GRAVITY), vehicle.automatic);
                        hubs = Some(car.vehicle.spec().hubs());
                        components.add(Wheels::at_rest(&car.vehicle)).add(car);
                        if let Some(track) = &track {
                            components.add(Lap::new(track));
                        }
                    }
                    Some(Err(e)) => errors.push(format!("{name}: {e}")),
                    None => errors.push(format!(
                        "{}: its vehicle {name} is missing from the scene's gameplay",
                        line.file
                    )),
                }
            }
            let entity = world.spawn(components.build());
            if actor.carried {
                carried.push(entity);
            }
            if let Some(eye) = actor.camera {
                let this = Player {
                    entity,
                    eye,
                    car: hubs.is_some(),
                };
                match (&player, &walker) {
                    (None, _) => player = Some(this),
                    // A car and whoever drives it: the car is played first, and its driver
                    // when they get out.
                    (Some(first), None) if first.car != this.car => {
                        if this.car {
                            walker = player.replace(this);
                        } else {
                            walker = Some(this);
                        }
                    }
                    _ => errors.push(format!(
                        "{at}: only one actor can have a camera, or a car and its driver, and there is one already"
                    )),
                }
            }
            if let Some(range) = &line.instances {
                let placed = Mat4::from_rotation_translation(rotation, line.position);
                let mut instances = Vec::new();
                let mut wheels = Vec::new();
                for id in range.clone() {
                    let Some(world) = instance(id) else {
                        errors.push(format!("{at}: the scene has no instance {id}"));
                        break;
                    };
                    let local = placed.inverse() * world;
                    // A piece of a car's mesh with its own origin at a hub is that wheel.
                    let origin = local.w_axis.truncate();
                    let wheel = hubs.and_then(|hubs: [Vec3; 4]| {
                        hubs.iter()
                            .position(|hub| hub.distance(origin) < HUB_REACH)
                            .map(|index| (index, hubs[index]))
                    });
                    match wheel {
                        Some((index, hub)) => {
                            let turns = hub.distance(origin) < SPIN_REACH;
                            instances.push((id, Mat4::from_translation(-hub) * local));
                            wheels.push(Some((index, turns)));
                        }
                        None => {
                            instances.push((id, local));
                            wheels.push(None);
                        }
                    }
                }
                if let Some(hubs) = hubs {
                    for (index, hub) in hubs.iter().enumerate() {
                        if !wheels.contains(&Some((index, true))) {
                            errors.push(format!(
                                "{at}: its mesh has no piece with its origin at the {} hub, \
                                 {} {} {}: a car's wheels are pieces of its mesh, each with its \
                                 own origin at its middle",
                                WHEEL_NAMES[index], hub.x, hub.y, hub.z
                            ));
                        }
                    }
                }
                drawn.push(Drawn {
                    entity,
                    animation: actor.animation.as_ref().map(|a| resolve(&line.file, a)),
                    instances,
                    wheels,
                    drawn: None,
                    rolled: None,
                });
            }
        }
        if !errors.is_empty() {
            return Err(errors);
        }

        let mut schedule = Schedule::new();
        ScriptHost::install(&host, &mut schedule);
        // The player's car takes what the player asks of it, after scripts have had their say.
        let driven = player.as_ref().filter(|p| p.car).map(|p| p.entity);
        // The driver sits in the car until they get out of it.
        if let Some(walker) = &walker {
            let _ = world.entities.insert_one(walker.entity, Idle);
        }
        let circuit = track.clone();
        // Whether the car is driving itself round the circuit, and where on it it last was.
        let (mut itself, mut last_at) = (false, None);
        // How long the car driving itself has been asking to go and going nowhere.
        let mut stuck_for = 0.0_f32;
        // Where it was when it last got anywhere: a car wedged in a hole can spin its wheels
        // and shake without going anywhere at all.
        let mut last_place = Vec3::ZERO;
        // Whether the player's own gearbox is automatic, while the car drives itself.
        let mut players_gearbox = true;
        let wetness: Rc<Cell<f32>> = Rc::default();
        let rained = wetness.clone();
        // The pace the car is to be driven at by the game, when whoever runs it asks.
        let chauffeur: Rc<Cell<Option<drive::Pace>>> = Rc::default();
        let (hired, mut was_hired) = (chauffeur.clone(), false);
        // What is wrong with the car, which it is driven with whoever drives it.
        let handicap: Rc<Cell<drive::Handicap>> = Rc::default();
        let wrong = handicap.clone();
        // On a bush track a car that drives itself tours; round a circuit, it races.
        let pace = if mapped {
            drive::Pace::TOURING
        } else {
            drive::Pace::RACING
        };
        // The lowest the track goes: a car far below that has fallen out of the world.
        let lowest = track.as_ref().map_or(f32::MIN, |track| {
            let heights = track.samples.iter().map(|sample| sample.position.y);
            heights.fold(f32::MAX, f32::min)
        });
        schedule.add(TickGroup::PrePhysics, "driving", move |world| {
            // Every car's tyres are on ground as wet as the weather has made it.
            for (_, car) in world.entities.query_mut::<(Entity, &mut Car)>() {
                car.vehicle.set_wetness(rained.get());
            }
            let Some(entity) = driven else {
                return;
            };
            // While the player is away flying the camera, the keys that fly it are not the
            // car's: it is asked for nothing, as a walker left standing hears nothing.
            // Nor is a car whose driver has got out of it, which stands on its handbrake.
            let idle = world.entities.get::<&Idle>(entity).is_ok();
            let away = world.input().away() || idle;
            let mut asked = if away {
                Default::default()
            } else {
                drive::asked(world.input())
            };
            if idle {
                asked.handbrake = 1.0;
            }
            let reset = !away && world.input().pressed("reset");
            // With no circuit there is nothing to drive round: the key does nothing.
            if !away && circuit.is_some() && world.input().pressed("autopilot") {
                itself = !itself;
                last_at = None;
                // The driver it is handed to changes gear itself, whatever gearbox the
                // player had; handed back, the player has their own again.
                if let Ok(mut car) = world.entities.get::<&mut Car>(entity) {
                    if itself {
                        players_gearbox = car.driver.automatic;
                        car.driver.automatic = true;
                        // Off the road it drives as anyone would: both axles driven, and
                        // the rear differential locked.
                        if mapped {
                            car.vehicle
                                .set_drive_mode(solstice_physics::vehicle::DriveMode::FourHigh);
                            car.vehicle.set_lockers(false, true);
                        }
                    } else {
                        car.driver.automatic = players_gearbox;
                    }
                }
            }
            // Whoever runs the game may have the car driven at a pace of their choosing, as
            // when another driver's run is shown: handed over, and handed back, as the key
            // does it.
            let hire = hired.get();
            if hire.is_some() != was_hired {
                was_hired = hire.is_some();
                last_at = None;
                if let Ok(mut car) = world.entities.get::<&mut Car>(entity) {
                    if was_hired {
                        if !itself {
                            players_gearbox = car.driver.automatic;
                        }
                        car.driver.automatic = true;
                        if mapped {
                            car.vehicle
                                .set_drive_mode(solstice_physics::vehicle::DriveMode::FourHigh);
                            car.vehicle.set_lockers(false, true);
                        }
                    } else if !itself {
                        car.driver.automatic = players_gearbox;
                    }
                }
            }
            let itself = itself || hire.is_some();
            let pace = hire.unwrap_or(pace);
            if let (true, Some(track)) = (itself, &circuit) {
                let car = world
                    .entities
                    .query_one_mut::<(&Transform, &Car)>(entity)
                    .ok();
                if let Some((transform, car)) = car {
                    let (driven, at) = drive::autopilot(
                        track,
                        transform.translation,
                        transform.rotation * Vec3::NEG_Z,
                        car.vehicle.telemetry().speed,
                        last_at,
                        &pace,
                        // On mapped ground it goes by what is ahead: a crawl through water
                        // and bog, slowly over rock and mud.
                        ground
                            .as_ref()
                            .map(|map| {
                                let map = map.clone();
                                move |point: Vec3| match map.at(point) {
                                    Surface::Ford | Surface::Bog => 2.6_f32,
                                    Surface::Rock => 3.2,
                                    Surface::Mud => 5.0,
                                    _ => f32::INFINITY,
                                }
                            })
                            .as_ref()
                            .map(|caution| caution as &dyn Fn(Vec3) -> f32),
                    );
                    (asked, last_at) = (driven, Some(at));
                }
            }
            let (shift_drive, shift_lock, air) = (
                !away && world.input().pressed("drive_mode"),
                !away && world.input().pressed("diff_lock"),
                !away && world.input().pressed("tyres"),
            );
            if let Ok(mut car) = world.entities.get::<&mut Car>(entity) {
                // A car nobody is in is asked for nothing, whatever is wrong with it.
                car.asked = if away {
                    asked
                } else {
                    wrong.get().apply(asked)
                };
                // A four-wheel drive's lever, and its lockers: the rear, then both, then none.
                if shift_drive {
                    let next = car.vehicle.drive_mode().next();
                    car.vehicle.set_drive_mode(next);
                }
                if shift_lock {
                    let (front, rear) = match car.vehicle.lockers() {
                        (false, false) => (false, true),
                        (false, true) => (true, true),
                        _ => (false, false),
                    };
                    car.vehicle.set_lockers(front, rear);
                }
                if air {
                    let aired = car.vehicle.aired_down();
                    car.vehicle.set_aired_down(!aired);
                }
            }
            // Put back on the track when the player asks, and when the car has fallen far
            // below it: off a cliff, or into the sea.
            // (In the bush a car is wherever it has been driven, down a hill from the track
            // as likely as not: there it is lost only if it is under all the land there is.)
            let fallen = circuit.as_ref().is_some_and(|track| {
                world.entities.get::<&Transform>(entity).is_ok_and(|at| {
                    if mapped {
                        return at.translation.y < lowest - drive::FALLEN * 4.0;
                    }
                    let place = track.locate(at.translation, last_at);
                    // Kept as the next step's hint, or every step searches the whole lap.
                    last_at = Some(place.sample);
                    at.translation.y < track.samples[place.sample].position.y - drive::FALLEN
                })
            });
            // A car driving itself that has gone nowhere for five seconds is stuck: against a
            // tree, in a bog, or on its side. It is put back, as a player would ask to be.
            let here = world
                .entities
                .get::<&Transform>(entity)
                .map_or(Vec3::ZERO, |at| at.translation);
            if !itself || here.distance(last_place) > 1.5 {
                last_place = here;
                stuck_for = 0.0;
            } else {
                stuck_for += STEP_SECONDS as f32;
            }
            // Going nowhere for two seconds, it locks the front differential as well.
            if itself && mapped && stuck_for > 2.0 {
                if let Ok(mut car) = world.entities.get::<&mut Car>(entity) {
                    car.vehicle.set_lockers(true, true);
                }
            }
            let stuck = stuck_for > 6.0;
            if let (true, Some(track)) = (reset || fallen || stuck, &circuit) {
                drive::put_back(world, entity, track);
                last_at = None;
                stuck_for = 0.0;
            }
        });
        Physics::install(&physics, &mut schedule);
        // Each car's wheels as they are after the step, and its laps.
        let race_log: Rc<RefCell<Vec<String>>> = Rc::default();
        let (circuit, log) = (track.clone(), race_log.clone());
        schedule.add(TickGroup::PostPhysics, "racing", move |world| {
            let mut log = log.borrow_mut();
            for (car, wheels, transform, lap) in world
                .entities
                .query::<(&Car, &mut Wheels, &Transform, Option<&mut Lap>)>()
                .iter()
            {
                wheels.follow(&car.vehicle);
                let (Some(lap), Some(track)) = (lap, &circuit) else {
                    continue;
                };
                // Off the track: every wheel that is on the ground is on neither asphalt nor
                // kerb.
                // On mapped ground the loop is a track through the bush, not a road with
                // edges: wherever a car goes on its way round counts.
                let mut grounded = car.vehicle.wheels.iter().filter(|wheel| wheel.on_ground);
                let off_track = !mapped
                    && grounded.clone().next().is_some()
                    && grounded
                        .all(|wheel| !matches!(wheel.surface, Surface::Asphalt | Surface::Kerb));
                for event in lap.step(track, transform.translation, off_track, STEP_SECONDS) {
                    match event {
                        LapEvent::Started => log.push("lap 1 started".to_owned()),
                        LapEvent::Sector { .. } => {}
                        LapEvent::Finished {
                            lap,
                            seconds,
                            valid,
                            best,
                        } => {
                            let note = if !valid {
                                "  (off track: doesn't count)".to_owned()
                            } else {
                                match best {
                                    Some(best) if seconds < best => {
                                        format!("  best, by {:.3}", best - seconds)
                                    }
                                    Some(best) => format!("  +{:.3}", seconds - best),
                                    None => String::new(),
                                }
                            };
                            log.push(format!("lap {lap}: {}{note}", format_time(seconds)));
                        }
                    }
                }
            }
        });
        // What the player's car hits, and what comes up through its wheels: measured each
        // step from what the physics made of it, and kept for whoever asks.
        let impacts: Rc<RefCell<Vec<crate::Impact>>> = Rc::default();
        let (struck, measured) = (impacts.clone(), physics.clone());
        let mut blows = crate::impact::Blows::default();
        schedule.add(TickGroup::PostPhysics, "blows", move |world| {
            let Some(entity) = driven else {
                return;
            };
            let measured = measured.borrow();
            let Some(knock) = measured.car_knock(entity) else {
                return;
            };
            let Ok((car, transform)) = world.entities.query_one_mut::<(&Car, &Transform)>(entity)
            else {
                return;
            };
            let spec = car.vehicle.spec();
            let mut struck = struck.borrow_mut();
            let touches = || measured.car_touches(entity);
            struck.extend(blows.body(knock, transform.rotation, spec, touches));
            let loads = std::array::from_fn(|index| car.vehicle.wheels[index].load);
            let lifted = car.vehicle.telemetry().g.z;
            let forces = crate::impact::wheel_forces(spec, loads, lifted);
            for (index, force) in forces.into_iter().enumerate() {
                struck.extend(blows.wheel(index, force, spec));
            }
            // Nobody asking is no reason to keep them for ever.
            let kept = struck.len().saturating_sub(64);
            struck.drain(..kept);
        });
        let animated: Rc<RefCell<AnimatedActors>> = Default::default();
        let running = animated.clone();
        schedule.add(TickGroup::PostPhysics, "animation", move |world| {
            let commands = world.read_cloned::<solstice_script::AnimCommand>();
            let mut running = running.borrow_mut();
            running.apply(&commands);
            let mut notifies = Vec::new();
            running.step(world, &mut notifies);
            for (entity, name) in notifies {
                world.send(ScriptEvent::to(entity, "anim").arg(name.as_str()));
            }
        });
        // What enters and leaves a sensor reaches its script as `on_enter` and `on_exit`.
        schedule.add(TickGroup::PostPhysics, "touches", |world| {
            for overlap in world.read_cloned::<Overlap>() {
                let name = if overlap.entered { "enter" } else { "exit" };
                world.send(ScriptEvent::to(overlap.sensor, name).arg(overlap.other));
            }
        });
        let mut messages = VecDeque::new();
        for warning in physics.borrow_mut().warnings.drain(..) {
            messages.push_back(Message {
                error: true,
                text: warning,
            });
        }
        Ok(Self {
            game: Game::new(world, schedule),
            host,
            drawn,
            player,
            walker,
            afoot: false,
            carried,
            carry: None,
            map,
            pitch: 0.0,
            unstepped: 0.0,
            presses: Vec::new(),
            scripts,
            messages,
            physics,
            animated,
            poses: Vec::new(),
            viewer: Vec3::ZERO,
            flying: false,
            track,
            wetness,
            chauffeur,
            chase: Chase::default(),
            hud: None,
            race_log,
            impacts,
            handicap,
        })
    }

    /// The blows the player's car has taken since this was last asked: what its body hit
    /// and what came up through its wheels, each with where, which way and how hard.
    pub fn take_impacts(&mut self) -> Vec<crate::Impact> {
        std::mem::take(&mut *self.impacts.borrow_mut())
    }

    /// Says what is wrong with the player's car, as its driver is to feel it. It stays so
    /// until said again.
    pub fn set_handicap(&self, handicap: drive::Handicap) {
        self.handicap.set(handicap);
    }

    /// The circuit, if the scene is one.
    pub fn track(&self) -> Option<&Track> {
        self.track.as_deref()
    }

    /// Says how wet the ground is, from 0, dry, to 1, soaked: whoever runs the weather tells
    /// the game, and every car's tyres feel it from the next step.
    pub fn set_wetness(&self, wetness: f32) {
        self.wetness.set(wetness.clamp(0.0, 1.0));
    }

    /// What tyres have pressed into the ground (`solstice_vehicle::rut`): the ruts every
    /// car's wheels stand in and deepen, for whoever draws them to read, and to lay what a
    /// wheel threw where it landed.
    pub fn ruts<R>(&self, with: impl FnOnce(&mut solstice_physics::vehicle::Ruts) -> R) -> R {
        with(self.physics.borrow_mut().ruts_mut())
    }

    /// The player's car's instruments as of the latest frame, if the player drives a car.
    pub fn hud(&self) -> Option<&Hud> {
        self.hud.as_ref()
    }

    /// How the devices' controls become the game's actions: the scene's `.input` file, or
    /// the built-in set. For whoever shows a player which key or button does what.
    pub fn input_map(&self) -> &InputMap {
        &self.map
    }

    /// Whether the player is a car, whose instruments [`Play::hud`] has while it is driven.
    pub fn drives(&self) -> bool {
        self.player.as_ref().is_some_and(|player| player.car)
    }

    /// Gives every animated actor its skeleton and clips, from what the scene holds. Returns every
    /// mistake found.
    pub fn attach_animation(
        &mut self,
        files: &[GameFile],
        assets: &AnimationAssets<'_>,
    ) -> Result<(), Vec<String>> {
        let actors: Vec<_> = self
            .drawn
            .iter()
            .filter_map(|d| {
                let path = d.animation.as_ref()?;
                // A rigid mesh can sit beside the skinned ones: the first that has a pose.
                let posed = d
                    .instances
                    .iter()
                    .map(|(id, _)| *id)
                    .find(|id| {
                        assets
                            .instance_poses
                            .get(*id as usize)
                            .is_some_and(|pose| *pose != NO_POSE)
                    })
                    .or_else(|| d.instances.first().map(|(id, _)| *id))?;
                Some((d.entity, path.clone(), posed))
            })
            .collect();
        *self.animated.borrow_mut() = AnimatedActors::load(files, assets, &actors)?;
        Ok(())
    }

    /// Where the camera is, which decides how often distant characters update, and is what
    /// scripts get from `viewer()`.
    pub fn set_viewer(&mut self, position: Vec3) {
        self.viewer = position;
    }

    /// Takes the poses to give the renderer: each pose's number, and its bones in the mesh's
    /// own space.
    pub fn take_poses(&mut self) -> Vec<(u32, Vec<Mat4>)> {
        std::mem::take(&mut self.poses)
    }

    /// Every renderer instance an actor draws with, which must be made movable before play.
    pub fn instances(&self) -> impl Iterator<Item = u32> + '_ {
        self.drawn
            .iter()
            .flat_map(|d| d.instances.iter().map(|(id, _)| *id))
    }

    /// Moves one instance of an actor within the actor: a door swung on its hinge. `by` is
    /// applied to where the instance now is, in the actor's own space.
    pub fn move_within(&mut self, instance: u32, by: Mat4) {
        for drawn in &mut self.drawn {
            for (id, local) in &mut drawn.instances {
                if *id == instance {
                    *local = by * *local;
                    // Drawn again, though the actor has not moved.
                    drawn.drawn = None;
                }
            }
        }
    }

    /// Takes an instance off its actor: the actor no longer moves it, and whoever took it
    /// off draws it where they will (a piece torn off a truck, lying where it fell). Gives
    /// where it was in the actor's own space, to put it back with ([`Play::attach`]). A
    /// car's wheel is not taken off: it is where its hub is.
    pub fn detach(&mut self, instance: u32) -> Option<Mat4> {
        for drawn in &mut self.drawn {
            let Some(at) = drawn.instances.iter().position(|(id, _)| *id == instance) else {
                continue;
            };
            if drawn.wheels[at].is_some() {
                return None;
            }
            drawn.wheels.remove(at);
            return Some(drawn.instances.remove(at).1);
        }
        None
    }

    /// Puts an instance on the player's car, there in the car's own space: one that was
    /// taken off it ([`Play::detach`]). Whether there is a car to put it on.
    pub fn attach(&mut self, instance: u32, local: Mat4) -> bool {
        let Some(car) = self.player.as_ref().filter(|player| player.car) else {
            return false;
        };
        let Some(drawn) = self
            .drawn
            .iter_mut()
            .find(|drawn| drawn.entity == car.entity)
        else {
            return false;
        };
        if !drawn.instances.iter().any(|(id, _)| *id == instance) {
            drawn.instances.push((instance, local));
            drawn.wheels.push(None);
            // Drawn again, though the car has not moved.
            drawn.drawn = None;
        }
        true
    }

    /// Gets the player out of the car and stands them somewhere: where they came out of a
    /// door. Whether they are on foot there.
    pub fn stand_at(&mut self, at: Vec3) -> bool {
        if !self.afoot && !self.get_out() {
            return false;
        }
        let Some(walker) = &self.walker else {
            return false;
        };
        let at = at + Vec3::Y * 0.2;
        if let Ok(mut transform) = self
            .game
            .world
            .entities
            .get::<&mut Transform>(walker.entity)
        {
            transform.translation = at;
        }
        self.physics.borrow_mut().place(walker.entity, at);
        true
    }

    /// The instances that draw the player's car, its wheels among them.
    pub fn car_instances(&self) -> Vec<u32> {
        let Some(car) = self.player.as_ref().filter(|player| player.car) else {
            return Vec::new();
        };
        self.drawn
            .iter()
            .filter(|drawn| drawn.entity == car.entity)
            .flat_map(|drawn| drawn.instances.iter().map(|(id, _)| *id))
            .collect()
    }

    /// The scripts the game runs, by name, which a viewer may watch and [`Play::reload`].
    pub fn scripts(&self) -> &[String] {
        &self.scripts
    }

    /// Whether some actor is the player, with a camera to look through.
    pub fn has_player(&self) -> bool {
        self.player.is_some()
    }

    /// Whether the camera is the player's now: there is a player, and they are not flying.
    pub fn has_camera(&self) -> bool {
        self.player.is_some() && !self.flying
    }

    /// Lets the player leave whoever they play and fly the camera, or brings them back. Flying,
    /// [`Play::frame`] gives no view, and the script of the one they left hears no input, so
    /// the keys that fly the camera don't walk them too. Everything else hears it all: a door
    /// opens to a camera flown up to it.
    pub fn set_flying(&mut self, flying: bool) {
        self.flying = flying;
    }

    /// Whether the player's car has a driver who can get out of it and walk.
    pub fn can_walk(&self) -> bool {
        self.walker.is_some() && self.player.as_ref().is_some_and(|player| player.car)
    }

    /// Whether the driver is out of the car, walking.
    pub fn afoot(&self) -> bool {
        self.afoot
    }

    /// Gets the driver out of the car, to stand beside its door: the car is left on its
    /// handbrake, and the keys walk the driver. Whether they got out.
    pub fn get_out(&mut self) -> bool {
        if self.afoot || self.flying {
            return false;
        }
        let (Some(car), Some(walker)) = (&self.player, &self.walker) else {
            return false;
        };
        let entities = &mut self.game.world.entities;
        let Ok(at) = entities.get::<&Transform>(car.entity).map(|at| *at) else {
            return false;
        };
        let yaw = at.rotation.to_euler(EulerRot::YXZ).0;
        let facing = Quat::from_rotation_y(yaw);
        // Out of the door they sit by: the left, unless the car's numbers say the right.
        let right = entities
            .get::<&Car>(car.entity)
            .is_ok_and(|car| car.vehicle.spec().driver_right);
        let side = if right { 1.95 } else { -1.95 };
        let beside = at.translation + facing * Vec3::new(side, 0.0, 0.3) + Vec3::Y * 0.35;
        if let Ok(mut transform) = entities.get::<&mut Transform>(walker.entity) {
            transform.translation = beside;
            transform.rotation = facing;
        }
        self.physics.borrow_mut().place(walker.entity, beside);
        let _ = entities.remove_one::<Idle>(walker.entity);
        let _ = entities.insert_one(car.entity, Idle);
        self.afoot = true;
        self.pitch = 0.0;
        true
    }

    /// Gets the driver back into the car, if they are within `reach` of it. Whether they did.
    pub fn get_in(&mut self, reach: f32) -> bool {
        if !self.afoot {
            return false;
        }
        let (Some(car), Some(walker)) = (&self.player, &self.walker) else {
            return false;
        };
        let entities = &mut self.game.world.entities;
        let place = |entity| entities.get::<&Transform>(entity).map(|at| at.translation);
        let (Ok(car_at), Ok(walker_at)) = (place(car.entity), place(walker.entity)) else {
            return false;
        };
        if car_at.distance(walker_at) > reach {
            return false;
        }
        let _ = entities.remove_one::<Idle>(car.entity);
        let _ = entities.insert_one(walker.entity, Idle);
        self.afoot = false;
        true
    }

    /// Puts the player's car somewhere, facing a way, at rest: where it was left. Whether
    /// there is a car to put.
    pub fn place_car(&mut self, position: Vec3, yaw_degrees: f32) -> bool {
        let Some(car) = self.player.as_ref().filter(|player| player.car) else {
            return false;
        };
        drive::put_at(&mut self.game.world, car.entity, position, yaw_degrees);
        true
    }

    /// Has the game drive the player's car round the track at a pace, whatever the player
    /// asks of it, or gives it back to them: for showing how somebody else drives it.
    pub fn set_autopilot(&mut self, pace: Option<drive::Pace>) {
        self.chauffeur.set(pace);
    }

    /// Puts the player's car `back` metres behind the start line, facing the way the lap
    /// runs, at rest, with no lap begun: the start of a timed run. Whether there is a car
    /// and a track to put it on.
    pub fn to_the_line(&mut self, back: f32) -> bool {
        let (Some(car), Some(track)) = (
            self.player.as_ref().filter(|player| player.car),
            self.track.clone(),
        ) else {
            return false;
        };
        let (centre, forward, left) = track.place(-back);
        let up = forward.cross(left).normalize_or(Vec3::Y);
        let entities = &mut self.game.world.entities;
        if let Ok(mut transform) = entities.get::<&mut Transform>(car.entity) {
            transform.rotation =
                Quat::from_mat3(&glam::Mat3::from_cols(-left, up, -forward)).normalize();
            transform.translation = centre + up * 0.08;
        }
        if let Ok(mut driven) = entities.get::<&mut Car>(car.entity) {
            driven.vehicle.settle();
            driven.asked = Default::default();
        }
        if let Ok(mut lap) = entities.get::<&mut Lap>(car.entity) {
            *lap = Lap::new(&track);
        }
        true
    }

    /// Loads the player's car with this many kilograms, which it then has to carry.
    pub fn set_payload(&mut self, kilograms: f32) {
        let Some(car) = self.player.as_ref().filter(|player| player.car) else {
            return;
        };
        if let Ok(mut car) = self.game.world.entities.get::<&mut Car>(car.entity) {
            car.vehicle.set_payload(kilograms);
        }
    }

    /// Says where what the driver carries is: there, turned so, or put away. It stays until
    /// said again.
    pub fn carry(&mut self, pose: Option<(Vec3, Quat)>) {
        self.carry = pose;
    }

    /// Puts a loose thing in the world: a box of this size, weighing this much, which falls,
    /// lies where it comes to rest, is pushed by what meets it and weighs on what it lies
    /// on. Nothing draws it: whoever put it there asks where it is ([`Play::thing`]).
    pub fn add_thing(&mut self, at: Vec3, rotation: Quat, size: Vec3, kilograms: f32) -> Entity {
        self.game.world.spawn((
            Name::new("thing"),
            Transform::from_translation(at).with_rotation(rotation),
            solstice_physics::ActorCollider {
                shape: solstice_physics::Shape::Box(size),
                kind: solstice_physics::BodyKind::Dynamic(kilograms.max(0.01)),
                offset: Vec3::ZERO,
            },
        ))
    }

    /// Takes a loose thing out of the world.
    pub fn remove_thing(&mut self, thing: Entity) {
        let _ = self.game.world.entities.despawn(thing);
    }

    /// Where a loose thing is, and how it is turned.
    pub fn thing(&self, thing: Entity) -> Option<(Vec3, Quat)> {
        self.game
            .world
            .entities
            .get::<&Transform>(thing)
            .ok()
            .map(|at| (at.translation, at.rotation))
    }

    /// Puts a loose thing somewhere else, at rest.
    pub fn move_thing(&mut self, thing: Entity, at: Vec3, rotation: Quat) {
        if let Ok(mut transform) = self.game.world.entities.get::<&mut Transform>(thing) {
            transform.translation = at;
            transform.rotation = rotation;
        }
    }

    /// The height of whatever is under a place: the ground, a floor, a truck's tray, a
    /// thing lying there.
    pub fn surface_below(&self, from: Vec3) -> Option<f32> {
        self.physics.borrow().ground_below(from)
    }

    /// The height of whatever of the scene itself is under a place: its land and what is
    /// built on it, and nothing that moves.
    pub fn fixed_below(&self, from: Vec3) -> Option<f32> {
        self.physics.borrow().fixed_below(from)
    }

    /// The height of the land itself under a place, whatever stands on it.
    pub fn land_below(&self, from: Vec3) -> Option<f32> {
        self.physics.borrow().land_below(from)
    }

    /// Where the player's car is, and how it is turned.
    pub fn car_pose(&self) -> Option<(Vec3, Quat)> {
        let car = self.player.as_ref().filter(|player| player.car)?;
        self.thing(car.entity)
    }

    /// Whether the player is flying the camera, away from whoever they play.
    pub fn flying(&self) -> bool {
        self.flying && self.player.is_some()
    }

    /// Presses an action in the next frame, whatever the devices do: for scripted runs.
    pub fn press(&mut self, action: &str) {
        self.presses.push(action.to_owned());
    }

    /// Runs a frame `seconds` long, with the devices as they are. `place` is told where to draw
    /// each instance that moved. Returns where the player's camera is, if there is a player
    /// and they are not [flying](Play::set_flying).
    ///
    /// Movement (the mouse, dragging) builds up in `devices` until a step uses it, and is
    /// given to that step as a rate over the time since the step before. So none is lost in
    /// frames that run no step, as at 144 Hz, and none is counted twice. Looking up and down
    /// follows the movement every frame, for a camera that never lags the mouse.
    pub fn frame(
        &mut self,
        seconds: f64,
        devices: &mut Devices,
        mut place: impl FnMut(u32, Mat4),
    ) -> Option<View> {
        self.unstepped += seconds;
        let mut input = self.map.actions(devices, self.unstepped as f32);
        input.set_viewer(self.viewer);
        let flying = self.flying();
        input.set_away(flying);
        for action in self.presses.drain(..) {
            input.press(&action);
        }
        // While the driver is in the car they are not standing anywhere: their body is kept
        // well up out of the way, where the car's wheels cannot find it to stand on.
        if let (false, Some(walker), Some(car)) = (self.afoot, &self.walker, &self.player) {
            let entities = &self.game.world.entities;
            let above = entities
                .get::<&Transform>(car.entity)
                .ok()
                .map(|at| at.translation + Vec3::Y * 120.0);
            if let Some(above) = above {
                if let Ok(mut transform) = entities.get::<&mut Transform>(walker.entity) {
                    transform.translation = above;
                }
                self.physics.borrow_mut().place(walker.entity, above);
            }
        }
        // What the driver carries is where it was last said to be, or put away.
        if !self.carried.is_empty() {
            let entities = &self.game.world.entities;
            for &entity in &self.carried {
                if let Ok(mut transform) = entities.get::<&mut Transform>(entity) {
                    match self.carry {
                        Some((at, turned)) => {
                            transform.translation = at;
                            transform.rotation = turned;
                        }
                        // Away under the ground.
                        None => transform.translation.y = -900.0,
                    }
                }
            }
        }
        let steps = self.game.frame(seconds, &input);
        devices.end_frame();
        self.animated.borrow_mut().frame(
            steps.alpha,
            &self.game.world,
            self.viewer,
            &mut self.poses,
        );
        // Flying, the mouse turns the flown camera: the player's own looks where it did.
        let looked = if flying { 0.0 } else { input.value("look_up") };
        let pitch = (self.pitch + looked * self.unstepped as f32).clamp(-MAX_PITCH, MAX_PITCH);
        if steps.steps > 0 {
            devices.clear_movement();
            self.unstepped = 0.0;
            self.pitch = pitch;
        }
        if input.pressed("camera") {
            self.chase.view = self.chase.view.next();
        }
        self.gather_messages();
        let entities = &self.game.world.entities;
        for drawn in &mut self.drawn {
            let Ok(global) = entities.get::<&GlobalTransform>(drawn.entity) else {
                continue;
            };
            let world = global.interpolated(steps.alpha);
            // A car's wheels, between the last two steps as the car is. Only a car's instances
            // name wheels, so nothing else looks for them.
            let car = drawn.wheels.iter().any(Option::is_some);
            let wheels = car
                .then(|| entities.get::<&Wheels>(drawn.entity).ok())
                .flatten();
            let rolled = wheels.map(|wheels| {
                std::array::from_fn(|index| {
                    let (before, now) = (&wheels.before[index], &wheels.now[index]);
                    (
                        DrawnWheel::between(before, now, steps.alpha),
                        DrawnWheel::upright(before, now, steps.alpha),
                    )
                })
            });
            if drawn.drawn == Some(world) && drawn.rolled == rolled {
                continue;
            }
            let moved = drawn.drawn != Some(world);
            for ((id, local), wheel) in drawn.instances.iter().zip(&drawn.wheels) {
                match (wheel, &rolled) {
                    (Some((wheel, true)), Some(rolled)) => {
                        place(*id, world * rolled[*wheel].0 * *local)
                    }
                    (Some((wheel, false)), Some(rolled)) => {
                        place(*id, world * rolled[*wheel].1 * *local)
                    }
                    _ if moved => place(*id, world * *local),
                    _ => {}
                }
            }
            drawn.drawn = Some(world);
            drawn.rolled = rolled;
        }

        if flying {
            // The car is not being driven, and its instruments go with the player.
            self.hud = None;
        }
        let player = if self.afoot {
            &self.walker
        } else {
            &self.player
        };
        let player = player.as_ref().filter(|_| !flying)?;
        let world = entities
            .get::<&GlobalTransform>(player.entity)
            .ok()?
            .interpolated(steps.alpha);
        if player.car {
            let car = entities.get::<&Car>(player.entity).ok()?;
            let telemetry = *car.vehicle.telemetry();
            let look = drive::Look {
                back: input.held("look_back"),
                // The right stick and the mouse do the same thing: swing the view, which
                // then stays where it was put. A stick held over turns it at a steady rate.
                around: 0.0,
                over: 0.0,
                orbit: (input.value("orbit") - input.value("look_round") * STICK_ORBIT)
                    * seconds as f32,
                orbit_up: (input.value("orbit_up") + input.value("look_over") * STICK_ORBIT * 0.6)
                    * seconds as f32,
            };
            // A truck is followed from further off than a racing car, and seen out of from
            // higher up.
            let body = car.vehicle.spec();
            if body.body_size.z > 4.8 || body.centre.y > 0.55 {
                let tall = 1.0 + (body.centre.y - 0.4).max(0.0) * 0.8;
                self.chase.reach = (body.body_size.z / 4.5).max(1.0) * tall;
                self.chase.eyes = Some([
                    Vec3::new(0.0, body.body_at.y + body.body_size.y * 0.42, -0.25),
                    Vec3::new(0.0, body.body_at.y - 0.2, -(body.body_size.z * 0.5 + 0.35)),
                ]);
            }
            let physics = self.physics.borrow();
            let land = |at: Vec3| physics.land_below(at);
            let shot = self
                .chase
                .frame(world, telemetry.speed, look, seconds as f32, Some(&land));
            // Filled in place each frame: the lap's times are copied into what the last frame
            // had, and the track's name is set once.
            let hud = self.hud.get_or_insert_with(|| Hud {
                afoot: false,
                walker: Vec3::ZERO,
                walker_yaw: 0.0,
                origin: Vec3::ZERO,
                rotation: Quat::IDENTITY,
                tray: None,
                telemetry,
                limit: 0.0,
                throttle: 0.0,
                brake: 0.0,
                steer: 0.0,
                lap: None,
                track: self
                    .track
                    .as_ref()
                    .map(|track| track.name.clone())
                    .unwrap_or_default(),
                view: self.chase.view,
                wheels: Default::default(),
                forward: Vec3::NEG_Z,
                velocity: Vec3::ZERO,
                four_wheel_drive: false,
                pitch: 0.0,
                roll: 0.0,
            });
            hud.afoot = false;
            hud.telemetry = telemetry;
            hud.limit = car.vehicle.spec().engine.limit;
            hud.throttle = car.controls.throttle;
            hud.brake = car.controls.brake;
            hud.steer = car.controls.steer;
            hud.view = self.chase.view;
            hud.four_wheel_drive = car.vehicle.spec().transfer.is_some();
            let (_, rotation, origin) = world.to_scale_rotation_translation();
            // How far its nose is up, and how far it leans to its right, as an inclinometer
            // on the dash shows them.
            let (_, pitch, roll) = rotation.to_euler(glam::EulerRot::YXZ);
            (hud.pitch, hud.roll) = (pitch.to_degrees(), -roll.to_degrees());
            hud.forward = rotation * Vec3::NEG_Z;
            (hud.origin, hud.rotation) = (origin, rotation);
            hud.tray = car.vehicle.spec().tray;
            hud.velocity = hud.forward * telemetry.speed;
            let spec = car.vehicle.spec();
            let (front, rear) = spec.static_loads(9.81);
            for (index, shown) in hud.wheels.iter_mut().enumerate() {
                let wheel = &car.vehicle.wheels[index];
                let axle = spec.axle(index);
                let (hub, _) = car.vehicle.wheel_pose(index);
                *shown = drive::HudWheel {
                    at: origin + rotation * (hub - Vec3::Y * axle.radius),
                    hub,
                    on_ground: wheel.on_ground,
                    surface: wheel.surface,
                    slip: wheel.speed * axle.radius - telemetry.speed,
                    sink: wheel.sink,
                    load: wheel.load / if index < 2 { front } else { rear },
                    carried: car.vehicle.carried()[index],
                    radius: axle.radius,
                    tread: spec.tyre.tread.1,
                };
            }
            match (entities.get::<&Lap>(player.entity).ok(), &mut hud.lap) {
                (Some(lap), Some(shown)) => shown.clone_from(&lap),
                (Some(lap), shown) => *shown = Some((*lap).clone()),
                (None, shown) => *shown = None,
            }
            return Some(View {
                position: shot.position,
                yaw: shot.yaw,
                pitch: shot.pitch,
            });
        }
        let (_, rotation, translation) = world.to_scale_rotation_translation();
        // Out of the car, its instruments are as it was left, and say where its driver is.
        if let Some(hud) = self.hud.as_mut().filter(|_| self.afoot) {
            hud.afoot = true;
            hud.walker = translation;
            hud.walker_yaw = rotation.to_euler(EulerRot::YXZ).0;
        }
        Some(View {
            position: translation + rotation * player.eye,
            yaw: rotation.to_euler(EulerRot::YXZ).0,
            pitch,
        })
    }

    /// Loads a new version of a script, keeping every actor's state. If it fails to load, the
    /// old one keeps running, and the error says why.
    pub fn reload(&mut self, name: &str, source: &str) -> Result<(), String> {
        self.host
            .borrow_mut()
            .load(name, source)
            .map_err(|e| e.message)
    }

    /// What scripts have logged, and every error, since this was last asked, oldest first.
    pub fn take_messages(&mut self) -> Vec<Message> {
        self.gather_messages();
        self.messages.drain(..).collect()
    }

    /// The simulation, for tests and tools to look into.
    pub fn physics(&self) -> std::cell::Ref<'_, Physics> {
        self.physics.borrow()
    }

    /// The world, for tests and tools to look into.
    pub fn world(&self) -> &World {
        &self.game.world
    }

    fn gather_messages(&mut self) {
        let mut host = self.host.borrow_mut();
        let world = &self.game.world;
        let who = |entity: Option<Entity>| {
            entity
                .and_then(|e| {
                    world
                        .entities
                        .get::<&Name>(e)
                        .ok()
                        .map(|n| n.as_str().to_owned())
                })
                .unwrap_or_default()
        };
        for error in host.take_errors() {
            let text = match error.step {
                Some(step) => format!("step {step}, {}: {}", who(error.entity), error.message),
                None => error.message,
            };
            self.messages.push_back(Message { error: true, text });
        }
        for line in host.take_log() {
            let text = format!("{}: {}", who(Some(line.entity)), line.text);
            self.messages.push_back(Message { error: false, text });
        }
        for text in self.race_log.borrow_mut().drain(..) {
            self.messages.push_back(Message { error: false, text });
        }
        // What the physics finds wrong while playing, not only while loading.
        for text in self.physics.borrow_mut().warnings.drain(..) {
            self.messages.push_back(Message { error: true, text });
        }
        let dropped = host.take_dropped();
        if dropped > 0 {
            self.messages.push_back(Message {
                error: true,
                text: format!("{dropped} more lines dropped"),
            });
        }
        while self.messages.len() > MAX_MESSAGES {
            self.messages.pop_front();
        }
    }
}

fn hecs_builder() -> solstice_world::hecs::EntityBuilder {
    solstice_world::hecs::EntityBuilder::new()
}
