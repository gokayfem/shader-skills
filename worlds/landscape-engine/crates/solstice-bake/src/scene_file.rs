//! Builds a scene from a small text file listing models and where to place them.
//!
//! ```text
//! # Paths are relative to this file. Everything after a # is ignored.
//! texture_size 1024
//! terrain 800 1024 0.5 60         # size, cells per side, height, X where the shore begins
//! water -0.6 0.02 0.09 0.11       # height, then colour
//! pond -2 -37 4 -23 1.5           # still water in a basin: corners x z, x z, then its height
//! pond 24 -6 59 5 1.9 coastal     # and what the ponds' water is like, if not clear: an ocean
//! camera 40 5 20 1.2 -0.05        # x y z yaw pitch
//! light 0 2 0 16 40 14 3          # x y z range r g b
//! model rocks/rock.gltf 0 0 0 45 2     # path x y z yaw_degrees scale
//! model rocks/rock.gltf 9 ground 0 45 2     # standing on the terrain at this spot
//! model rocks/rock.gltf 9 ground:0.2 0 45 2 # with its lowest 20% buried, as rocks are
//! part kit/kit.gltf wall_01 0 0 0 90 1  # one named piece of a model, by its own origin
//! model plants/fern.gltf 3 ground 1 0 1 collide=none  # how it collides: none, hull or coarse
//! model people/keeper.glb 2 0 3 0 1 skinned  # keeps its skeleton, so that it can bend
//! blocker 40 2 0 1 4 90 0              # an invisible wall: x y z, full size, yaw_degrees
//! actor cove/gate.actor 20.7 0 0 90 locked=true  # an actor, with its properties' values
//! input cove/play.input                # the input actions, if not the built-in ones
//! track circuit/circuit.track          # a circuit: its centre line, and what lies beside it
//! circuit 1900                         # the land, road and planting round that track, this wide
//! surfaces cove/ground.surfaces        # what the ground is made of, place by place, off a track
//! actor cove/car.actor 0 0 0 0 shared   # an actor whose model is in the file read with this one
//! lends                                # this scene lends all it has to those read with it (`lend.rs`)
//! land 10 6                            # the terrain in bands: sand this far in, then dirt, then grass
//! path 22 0 60 -30 3                   # a dirt path: from x z, to x z, width
//! yard -21 -21 21 21                   # a dirt yard: opposite corners x z
//! clear -68 0 40                       # nothing planted within this radius of x z
//! meadow 110                           # real grass on the grass, this far from the origin
//! wood -10 -60 30 7                    # trees over the grass: centre x z, radius, spacing
//! ```
//!
//! Actors, their scripts and the input actions travel inside the scene file, as its gameplay
//! (`solstice_game::scene`). An actor's mesh is placed here, like a model, and moves with it.
//!
//! A model placed several times is imported once and shared, so repeats cost almost nothing.

use crate::meshes;
use glam::{Mat4, Quat, Vec3};
use solstice_asset::scene::Viewpoint;
use solstice_asset::{
    Collide, GameFile, Heightfield, Instance, Light, Material, SceneAsset, Sea, Source, StillWater,
};
use solstice_builder::{build_virtual_mesh, BuildOptions};
use solstice_game::{resolve, ActorFile, ActorLine, Blocker, InputMap, SceneGame, GAME_FILE};
use solstice_import::{append_model, import_gltf, ImportOptions, ModelPlacements};
use solstice_procgen::noise::fbm;
use solstice_procgen::surface::TILE_METRES;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// One line of a scene file, parsed.
#[derive(Debug, PartialEq)]
pub(crate) enum Command {
    TextureSize(u32),
    /// The terrain in bands of sand, dirt and grass (`land.rs`).
    Land {
        sand: f32,
        dirt: f32,
    },
    Path {
        from: glam::Vec2,
        to: glam::Vec2,
        width: f32,
    },
    Yard {
        from: glam::Vec2,
        to: glam::Vec2,
    },
    Clear {
        at: glam::Vec2,
        radius: f32,
    },
    Meadow(f32),
    Wood {
        at: glam::Vec2,
        radius: f32,
        spacing: f32,
    },
    Terrain {
        size: f32,
        cells: u32,
        height: f32,
        shore_x: f32,
    },
    Water {
        height: f32,
        color: [f32; 3],
    },
    /// An open sea at `height`: a built-in ocean's name, or a `.ocean` file beside the scene.
    Ocean {
        height: f32,
        ocean: String,
    },
    /// Still water in a basin: a level sheet over a rectangle, at `level`. `water` is what the
    /// scene's ponds hold, if it says: a built-in ocean's name, or a `.ocean` file beside the
    /// scene.
    Pond {
        min: glam::Vec2,
        max: glam::Vec2,
        level: f32,
        water: Option<String>,
    },
    Camera(Viewpoint),
    Light(Light),
    Model {
        path: String,
        position: Vec3,
        /// If set, `position.y` is ignored and the model stands on the terrain, with this
        /// fraction of its height below ground.
        seat: Option<f32>,
        /// Degrees about X, Y and Z: see [`placement_transform`].
        rotation: Vec3,
        scale: f32,
        collide: Collide,
        /// Whether it bends: it keeps its skeleton and follows a pose.
        skinned: bool,
    },
    Part {
        path: String,
        name: String,
        position: Vec3,
        rotation: Vec3,
        scale: f32,
        collide: Collide,
    },
    Actor(ActorLine),
    Blocker(Blocker),
    Input(String),
    Track(String),
    /// The land round the scene's track, this many metres across.
    Circuit(f32),
    /// The map of what the ground is made of, place by place (`solstice_game::SurfaceMap`).
    Surfaces(String),
    /// This scene lends every mesh and texture it has to the scenes read with it
    /// (`crate::lend`).
    Lends,
}

pub(crate) fn parse_line(line: &str) -> Result<Option<Command>, String> {
    let line = line.split('#').next().unwrap_or("").trim();
    let mut words = line.split_whitespace();
    let Some(keyword) = words.next() else {
        return Ok(None);
    };
    let mut values: Vec<&str> = words.collect();
    // On a line that places geometry, a last word may say how it collides.
    let mut collide = Collide::Auto;
    let mut skinned = false;
    if matches!(keyword, "model" | "part") {
        if values.last() == Some(&"skinned") {
            skinned = true;
            values.pop();
        }
        if let Some(word) = values.last().and_then(|w| w.strip_prefix("collide=")) {
            collide = match word {
                "none" => Collide::None,
                "hull" => Collide::Hull,
                "coarse" => Collide::Coarse,
                other => {
                    return Err(format!(
                        "'collide={other}': a line collides as none, hull or coarse"
                    ))
                }
            };
            values.pop();
        }
        // It may come after how it collides as well.
        if values.last() == Some(&"skinned") {
            skinned = true;
            values.pop();
        }
    }
    let values = values;
    let number = |index: usize| -> Result<f32, String> {
        let text = values
            .get(index)
            .ok_or_else(|| format!("'{keyword}' is missing value {}", index + 1))?;
        text.parse()
            .map_err(|_| format!("'{text}' is not a number"))
    };
    let expect = |count: usize| -> Result<(), String> {
        if values.len() == count {
            Ok(())
        } else {
            Err(format!(
                "'{keyword}' takes {count} values, found {}",
                values.len()
            ))
        }
    };
    Ok(Some(match keyword {
        "texture_size" => {
            expect(1)?;
            Command::TextureSize(number(0)? as u32)
        }
        "land" => {
            expect(2)?;
            let (sand, dirt) = (number(0)?, number(1)?);
            if sand < 0.0 || dirt < 0.0 {
                return Err("'land' takes widths of sand and dirt of 0 or more".to_string());
            }
            Command::Land { sand, dirt }
        }
        "path" => {
            expect(5)?;
            Command::Path {
                from: glam::Vec2::new(number(0)?, number(1)?),
                to: glam::Vec2::new(number(2)?, number(3)?),
                width: number(4)?,
            }
        }
        "yard" => {
            expect(4)?;
            Command::Yard {
                from: glam::Vec2::new(number(0)?, number(1)?),
                to: glam::Vec2::new(number(2)?, number(3)?),
            }
        }
        "clear" => {
            expect(3)?;
            Command::Clear {
                at: glam::Vec2::new(number(0)?, number(1)?),
                radius: number(2)?,
            }
        }
        "meadow" => {
            expect(1)?;
            let radius = number(0)?;
            if radius <= 0.0 {
                return Err("'meadow' takes a radius above 0".to_string());
            }
            Command::Meadow(radius)
        }
        "wood" => {
            expect(4)?;
            let (radius, spacing) = (number(2)?, number(3)?);
            if radius <= 0.0 || spacing <= 0.5 {
                return Err(
                    "'wood' takes a radius above 0 and trees more than half a metre apart"
                        .to_string(),
                );
            }
            Command::Wood {
                at: glam::Vec2::new(number(0)?, number(1)?),
                radius,
                spacing,
            }
        }
        "ocean" => {
            expect(2)?;
            Command::Ocean {
                height: number(0)?,
                ocean: values[1].to_string(),
            }
        }
        "pond" => {
            if values.len() != 6 {
                expect(5)?;
            }
            let (a, b) = (
                glam::Vec2::new(number(0)?, number(1)?),
                glam::Vec2::new(number(2)?, number(3)?),
            );
            let level = number(4)?;
            // The viewer drops a pond with a corner or level that isn't a number; a bake says so.
            if !(a.is_finite() && b.is_finite() && level.is_finite()) {
                return Err("a pond's corners and level must be finite numbers".into());
            }
            if a.x == b.x || a.y == b.y {
                return Err("a pond's corners must differ along both x and z".into());
            }
            Command::Pond {
                min: a.min(b),
                max: a.max(b),
                level,
                water: values.get(5).map(|water| water.to_string()),
            }
        }
        "water" => {
            expect(4)?;
            Command::Water {
                height: number(0)?,
                color: [number(1)?, number(2)?, number(3)?],
            }
        }
        "part" => {
            expect(7)?;
            Command::Part {
                path: values[0].to_string(),
                name: values[1].to_string(),
                position: Vec3::new(number(2)?, number(3)?, number(4)?),
                rotation: Vec3::new(0.0, number(5)?, 0.0),
                scale: number(6)?,
                collide,
            }
        }
        "terrain" => {
            expect(4)?;
            Command::Terrain {
                size: number(0)?,
                cells: number(1)? as u32,
                height: number(2)?,
                shore_x: number(3)?,
            }
        }
        "camera" => {
            expect(5)?;
            Command::Camera(Viewpoint {
                position: [number(0)?, number(1)?, number(2)?],
                yaw: number(3)?,
                pitch: number(4)?,
            })
        }
        "light" => {
            expect(7)?;
            Command::Light(Light {
                position: [number(0)?, number(1)?, number(2)?],
                range: number(3)?,
                intensity: [number(4)?, number(5)?, number(6)?],
                _pad: 0,
            })
        }
        "model" => {
            expect(6)?;
            let (height, seat) = parse_height(values[2])?;
            Command::Model {
                path: values[0].to_string(),
                position: Vec3::new(number(1)?, height, number(3)?),
                seat,
                rotation: Vec3::new(0.0, number(4)?, 0.0),
                scale: number(5)?,
                collide,
                skinned,
            }
        }
        "actor" => Command::Actor(ActorLine::parse(&values)?),
        "blocker" => Command::Blocker(Blocker::parse(&values)?),
        "input" => {
            expect(1)?;
            Command::Input(values[0].to_string())
        }
        "track" => {
            expect(1)?;
            Command::Track(values[0].to_string())
        }
        "surfaces" => {
            expect(1)?;
            Command::Surfaces(values[0].to_string())
        }
        "lends" => {
            expect(0)?;
            Command::Lends
        }
        "circuit" => {
            expect(1)?;
            let size = number(0)?;
            if !(200.0..=8000.0).contains(&size) {
                return Err(format!(
                    "a circuit's land is 200 to 8000 metres across, not {size}"
                ));
            }
            Command::Circuit(size)
        }
        other => return Err(format!("unknown command '{other}'")),
    }))
}

/// Reads a model's height: a number, `ground`, or `ground:FRACTION`.
fn parse_height(text: &str) -> Result<(f32, Option<f32>), String> {
    /// Enough to close the gap left by small bumps, without visibly sinking anything.
    const STANDING: f32 = 0.02;
    match text.split_once(':') {
        None if text == "ground" => Ok((0.0, Some(STANDING))),
        Some(("ground", fraction)) => {
            let fraction: f32 = fraction
                .parse()
                .map_err(|_| format!("'{fraction}' is not a number"))?;
            if (0.0..1.0).contains(&fraction) {
                Ok((0.0, Some(fraction)))
            } else {
                Err(format!("'{text}': the buried fraction must be below 1"))
            }
        }
        _ => text
            .parse()
            .map(|height| (height, None))
            .map_err(|_| format!("'{text}' is not a number")),
    }
}

/// Every model vertex, thinned out, in the model's own space.
fn model_points(scene: &SceneAsset) -> Vec<Vec3> {
    const MAX_PER_INSTANCE: usize = 6000;
    let mut points = Vec::new();
    for instance in &scene.instances {
        let mesh = &scene.meshes[instance.mesh as usize];
        let stride = (mesh.vertices.len() / MAX_PER_INSTANCE).max(1);
        for vertex in mesh.vertices.iter().step_by(stride) {
            let grid = Vec3::new(
                f32::from(vertex.data[0]),
                f32::from(vertex.data[1]),
                f32::from(vertex.data[2]),
            );
            let local = mesh.quant_origin + grid * mesh.quant_step;
            points.push(instance.transform.transform_point3(local));
        }
    }
    points
}

/// The height to place a model at so that its lowest `buried` fraction is wholly below `ground`.
/// Rounded rock touches flat ground at a point and appears to float, so it has to sink into it,
/// and on a slope this leaves no gap on the downhill side.
fn seat_height(
    points: &[Vec3],
    position: Vec3,
    rotation: Vec3,
    scale: f32,
    buried: f32,
    ground: impl Fn(f32, f32) -> f32,
) -> f32 {
    let transform = placement_transform(Vec3::new(position.x, 0.0, position.z), rotation, scale);
    let placed: Vec<Vec3> = points
        .iter()
        .map(|&p| transform.transform_point3(p))
        .collect();
    let lowest = placed.iter().map(|p| p.y).fold(f32::MAX, f32::min);
    let highest = placed.iter().map(|p| p.y).fold(f32::MIN, f32::max);
    let band = lowest + (highest - lowest) * buried;
    placed
        .iter()
        .filter(|p| p.y <= band)
        .map(|p| ground(p.x, p.z) - p.y)
        .fold(f32::MAX, f32::min)
}

/// How far inland the coast around the rest of the island lies from the origin, before its bays
/// and points. With the slope down to the seabed, this keeps the land well inside the terrain
/// square, so that its edges lie under the sea rather than showing as straight lines.
const ISLAND_RADIUS: f32 = 270.0;
/// How far the seabed lies beyond the waterline.
const SHELF_WIDTH: f32 = 90.0;
const SEABED_DEPTH: f32 = 9.0;

/// Gently rolling ground that falls away into the sea beyond `shore_x`, and beyond a wandering
/// coast all the way round, so that the land is an island whose edges never show.
fn terrain_height(x: f32, z: f32, amplitude: f32, shore_x: f32) -> f32 {
    let p = Vec3::new(x, 0.0, z);
    let broad = fbm(p * 0.015, 4, 301) * amplitude;
    let ripples = fbm(p * 0.25, 4, 303) * 0.04;
    let shelf = |offshore: f32| {
        let t = (offshore / SHELF_WIDTH).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    };
    let (cove, island) = offshore(x, z, shore_x);
    broad + ripples - shelf(cove).max(shelf(island)) * SEABED_DEPTH
}

/// How far out to sea a spot lies from where the shore begins: across the cove, and round the
/// rest of the island. Negative inland.
fn offshore(x: f32, z: f32, shore_x: f32) -> (f32, f32) {
    // Bays and points, so that the waterline wanders rather than running ruler-straight.
    let wander = fbm(Vec3::new(0.0, 7.0, z) * 0.011, 4, 305) * 28.0;
    let cove = x - shore_x - wander;
    // The coast round the rest of the island, sampled around a circle so that it wanders
    // seamlessly: broad headlands and bays, with smaller points along them.
    let radius = x.hypot(z);
    let around = Vec3::new(x, 0.0, z) / radius.max(1.0);
    let headlands = fbm(around * 1.6 + Vec3::new(0.0, 3.0, 0.0), 3, 307) * 45.0;
    let points = fbm(around * 7.0 + Vec3::new(0.0, 11.0, 0.0), 3, 309) * 14.0;
    (cove, radius - ISLAND_RADIUS - headlands - points)
}

/// How far inland a spot lies from where the shore begins, the nearer coast's.
fn inland(x: f32, z: f32, shore_x: f32) -> f32 {
    let (cove, island) = offshore(x, z, shore_x);
    -cove.max(island)
}

/// The most samples a side the ground's collision has, and about how far apart, in metres.
const GROUND_SAMPLES: u32 = 1025;
const GROUND_SPACING: f32 = 1.0;

/// The terrain as a grid of heights, for walking on: a metre or so apart.
fn ground_heights(size: f32, amplitude: f32, shore_x: f32) -> Heightfield {
    let cells = ((size / GROUND_SPACING).ceil() as u32).clamp(1, GROUND_SAMPLES - 1);
    let spacing = size / cells as f32;
    let origin = -size * 0.5;
    let mut heights = Vec::with_capacity(((cells + 1) * (cells + 1)) as usize);
    for row in 0..=cells {
        let z = origin + row as f32 * spacing;
        for column in 0..=cells {
            let x = origin + column as f32 * spacing;
            heights.push(terrain_height(x, z, amplitude, shore_x));
        }
    }
    Heightfield {
        columns: cells + 1,
        rows: cells + 1,
        origin: [origin, origin],
        spacing: [spacing, spacing],
        heights,
    }
}

/// Adds the terrain as one mesh, with its own ground material and texture.
fn add_terrain(scene: &mut SceneAsset, size: f32, cells: u32, height: f32, shore_x: f32) -> i32 {
    let mesh = meshes::terrain(size, cells, |x, z| terrain_height(x, z, height, shore_x));
    let layer = scene.textures.len() as i32;
    scene
        .textures
        .push(crate::textures::surface("sand", scene.texture_size));
    scene.materials.push(Material {
        // Darkened to about 0.35 albedo, as damp beach sand is, so that it sits correctly next to
        // scanned materials and does not skew auto exposure.
        base_color: [0.68, 0.68, 0.68, 1.0],
        roughness: 1.0,
        uv_scale: 1.0 / TILE_METRES,
        normal_strength: 1.0,
        texture_layer: layer,
        macro_variation: 1.0,
        ..Default::default()
    });
    scene.instances.push(Instance {
        mesh: scene.meshes.len() as u32,
        material: scene.materials.len() as u32 - 1,
        transform: Mat4::IDENTITY,
    });
    scene
        .meshes
        .push(build_virtual_mesh(&mesh, &BuildOptions::default()));
    layer
}

/// Adds a flat, glossy water surface. The ground texture supplies its ripples, at a scale
/// where the sand's ripples read as small waves, and is too dark to be seen as colour.
fn add_water(scene: &mut SceneAsset, height: f32, color: [f32; 3], ripples_layer: i32) {
    // Ripples need fine UVs, which the 16-bit grid only allows over a limited area.
    const SIZE: f32 = 4000.0;
    // Beyond that, a plain surface carries the sea out to the horizon, just below the first.
    const HORIZON_SIZE: f32 = 80000.0;
    let mesh = meshes::terrain(SIZE, 96, |_, _| height);
    scene.materials.push(Material {
        base_color: [color[0], color[1], color[2], 1.0],
        roughness: 0.14,
        uv_scale: 0.35,
        normal_strength: 0.22,
        texture_layer: ripples_layer,
        ..Default::default()
    });
    scene.instances.push(Instance {
        mesh: scene.meshes.len() as u32,
        material: scene.materials.len() as u32 - 1,
        transform: Mat4::IDENTITY,
    });
    scene
        .meshes
        .push(build_virtual_mesh(&mesh, &BuildOptions::default()));

    let far = meshes::terrain(HORIZON_SIZE, 32, |_, _| height - 0.3);
    scene.materials.push(Material {
        base_color: [color[0], color[1], color[2], 1.0],
        roughness: 0.2,
        ..Default::default()
    });
    scene.instances.push(Instance {
        mesh: scene.meshes.len() as u32,
        material: scene.materials.len() as u32 - 1,
        transform: Mat4::IDENTITY,
    });
    scene
        .meshes
        .push(build_virtual_mesh(&far, &BuildOptions::default()));
}

/// Places something: turned by `rotation`, degrees about X, Y and Z applied as yaw (Y), then
/// pitch (X), then roll (Z), and scaled evenly.
fn placement_transform(position: Vec3, rotation: Vec3, scale: f32) -> Mat4 {
    let rotation = if rotation.x == 0.0 && rotation.z == 0.0 {
        // Exactly as scenes were always placed, so their bakes don't change by a bit.
        Quat::from_rotation_y(rotation.y.to_radians())
    } else {
        Quat::from_euler(
            glam::EulerRot::YXZ,
            rotation.y.to_radians(),
            rotation.x.to_radians(),
            rotation.z.to_radians(),
        )
    };
    Mat4::from_scale_rotation_translation(Vec3::splat(scale), rotation, position)
}

/// The placements for a model, registering it in first-seen order on first use.
fn placements_of<'a>(
    order: &mut Vec<PathBuf>,
    models: &'a mut HashMap<PathBuf, ModelPlacements>,
    base: &Path,
    model: String,
) -> &'a mut ModelPlacements {
    let full = base.join(model);
    if !order.contains(&full) {
        order.push(full.clone());
    }
    models.entry(full).or_default()
}

/// A placement that waits for its height: position, rotation, scale, buried share, how it
/// collides, and which entity placed it.
type Seated = (Vec3, Vec3, f32, f32, Collide, Option<String>);

/// An actor's mesh: the model's path relative to the scene, its scale, and which named part.
type ActorMesh = (String, f32, Option<String>, f32, Option<String>);

/// Which placement of a model something became.
#[derive(Clone, Copy)]
enum Placed {
    Whole(usize),
    Part(usize),
}

/// How each placement of a model collides, and which entity placed it, in the order placed.
#[derive(Default)]
struct PlacedCollision {
    whole: Vec<Collide>,
    parts: Vec<Collide>,
    whole_owners: Vec<Option<String>>,
    part_owners: Vec<Option<String>>,
}

/// A scene's gameplay, gathered while its file is read.
#[derive(Default)]
struct Gameplay {
    game: SceneGame,
    /// Each actor file, script and input file, by name, once each.
    files: Vec<GameFile>,
    /// For each actor with a mesh: which actor, which model, and which of its placements.
    meshes: Vec<(usize, PathBuf, Placed)>,
    /// Whether an actor has a camera already: only one can.
    has_player: [bool; 2],
    /// The actors whose meshes are kits: the pieces of each are listed for the game.
    kits: Vec<usize>,
}

/// Which of an actor's instances draw each named piece of its mesh, as text: a line a piece,
/// `NAME FIRST COUNT`, counted from the actor's first instance. A piece of several materials
/// is several instances, one after another.
fn pieces_text(names: &[String]) -> String {
    let mut text = String::new();
    let mut first = 0;
    while first < names.len() {
        let count = names[first..]
            .iter()
            .take_while(|name| **name == names[first])
            .count();
        if !names[first].is_empty() {
            text.push_str(&format!("{} {first} {count}\n", names[first]));
        }
        first += count;
    }
    text
}

impl Gameplay {
    /// Reads a gameplay file once, by its name relative to the scene.
    fn read(&mut self, base: &Path, name: &str) -> Result<&str, String> {
        if let Some(index) = self.files.iter().position(|f| f.name == name) {
            return Ok(&self.files[index].text);
        }
        let path = base.join(name);
        let text =
            std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        self.files.push(GameFile {
            name: name.to_owned(),
            text,
        });
        Ok(&self.files.last().expect("just pushed").text)
    }

    /// Adds an actor, reading and checking its file, its settings and its script. Returns its
    /// mesh, if it has one: the model's path relative to the scene, its scale, and which part.
    fn add_actor(&mut self, base: &Path, mut line: ActorLine) -> Result<Option<ActorMesh>, String> {
        if line.instances.is_some() {
            return Err("`instances` is worked out by the bake, not written by hand".to_owned());
        }
        line.file = resolve(GAME_FILE, &line.file);
        let actor = ActorFile::parse(self.read(base, &line.file)?)
            .map_err(|e| format!("{}: {e}", line.file))?;
        // Each of these would stop the game starting, so they stop the bake instead.
        if actor.script.is_none() && !line.settings.is_empty() {
            return Err(format!("{} has no script, so no properties", line.file));
        }
        if actor.camera.is_some() {
            // One actor is played; or two, a car and whoever drives it, who gets out.
            let car = usize::from(actor.vehicle.is_some());
            if self.has_player[car] {
                return Err(format!(
                    "{} has a camera, and another actor already does: only one can, or a car and its driver",
                    line.file
                ));
            }
            self.has_player[car] = true;
        }
        for (name, value) in &line.settings {
            let property = actor
                .property(name)
                .ok_or_else(|| format!("{} has no property `{name}`", line.file))?;
            property
                .kind
                .parse_setting(value)
                .map_err(|e| format!("`{name}`: {e}"))?;
        }
        if let Some(script) = &actor.script {
            let name = resolve(&line.file, script);
            let source = self.read(base, &name)?.to_owned();
            // Checked now, so a mistake is found while baking, not when someone plays.
            solstice_script::ScriptHost::new()
                .load(&name, &source)
                .map_err(|e| e.message)?;
        }
        let animation = match &actor.animation {
            None => None,
            Some(path) => {
                let name = resolve(&line.file, path);
                let text = self.read(base, &name)?;
                solstice_anim::AnimGraph::parse(text).map_err(|e| format!("{name}: {e}"))?;
                Some(name)
            }
        };
        if animation.is_some() && actor.mesh.is_none() {
            return Err(format!("{} is animated but has no mesh", line.file));
        }
        if let Some(vehicle) = &actor.vehicle {
            let name = resolve(&line.file, &vehicle.path);
            solstice_game::VehicleSpec::parse(self.read(base, &name)?)
                .map_err(|e| format!("{name}: {e}"))?;
        }
        if let Some(kit) = &actor.kit {
            // The game reads it: here it is only brought along.
            let name = resolve(&line.file, kit);
            self.read(base, &name)?;
            self.kits.push(self.game.actors.len());
        }
        // Its model may be in the file every map shares, and not in this one. Baked whole,
        // to be looked at on its own, it is in this one after all.
        if std::env::var_os("SOLSTICE_WHOLE").is_some() {
            line.shared = false;
        }
        let mesh = actor.mesh.as_ref().filter(|_| !line.shared).map(|mesh| {
            (
                resolve(&line.file, &mesh.path),
                mesh.scale,
                mesh.part.clone(),
                mesh.yaw_degrees,
                animation.clone(),
            )
        });
        self.game.actors.push(line);
        Ok(mesh)
    }

    fn set_input(&mut self, base: &Path, name: &str) -> Result<(), String> {
        if self.game.input.is_some() {
            return Err("`input` is given twice".to_owned());
        }
        let name = resolve(GAME_FILE, name);
        InputMap::parse(self.read(base, &name)?).map_err(|e| format!("{name}: {e}"))?;
        self.game.input = Some(name);
        Ok(())
    }

    fn set_track(&mut self, base: &Path, name: &str) -> Result<(), String> {
        if self.game.track.is_some() {
            return Err("`track` is given twice".to_owned());
        }
        let name = resolve(GAME_FILE, name);
        solstice_game::Track::parse(self.read(base, &name)?).map_err(|e| format!("{name}: {e}"))?;
        self.game.track = Some(name);
        Ok(())
    }

    /// The map of what the ground is made of, which each tyre reads the square it is on of:
    /// for land that is not a circuit, whose track says what lies beside its road.
    fn set_surfaces(&mut self, base: &Path, name: &str) -> Result<(), String> {
        if self.game.surfaces.is_some() {
            return Err("`surfaces` is given twice".to_owned());
        }
        let name = resolve(GAME_FILE, name);
        solstice_game::SurfaceMap::parse(self.read(base, &name)?)
            .map_err(|e| format!("{name}: {e}"))?;
        self.game.surfaces = Some(name);
        Ok(())
    }

    /// The scene's gameplay files, with the list of its actors first, or none if it has none.
    fn finish(mut self) -> Vec<GameFile> {
        if self.game.actors.is_empty()
            && self.game.blockers.is_empty()
            && self.game.input.is_none()
            && self.game.track.is_none()
            && self.game.surfaces.is_none()
        {
            return Vec::new();
        }
        self.files.insert(
            0,
            GameFile {
                name: GAME_FILE.to_owned(),
                text: self.game.to_text(),
            },
        );
        self.files
    }
}

/// Whether this `.anim` file's clips were already baked for this skeleton: two models can share
/// a graph, and each needs clips fitted to its own skeleton.
fn has_clips_for(clips: &[solstice_asset::Clip], graph_file: &str, skeleton: u32) -> bool {
    let prefix = solstice_game::animation::clip_key(graph_file, "");
    clips
        .iter()
        .any(|c| c.skeleton == skeleton && c.name.starts_with(&prefix))
}

/// Reads the clips an `.anim` file names, from the glTF it names, fitted to `skeleton`, and
/// quantises them. Each is named as `solstice_game::animation::clip_key` says.
fn bake_clips(
    base: &Path,
    graph_file: &str,
    text: &str,
    target: &solstice_asset::Skeleton,
    skeleton: u32,
) -> Result<Vec<solstice_asset::Clip>, String> {
    let graph = solstice_anim::AnimGraph::parse(text).map_err(|e| format!("{graph_file}: {e}"))?;
    if graph.sources.is_empty() {
        return Err(format!("{graph_file}: has no `source` line"));
    }
    let looping = |name: &str| {
        graph
            .clips
            .iter()
            .find(|clip| clip.source_name == name)
            .is_none_or(|clip| clip.looping)
    };
    let mut files = Vec::with_capacity(graph.sources.len());
    for source in &graph.sources {
        let path = base.join(resolve(graph_file, source));
        files.push(
            solstice_import::animation::read_clips(&path, looping)
                .map_err(|e| format!("{}: {e}", path.display()))?,
        );
    }
    let roles = match &graph.roles {
        None => None,
        Some((from, to)) => {
            let read = |name: &str| {
                let path = base.join(resolve(graph_file, name));
                std::fs::read_to_string(&path)
                    .map_err(|e| format!("{}: {e}", path.display()))
                    .and_then(|t| {
                        solstice_import::character::Roles::parse(&t)
                            .map_err(|e| format!("{}: {e}", path.display()))
                    })
            };
            Some((read(from)?, read(to)?))
        }
    };
    let mut out = Vec::new();
    for line in &graph.clips {
        let (file, frames) = files
            .iter()
            .find_map(|file| {
                let (_, frames) = file
                    .clips
                    .iter()
                    .find(|(name, _)| *name == line.source_name)?;
                Some((file, frames))
            })
            .ok_or_else(|| {
                format!(
                    "{graph_file}: {} has no clip `{}`",
                    graph.sources.join(" or "),
                    line.source_name
                )
            })?;
        let fitted = match &roles {
            Some((from, to)) => {
                solstice_import::animation::retarget(&file.rig, from, target, to, frames)
            }
            None => solstice_import::animation::by_name(&file.rig, target, frames),
        };
        out.push(solstice_anim::compress(
            &solstice_game::animation::clip_key(graph_file, &line.name),
            skeleton,
            &fitted,
            line.looping,
        ));
    }
    Ok(out)
}

/// One command to build, and where it came from.
#[derive(Debug, PartialEq)]
pub(crate) struct Sourced {
    pub command: Command,
    /// Where to point an error: `scene.txt:12`, or an entity's name and id.
    pub origin: String,
    /// The entity it is, if it came from a project: the scene file maps which entity made
    /// which instances.
    pub owner: Option<String>,
}

/// Reads a scene's text into the commands it holds.
pub(crate) fn parse_text(path: &Path, text: &str) -> Result<Vec<Sourced>, String> {
    let mut commands = Vec::new();
    for (number, line) in text.lines().enumerate() {
        let origin = format!("{}:{}", path.display(), number + 1);
        if let Some(command) = parse_line(line).map_err(|e| format!("{origin}: {e}"))? {
            commands.push(Sourced {
                command,
                origin,
                owner: None,
            });
        }
    }
    Ok(commands)
}

/// Builds the scene described by the file at `path`. Returns it with any import warnings.
pub fn build(path: &Path, texture_size: Option<u32>) -> Result<(SceneAsset, Vec<String>), String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let base = path.parent().unwrap_or(Path::new("."));
    build_commands(base, parse_text(path, &text)?, texture_size)
}

/// The id under which the scene file maps the terrain's instances, and all that grows on it.
pub(crate) const TERRAIN_SOURCE: &str = "terrain";
/// The id under which it maps a circuit's land, road and planting.
pub(crate) const CIRCUIT_SOURCE: &str = "circuit";

/// Builds a scene from its commands, with files named relative to `base`. Returns it with any
/// import warnings. Commands from a project are mapped to the instances they made.
pub(crate) fn build_commands(
    base: &Path,
    commands: Vec<Sourced>,
    texture_size: Option<u32>,
) -> Result<(SceneAsset, Vec<String>), String> {
    let mapped = commands.iter().any(|c| c.owner.is_some());

    let mut scene = SceneAsset {
        texture_size: 1024,
        ..Default::default()
    };
    let mut terrain = None;
    let mut land_plan = crate::land::Plan::default();
    let mut water = None;
    let mut circuit = None;
    let mut lends = false;
    // Whether there has been a circuit, and a line of land, to refuse the two together.
    let (mut has_land, mut has_circuit) = (false, false);
    // Grouped by file, keeping first-seen order, so that each model is imported only once.
    let mut order: Vec<PathBuf> = Vec::new();
    let mut models: HashMap<PathBuf, ModelPlacements> = HashMap::new();
    // Placed once the model has been imported, since its shape decides the height.
    let mut seated: HashMap<PathBuf, Vec<Seated>> = HashMap::new();
    let mut gameplay = Gameplay::default();
    let mut skinned_models: std::collections::HashSet<PathBuf> = Default::default();
    // Each model an actor animates, and the `.anim` file it does so with.
    let mut animations: Vec<(PathBuf, String)> = Vec::new();
    let mut collides: HashMap<PathBuf, PlacedCollision> = HashMap::new();
    for Sourced {
        command,
        origin,
        owner,
    } in commands
    {
        let at = |e: String| format!("{origin}: {e}");
        // A circuit builds its own land, ground and planting: the lines that would build
        // another, one inside it, are refused wherever in the file they come.
        let land = match &command {
            Command::Terrain { .. } => Some("terrain"),
            Command::Land { .. } => Some("land"),
            Command::Path { .. } => Some("path"),
            Command::Yard { .. } => Some("yard"),
            Command::Clear { .. } => Some("clear"),
            Command::Meadow(_) => Some("meadow"),
            Command::Wood { .. } => Some("wood"),
            Command::Water { .. } => Some("water"),
            _ => None,
        };
        let circuits = matches!(command, Command::Circuit(_));
        has_land |= land.is_some();
        has_circuit |= circuits;
        if (land.is_some() || circuits) && has_land && has_circuit {
            let word = land.unwrap_or("circuit");
            return Err(at(format!(
                "`{word}` with `circuit`: a circuit builds its own land, so leave out `terrain`, `water` and the land's lines"
            )));
        }
        match Some(command) {
            None => {}
            Some(Command::TextureSize(size)) => scene.texture_size = size,
            Some(Command::Land { sand, dirt }) => {
                land_plan.banded = true;
                land_plan.sand = sand;
                land_plan.dirt = dirt;
            }
            Some(Command::Path { from, to, width }) => land_plan.paths.push((from, to, width)),
            Some(Command::Yard { from, to }) => land_plan.yards.push((from, to)),
            Some(Command::Clear { at, radius }) => land_plan.clear.push((at, radius)),
            Some(Command::Meadow(radius)) => land_plan.meadow = Some(radius),
            Some(Command::Wood {
                at,
                radius,
                spacing,
            }) => land_plan.woods.push((at, radius, spacing)),
            Some(Command::Terrain {
                size,
                cells,
                height,
                shore_x,
            }) => {
                terrain = Some((size, cells, height, shore_x));
            }
            Some(Command::Water { height, color }) => water = Some((height, color)),
            Some(Command::Ocean { height, ocean }) => {
                // A file's text travels with the scene; a built-in ocean goes by its name.
                let ocean = if ocean.ends_with(".ocean") {
                    let file = base.join(&ocean);
                    std::fs::read_to_string(&file)
                        .map_err(|e| format!("{}: {e}", file.display()))?
                } else {
                    ocean
                };
                scene.sea = Some(Sea {
                    level: height,
                    ocean,
                });
            }
            Some(Command::Pond {
                min,
                max,
                level,
                water,
            }) => {
                let still = scene.still_water.get_or_insert_with(StillWater::default);
                // Twin of `MAX_PONDS` in the renderer, which draws no more.
                if still.ponds.len() == 8 {
                    return Err("a scene has at most 8 ponds".into());
                }
                still.ponds.push([min.x, min.y, max.x, max.y, level]);
                if let Some(water) = water {
                    // As a sea's: a file's text travels with the scene, a built-in goes by name.
                    let water = if water.ends_with(".ocean") {
                        let file = base.join(&water);
                        std::fs::read_to_string(&file)
                            .map_err(|e| format!("{}: {e}", file.display()))?
                    } else {
                        water
                    };
                    if !still.water.is_empty() && still.water != water {
                        return Err(at(
                            "a scene's ponds hold one water, and another is already named"
                                .to_string(),
                        ));
                    }
                    still.water = water;
                }
            }
            Some(Command::Camera(viewpoint)) => scene.viewpoint = Some(viewpoint),
            Some(Command::Light(light)) => scene.lights.push(light),
            Some(Command::Model {
                path: model,
                position,
                seat,
                rotation,
                scale,
                collide,
                skinned,
            }) => {
                let full = base.join(&model);
                if skinned {
                    skinned_models.insert(full.clone());
                }
                let placements = placements_of(&mut order, &mut models, base, model);
                match seat {
                    Some(buried) => seated
                        .entry(full)
                        .or_default()
                        .push((position, rotation, scale, buried, collide, owner)),
                    None => {
                        placements
                            .whole
                            .push(placement_transform(position, rotation, scale));
                        let placed = collides.entry(full).or_default();
                        placed.whole.push(collide);
                        placed.whole_owners.push(owner);
                    }
                }
            }
            Some(Command::Part {
                path: model,
                name,
                position,
                rotation,
                scale,
                collide,
            }) => {
                let transform = placement_transform(position, rotation, scale);
                let full = base.join(&model);
                placements_of(&mut order, &mut models, base, model)
                    .parts
                    .push((name, transform));
                let placed = collides.entry(full).or_default();
                placed.parts.push(collide);
                placed.part_owners.push(owner);
            }
            Some(Command::Actor(actor)) => {
                let (position, yaw) = (actor.position, actor.yaw_degrees);
                let index = gameplay.game.actors.len();
                if let Some((model, scale, part, mesh_yaw, animation)) =
                    gameplay.add_actor(base, actor).map_err(at)?
                {
                    let full = base.join(&model);
                    if let Some(animation) = animation {
                        skinned_models.insert(full.clone());
                        animations.push((full.clone(), animation));
                    }
                    let placements = placements_of(&mut order, &mut models, base, model);
                    let transform =
                        placement_transform(position, Vec3::new(0.0, yaw + mesh_yaw, 0.0), scale);
                    // An actor bumps into things with its own collider, not its mesh.
                    let collide = collides.entry(full.clone()).or_default();
                    let placed = match part {
                        Some(name) => {
                            placements.parts.push((name, transform));
                            collide.parts.push(Collide::None);
                            collide.part_owners.push(owner);
                            Placed::Part(placements.parts.len() - 1)
                        }
                        None => {
                            placements.whole.push(transform);
                            collide.whole.push(Collide::None);
                            collide.whole_owners.push(owner);
                            Placed::Whole(placements.whole.len() - 1)
                        }
                    };
                    gameplay.meshes.push((index, full, placed));
                }
            }
            Some(Command::Blocker(blocker)) => gameplay.game.blockers.push(blocker),
            Some(Command::Input(input)) => gameplay.set_input(base, &input).map_err(at)?,
            Some(Command::Track(track)) => gameplay.set_track(base, &track).map_err(at)?,
            Some(Command::Surfaces(file)) => gameplay.set_surfaces(base, &file).map_err(at)?,
            Some(Command::Circuit(size)) => circuit = Some(size),
            Some(Command::Lends) => lends = true,
        }
    }

    if let Some(size) = texture_size {
        scene.texture_size = size;
    }
    let options = ImportOptions {
        texture_size: scene.texture_size,
        ..Default::default()
    };
    let mut warnings = Vec::new();
    for model in &order {
        println!("importing {}...", model.display());
        let options = ImportOptions {
            skinned: skinned_models.contains(model),
            ..options
        };
        let imported =
            import_gltf(model, &options).map_err(|e| format!("{}: {e}", model.display()))?;
        let name = model
            .file_stem()
            .map_or(String::new(), |s| s.to_string_lossy().to_string());
        warnings.extend(imported.warnings.iter().map(|w| format!("{name}: {w}")));
        if let Some(list) = seated.get(model) {
            let Some((_, _, amplitude, shore_x)) = terrain else {
                return Err(format!("{name}: 'ground' needs a terrain line"));
            };
            // Water hides whatever is below it, so the sea floor does not count.
            let water_level = water.map_or(f32::MIN, |(level, _)| level);
            let ground = |x: f32, z: f32| terrain_height(x, z, amplitude, shore_x).max(water_level);
            let points = model_points(&imported.scene);
            let placements = models.get_mut(model).expect("registered while parsing");
            for (position, rotation, scale, buried, collide, owner) in list {
                let (position, rotation, scale) = (*position, *rotation, *scale);
                let y = seat_height(&points, position, rotation, scale, *buried, ground);
                println!("  at {:.0} {:.0}: height {y:.2}", position.x, position.z);
                placements.whole.push(placement_transform(
                    Vec3::new(position.x, y, position.z),
                    rotation,
                    scale,
                ));
                let placed = collides.entry(model.clone()).or_default();
                placed.whole.push(*collide);
                placed.whole_owners.push(owner.clone());
            }
        }
        let names = imported.names.clone();
        let mut kit_pieces: Vec<GameFile> = Vec::new();
        let ranges = append_model(&mut scene, imported, &models[model])
            .map_err(|e| format!("{}: {e}", model.display()))?;
        for (_, graph_file) in animations.iter().filter(|(m, _)| m == model) {
            let skeleton = scene.skeletons.len() as u32 - 1;
            if has_clips_for(&scene.clips, graph_file, skeleton) {
                continue;
            }
            let text = gameplay
                .files
                .iter()
                .find(|f| f.name == *graph_file)
                .map(|f| f.text.clone())
                .ok_or_else(|| format!("{graph_file}: not read"))?;
            let clips = bake_clips(
                base,
                graph_file,
                &text,
                &scene.skeletons[skeleton as usize],
                skeleton,
            )?;
            scene.clips.extend(clips);
        }
        let whole_count = models[model].whole.len();
        for (actor, _, placed) in gameplay.meshes.iter().filter(|(_, m, _)| m == model) {
            let index = match placed {
                Placed::Whole(i) => *i,
                Placed::Part(i) => whole_count + i,
            };
            gameplay.game.actors[*actor].instances = Some(ranges[index].clone());
            // A kit's pieces are listed, for the game to show some and not others.
            if gameplay.kits.contains(actor) && matches!(placed, Placed::Whole(_)) {
                kit_pieces.push(GameFile {
                    name: solstice_game::actor::pieces_file(&gameplay.game.actors[*actor].file),
                    text: pieces_text(&names),
                });
            }
        }
        for file in kit_pieces.drain(..) {
            if !gameplay.files.iter().any(|f| f.name == file.name) {
                gameplay.files.push(file);
            }
        }
        let collide = collides.remove(model).unwrap_or_default();
        scene.collide.resize(scene.instances.len(), Collide::Auto);
        for (range, how) in ranges
            .iter()
            .zip(collide.whole.iter().chain(&collide.parts))
        {
            for instance in range.clone() {
                scene.collide[instance as usize] = *how;
            }
        }
        let placements = &models[model];
        let placed = placements
            .whole
            .iter()
            .chain(placements.parts.iter().map(|(_, at)| at));
        for ((range, owner), placement) in ranges
            .iter()
            .zip(collide.whole_owners.iter().chain(&collide.part_owners))
            .zip(placed)
        {
            if let Some(id) = owner {
                scene.sources.push(Source {
                    id: id.clone(),
                    instances: range.clone(),
                    placement: *placement,
                });
            }
        }
    }
    // A circuit's land, road and planting are built from its track, which also gives the
    // ground to drive on and the barriers to stop at.
    if let Some(size) = circuit {
        let name = gameplay
            .game
            .track
            .clone()
            .ok_or("`circuit` needs a `track` line: the land is shaped to the track")?;
        let text = gameplay
            .files
            .iter()
            .find(|f| f.name == name)
            .map(|f| f.text.clone())
            .ok_or_else(|| format!("{name}: not read"))?;
        let track = solstice_game::Track::parse(&text).map_err(|e| format!("{name}: {e}"))?;
        scene.collide.resize(scene.instances.len(), Collide::Auto);
        let first = scene.instances.len() as u32;
        let built = crate::circuit::build(&mut scene, &track, size);
        scene.collide.resize(scene.instances.len(), Collide::None);
        if mapped {
            scene.sources.push(Source {
                id: CIRCUIT_SOURCE.to_owned(),
                instances: first..scene.instances.len() as u32,
                placement: Mat4::IDENTITY,
            });
        }
        scene.ground = Some(built.ground);
        gameplay.game.blockers.extend(built.blockers);
    }
    scene.gameplay = gameplay.finish();

    scene.collide.resize(scene.instances.len(), Collide::Auto);
    if let Some((size, cells, height, shore_x)) = terrain {
        let first = scene.instances.len() as u32;
        let ground_layer = if land_plan.banded {
            crate::land::add(
                &mut scene,
                &land_plan,
                size,
                cells,
                &|x, z| terrain_height(x, z, height, shore_x),
                &|x, z| inland(x, z, shore_x),
            )
        } else {
            add_terrain(&mut scene, size, cells, height, shore_x)
        };
        if let Some((level, color)) = water {
            add_water(&mut scene, level, color, ground_layer);
        }
        // The ground collides as a grid of heights, and water not at all.
        scene.collide.resize(scene.instances.len(), Collide::None);
        scene.ground = Some(ground_heights(size, height, shore_x));
        if mapped {
            scene.sources.push(Source {
                id: TERRAIN_SOURCE.to_owned(),
                instances: first..scene.instances.len() as u32,
                placement: Mat4::IDENTITY,
            });
        }
    }
    scene.sources.sort_by_key(|source| source.instances.start);
    // A scene that lends says what it has, for those baked beside it to leave out.
    if lends {
        scene.gameplay.push(GameFile {
            name: solstice_asset::join::LENDS.to_owned(),
            text: crate::lend::lends(&scene),
        });
    }
    Ok((scene, warnings))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_graph_shared_by_two_models_is_baked_for_each_skeleton() {
        let clip = |skeleton| solstice_asset::Clip {
            name: solstice_game::animation::clip_key("a.anim", "walk"),
            skeleton,
            ..Default::default()
        };
        let clips = vec![clip(0)];
        assert!(has_clips_for(&clips, "a.anim", 0));
        assert!(!has_clips_for(&clips, "a.anim", 1));
        assert!(!has_clips_for(&clips, "b.anim", 0));
    }

    #[test]
    fn the_land_commands_parse_and_refuse_nonsense() {
        assert_eq!(
            parse_line("land 10 5"),
            Ok(Some(Command::Land {
                sand: 10.0,
                dirt: 5.0
            }))
        );
        assert_eq!(
            parse_line("path 1 2 3 4 2.5"),
            Ok(Some(Command::Path {
                from: glam::Vec2::new(1.0, 2.0),
                to: glam::Vec2::new(3.0, 4.0),
                width: 2.5
            }))
        );
        assert!(matches!(
            parse_line("yard -1 -1 1 1"),
            Ok(Some(Command::Yard { .. }))
        ));
        assert!(matches!(
            parse_line("clear 0 0 30"),
            Ok(Some(Command::Clear { .. }))
        ));
        assert_eq!(parse_line("meadow 75"), Ok(Some(Command::Meadow(75.0))));
        assert!(matches!(
            parse_line("wood 1 2 20 7"),
            Ok(Some(Command::Wood { .. }))
        ));
        for bad in [
            "land -1 5",
            "meadow 0",
            "wood 1 2 20 0",
            "wood 1 2 0 7",
            "path 1 2 3 4",
            "land 1",
        ] {
            assert!(parse_line(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn blank_lines_and_comments_are_ignored() {
        assert_eq!(parse_line(""), Ok(None));
        assert_eq!(parse_line("   # just a note"), Ok(None));
    }

    #[test]
    fn model_line_parses_every_field() {
        let command = parse_line("model a/b.gltf 1 2 3 90 0.5  # note").unwrap();
        assert_eq!(
            command,
            Some(Command::Model {
                path: "a/b.gltf".to_string(),
                position: Vec3::new(1.0, 2.0, 3.0),
                seat: None,
                rotation: Vec3::new(0.0, 90.0, 0.0),
                scale: 0.5,
                collide: Collide::Auto,
                skinned: false,
            })
        );
    }

    #[test]
    fn a_height_can_ask_for_the_ground() {
        assert_eq!(parse_height("1.5"), Ok((1.5, None)));
        assert_eq!(parse_height("ground:0.25"), Ok((0.0, Some(0.25))));
        assert!(parse_height("ground").unwrap().1.is_some());
        assert!(parse_height("ground:2").is_err());
        assert!(parse_height("grund").is_err());
    }

    /// A tall column, narrowest at its foot: one point at the bottom, then a wide ring.
    fn column() -> Vec<Vec3> {
        vec![
            Vec3::ZERO,
            Vec3::new(1.0, 1.0, 0.0),
            Vec3::new(-1.0, 1.0, 0.0),
            Vec3::new(0.0, 10.0, 0.0),
        ]
    }

    #[test]
    fn seating_on_flat_ground_rests_the_lowest_point_on_it() {
        let y = seat_height(&column(), Vec3::ZERO, Vec3::ZERO, 1.0, 0.0, |_, _| 2.0);
        assert!((y - 2.0).abs() < 1.0e-5);
    }

    #[test]
    fn burying_sinks_the_model_by_that_share_of_its_height() {
        let y = seat_height(&column(), Vec3::ZERO, Vec3::ZERO, 1.0, 0.2, |_, _| 0.0);
        assert!(
            (y + 1.0).abs() < 1.0e-5,
            "the ring at height 1 is at ground level: {y}"
        );
    }

    #[test]
    fn seating_leaves_no_gap_on_a_slope() {
        let ground = |x: f32, _: f32| x * 0.5;
        let y = seat_height(&column(), Vec3::ZERO, Vec3::ZERO, 1.0, 0.2, ground);
        for point in &column()[..3] {
            assert!(
                point.y + y <= ground(point.x, point.z) + 1.0e-5,
                "{point} floats"
            );
        }
    }

    #[test]
    fn part_and_water_lines_parse() {
        assert_eq!(
            parse_line("part kit.gltf wall_01 1 2 3 90 2").unwrap(),
            Some(Command::Part {
                path: "kit.gltf".to_string(),
                name: "wall_01".to_string(),
                position: Vec3::new(1.0, 2.0, 3.0),
                rotation: Vec3::new(0.0, 90.0, 0.0),
                scale: 2.0,
                collide: Collide::Auto,
            })
        );
        assert_eq!(
            parse_line("water -1 0.1 0.2 0.3").unwrap(),
            Some(Command::Water {
                height: -1.0,
                color: [0.1, 0.2, 0.3],
            })
        );
        assert_eq!(
            parse_line("ocean -0.6 coastal").unwrap(),
            Some(Command::Ocean {
                height: -0.6,
                ocean: "coastal".to_string(),
            })
        );
        assert!(parse_line("ocean -0.6").is_err());
    }

    #[test]
    fn pond_lines_parse_with_their_corners_in_either_order() {
        assert_eq!(
            parse_line("pond 4 -23 -2 -37 1.5").unwrap(),
            Some(Command::Pond {
                min: glam::Vec2::new(-2.0, -37.0),
                max: glam::Vec2::new(4.0, -23.0),
                level: 1.5,
                water: None,
            })
        );
        assert_eq!(
            parse_line("pond 0 0 10 5 0.2 coastal").unwrap(),
            Some(Command::Pond {
                min: glam::Vec2::ZERO,
                max: glam::Vec2::new(10.0, 5.0),
                level: 0.2,
                water: Some("coastal".to_string()),
            })
        );
        assert!(parse_line("pond 0 0 10 5").is_err());
        assert!(parse_line("pond nan 0 10 5 0.2").is_err());
        assert!(parse_line("pond 0 0 10 5 inf").is_err());
        assert!(parse_line("pond 0 0 0 5 0.2").is_err());
        assert!(parse_line("pond 0 0 10 5 0.2 coastal more").is_err());
    }

    #[test]
    fn the_seabed_falls_below_the_shore_only_on_the_sea_side() {
        assert!(
            terrain_height(-50.0, 0.0, 0.0, 40.0).abs() < 0.1,
            "inland stays level"
        );
        assert!(
            terrain_height(10.0, 0.0, 0.0, 40.0).abs() < 0.1,
            "level behind the shoreline, wherever it wanders"
        );
        assert!(
            terrain_height(130.0, 0.0, 0.0, 40.0) < -8.0,
            "deep offshore"
        );
    }

    #[test]
    fn the_terrain_edges_lie_under_the_sea_on_every_side() {
        // The cove's terrain is 900 m square, with rolling ground of amplitude 0.3.
        let water = -0.6;
        for i in 0..=200 {
            let along = -450.0 + 4.5 * i as f32;
            for (x, z) in [
                (-445.0, along),
                (along, 445.0),
                (along, -445.0),
                (445.0, along),
            ] {
                let height = terrain_height(x, z, 0.3, 52.0);
                assert!(
                    height < water - 4.0,
                    "({x}, {z}) is {height}, above the seabed"
                );
            }
        }
        // The fort and the ground around it are untouched.
        for (x, z) in [
            (0.0, 0.0),
            (-68.0, 0.0),
            (-30.0, 92.0),
            (-30.0, -92.0),
            (-150.0, 0.0),
        ] {
            assert!(
                terrain_height(x, z, 0.0, 52.0).abs() < 0.1,
                "({x}, {z}) moved"
            );
        }
    }

    /// A scratch folder holding the files given, removed when dropped.
    struct Folder(PathBuf);

    impl Folder {
        fn with(files: &[(&str, &[u8])]) -> Self {
            static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
            let unique = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let dir =
                std::env::temp_dir().join(format!("solstice-bake-{}-{unique}", std::process::id()));
            for (name, bytes) in files {
                let path = dir.join(name);
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(path, bytes).unwrap();
            }
            Self(dir)
        }
    }

    impl Drop for Folder {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A glTF of one triangle drawn twice, by two nodes, so that each placement is two instances.
    fn two_triangles() -> (Vec<u8>, Vec<u8>) {
        let mut bin = Vec::new();
        for value in [0.0f32, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0] {
            bin.extend_from_slice(&value.to_le_bytes());
        }
        let json = r#"{
            "asset": {"version": "2.0"},
            "scene": 0,
            "scenes": [{"nodes": [0, 1]}],
            "nodes": [{"name": "kit_door", "mesh": 0}, {"name": "kit_post", "mesh": 0, "translation": [0, 2, 0]}],
            "meshes": [{"primitives": [{"attributes": {"POSITION": 0}}]}],
            "buffers": [{"uri": "tri.bin", "byteLength": 36}],
            "bufferViews": [{"buffer": 0, "byteLength": 36}],
            "accessors": [{"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3",
                           "min": [0, 0, 0], "max": [1, 1, 0]}]
        }"#;
        (json.as_bytes().to_vec(), bin)
    }

    #[test]
    fn actors_their_meshes_and_scripts_are_baked_into_the_scene() {
        let (gltf, bin) = two_triangles();
        let folder = Folder::with(&[
            (
                "scene.txt",
                b"model models/tri.gltf 0 0 0 0 1\nactor fort/gate.actor 20 0 3 90 locked=true\nactor fort/gate.actor -5 1 0 0\ninput fort/play.input\n",
            ),
            ("models/tri.gltf", &gltf),
            ("models/tri.bin", &bin),
            (
                "fort/gate.actor",
                b"name gate\nmesh ../models/tri.gltf 2\nscript gate.star\nproperty locked bool false\n",
            ),
            ("fort/gate.star", b"def on_tick(self, dt):\n    self.yaw += dt\n"),
            ("fort/play.input", b"interact key KeyF\n"),
        ]);
        let (scene, _) = build(&folder.0.join("scene.txt"), None).unwrap();
        // The model line's two instances, then two for each gate.
        assert_eq!(scene.instances.len(), 6);
        let names: Vec<&str> = scene.gameplay.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(
            names,
            [
                GAME_FILE,
                "fort/gate.actor",
                "fort/gate.star",
                "fort/play.input"
            ]
        );
        let game = SceneGame::parse(&scene.gameplay[0].text).unwrap();
        assert_eq!(game.input.as_deref(), Some("fort/play.input"));
        assert_eq!(game.actors[0].instances, Some(2..4));
        assert_eq!(game.actors[1].instances, Some(4..6));
        let second_node = scene.instances[3].transform.transform_point3(Vec3::ZERO);
        let expected =
            placement_transform(Vec3::new(20.0, 0.0, 3.0), Vec3::new(0.0, 90.0, 0.0), 2.0)
                .transform_point3(Vec3::new(0.0, 2.0, 0.0));
        assert!(second_node.abs_diff_eq(expected, 1.0e-3), "{second_node}");

        // And it plays: drawn first where it was baked.
        let instances = scene.instances.clone();
        let mut play = solstice_game::Play::load(
            &scene.gameplay,
            |id| instances.get(id as usize).map(|i| i.transform),
            None,
        )
        .unwrap();
        let mut placed = Vec::new();
        // One step in, before the script has turned it far.
        play.frame(
            1.0 / 60.0 * 1.001,
            &mut solstice_game::Devices::new(),
            |id, at| placed.push((id, at)),
        );
        assert_eq!(placed.len(), 4);
        for (id, at) in placed {
            assert!(
                at.abs_diff_eq(scene.instances[id as usize].transform, 0.1),
                "{id}"
            );
        }
    }

    #[test]
    fn a_line_says_how_it_collides() {
        let collide = |line: &str| match parse_line(line).unwrap().unwrap() {
            Command::Model { collide, .. } | Command::Part { collide, .. } => collide,
            _ => panic!("{line}"),
        };
        assert_eq!(collide("model a.gltf 1 2 3 0 1"), Collide::Auto);
        assert_eq!(
            collide("model a.gltf 1 2 3 0 1 collide=none"),
            Collide::None
        );
        assert_eq!(
            collide("part a.gltf b 1 2 3 0 1 collide=hull"),
            Collide::Hull
        );
        assert_eq!(
            collide("part a.gltf b 1 2 3 0 1 collide=coarse"),
            Collide::Coarse
        );
        let error = parse_line("model a.gltf 1 2 3 0 1 collide=soft").unwrap_err();
        assert!(error.contains("none, hull or coarse"), "{error}");
        assert_eq!(
            parse_line("blocker 1 2 3 4 5 6 90").unwrap(),
            Some(Command::Blocker(Blocker {
                centre: Vec3::new(1.0, 2.0, 3.0),
                size: Vec3::new(4.0, 5.0, 6.0),
                yaw_degrees: 90.0,
            }))
        );
    }

    #[test]
    fn collision_is_baked_for_what_can_be_walked_on() {
        let (gltf, bin) = two_triangles();
        let folder = Folder::with(&[
            (
                "scene.txt",
                b"terrain 20 8 0.2 100\n\
                  model tri.gltf 0 0 0 0 1\n\
                  model tri.gltf 5 0 0 0 1 collide=none\n\
                  part tri.gltf door 9 0 0 0 1 collide=hull\n\
                  actor door.actor 2 0 0 0\n\
                  actor post.actor 3 0 0 0\n\
                  blocker 7 1 0 2 2 2 0\n",
            ),
            ("tri.gltf", &gltf),
            ("tri.bin", &bin),
            (
                "door.actor",
                b"name door\nmesh tri.gltf 1 part=door\ncollider box 1 2 0.1\n",
            ),
            ("post.actor", b"name post\nmesh tri.gltf 2\n"),
        ]);
        let (scene, _) = build(&folder.0.join("scene.txt"), None).unwrap();
        // Whole placements first, each two instances; parts after, one each; terrain last.
        let collide = &scene.collide;
        assert_eq!(collide.len(), scene.instances.len());
        assert_eq!(
            &collide[0..2],
            [Collide::Auto, Collide::Auto],
            "the first model"
        );
        assert_eq!(
            &collide[2..4],
            [Collide::None, Collide::None],
            "collide=none"
        );
        assert_eq!(
            &collide[4..6],
            [Collide::None, Collide::None],
            "the post actor"
        );
        assert_eq!(collide[6], Collide::Hull, "the part");
        assert_eq!(
            collide[7],
            Collide::None,
            "the door actor: its collider does the work"
        );
        assert_eq!(
            *collide.last().unwrap(),
            Collide::None,
            "the terrain: a heightfield instead"
        );

        let game = SceneGame::parse(&scene.gameplay[0].text).unwrap();
        assert_eq!(game.actors[0].instances, Some(7..8), "a part's instance");
        assert_eq!(game.actors[1].instances, Some(4..6), "the post's two");
        assert_eq!(game.blockers.len(), 1);

        let ground = scene.ground.as_ref().unwrap();
        assert_eq!((ground.columns, ground.rows), (21, 21));
        assert_eq!(ground.origin, [-10.0, -10.0]);
        assert_eq!(ground.spacing, [1.0, 1.0]);
        let (x, z) = (3.0, -2.0);
        let row = 8;
        let column = 13;
        let expected = terrain_height(x, z, 0.2, 100.0);
        assert_eq!(ground.heights[row * 21 + column], expected);

        let collision = solstice_builder::collision::build_collision(&scene).unwrap();
        assert_eq!(collision.heightfield.as_ref(), Some(ground));
        assert_eq!(collision.instance_shapes[2], solstice_asset::NO_SHAPE);
        assert_ne!(collision.instance_shapes[0], solstice_asset::NO_SHAPE);
    }

    #[test]
    fn a_blocker_alone_is_gameplay() {
        let (gltf, bin) = two_triangles();
        let folder = Folder::with(&[
            (
                "scene.txt",
                b"model tri.gltf 0 0 0 0 1\nblocker 0 0 0 1 1 1 0\n",
            ),
            ("tri.gltf", &gltf),
            ("tri.bin", &bin),
        ]);
        let (scene, _) = build(&folder.0.join("scene.txt"), None).unwrap();
        assert_eq!(scene.gameplay.len(), 1);
    }

    /// A circuit's track: three points.
    const TRACK: &[u8] = b"p 0 0 0 1 0 0 6 6
p 0 0 50 1 0 0 6 6
p 50 0 50 1 0 0 6 6
";

    #[test]
    fn a_circuit_and_a_terrain_are_not_both_built() {
        let folder = Folder::with(&[
            (
                "scene.txt",
                b"terrain 400 64 0.3 52\ntrack c.track\ncircuit 900\n",
            ),
            (
                "c.track",
                b"p 0 0 0 1 0 0 6 6\np 0 0 50 1 0 0 6 6\np 50 0 50 1 0 0 6 6\n",
            ),
        ]);
        let error = build(&folder.0.join("scene.txt"), None).unwrap_err();
        assert!(error.contains("a circuit builds its own land"), "{error}");
        assert!(error.contains(":3:"), "on the circuit's line: {error}");
        // And the other way round, with a line of the land's after the circuit.
        let folder = Folder::with(&[
            (
                "scene.txt",
                b"track c.track
circuit 900
wood 0 0 40 9
",
            ),
            ("c.track", TRACK),
        ]);
        let error = build(&folder.0.join("scene.txt"), None).unwrap_err();
        assert!(error.contains(":3: `wood` with `circuit`"), "{error}");
    }

    #[test]
    fn a_scene_names_the_map_of_its_ground_and_one_that_lends_lists_what_it_has() {
        let (gltf, bin) = two_triangles();
        let folder = Folder::with(&[
            (
                "scene.txt",
                b"model tri.gltf 0 0 0 0 1
surfaces ground.surfaces
lends
",
            ),
            ("tri.gltf", &gltf),
            ("tri.bin", &bin),
            // Four squares of mud and four of rock.
            (
                "ground.surfaces",
                b"surfaces -2 -1 1 4 2
4g
4j
",
            ),
        ]);
        let (scene, _) = build(&folder.0.join("scene.txt"), None).unwrap();
        let file = |name: &str| {
            let found = scene.gameplay.iter().find(|file| file.name == name);
            found.map(|file| file.text.as_str())
        };
        // The game is told of the map, which is brought along as it was written.
        let game = SceneGame::parse(file(GAME_FILE).expect("the scene has a game")).unwrap();
        assert_eq!(game.surfaces.as_deref(), Some("ground.surfaces"));
        let map = solstice_game::SurfaceMap::parse(file("ground.surfaces").unwrap()).unwrap();
        let at = |z: f32| map.at(Vec3::new(0.0, 0.0, z));
        assert_eq!(
            (at(-0.5), at(0.5)),
            (solstice_game::Surface::Mud, solstice_game::Surface::Rock)
        );
        // And it lists every mesh it has, each with its key, for scenes baked beside it.
        let lends = file(solstice_asset::join::LENDS).expect("it lends");
        let meshes = lends
            .lines()
            .filter(|line| line.starts_with("mesh "))
            .count();
        assert_eq!(meshes, scene.meshes.len());
        assert!(meshes > 0);

        // A map that cannot be read is found while baking, with its line.
        let folder = Folder::with(&[
            (
                "scene.txt",
                b"surfaces ground.surfaces
",
            ),
            (
                "ground.surfaces",
                b"4g
",
            ),
        ]);
        let error = build(&folder.0.join("scene.txt"), None).unwrap_err();
        assert!(error.contains("ground.surfaces"), "{error}");
        assert!(error.contains("expected `surfaces"), "{error}");
    }

    #[test]
    fn a_scene_without_actors_has_no_gameplay() {
        let (gltf, bin) = two_triangles();
        let folder = Folder::with(&[
            ("scene.txt", b"model tri.gltf 0 0 0 0 1\n"),
            ("tri.gltf", &gltf),
            ("tri.bin", &bin),
        ]);
        let (scene, _) = build(&folder.0.join("scene.txt"), None).unwrap();
        assert!(scene.gameplay.is_empty());
    }

    #[test]
    fn gameplay_mistakes_are_found_while_baking() {
        let cases: [(&[u8], &str); 8] = [
            (
                b"actor gate.actor 0 0 0 0 colour=red\n",
                "has no property `colour`",
            ),
            (b"actor gate.actor 0 0 0 0 locked=maybe\n", "true or false"),
            (b"actor nowhere.actor 0 0 0 0\n", "nowhere.actor"),
            (b"actor broken.actor 0 0 0 0\n", "broken.star:2"),
            (b"input bad.input\n", "bad.input: line 1"),
            (
                b"actor rock.actor 0 0 0 0 size=2\n",
                "has no script, so no properties",
            ),
            (
                b"actor eye.actor 0 0 0 0\nactor eye.actor 1 0 0 0\n",
                "only one can",
            ),
            (
                b"actor gate.actor 0 0 0 0 instances=0+5\n",
                "worked out by the bake",
            ),
        ];
        for (scene, part) in cases {
            let folder = Folder::with(&[
                ("scene.txt", scene),
                (
                    "gate.actor",
                    b"name gate\nscript gate.star\nproperty locked bool false\n",
                ),
                ("gate.star", b"def on_begin(self):\n    pass\n"),
                ("rock.actor", b"name rock\n"),
                ("eye.actor", b"name eye\ncamera 0 1 0\n"),
                ("broken.actor", b"name broken\nscript broken.star\n"),
                ("broken.star", b"def on_tick(self, dt):\n    x = = 1\n"),
                ("bad.input", b"jump\n"),
            ]);
            let error = build(&folder.0.join("scene.txt"), None).unwrap_err();
            assert!(error.contains(part), "{error}");
            assert!(error.contains("scene.txt:"), "{error}");
        }
    }

    #[test]
    fn mistakes_are_reported_clearly() {
        assert!(parse_line("nonsense 1 2")
            .unwrap_err()
            .contains("unknown command"));
        assert!(parse_line("camera 1 2 3")
            .unwrap_err()
            .contains("takes 5 values"));
        assert!(parse_line("terrain 1 x 3 4")
            .unwrap_err()
            .contains("not a number"));
    }
}
