//! Editor projects (`docs/editor.md`, `solstice-project`): baking a project's scene, and
//! converting a scene's text into one.
//!
//! A scene is baked by lowering each of its settings and entities to the line of scene text that
//! says the same thing, read by the same parser, so a project and a text can never mean
//! different things, and what the text checks the project is checked for too. The one thing a
//! line can't say, a model tilted about X or Z, is set on the parsed command afterwards.
//!
//! Paths in a project are relative to its folder, the one holding `solstice.project`.

use crate::scene_file::{self, parse_line, Command, Sourced};
use glam::Vec3;
use solstice_asset::{Collide, SceneAsset};
use solstice_project::{self as project, Entity, Project, Scene, Transform, PROJECT_FILE};
use std::fmt::Write;
use std::path::{Path, PathBuf};

/// The folder of the project a scene file belongs to: the nearest one above it holding a
/// `solstice.project`.
pub fn project_folder(scene: &Path) -> Result<PathBuf, String> {
    let start = std::path::absolute(scene).map_err(|e| format!("{}: {e}", scene.display()))?;
    start
        .ancestors()
        .skip(1)
        .find(|folder| folder.join(PROJECT_FILE).is_file())
        .map(Path::to_path_buf)
        .ok_or_else(|| {
            format!(
                "{}: no {PROJECT_FILE} in its folder or any above it",
                scene.display()
            )
        })
}

/// Builds a project's scene from its file. Returns it with any import warnings.
pub fn build(path: &Path, texture_size: Option<u32>) -> Result<(SceneAsset, Vec<String>), String> {
    let folder = project_folder(path)?;
    let read =
        |file: &Path| std::fs::read_to_string(file).map_err(|e| format!("{}: {e}", file.display()));
    Project::parse(&read(&folder.join(PROJECT_FILE))?)
        .map_err(|e| format!("{}: {e}", folder.join(PROJECT_FILE).display()))?;
    let scene = Scene::parse(&read(path)?).map_err(|e| format!("{}: {e}", path.display()))?;
    let name = path
        .file_name()
        .map_or(String::new(), |n| n.to_string_lossy().into());
    build_scene(&scene, &name, &folder, texture_size)
}

/// Builds a project's scene, already read, with its files named relative to `folder`. `name`
/// names the scene in errors.
pub fn build_scene(
    scene: &Scene,
    name: &str,
    folder: &Path,
    texture_size: Option<u32>,
) -> Result<(SceneAsset, Vec<String>), String> {
    scene_file::build_commands(folder, lower(scene, name)?, texture_size)
}

/// A built scene as its file holds it: textures compressed, paged and encoded, as `bake` writes
/// it and the editor's bake in the browser does too.
pub fn encode(mut scene: SceneAsset) -> Vec<u8> {
    solstice_builder::compress_textures(&mut scene);
    solstice_asset::file::encode(&solstice_builder::paginate(&scene))
}

/// The commands a scene says, in the order the scene text would have them: its settings, then
/// its entities, parents before their children. `file` names the scene in errors.
pub(crate) fn lower(scene: &Scene, file: &str) -> Result<Vec<Sourced>, String> {
    let mut out = Vec::new();
    let mut setting = |line: String| -> Result<(), String> {
        let origin = format!("{file}: `{line}`");
        let command = parse_line(&line)
            .map_err(|e| format!("{origin}: {e}"))?
            .expect("a setting is never blank");
        out.push(Sourced {
            command,
            origin,
            owner: None,
        });
        Ok(())
    };
    if let Some(size) = scene.texture_size {
        setting(format!("texture_size {size}"))?;
    }
    if let Some(t) = &scene.terrain {
        setting(format!(
            "terrain {} {} {} {}",
            t.size, t.cells, t.height, t.shore_x
        ))?;
    }
    if let Some(land) = &scene.land {
        setting(format!("land {} {}", land.sand, land.dirt))?;
    }
    if let Some(radius) = scene.meadow {
        setting(format!("meadow {radius}"))?;
    }
    if let Some(water) = &scene.water {
        let [r, g, b] = water.color;
        setting(format!("water {} {r} {g} {b}", water.level))?;
    }
    if let Some(sea) = &scene.sea {
        setting(format!(
            "ocean {} {}",
            sea.level,
            word(&sea.ocean, "an ocean")?
        ))?;
    }
    if let Some(camera) = &scene.camera {
        let [x, y, z] = camera.position;
        setting(format!(
            "camera {x} {y} {z} {} {}",
            camera.yaw_radians, camera.pitch_radians
        ))?;
    }
    if let Some(input) = &scene.game.input {
        setting(format!("input {}", word(input, "an input file")?))?;
    }
    if let Some(track) = &scene.game.track {
        setting(format!("track {}", word(track, "a track")?))?;
    }
    if let Some(size) = scene.game.circuit {
        setting(format!("circuit {size}"))?;
    }
    for entity in scene.walk() {
        let origin = format!("{file}: {} ({})", entity.name, entity.id);
        let Some(line) = entity_line(entity).map_err(|e| format!("{origin}: {e}"))? else {
            continue;
        };
        let mut command = parse_line(&line)
            .map_err(|e| format!("{origin}: {e}"))?
            .expect("an entity's line is never blank");
        // A line turns things only about Y; the rest of the turn is set here.
        if let Some(t) = &entity.transform {
            if let Command::Model { rotation, .. } | Command::Part { rotation, .. } = &mut command {
                *rotation = Vec3::from(t.rotation);
            }
        }
        out.push(Sourced {
            command,
            origin,
            owner: Some(entity.id.clone()),
        });
    }
    Ok(out)
}

/// A path or name as one word of a line, which is all the scene text can hold.
fn word<'a>(text: &'a str, what: &str) -> Result<&'a str, String> {
    if text.is_empty() || text.contains(char::is_whitespace) || text.contains('#') {
        Err(format!(
            "{what} named `{text}`: names can't yet be empty or hold spaces or #"
        ))
    } else {
        Ok(text)
    }
}

/// The scene text line that says what an entity is, or nothing for a folder.
fn entity_line(entity: &Entity) -> Result<Option<String>, String> {
    let need_transform = || {
        entity
            .transform
            .clone()
            .ok_or_else(|| "it has no transform, so it is nowhere".to_owned())
    };
    // Only models and parts can turn other than about Y, be scaled or stand on the ground.
    let only_yaw = |t: &Transform, what: &str| {
        if t.rotation[0] != 0.0 || t.rotation[2] != 0.0 {
            Err(format!("{what} turns only about Y"))
        } else if t.scale != 1.0 {
            Err(format!("{what} can't be scaled"))
        } else if t.ground.is_some() {
            Err(format!("{what} can't stand on the ground: give its height"))
        } else {
            Ok(())
        }
    };
    let mut line = String::new();
    if let Some(mesh) = &entity.mesh {
        let t = need_transform()?;
        let [x, y, z] = t.position;
        let height = match t.ground {
            None => y.to_string(),
            Some(buried) => format!("ground:{buried}"),
        };
        let asset = word(&mesh.asset, "a model")?;
        match &mesh.part {
            None => write!(line, "model {asset} {x} {height} {z}"),
            Some(part) => {
                if t.ground.is_some() {
                    return Err("a part can't stand on the ground: give its height".into());
                }
                if mesh.skinned {
                    return Err("a part can't be skinned".into());
                }
                write!(line, "part {asset} {} {x} {y} {z}", word(part, "a part")?)
            }
        }
        .expect("writing to a string");
        write!(line, " {} {}", t.rotation[1], t.scale).expect("writing to a string");
        match mesh.collide {
            project::Collide::Auto => {}
            project::Collide::None => line.push_str(" collide=none"),
            project::Collide::Hull => line.push_str(" collide=hull"),
            project::Collide::Coarse => line.push_str(" collide=coarse"),
        }
        if mesh.skinned {
            line.push_str(" skinned");
        }
    } else if let Some(actor) = &entity.actor {
        let t = need_transform()?;
        only_yaw(&t, "an actor")?;
        let [x, y, z] = t.position;
        let asset = word(&actor.asset, "an actor")?;
        write!(line, "actor {asset} {x} {y} {z} {}", t.rotation[1]).expect("writing");
        for (name, value) in &actor.properties {
            write!(
                line,
                " {}={}",
                word(name, "a property")?,
                word(value, "a value")?
            )
            .expect("writing");
        }
    } else if let Some(light) = &entity.light {
        let t = need_transform()?;
        let [x, y, z] = t.position;
        let [r, g, b] = light.intensity;
        line = format!("light {x} {y} {z} {} {r} {g} {b}", light.range);
    } else if let Some(blocker) = &entity.blocker {
        let t = need_transform()?;
        only_yaw(&t, "a blocker")?;
        let [x, y, z] = t.position;
        let [sx, sy, sz] = blocker.size;
        line = format!("blocker {x} {y} {z} {sx} {sy} {sz} {}", t.rotation[1]);
    } else if let Some(wood) = &entity.wood {
        let [x, _, z] = need_transform()?.position;
        line = format!("wood {x} {z} {} {}", wood.radius, wood.spacing);
    } else if let Some(clear) = &entity.clear {
        let [x, _, z] = need_transform()?.position;
        line = format!("clear {x} {z} {}", clear.radius);
    } else if let Some(path) = &entity.path {
        let ([a, b], [c, d]) = (path.from, path.to);
        line = format!("path {a} {b} {c} {d} {}", path.width);
    } else if let Some(yard) = &entity.yard {
        let ([a, b], [c, d]) = (yard.from, yard.to);
        line = format!("yard {a} {b} {c} {d}");
    } else if let Some(pond) = &entity.pond {
        let ([a, b], [c, d]) = (pond.min, pond.max);
        line = format!("pond {a} {b} {c} {d} {}", pond.level);
        if let Some(water) = &pond.water {
            write!(line, " {}", word(water, "a water")?).expect("writing");
        }
    } else {
        return Ok(None);
    }
    Ok(Some(line))
}

/// Converts a scene's text into a project's scene. Each line that places something becomes an
/// entity, named from what it places; the comments above it become its note; and a run of
/// banner comments (`# ---- Towers on the four corners.`) starts a folder named by its first
/// sentence, holding what follows until the next. `name` seeds the entities' ids, so the same
/// text always converts to the same ids.
pub fn convert(text: &str, name: &str) -> Result<Scene, String> {
    let mut scene = Scene::new();
    let seed = name.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3)
    });
    // A banner's folder and the line after it share a line number, but not an id.
    let id = |line: usize| project::make_id(seed ^ ((line as u64) << 1));
    let folder_id = |line: usize| project::make_id(seed ^ ((line as u64) << 1 | 1));
    // What the comments just above say, and the banner being read.
    let mut note: Vec<String> = Vec::new();
    let mut banner: Vec<String> = Vec::new();
    let mut folder: Option<Entity> = None;
    let mut started = false;
    let mut entities: Vec<Entity> = Vec::new();
    let close = |folder: &mut Option<Entity>, entities: &mut Vec<Entity>| {
        if let Some(folder) = folder.take() {
            entities.push(folder);
        }
    };
    for (number, line) in text.lines().enumerate() {
        let trimmed = line.trim();
        let at = |e: String| format!("{name}:{}: {e}", number + 1);
        if let Some(comment) = trimmed.strip_prefix('#') {
            let comment = comment.trim();
            if let Some(rest) = comment.strip_prefix("----") {
                let rest = rest.trim();
                // A rule, `# ---------`, divides but names nothing.
                if !rest.trim_start_matches('-').is_empty() {
                    banner.push(rest.to_owned());
                }
            } else if !banner.is_empty() || !comment.is_empty() {
                // A plain comment after a banner ends it.
                if !banner.is_empty() {
                    close(&mut folder, &mut entities);
                    folder = Some(open_folder(&mut banner, &mut note, folder_id(number)));
                }
                note.push(comment.to_owned());
            }
            continue;
        }
        if !banner.is_empty() {
            close(&mut folder, &mut entities);
            folder = Some(open_folder(&mut banner, &mut note, folder_id(number)));
        }
        let trailing = line
            .split_once('#')
            .map(|(_, c)| c.trim().to_owned())
            .filter(|c| !c.is_empty());
        let Some(command) = parse_line(line).map_err(at)? else {
            // A blank line: comments before the first command describe the whole scene.
            if !started && !note.is_empty() {
                scene.note = std::mem::take(&mut note).join("\n");
            }
            continue;
        };
        started = true;
        let mut lines: Vec<String> = std::mem::take(&mut note);
        lines.extend(trailing);
        let entity = to_entity(&mut scene, command, id(number)).map_err(at)?;
        match entity {
            Some(mut entity) => {
                entity.note = lines.join("\n");
                match &mut folder {
                    Some(folder) => folder.children.push(entity),
                    None => entities.push(entity),
                }
            }
            // A setting's comment stays with the scene.
            None if !lines.is_empty() => {
                if !scene.note.is_empty() {
                    scene.note.push('\n');
                }
                scene.note.push_str(&lines.join("\n"));
            }
            None => {}
        }
    }
    // Comments after the last command, or a banner with nothing under it, describe the scene.
    let rest: Vec<String> = note.into_iter().chain(banner).collect();
    if !rest.is_empty() {
        if !scene.note.is_empty() {
            scene.note.push('\n');
        }
        scene.note.push_str(&rest.join("\n"));
    }
    close(&mut folder, &mut entities);
    scene.entities = entities;
    scene.check().map_err(|e| format!("{name}: {e}"))?;
    Ok(scene)
}

/// A folder for a banner, with the comments just above the banner kept in its note.
fn open_folder(banner: &mut Vec<String>, note: &mut Vec<String>, id: String) -> Entity {
    let mut folder = start_folder(banner, id);
    if !note.is_empty() {
        let mut lines = std::mem::take(note);
        if !folder.note.is_empty() {
            lines.push(std::mem::take(&mut folder.note));
        }
        folder.note = lines.join("\n");
    }
    folder
}

/// A folder for a banner, named by its first sentence, with the whole banner as its note.
fn start_folder(banner: &mut Vec<String>, id: String) -> Entity {
    let text = std::mem::take(banner).join(" ");
    let name = text
        .split_once(['.', ':', ','])
        .map_or(text.as_str(), |(first, _)| first)
        .trim()
        .to_owned();
    let mut folder = Entity::folder(id, name);
    if folder.name != text.trim_end_matches('.') {
        folder.note = text;
    }
    folder
}

/// The entity a command places, or nothing for a scene's setting, which is set on `scene`.
fn to_entity(scene: &mut Scene, command: Command, id: String) -> Result<Option<Entity>, String> {
    let entity = |name: &str| Entity {
        id: id.clone(),
        name: name.to_owned(),
        ..Default::default()
    };
    let stem = |path: &str| {
        Path::new(path)
            .file_stem()
            .map_or(path.to_owned(), |s| s.to_string_lossy().into_owned())
    };
    let at = |position: Vec3, yaw: f32| Transform {
        rotation: [0.0, yaw, 0.0],
        ..Transform::at(position.to_array())
    };
    let ground = |x: f32, z: f32| Transform::at([x, 0.0, z]);
    let collide = |c: Collide| match c {
        Collide::Auto => project::Collide::Auto,
        Collide::None => project::Collide::None,
        Collide::Hull => project::Collide::Hull,
        Collide::Coarse => project::Collide::Coarse,
    };
    Ok(Some(match command {
        Command::TextureSize(size) => {
            scene.texture_size = Some(size);
            return Ok(None);
        }
        Command::Terrain {
            size,
            cells,
            height,
            shore_x,
        } => {
            scene.terrain = Some(project::Terrain {
                size,
                cells,
                height,
                shore_x,
            });
            return Ok(None);
        }
        Command::Land { sand, dirt } => {
            scene.land = Some(project::Land { sand, dirt });
            return Ok(None);
        }
        Command::Meadow(radius) => {
            scene.meadow = Some(radius);
            return Ok(None);
        }
        Command::Water { height, color } => {
            scene.water = Some(project::Water {
                level: height,
                color,
            });
            return Ok(None);
        }
        Command::Ocean { height, ocean } => {
            scene.sea = Some(project::Sea {
                level: height,
                ocean,
            });
            return Ok(None);
        }
        Command::Camera(view) => {
            scene.camera = Some(project::Camera {
                position: view.position,
                yaw_radians: view.yaw,
                pitch_radians: view.pitch,
            });
            return Ok(None);
        }
        Command::Input(file) => {
            scene.game.input = Some(file);
            return Ok(None);
        }
        Command::Track(file) => {
            scene.game.track = Some(file);
            return Ok(None);
        }
        Command::Circuit(size) => {
            scene.game.circuit = Some(size);
            return Ok(None);
        }
        Command::Surfaces(_) => {
            return Err(
                "a project has no map of its ground yet: bake the scene's text instead".into(),
            );
        }
        Command::Lends => {
            return Err("a project lends nothing yet: bake the scene's text instead".into());
        }
        Command::Model {
            path,
            position,
            seat,
            rotation,
            scale,
            collide: how,
            skinned,
        } => Entity {
            transform: Some(Transform {
                ground: seat,
                rotation: rotation.to_array(),
                scale,
                ..Transform::at(position.to_array())
            }),
            mesh: Some(project::Mesh {
                asset: path.clone(),
                part: None,
                collide: collide(how),
                skinned,
            }),
            ..entity(&stem(&path))
        },
        Command::Part {
            path,
            name,
            position,
            rotation,
            scale,
            collide: how,
        } => Entity {
            transform: Some(Transform {
                rotation: rotation.to_array(),
                scale,
                ..Transform::at(position.to_array())
            }),
            mesh: Some(project::Mesh {
                asset: path,
                part: Some(name.clone()),
                collide: collide(how),
                skinned: false,
            }),
            ..entity(&name)
        },
        Command::Actor(actor) => Entity {
            transform: Some(at(actor.position, actor.yaw_degrees)),
            actor: Some(project::Actor {
                asset: actor.file.clone(),
                properties: actor.settings,
            }),
            ..entity(&stem(&actor.file))
        },
        Command::Blocker(blocker) => Entity {
            transform: Some(at(blocker.centre, blocker.yaw_degrees)),
            blocker: Some(project::Blocker {
                size: blocker.size.to_array(),
            }),
            ..entity("Blocker")
        },
        Command::Light(light) => Entity {
            transform: Some(Transform::at(light.position)),
            light: Some(project::Light {
                range: light.range,
                intensity: light.intensity,
            }),
            ..entity("Light")
        },
        Command::Wood {
            at: centre,
            radius,
            spacing,
        } => Entity {
            transform: Some(ground(centre.x, centre.y)),
            wood: Some(project::Wood { radius, spacing }),
            ..entity("Wood")
        },
        Command::Clear { at: centre, radius } => Entity {
            transform: Some(ground(centre.x, centre.y)),
            clear: Some(project::Clear { radius }),
            ..entity("Clear")
        },
        Command::Path { from, to, width } => Entity {
            path: Some(project::Path {
                from: from.to_array(),
                to: to.to_array(),
                width,
            }),
            ..entity("Path")
        },
        Command::Yard { from, to } => Entity {
            yard: Some(project::Yard {
                from: from.to_array(),
                to: to.to_array(),
            }),
            ..entity("Yard")
        },
        Command::Pond {
            min,
            max,
            level,
            water,
        } => Entity {
            pond: Some(project::Pond {
                min: min.to_array(),
                max: max.to_array(),
                level,
                water,
            }),
            ..entity("Pond")
        },
    }))
}

/// Converts the scene text at `path` into `NAME.scene` beside it, and lists it in the
/// `solstice.project` there, making one if there is none. Refuses to replace a scene.
pub fn convert_file(path: &Path) -> Result<PathBuf, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let name = path
        .file_name()
        .map_or(String::new(), |n| n.to_string_lossy().into_owned());
    let scene = convert(&text, &name)?;
    let output = path.with_extension("scene");
    if output.exists() {
        return Err(format!("{} exists already", output.display()));
    }
    let folder = path.parent().unwrap_or(Path::new("."));
    let project_file = folder.join(PROJECT_FILE);
    let mut project = if project_file.is_file() {
        let text = std::fs::read_to_string(&project_file)
            .map_err(|e| format!("{}: {e}", project_file.display()))?;
        Project::parse(&text).map_err(|e| format!("{}: {e}", project_file.display()))?
    } else {
        let folder_name = std::path::absolute(folder)
            .ok()
            .and_then(|f| f.file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_else(|| "Project".to_owned());
        Project::new(&folder_name)
    };
    let listed = output
        .file_name()
        .expect("a file")
        .to_string_lossy()
        .into_owned();
    if !project.scenes.contains(&listed) {
        project.scenes.push(listed);
    }
    std::fs::write(&output, scene.to_json()).map_err(|e| format!("{}: {e}", output.display()))?;
    std::fs::write(&project_file, project.to_json())
        .map_err(|e| format!("{}: {e}", project_file.display()))?;
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scene's text, with something of every kind.
    const TEXT: &str = "\
# A test cove.
# Two lines about it.

texture_size 2048
terrain 900 1024 0.3 52
ocean -0.6 coastal
land 10 5
yard -21 -21 21 21
path 23 0 58 -6 3.2
# Nothing grows on the fort.
clear 0 0 33
meadow 75
wood -58 -38 20 7
camera 66 1.5 -41 2.31 0.03

# ---- Towers on the four corners. Each is wider than the walls,
# ---- which hides the joints.
part kit/fort.gltf tower_round 22 0 22 0 1
part kit/fort.gltf tower_round 22 0 -22 90 1 collide=hull

# ---- The courtyard
model scans/statue.gltf -7 0 0 90 2.4
model scans/boulder.gltf 31 ground:0.2 -24 40 2.2   # half buried
model scans/rock.gltf 9 ground 0 45 2 collide=coarse
model people/keeper.glb 2 0 3 0 1 skinned
blocker -1.1 0.19 17.3 0.4 0.38 3 0
actor cove/gate.actor 20.7 0 0 90 locked=true speed=2
light 0 2 0 16 40 14 3
pond -10 -10 -4 -2 0.5 coastal
";

    fn commands(sourced: Vec<Sourced>) -> Vec<Command> {
        sourced.into_iter().map(|s| s.command).collect()
    }

    /// The commands a text's lines give, settings first, as a project lowers them.
    fn text_commands(text: &str) -> Vec<Command> {
        let all = commands(scene_file::parse_text(Path::new("t.txt"), text).unwrap());
        let setting = |c: &Command| {
            matches!(
                c,
                Command::TextureSize(_)
                    | Command::Terrain { .. }
                    | Command::Land { .. }
                    | Command::Meadow(_)
                    | Command::Water { .. }
                    | Command::Ocean { .. }
                    | Command::Camera(_)
                    | Command::Input(_)
                    | Command::Track(_)
                    | Command::Circuit(_)
            )
        };
        let rank = |c: &Command| match c {
            Command::TextureSize(_) => 0,
            Command::Terrain { .. } => 1,
            Command::Land { .. } => 2,
            Command::Meadow(_) => 3,
            Command::Water { .. } => 4,
            Command::Ocean { .. } => 5,
            Command::Camera(_) => 6,
            Command::Input(_) => 7,
            Command::Track(_) => 8,
            _ => 9,
        };
        let (mut settings, placed): (Vec<_>, Vec<_>) = all.into_iter().partition(|c| setting(c));
        settings.sort_by_key(rank);
        settings.extend(placed);
        settings
    }

    #[test]
    fn a_converted_scene_says_what_its_text_said() {
        let scene = convert(TEXT, "t.txt").unwrap();
        let lowered = lower(&scene, "t.scene").unwrap();
        assert!(lowered.iter().filter(|s| s.owner.is_some()).count() == 14);
        assert_eq!(commands(lowered), text_commands(TEXT));
    }

    #[test]
    fn it_still_does_after_a_trip_through_its_file() {
        let scene = convert(TEXT, "t.txt").unwrap();
        let read = Scene::parse(&scene.to_json()).unwrap();
        assert_eq!(read, scene);
        assert_eq!(
            commands(lower(&read, "t.scene").unwrap()),
            text_commands(TEXT)
        );
    }

    #[test]
    fn comments_become_notes_and_banners_folders() {
        let scene = convert(TEXT, "t.txt").unwrap();
        assert_eq!(scene.note, "A test cove.\nTwo lines about it.");
        let names: Vec<&str> = scene.entities.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "Yard",
                "Path",
                "Clear",
                "Wood",
                "Towers on the four corners",
                "The courtyard"
            ]
        );
        assert_eq!(scene.entities[2].note, "Nothing grows on the fort.");
        let towers = &scene.entities[4];
        assert_eq!(towers.children.len(), 2);
        assert!(towers.note.contains("which hides the joints"));
        // The courtyard's banner is its name and nothing more, so there's no note.
        let courtyard = &scene.entities[5];
        assert_eq!(courtyard.note, "");
        assert_eq!(courtyard.children[0].name, "statue");
        assert_eq!(courtyard.children[1].note, "half buried");
        assert_eq!(
            courtyard.children[2].transform.as_ref().unwrap().ground,
            Some(0.02)
        );
    }

    #[test]
    fn no_comment_is_lost_or_misplaced() {
        let text = "# about the tower
model a.gltf 0 0 0 0 1
# -------------------

# above the walls
# ---- Walls
model b.gltf 1 0 0 0 1
# still to do: the keep
";
        let scene = convert(text, "t.txt").unwrap();
        assert_eq!(scene.entities[0].note, "about the tower");
        let walls = &scene.entities[1];
        assert_eq!(walls.name, "Walls");
        assert_eq!(walls.note, "above the walls");
        assert_eq!(walls.children[0].note, "");
        assert_eq!(scene.note, "still to do: the keep");
        // The rule made no folder.
        assert_eq!(scene.entities.len(), 2);
    }

    #[test]
    fn converting_twice_gives_the_same_ids() {
        assert_eq!(
            convert(TEXT, "t.txt").unwrap(),
            convert(TEXT, "t.txt").unwrap()
        );
        let other = convert(TEXT, "u.txt").unwrap();
        assert_ne!(
            other.entities[0].id,
            convert(TEXT, "t.txt").unwrap().entities[0].id
        );
    }

    #[test]
    fn a_tilted_model_keeps_its_tilt() {
        let mut scene = convert("model a.gltf 1 2 3 45 2\n", "t.txt").unwrap();
        scene.entities[0].transform.as_mut().unwrap().rotation = [10.0, 45.0, -5.0];
        let lowered = lower(&scene, "t.scene").unwrap();
        let Command::Model { rotation, .. } = &lowered[0].command else {
            panic!("a model");
        };
        assert_eq!(*rotation, Vec3::new(10.0, 45.0, -5.0));
    }

    #[test]
    fn what_the_text_cant_say_is_refused_by_name() {
        let mut scene = convert("actor cove/gate.actor 1 0 0 90\n", "t.txt").unwrap();
        scene.entities[0].transform.as_mut().unwrap().rotation = [10.0, 90.0, 0.0];
        let error = lower(&scene, "t.scene").unwrap_err();
        assert!(
            error.contains("gate") && error.contains("turns only about Y"),
            "{error}"
        );

        let mut scene = convert("model a.gltf 1 2 3 45 2\n", "t.txt").unwrap();
        scene.entities[0].mesh.as_mut().unwrap().asset = "my models/a.gltf".into();
        assert!(lower(&scene, "t.scene").unwrap_err().contains("spaces"));

        // And what the text refuses, the project is refused too.
        let mut scene = convert("wood 0 0 20 7\n", "t.txt").unwrap();
        scene.entities[0].wood.as_mut().unwrap().spacing = 0.1;
        assert!(lower(&scene, "t.scene").is_err());
    }

    /// Each of the repository's scenes converts to a project that says just what its text does.
    #[test]
    fn the_repositorys_scenes_convert_faithfully() {
        let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scenes");
        // Not `cove_flyover.txt`, which is a camera's path, not a scene.
        for name in ["cove.txt", "harbour.txt", "walker.txt"] {
            let path = folder.join(name);
            {
                let text = std::fs::read_to_string(&path).unwrap();
                let scene = convert(&text, name).unwrap();
                let read = Scene::parse(&scene.to_json()).unwrap();
                assert_eq!(
                    commands(lower(&read, name).unwrap()),
                    text_commands(&text),
                    "{name}"
                );
            }
        }
    }
}

/// Baking a scene's project gives the same scene as baking its text, apart from the map of which
/// entity made which instances. Needs the scans the scenes use (`scripts/fetch-scans.py`), so it
/// is run by hand: `cargo test --release -p bake -- --ignored bakes_the_same`.
#[cfg(test)]
mod bakes {
    use super::*;

    #[test]
    #[ignore = "needs the scans in target/scan"]
    fn every_scene_bakes_the_same_from_its_project() {
        let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scenes");
        for name in ["cove.txt", "harbour.txt", "walker.txt"] {
            let path = folder.join(name);
            let text = std::fs::read_to_string(&path).unwrap();
            let scene = convert(&text, name).unwrap();
            let missing = scene.walk().into_iter().find_map(|e| {
                let asset = &e.mesh.as_ref()?.asset;
                (!folder.join(asset).exists()).then_some(asset.clone())
            });
            if let Some(asset) = missing {
                println!("{name}: skipped, {asset} isn't here");
                continue;
            }
            let size = Some(256);
            // An actor's model is named in its actor file, so it is only found missing here.
            let from_text = match scene_file::build(&path, size) {
                Ok((scene, _)) => scene,
                Err(e) if e.contains("os error 2") || e.contains("os error 3") => {
                    println!("{name}: skipped, {e}");
                    continue;
                }
                Err(e) => panic!("{e}"),
            };
            let (mut from_project, _) =
                scene_file::build_commands(&folder, lower(&scene, name).unwrap(), size).unwrap();

            // Every model is mapped to the instances it made.
            for entity in scene.walk().into_iter().filter(|e| e.mesh.is_some()) {
                assert!(
                    from_project.sources.iter().any(|s| s.id == entity.id),
                    "{name}: {} isn't mapped",
                    entity.name
                );
            }
            assert!(from_project
                .sources
                .windows(2)
                .all(|w| w[0].instances.end <= w[1].instances.start));

            from_project.sources.clear();
            let encode =
                |s: &SceneAsset| solstice_asset::file::encode(&solstice_builder::paginate(s));
            assert!(encode(&from_text) == encode(&from_project), "{name}");
            println!("{name}: the same, {} instances", from_text.instances.len());
        }
    }
}
