//! A scene's game, from its gameplay files to what is drawn.

use glam::{EulerRot, Mat4, Quat, Vec3};
use solstice_asset::{Collision, CollisionShape, GameFile};
use solstice_game::{Devices, Play, View, GAME_FILE};
use solstice_script::{Script, ScriptValue};
use solstice_world::{Name, STEP_SECONDS};
use std::collections::HashMap;

fn files(list: &[(&str, &str)]) -> Vec<GameFile> {
    list.iter()
        .map(|(name, text)| GameFile {
            name: (*name).to_owned(),
            text: (*text).to_owned(),
        })
        .collect()
}

/// Where the bake placed each instance: none of them at the actor's own origin.
fn baked(id: u32) -> Option<Mat4> {
    (id < 10).then(|| {
        Mat4::from_rotation_translation(
            Quat::from_rotation_y(std::f32::consts::FRAC_PI_2),
            Vec3::new(20.0, 0.0, id as f32),
        ) * Mat4::from_translation(Vec3::new(0.5, 1.0, 0.0))
    })
}

fn var(play: &Play, name: &str, key: &str) -> Option<ScriptValue> {
    let world = &play.world().entities;
    let mut query = world.query::<(&Name, &Script)>();
    let (_, script) = query.iter().find(|(n, _)| n.as_str() == name)?;
    script.vars.get(key).cloned()
}

fn close(a: Mat4, b: Mat4) -> bool {
    a.abs_diff_eq(b, 1.0e-4)
}

const GATE_ACTOR: &str = "name gate\nmesh gate.gltf 1.15\nscript gate.star\nproperty locked bool false\nproperty swing float 1.5\n";
const GATE: &str = r#"
def on_tick(self, dt):
    if pressed("interact") and not self.locked:
        self.yaw += self.swing
"#;

#[test]
fn a_placed_actor_moves_its_instances_when_its_script_moves_it() {
    let scene = files(&[
        (
            GAME_FILE,
            "actor cove/gate.actor 20 0 0 90 swing=0.5 instances=2+2\n",
        ),
        ("cove/gate.actor", GATE_ACTOR),
        ("cove/gate.star", GATE),
    ]);
    let mut play = Play::load(&scene, baked, None).unwrap();
    assert_eq!(play.instances().collect::<Vec<_>>(), vec![2, 3]);
    assert!(!play.has_player());
    assert_eq!(play.scripts(), ["cove/gate.star"]);

    // Standing still, it is drawn where the bake put it, once, and then left alone.
    let mut placed = HashMap::new();
    let mut devices = Devices::new();
    play.frame(STEP_SECONDS * 1.5, &mut devices, |id, at| {
        placed.insert(id, at);
    });
    assert!(close(placed[&2], baked(2).unwrap()) && close(placed[&3], baked(3).unwrap()));
    let mut again = 0;
    play.frame(STEP_SECONDS, &mut devices, |_, _| again += 1);
    assert_eq!(again, 0, "nothing moved, so nothing is redrawn");

    // Pressed, it turns by its placement's `swing`, about its own origin.
    let mut devices = Devices::new();
    devices.key("KeyE", true);
    // Two whole steps, so the frame ends on a step and nothing is between two.
    play.frame(STEP_SECONDS * 2.0, &mut devices, |id, at| {
        placed.insert(id, at);
    });
    let turned = Mat4::from_rotation_translation(
        Quat::from_rotation_y(90_f32.to_radians() + 0.5),
        Vec3::new(20.0, 0.0, 0.0),
    );
    let origin = Mat4::from_rotation_translation(
        Quat::from_rotation_y(90_f32.to_radians()),
        Vec3::new(20.0, 0.0, 0.0),
    );
    for id in [2, 3] {
        let expected = turned * origin.inverse() * baked(id).unwrap();
        assert!(close(placed[&id], expected), "{id}: {:?}", placed[&id]);
    }
    assert!(play.take_messages().is_empty());
}

#[test]
fn a_placements_settings_override_its_properties() {
    let scene = files(&[
        (
            GAME_FILE,
            "actor gate.actor 0 0 0 0 locked=true\nactor gate.actor 5 0 0 0\n",
        ),
        ("gate.actor", GATE_ACTOR),
        ("gate.star", GATE),
    ]);
    let play = Play::load(&scene, baked, None).unwrap();
    let world = &play.world().entities;
    let mut locks: Vec<_> = world
        .query::<&Script>()
        .iter()
        .map(|s| s.properties["locked"].clone())
        .collect();
    locks.sort_by_key(|v| format!("{v:?}"));
    assert_eq!(
        locks,
        vec![ScriptValue::Bool(false), ScriptValue::Bool(true)]
    );
}

const RECORDER: &str = r#"
def on_tick(self, dt):
    seen = {}
    for action in ["forward", "turn", "interact", "jump"]:
        if input(action) != 0:
            seen[action] = input(action)
        if pressed(action):
            seen[action + "_pressed"] = True
    self.vars.seen = seen
"#;

/// What the recorder saw of an action.
fn seen(play: &Play, key: &str) -> Option<ScriptValue> {
    match var(play, "recorder", "seen")? {
        ScriptValue::Dict(entries) => entries.into_iter().find(|(k, _)| k == key).map(|(_, v)| v),
        other => panic!("{other:?}"),
    }
}

/// Runs one frame of a game that records each action it sees.
fn recorded(mut devices: Devices) -> Play {
    let scene = files(&[
        (GAME_FILE, "actor recorder.actor 0 0 0 0\n"),
        ("recorder.actor", "name recorder\nscript recorder.star\n"),
        ("recorder.star", RECORDER),
    ]);
    let mut play = Play::load(&scene, baked, None).unwrap();
    play.frame(STEP_SECONDS * 1.01, &mut devices, |_, _| {});
    let messages = play.take_messages();
    assert!(messages.is_empty(), "{messages:?}");
    play
}

#[test]
fn an_action_reaches_a_script_from_each_kind_of_input() {
    let float = |v: f64| Some(ScriptValue::Float(v));
    let yes = Some(ScriptValue::Bool(true));

    let mut keys = Devices::new();
    keys.key("KeyW", true);
    let play = recorded(keys);
    assert_eq!(seen(&play, "forward"), float(1.0));
    assert_eq!(seen(&play, "forward_pressed"), yes);

    let mut mouse = Devices::new();
    mouse.mouse_button("left", true);
    mouse.mouse_motion(-12.0, 0.0);
    let play = recorded(mouse);
    assert_eq!(seen(&play, "interact_pressed"), yes);
    let turn = match seen(&play, "turn") {
        Some(ScriptValue::Float(t)) => t,
        other => panic!("{other:?}"),
    };
    // 12 pixels left in the frame's time, at the built-in 0.002 radians a pixel.
    let expected = 12.0 / (STEP_SECONDS * 1.01) * 0.002;
    assert!((turn - expected).abs() < 1.0e-4, "{turn} vs {expected}");

    let mut pad = Devices::new();
    pad.gamepad_axis("left_y", -1.0);
    pad.gamepad_button("south", 1.0);
    let play = recorded(pad);
    assert_eq!(seen(&play, "forward"), float(1.0));
    assert_eq!(seen(&play, "jump"), float(1.0));

    let mut touch = Devices::new();
    touch.touch_stick(0.0, -0.5);
    touch.touch_tap();
    let play = recorded(touch);
    assert_eq!(seen(&play, "forward"), float(0.5));
    assert_eq!(seen(&play, "interact_pressed"), yes);
    assert_eq!(
        seen(&play, "interact"),
        None,
        "a tap presses but isn't held"
    );
}

#[test]
fn a_scene_can_bring_its_own_input_actions() {
    let scene = files(&[
        (
            GAME_FILE,
            "input keys.input\nactor recorder.actor 0 0 0 0\n",
        ),
        ("keys.input", "jump key KeyJ\n"),
        ("recorder.actor", "name recorder\nscript recorder.star\n"),
        ("recorder.star", RECORDER),
    ]);
    let mut play = Play::load(&scene, baked, None).unwrap();
    let mut devices = Devices::new();
    devices.key("KeyJ", true);
    devices.key("KeyW", true);
    play.frame(STEP_SECONDS * 1.01, &mut devices, |_, _| {});
    assert_eq!(seen(&play, "jump"), Some(ScriptValue::Float(1.0)));
    assert_eq!(seen(&play, "forward"), None, "only its own actions");
}

const WALKER: &str = r#"
def on_tick(self, dt):
    self.yaw += input("turn") * dt
    ahead = vec3(0, 0, -1) * input("forward") * dt * 3
    self.position += ahead
"#;

#[test]
fn the_player_camera_follows_the_player_and_looks_up_and_down() {
    let scene = files(&[
        (GAME_FILE, "actor player.actor 1 0 2 0\n"),
        (
            "player.actor",
            "name player\nscript walker.star\ncamera 0 1.6 0\n",
        ),
        ("walker.star", WALKER),
    ]);
    let mut play = Play::load(&scene, baked, None).unwrap();
    assert!(play.has_player());
    let mut devices = Devices::new();
    devices.key("KeyW", true);
    devices.mouse_motion(0.0, -100.0);
    let mut view = None;
    for _ in 0..60 {
        view = play.frame(STEP_SECONDS, &mut devices, |_, _| {});
    }
    let view = view.unwrap();
    // A second at 3 m/s along -Z, from (1, 0, 2), with the eye 1.6 m up.
    assert!(
        view.position.abs_diff_eq(Vec3::new(1.0, 1.6, -1.0), 0.06),
        "{view:?}"
    );
    assert_eq!(view.yaw, 0.0);
    // The mouse moved up once, by 100 pixels: 0.2 radians up.
    assert!((view.pitch - 0.2).abs() < 1.0e-3, "{view:?}");

    // Turned half round, the eye turns with it.
    let scene = files(&[
        (GAME_FILE, "actor player.actor 0 0 0 180\n"),
        ("player.actor", "name player\ncamera 0.5 1.6 0\n"),
    ]);
    let mut play = Play::load(&scene, baked, None).unwrap();
    let view = play
        .frame(STEP_SECONDS, &mut Devices::new(), |_, _| {})
        .unwrap();
    assert!(
        view.position.abs_diff_eq(Vec3::new(-0.5, 1.6, 0.0), 1.0e-5),
        "{view:?}"
    );
    let expected = Quat::from_rotation_y(std::f32::consts::PI)
        .to_euler(EulerRot::YXZ)
        .0;
    assert!((view.yaw - expected).abs() < 1.0e-5);
}

#[test]
fn every_mistake_in_a_scenes_gameplay_is_reported_with_its_file() {
    let scene = files(&[
        (
            GAME_FILE,
            "actor gate.actor 0 0 0 0 colour=red locked=maybe\nactor door.actor 0 0 0 0\nactor bad.actor 0 0 0 0\nactor broken.actor 0 0 0 0 instances=8+4\nactor eye.actor 0 0 0 0\nactor eye.actor 0 0 0 0\nactor rock.actor 0 0 0 0 size=2\n",
        ),
        ("gate.actor", GATE_ACTOR),
        ("gate.star", GATE),
        ("bad.actor", "name bad\nscript bad.star\nproperty x\n"),
        ("broken.actor", "name broken\nscript broken.star\n"),
        ("broken.star", "def on_tick(self, dt):\n    x = = 1\n"),
        ("eye.actor", "name eye\ncamera 0 1 0\nscript nowhere.star\n"),
        ("rock.actor", "name rock\n"),
    ]);
    let errors = Play::load(&scene, baked, None).err().unwrap();
    let expect = [
        "gate.actor has no property `colour`",
        "`locked`: a bool is true or false",
        "actor 2 (door.actor): missing from the scene's gameplay",
        "bad.actor: line 3: expected `property NAME KIND VALUE`",
        "broken.star:2",
        "the scene has no instance 10",
        "its script nowhere.star is missing",
        "only one actor can have a camera",
        "rock.actor has no script, so no properties",
    ];
    for part in expect {
        assert!(
            errors.iter().any(|e| e.contains(part)),
            "{part} not in {errors:#?}"
        );
    }
    // Each script is loaded, and reported, once.
    assert_eq!(
        errors.iter().filter(|e| e.contains("nowhere.star")).count(),
        1
    );

    assert!(Play::load(&[], baked, None).err().unwrap()[0].contains("has no scene.game"));
    let errors = Play::load(&files(&[(GAME_FILE, "actor x\n")]), baked, None)
        .err()
        .unwrap();
    assert!(errors[0].starts_with("scene.game: line 1"), "{errors:?}");
    let errors = Play::load(
        &files(&[(GAME_FILE, "input a.input\n"), ("a.input", "jump key\n")]),
        baked,
        None,
    )
    .err()
    .unwrap();
    assert!(errors[0].starts_with("a.input: line 1"), "{errors:?}");
    let errors = Play::load(&files(&[(GAME_FILE, "input b.input\n")]), baked, None)
        .err()
        .unwrap();
    assert!(errors[0].contains("b.input: missing"), "{errors:?}");
}

#[test]
fn scripts_reload_while_playing_and_report_what_they_log() {
    let scene = files(&[
        (GAME_FILE, "actor talker.actor 0 0 0 0\n"),
        ("talker.actor", "name talker\nscript talk.star\n"),
        ("talk.star", "def on_begin(self):\n    log(\"hello\")\n"),
    ]);
    let mut play = Play::load(&scene, baked, None).unwrap();
    play.frame(STEP_SECONDS * 1.01, &mut Devices::new(), |_, _| {});
    let messages = play.take_messages();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].text, "talker: hello");
    assert!(!messages[0].error);

    let error = play.reload("talk.star", "def on_tick(self, dt):\n    fail(\"no\")\n");
    assert!(error.is_ok());
    play.frame(STEP_SECONDS, &mut Devices::new(), |_, _| {});
    let messages = play.take_messages();
    assert!(messages[0].error);
    assert!(
        messages[0].text.starts_with("step 2, talker: on_tick:"),
        "{:?}",
        messages[0]
    );
    assert!(play.reload("talk.star", "def on_tick(self dt):\n").is_err());
}

#[test]
fn the_mouse_turns_as_far_however_fast_the_frames_come() {
    // The same 300 pixels of mouse, spread over a second at 60, 144 and 37 frames a second,
    // turns the player by the same amount: none is lost in frames with no step.
    let turned = |rate: f64| {
        let scene = files(&[
            (GAME_FILE, "actor player.actor 0 0 0 0\n"),
            (
                "player.actor",
                "name player\nscript walker.star\ncamera 0 1.6 0\n",
            ),
            ("walker.star", WALKER),
        ]);
        let mut play = Play::load(&scene, baked, None).unwrap();
        let mut devices = Devices::new();
        let frames = rate.round() as u32;
        let mut view = None;
        for _ in 0..frames {
            devices.mouse_motion(-300.0 / frames as f32, 30.0 / frames as f32);
            view = play.frame(1.0 / rate, &mut devices, |_, _| {});
        }
        // Let the last movement reach a step.
        view = play.frame(STEP_SECONDS, &mut devices, |_, _| {}).or(view);
        view.unwrap()
    };
    let at_60 = turned(60.0);
    // 300 pixels at 0.002 radians each; 30 down, looking down 0.06.
    assert!((at_60.yaw - 0.6).abs() < 1.0e-3, "{at_60:?}");
    assert!((at_60.pitch + 0.06).abs() < 1.0e-4, "{at_60:?}");
    for rate in [144.0, 37.0] {
        let view = turned(rate);
        assert!((view.yaw - at_60.yaw).abs() < 1.0e-3, "{rate} Hz: {view:?}");
        assert!(
            (view.pitch - at_60.pitch).abs() < 1.0e-4,
            "{rate} Hz: {view:?}"
        );
    }
}

#[test]
fn a_quarter_turn_looks_along_minus_x_as_the_renderers_camera_does() {
    let scene = files(&[
        (GAME_FILE, "actor cove/player.actor 0 0 0 90\n"),
        (
            "cove/player.actor",
            include_str!("../../../scenes/cove/player.actor"),
        ),
        (
            "cove/player.star",
            include_str!("../../../scenes/cove/player.star"),
        ),
    ]);
    let mut play = Play::load(&scene, baked, None).unwrap();
    let mut devices = Devices::new();
    devices.key("KeyW", true);
    // Two steps' walk, at 1.6 m/s.
    let view = play.frame(STEP_SECONDS * 2.0, &mut devices, |_, _| {});
    let view = view.unwrap();
    // The renderer's camera looks along (-sin yaw, 0, -cos yaw): -X at a quarter turn, and
    // walking forward goes that way.
    assert!(
        (view.yaw - std::f32::consts::FRAC_PI_2).abs() < 1.0e-5,
        "{view:?}"
    );
    // Drawn between the last two steps, and a frame ending just on a step is drawn at the one
    // before it: one step's walk.
    let walked = 1.6 * STEP_SECONDS as f32;
    assert!(
        // Its height can sag a little: the player has weight, and here no ground.
        view.position
            .abs_diff_eq(Vec3::new(-walked, 1.65, 0.0), 0.01),
        "{view:?}"
    );
}

#[test]
fn in_the_cove_the_player_walks_to_the_gate_and_opens_it() {
    // The cove's own actors and scripts, as the bake carries them.
    let scene = files(&[
        (
            GAME_FILE,
            "actor cove/gate.actor 20.7 0 0 90 instances=0+1\nactor cove/player.actor 30 0 0 90\n",
        ),
        (
            "cove/gate.actor",
            include_str!("../../../scenes/cove/gate.actor"),
        ),
        (
            "cove/gate.star",
            include_str!("../../../scenes/cove/gate.star"),
        ),
        (
            "cove/player.actor",
            include_str!("../../../scenes/cove/player.actor"),
        ),
        (
            "cove/player.star",
            include_str!("../../../scenes/cove/player.star"),
        ),
    ]);
    let gate_at = |_: u32| Some(Mat4::from_translation(Vec3::new(20.7, 0.0, 0.0)));
    let mut play = Play::load(&scene, gate_at, None).unwrap();
    let gate_y = std::cell::Cell::new(0.0);
    let frame = |play: &mut Play, devices: &mut Devices| {
        play.frame(STEP_SECONDS, devices, |_, at| gate_y.set(at.w_axis.y));
    };
    // Too far to reach: pressing E does nothing.
    let mut devices = Devices::new();
    devices.key("KeyE", true);
    frame(&mut play, &mut devices);
    devices.key("KeyE", false);
    for _ in 0..30 {
        frame(&mut play, &mut devices);
    }
    assert_eq!(gate_y.get(), 0.0);
    // Walk towards it for 4 seconds, at 1.6 m/s: from 9.3 m to 2.9 m away.
    devices.key("KeyW", true);
    for _ in 0..240 {
        frame(&mut play, &mut devices);
    }
    devices.key("KeyW", false);
    devices.key("KeyE", true);
    frame(&mut play, &mut devices);
    devices.key("KeyE", false);
    for _ in 0..300 {
        frame(&mut play, &mut devices);
    }
    let messages: Vec<String> = play.take_messages().into_iter().map(|m| m.text).collect();
    assert_eq!(messages, ["gate: The gate rises."]);
    assert!(
        (gate_y.get() - 3.2).abs() < 1.0e-4,
        "the gate rose to {}",
        gate_y.get()
    );
}

/// A flat floor at height zero, 40 m across.
fn floor() -> Collision {
    Collision {
        shapes: vec![CollisionShape {
            hull: false,
            vertices: vec![
                [-20.0, 0.0, -20.0],
                [20.0, 0.0, -20.0],
                [20.0, 0.0, 20.0],
                [-20.0, 0.0, 20.0],
            ],
            triangles: vec![[0, 2, 1], [0, 3, 2]],
        }],
        instance_shapes: vec![0],
        heightfield: None,
    }
}

const WALKS: &str = r#"
def on_tick(self, dt):
    self.walk = vec3(0, 0, -input("forward") * 3)
    self.vars.z = self.position.z
"#;

fn walking_game(extra: &[(&str, &str)], scene: &str) -> Play {
    let mut list = vec![
        (GAME_FILE, scene),
        (
            "hero.actor",
            "name hero\nscript hero.star\ncamera 0 1.6 0\ncharacter 0.3 1.8\n",
        ),
        ("hero.star", WALKS),
    ];
    list.extend_from_slice(extra);
    Play::load(&files(&list), |_| Some(Mat4::IDENTITY), Some(&floor())).unwrap()
}

fn walk_forward(play: &mut Play, frames: u32) -> View {
    let mut devices = Devices::new();
    devices.key("KeyW", true);
    let mut view = None;
    for _ in 0..frames {
        view = play.frame(STEP_SECONDS, &mut devices, |_, _| {});
    }
    view.unwrap()
}

#[test]
fn the_player_stands_on_the_scenes_collision_and_walks() {
    let mut play = walking_game(&[], "actor hero.actor 0 0.5 0 0\n");
    let view = walk_forward(&mut play, 60);
    assert!(
        view.position.y > 1.5 && view.position.y < 1.7,
        "on the floor: {view:?}"
    );
    assert!(view.position.z < -2.0, "walked: {view:?}");
    let messages = play.take_messages();
    assert!(messages.is_empty(), "{messages:?}");
}

#[test]
fn a_blocker_in_the_scene_stops_the_player() {
    let mut play = walking_game(&[], "actor hero.actor 0 0.1 0 0\nblocker 0 2 -3 10 4 1 0\n");
    let view = walk_forward(&mut play, 180);
    assert!(view.position.z > -3.0, "{view:?}");
}

const LATCH: &str = r#"
def on_begin(self):
    self.vars.used = 0
    self.vars.ahead = 0.0

def on_tick(self, dt):
    if pressed("interact"):
        self.vars.used += 1
    self.vars.ahead = max(self.vars.ahead, input("forward"))
"#;

#[test]
fn flying_leaves_the_player_standing_while_the_rest_of_the_game_hears_the_keys() {
    let latch = ("latch.actor", "name latch\nscript latch.star\n");
    let mut play = walking_game(
        &[latch, ("latch.star", LATCH)],
        "actor hero.actor 0 0.5 0 0\nactor latch.actor 0 0 -8 0\n",
    );
    let walked = walk_forward(&mut play, 60);
    assert!(play.has_camera() && !play.flying());
    let hero_z = |play: &Play| match var(play, "hero", "z") {
        Some(ScriptValue::Float(z)) => z as f32,
        other => panic!("{other:?}"),
    };

    // Flown from, he stands where he was left, though the keys that fly the camera are his
    // to walk with, and the camera is nobody's.
    play.set_flying(true);
    assert!(play.has_player() && play.flying() && !play.has_camera());
    let mut devices = Devices::new();
    devices.key("KeyW", true);
    devices.mouse_motion(0.0, -100.0);
    for frame in 0..60 {
        if frame == 30 {
            devices.key("KeyE", true);
        }
        assert_eq!(play.frame(STEP_SECONDS, &mut devices, |_, _| {}), None);
    }
    assert!(
        (hero_z(&play) - walked.position.z).abs() < 0.1,
        "stood at {}, left at {walked:?}",
        hero_z(&play)
    );
    // Everything else heard them: the key used, and the one held.
    assert_eq!(var(&play, "latch", "used"), Some(ScriptValue::Int(1)));
    assert_eq!(var(&play, "latch", "ahead"), Some(ScriptValue::Float(1.0)));

    // Back, the camera is his again, looking where it did, and he walks on.
    play.set_flying(false);
    let view = walk_forward(&mut play, 60);
    assert!(view.position.z < walked.position.z - 2.0, "{view:?}");
    assert_eq!(view.pitch, walked.pitch);
    assert!(play.take_messages().is_empty());

    // With nobody to play, there is nobody to leave: the camera was never theirs.
    let scene = files(&[
        (GAME_FILE, "actor latch.actor 0 0 0 0\n"),
        ("latch.actor", "name latch\nscript latch.star\n"),
        ("latch.star", LATCH),
    ]);
    let mut play = Play::load(&scene, baked, None).unwrap();
    play.set_flying(true);
    assert!(!play.flying() && !play.has_camera());
}

const GATEKEEPER: &str = r#"
def on_enter(self, who):
    self.vars.came = who
    self.position = self.position + vec3(0, 5, 0)

def on_exit(self, who):
    self.vars.left = True
"#;

#[test]
fn a_door_stops_the_player_and_a_sensor_opens_it() {
    // A sensor across the way at z = -2 lifts itself and tells its script. A door at z = -4,
    // blocking the way, is an ordinary collider, tested first without the sensor.
    let door = ("door.actor", "name door\ncollider box 10 4 0.5 at 0 2 0\n");
    let mut play = walking_game(
        &[door],
        "actor hero.actor 0 0.1 0 0\nactor door.actor 0 0 -4 0\n",
    );
    let view = walk_forward(&mut play, 180);
    assert!(view.position.z > -4.0, "the door stops him: {view:?}");

    let plate = (
        "plate.actor",
        "name plate\nscript plate.star\ncollider box 10 4 0.5 at 0 2 0 sensor\n",
    );
    let mut play = walking_game(
        &[plate, ("plate.star", GATEKEEPER)],
        "actor hero.actor 0 0.1 0 0\nactor plate.actor 0 0 -2 0\n",
    );
    let view = walk_forward(&mut play, 120);
    assert!(view.position.z < -3.0, "a sensor stops nobody: {view:?}");
    assert!(matches!(
        var(&play, "plate", "came"),
        Some(ScriptValue::Entity(_))
    ));
    assert_eq!(var(&play, "plate", "left"), Some(ScriptValue::Bool(true)));
    assert!(play.take_messages().is_empty());
}

#[test]
fn the_world_comes_from_the_scenes_collision_and_nothing_else() {
    // Without collision, a character has nothing to stand on.
    let scene = files(&[
        (GAME_FILE, "actor hero.actor 0 0.5 0 0\n"),
        (
            "hero.actor",
            "name hero\ncamera 0 1.6 0\ncharacter 0.3 1.8\n",
        ),
    ]);
    let mut play = Play::load(&scene, |_| Some(Mat4::IDENTITY), None).unwrap();
    let view = walk_forward(&mut play, 60);
    assert!(view.position.y < -2.0, "it falls: {view:?}");
    // With it, the world has the floor and the character.
    let play = walking_game(&[], "actor hero.actor 0 0.5 0 0\n");
    assert_eq!(play.physics().collider_count(), 1);
}

const WALKER_ACTOR: &str = "name walker\nmesh m.glb 1 yaw=180\nanimation a.anim\nscript w.star\n";
const WALKER_GRAPH: &str = "
clip idle I
clip walk W
notify walk 0.0 step
param speed 0
state idle clip idle
state walking clip walk
state dancing clip walk
start idle
transition idle walking speed > 0.1 fade 0
transition walking idle speed < 0.1 fade 0
";

/// A skeleton of one bone, and two clips that stand it at different heights, as the bake names
/// them for `cove/a.anim`.
fn walker_assets() -> (Vec<solstice_asset::Skeleton>, Vec<solstice_asset::Clip>) {
    use solstice_anim::{compress, Frames};
    let clip = |name: &str, y: f32| {
        let mut clip = compress(
            name,
            0,
            &Frames {
                rotations: vec![vec![Quat::IDENTITY]; 8],
                translations: vec![vec![Vec3::Y * y]; 8],
            },
            true,
        );
        clip.name = solstice_game::animation::clip_key("cove/a.anim", name);
        clip
    };
    (
        vec![solstice_asset::Skeleton {
            bones: vec![solstice_asset::Bone {
                name: "root".to_owned(),
                ..Default::default()
            }],
        }],
        vec![clip("idle", 1.0), clip("walk", 2.0)],
    )
}

fn walker_play(script: &str) -> Play {
    let scene = files(&[
        (GAME_FILE, "actor cove/walker.actor 0 0 0 0 instances=0+1\n"),
        ("cove/walker.actor", WALKER_ACTOR),
        ("cove/a.anim", WALKER_GRAPH),
        ("cove/w.star", script),
    ]);
    let (skeletons, clips) = walker_assets();
    let mut play = Play::load(&scene, baked, None).unwrap();
    play.attach_animation(
        &scene,
        &solstice_game::animation::AnimationAssets {
            skeletons: &skeletons,
            clips: &clips,
            poses: &[0],
            instance_poses: &[0],
        },
    )
    .unwrap();
    play
}

/// Runs a few frames and returns the latest pose's height of its one bone.
fn pose_height(play: &mut Play, devices: &mut Devices, frames: u32) -> Option<f32> {
    let mut height = None;
    for _ in 0..frames {
        play.frame(STEP_SECONDS, devices, |_, _| {});
        if let Some((_, bones)) = play.take_poses().pop() {
            height = Some(bones[0].w_axis.y);
        }
    }
    height
}

#[test]
fn an_actor_that_walks_plays_its_walk_and_its_script_hears_the_footsteps() {
    let mut play = walker_play(
        r#"
def on_tick(self, dt):
    self.walk = vec3(1, 0, 0)

def on_anim(self, name):
    log("heard " + name)
"#,
    );
    let mut devices = Devices::new();
    let height = pose_height(&mut play, &mut devices, 30).unwrap();
    assert!((height - 2.0).abs() < 0.01, "standing at {height}");
    let messages = play.take_messages();
    assert!(
        messages.iter().any(|m| m.text.contains("heard step")),
        "{messages:?}"
    );
}

#[test]
fn footsteps_come_at_the_same_steps_at_any_frame_rate() {
    let script = r#"
def on_tick(self, dt):
    self.walk = vec3(1, 0, 0)

def on_anim(self, name):
    log("step at " + str(now()))
"#;
    let run = |frame_seconds: f64, frames: u32| {
        let mut play = walker_play(script);
        let mut devices = Devices::new();
        for _ in 0..frames {
            play.frame(frame_seconds, &mut devices, |_, _| {});
        }
        play.take_messages()
    };
    let fine = run(STEP_SECONDS, 240);
    let coarse = run(STEP_SECONDS * 3.0, 80);
    assert!(fine.len() > 4);
    // The two runs may end a step apart: what they share must be the same.
    let shared = fine.len().min(coarse.len());
    assert!(fine.len().abs_diff(coarse.len()) <= 1);
    assert_eq!(fine[..shared], coarse[..shared]);
}

#[test]
fn a_script_can_set_parameters_and_force_a_state() {
    let mut play = walker_play(
        r#"
def on_tick(self, dt):
    anim_set("speed", 1.0)
"#,
    );
    let mut devices = Devices::new();
    let height = pose_height(&mut play, &mut devices, 10).unwrap();
    assert!((height - 2.0).abs() < 0.01, "standing at {height}");

    let mut play = walker_play(
        r#"
def on_tick(self, dt):
    anim_play("dancing", 0.0)
"#,
    );
    let height = pose_height(&mut play, &mut devices, 10).unwrap();
    assert!((height - 2.0).abs() < 0.01, "standing at {height}");

    // Wrong names for parameters and states change nothing, and aren't fatal.
    let mut play = walker_play(
        r#"
def on_tick(self, dt):
    anim_set("nothing", 1.0)
    anim_play("nowhere")
"#,
    );
    let height = pose_height(&mut play, &mut devices, 10).unwrap();
    assert!((height - 1.0).abs() < 0.01, "standing at {height}");
}

#[test]
fn an_animated_actor_with_no_skeleton_says_so() {
    let scene = files(&[
        (GAME_FILE, "actor cove/walker.actor 0 0 0 0 instances=0+1\n"),
        ("cove/walker.actor", WALKER_ACTOR),
        ("cove/a.anim", WALKER_GRAPH),
        ("cove/w.star", "def on_tick(self, dt):\n    pass\n"),
    ]);
    let mut play = Play::load(&scene, baked, None).unwrap();
    let (skeletons, clips) = walker_assets();
    let errors = play
        .attach_animation(
            &scene,
            &solstice_game::animation::AnimationAssets {
                skeletons: &skeletons,
                clips: &clips,
                poses: &[0],
                instance_poses: &[solstice_asset::NO_POSE],
            },
        )
        .unwrap_err();
    assert!(errors[0].contains("no skeleton"), "{errors:?}");
}

const DOOR_ACTOR: &str = include_str!("../../../scenes/zorah/door.actor");
const DOOR: &str = include_str!("../../../scenes/zorah/door.star");
const DOOR_INPUT: &str = include_str!("../../../scenes/zorah/zorah.input");

/// Runs a game for some seconds with a key down from the start, and says where each instance
/// was last drawn.
fn after(play: &mut Play, key: Option<&str>, seconds: f64) -> HashMap<u32, Mat4> {
    let mut placed = HashMap::new();
    let mut devices = Devices::new();
    if let Some(key) = key {
        devices.key(key, true);
    }
    let steps = (seconds / STEP_SECONDS).ceil() as usize;
    for step in 0..steps {
        if step == 1 {
            if let Some(key) = key {
                devices.key(key, false);
            }
        }
        play.frame(STEP_SECONDS, &mut devices, |id, at| {
            placed.insert(id, at);
        });
    }
    placed
}

#[test]
fn a_doorway_given_to_a_scene_opens_for_whoever_is_watching_from_near_it() {
    // Two leaves the scene already draws, hinged either side of a doorway two metres wide.
    let scene = files(&[
        (
            GAME_FILE,
            "input zorah.input\n\
             actor door.actor 20 0 1 0 middle=20,0,0 swing=90 reach=5 seconds=1 instances=2+1\n\
             actor door.actor 20 0 -1 0 middle=20,0,0 swing=-90 reach=5 seconds=1 instances=3+1\n",
        ),
        ("door.actor", DOOR_ACTOR),
        ("door.star", DOOR),
        ("zorah.input", DOOR_INPUT),
    ]);
    assert!(solstice_game::missing_files(&scene).is_empty());
    // Each leaf reaches from its hinge to the middle.
    let baked = |id: u32| match id {
        2 => Some(Mat4::from_translation(Vec3::new(20.0, 1.0, 0.5))),
        3 => Some(Mat4::from_translation(Vec3::new(20.0, 1.0, -0.5))),
        _ => None,
    };
    let mut play = Play::load(&scene, baked, None).unwrap();
    assert!(!play.has_player());
    let shut = after(&mut play, None, 0.1);
    assert!(close(shut[&2], baked(2).unwrap()) && close(shut[&3], baked(3).unwrap()));

    // From across the courtyard nothing answers, and the flying camera's own key is not it.
    play.set_viewer(Vec3::new(60.0, 3.0, 0.0));
    assert!(after(&mut play, Some("KeyF"), 1.5).is_empty());
    play.set_viewer(Vec3::new(23.0, 9.0, 1.0));
    assert!(after(&mut play, Some("KeyE"), 1.5).is_empty());

    // From near, however high, both leaves swing a quarter turn about their own hinges.
    let open = after(&mut play, Some("KeyF"), 1.5);
    for (id, hinge, swing) in [(2, 1.0, 90_f32), (3, -1.0, -90_f32)] {
        let hinge = Mat4::from_translation(Vec3::new(20.0, 0.0, hinge));
        let turned = hinge * Mat4::from_rotation_y(swing.to_radians()) * hinge.inverse();
        assert!(
            close(open[&id], turned * baked(id).unwrap()),
            "{id}: {:?}",
            open[&id]
        );
    }
    // Both open the same way, into the room at -X, and used again they shut.
    assert!(open[&2].w_axis.x < 19.6 && open[&3].w_axis.x < 19.6);
    let shut = after(&mut play, Some("KeyF"), 1.5);
    assert!(close(shut[&2], baked(2).unwrap()) && close(shut[&3], baked(3).unwrap()));
    assert!(play.take_messages().is_empty());
}

#[test]
fn zorahs_doors_are_placed_so_that_the_game_loads() {
    let scene = files(&[
        (GAME_FILE, include_str!("../../../scenes/zorah/scene.game")),
        ("door.actor", DOOR_ACTOR),
        ("door.star", DOOR),
        ("zorah.input", DOOR_INPUT),
    ]);
    assert!(solstice_game::missing_files(&scene).is_empty());
    // Its bake has this many instances; each may be one leaf's only.
    let play = Play::load(&scene, |id| (id < 43_069).then_some(Mat4::IDENTITY), None).unwrap();
    let mut instances: Vec<u32> = play.instances().collect();
    let count = instances.len();
    instances.sort_unstable();
    instances.dedup();
    assert_eq!(instances.len(), count, "an instance is on two leaves");
    assert_eq!(count, 2 * 6 + 2 + 2 * 120 + 2 * 21 + 7 * 12);
}
