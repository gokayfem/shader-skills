//! What several scenes share: actors whose models are in a file of their own.
//!
//! A game's maps have things in common that are no part of any of them: its car, what its
//! player carries. They are not in a map's file. They are in one file beside the maps,
//! [`FILE`], baked from a scene of its own, and whichever map is played is joined with it as
//! it is read (`solstice_asset::join`). A map's text says which of its actors are there
//! rather than in it: `actor cove/car.actor X Y Z YAW shared`. The shared file has an actor
//! of the same file, with the instances that draw it; joined, the map's actor is given those
//! instances, and they are put where the map puts the actor ([`adopt`]).
//!
//! A map that is read with the shared file also leaves out of its own whatever else that
//! file lends: meshes and textures the two both have. They are put back as the two are
//! joined (`solstice_asset::join::fill`, docs/gameplay.md).
use glam::{Mat4, Quat};
use solstice_asset::join::Joined;
use solstice_asset::{GameFile, PagedScene};
use solstice_game::scene::ActorLine;
use solstice_game::GAME_FILE;

/// The file of what is shared, beside the scenes'.
pub const FILE: &str = "shared.solstice";

/// The actors of a scene's game, each with the line it is on.
fn actors(files: &[GameFile]) -> Vec<(usize, ActorLine)> {
    let Some(game) = files.iter().find(|file| file.name == GAME_FILE) else {
        return Vec::new();
    };
    game.text
        .lines()
        .enumerate()
        .filter_map(|(index, line)| {
            let words: Vec<&str> = line.split_whitespace().collect();
            match words.split_first() {
                Some((&"actor", rest)) => ActorLine::parse(rest).ok().map(|actor| (index, actor)),
                _ => None,
            }
        })
        .collect()
}

/// What a game is played by is the same on every map: its actors' files, their scripts, a
/// car's numbers and the input's actions. A map has copies of them, from when it was baked;
/// the shared file's take their place as the two are joined, and any it has that the map has
/// not are added. So what they do is changed by baking the shared file alone.
///
/// (What a map lists of its own is its own: its actors and where they stand, its track.)
pub fn share_code(files: &mut Vec<GameFile>, shared: &[GameFile]) {
    let code = |name: &str| {
        [".actor", ".star", ".input", ".vehicle"]
            .iter()
            .any(|kind| name.ends_with(kind))
    };
    for theirs in shared.iter().filter(|file| code(&file.name)) {
        match files.iter_mut().find(|own| own.name == theirs.name) {
            Some(own) => own.text.clone_from(&theirs.text),
            None => files.push(theirs.clone()),
        }
    }
}

/// Whether a scene's game has actors that are in the shared file.
pub fn wanted(files: &[GameFile]) -> bool {
    actors(files).iter().any(|(_, actor)| actor.shared)
}

/// A scene has been joined with the shared file, whose gameplay files `shared` are: gives
/// each of the scene's shared actors the instances that draw it, and puts them where the
/// scene puts the actor.
pub fn adopt(scene: &mut PagedScene, shared: &[GameFile], joined: Joined) -> Result<(), String> {
    share_code(&mut scene.gameplay, shared);
    let there = actors(shared);
    let mut changed = Vec::new();
    for (line, mut actor) in actors(&scene.gameplay) {
        if !actor.shared {
            continue;
        }
        let (_, theirs) = there
            .iter()
            .find(|(_, theirs)| theirs.file == actor.file)
            .ok_or_else(|| format!("{} is not in {FILE}", actor.file))?;
        let Some(instances) = theirs.instances.clone() else {
            // It has no model: nothing draws it, there or here.
            actor.shared = false;
            changed.push((line, actor));
            continue;
        };
        let instances = instances.start + joined.instances..instances.end + joined.instances;
        // The shared file has it where it was baked; the scene has it where it starts.
        let place = |actor: &ActorLine| {
            Mat4::from_rotation_translation(
                Quat::from_rotation_y(actor.yaw_degrees.to_radians()),
                actor.position,
            )
        };
        let moved = place(&actor) * place(theirs).inverse();
        for id in instances.clone() {
            let instance = scene
                .instances
                .get_mut(id as usize)
                .ok_or_else(|| format!("{FILE} has no instance {id} for {}", actor.file))?;
            instance.transform = moved * instance.transform;
        }
        // If it is a kit, the list of its pieces comes with it: it counts from the actor's
        // first instance, wherever that now is.
        let pieces = solstice_game::actor::pieces_file(&actor.file);
        if let Some(file) = shared.iter().find(|file| file.name == pieces) {
            if !scene.gameplay.iter().any(|f| f.name == pieces) {
                scene.gameplay.push(file.clone());
            }
        }
        actor.instances = Some(instances);
        actor.shared = false;
        changed.push((line, actor));
    }
    if let Some(game) = scene
        .gameplay
        .iter_mut()
        .find(|file| file.name == GAME_FILE)
    {
        let mut lines: Vec<String> = game.text.lines().map(str::to_owned).collect();
        for (line, actor) in changed {
            lines[line] = actor.to_text();
        }
        game.text = lines.join("\n") + "\n";
    }
    // And what the scene left out because the shared file has it: its meshes and textures.
    let text = |files: &[GameFile], name: &str| {
        files
            .iter()
            .find(|file| file.name == name)
            .map(|file| file.text.clone())
    };
    if let Some(borrows) = text(&scene.gameplay, solstice_asset::join::BORROWS) {
        let lends = text(shared, solstice_asset::join::LENDS).unwrap_or_default();
        solstice_asset::join::fill(scene, &borrows, &lends, joined)
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str, text: &str) -> GameFile {
        GameFile {
            name: name.to_owned(),
            text: text.to_owned(),
        }
    }

    #[test]
    fn the_shared_files_code_takes_the_place_of_a_scenes_copy_of_it() {
        let mut scene = vec![
            file(GAME_FILE, "actor cove/player.actor 1 2 3 0\n"),
            file("cove/player.star", "old\n"),
        ];
        let shared = [
            file(GAME_FILE, "actor cove/player.actor 0 0 0 0\n"),
            file("cove/player.star", "new\n"),
            file("cove/play.input", "sprint gamepad l3\n"),
            file("circuit.track", "elsewhere\n"),
        ];
        share_code(&mut scene, &shared);
        let text = |name: &str| {
            let found = scene.iter().find(|file| file.name == name);
            found.map(|file| file.text.as_str())
        };
        // Its script is the shared one, and the actions it had none of are there.
        assert_eq!(text("cove/player.star"), Some("new\n"));
        assert_eq!(text("cove/play.input"), Some("sprint gamepad l3\n"));
        // Where its actors stand is its own, and it is given nothing else of the other's.
        assert_eq!(text(GAME_FILE), Some("actor cove/player.actor 1 2 3 0\n"));
        assert_eq!(text("circuit.track"), None);
    }

    #[test]
    fn a_shared_actor_is_given_the_other_files_instances_where_the_scene_puts_it() {
        use glam::Vec3;
        use solstice_asset::Instance;
        let at = |x: f32| Instance {
            mesh: 0,
            material: 0,
            transform: Mat4::from_translation(Vec3::new(x, 0.0, 0.0)),
        };
        // A scene of two instances, joined with a shared file whose car is its second and
        // third: they are now the scene's fourth and fifth.
        let mut scene = PagedScene {
            instances: vec![at(0.0), at(0.0), at(0.0), at(1.0), at(2.0)],
            gameplay: vec![file(
                GAME_FILE,
                "input keys.input\nactor car.actor 10 0 5 0 shared\n",
            )],
            ..Default::default()
        };
        assert!(wanted(&scene.gameplay));
        let shared = [file(GAME_FILE, "actor car.actor 1 0 0 0 instances=1+2\n")];
        let joined = Joined {
            instances: 2,
            ..Default::default()
        };
        adopt(&mut scene, &shared, joined).expect("the car is in the shared file");
        let game = &scene.gameplay[0].text;
        assert!(game.contains("instances=3+2"), "{game}");
        assert!(!wanted(&scene.gameplay), "{game}");
        // Baked at x = 1 and put at x = 10, z = 5: each of its instances has moved as far.
        let places: Vec<Vec3> = scene.instances[3..]
            .iter()
            .map(|instance| instance.transform.w_axis.truncate())
            .collect();
        assert_eq!(
            places,
            [Vec3::new(10.0, 0.0, 5.0), Vec3::new(11.0, 0.0, 5.0)]
        );
        // An actor the shared file has none of is said, and not drawn from nowhere.
        scene.gameplay[0].text = "actor boat.actor 0 0 0 0 shared\n".to_owned();
        assert!(adopt(&mut scene, &shared, joined).is_err());
    }
}
