//! Input actions: a text file mapping keys, the mouse, gamepads and touch to the actions scripts
//! read, so the same game works with any of them, on a phone too.
//!
//! ```text
//! # ACTION  DEVICE   CONTROL  [SCALE]
//! forward   key      KeyW
//! forward   key      KeyS     -1
//! forward   gamepad  left_y   -1
//! forward   touch    stick_y  -1
//! turn      mouse    x        -0.002
//! interact  touch    tap
//! ```
//!
//! * `key` controls are the browser's `KeyboardEvent.code` names: `KeyW`, `Space`, `ArrowUp`,
//!   `ShiftLeft`, `Digit1`. The native viewer uses the same names.
//! * `mouse`: `x` and `y`, the pointer's movement, and the buttons `left`, `right` and `middle`.
//! * `gamepad`, in the standard layout: the sticks `left_x`, `left_y`, `right_x`, `right_y`
//!   (down and right are positive), the face buttons `south`, `east`, `west`, `north`, and
//!   `l1`, `r1`, `l2`, `r2`, `select`, `start`, `l3`, `r3`, `up`, `down`, `left`, `right`.
//! * `touch`: a stick under the left thumb, `stick_x` and `stick_y`; dragging on the right of
//!   the screen, `look_x` and `look_y`; and a quick `tap`.
//!
//! Each action's value is the sum of what its controls give, times their scales: buttons and
//! sticks between -1 and 1 in all, and movement (the mouse, `look_x`, `look_y`) in pixels a
//! second, unlimited. So `self.yaw += input("turn") * dt` turns by the mouse's movement times
//! its scale, however the frames fall. Tapping a button, or the screen, also presses its action.

use crate::{is_name, lines, number, FileError};
use solstice_world::Input;
use std::collections::{HashMap, HashSet};
use std::fmt::Write;

/// The actions a scene gets if it names no `.input` file.
pub const DEFAULT_INPUT: &str = "\
# Moving: WASD or the arrows, the left stick, or a thumb on the left of the screen.
forward key KeyW
forward key ArrowUp
forward key KeyS -1
forward key ArrowDown -1
forward gamepad left_y -1
forward touch stick_y -1
right key KeyD
right key ArrowRight
right key KeyA -1
right key ArrowLeft -1
right gamepad left_x
right touch stick_x
run key ShiftLeft
run gamepad l3
# Looking, in radians a second: the mouse, the right stick, or dragging on the right.
turn mouse x -0.002
turn gamepad right_x -2.5
turn touch look_x -0.004
look_up mouse y -0.002
look_up gamepad right_y -2
look_up touch look_y -0.004
# Doing.
jump key Space
jump gamepad south
interact key KeyE
interact mouse left
interact gamepad west
interact touch tap
";

/// The built-in actions for driving, which a scene whose player can be a car has as well.
pub const DRIVING_INPUT: &str = "\
# Driving: W and S or the arrows for the pedals, A and D to steer; the triggers and the left
# stick; or a thumb on the left of the screen. E and Q, or the bumpers, change gear.
throttle key KeyW
throttle key ArrowUp
throttle gamepad r2
throttle touch stick_y -1
brake key KeyS
brake key ArrowDown
brake gamepad l2
brake touch stick_y
steer key KeyD
steer key ArrowRight
steer key KeyA -1
steer key ArrowLeft -1
steer gamepad left_x
steer touch stick_x
handbrake key Space
handbrake gamepad east
shift_up key KeyE
shift_up gamepad r1
shift_down key KeyQ
shift_down gamepad l1
reset key KeyR
reset gamepad north
camera key KeyC
camera gamepad select
look_back key KeyB
look_back gamepad r3
# The right stick looks round the car, and tips the view up over it or down behind it; so do
# J and L, and I and K.
look_round gamepad right_x
look_round key KeyL
look_round key KeyJ -1
look_over gamepad right_y
look_over key KeyI
look_over key KeyK -1
# The mouse swings the view right round the car, and up over it.
orbit mouse x -0.0032
orbit_up mouse y 0.0026
# P hands the car to a driver of its own, round the circuit, and takes it back.
autopilot key KeyP
autopilot gamepad start
# A four-wheel drive: T moves its transfer case on, from two-wheel drive to four high to four
# low; G locks its differentials, the rear, then both, then neither.
drive_mode key KeyT
drive_mode gamepad west
diff_lock key KeyG
diff_lock gamepad south
# Y lets its tyres down for mud and sand, and pumps them up again for hard ground.
tyres key KeyY
tyres gamepad up
";

/// A kind of input device.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Device {
    Key,
    Mouse,
    Gamepad,
    Touch,
}

impl Device {
    const ALL: [(Self, &'static str); 4] = [
        (Self::Key, "key"),
        (Self::Mouse, "mouse"),
        (Self::Gamepad, "gamepad"),
        (Self::Touch, "touch"),
    ];

    pub fn name(self) -> &'static str {
        Self::ALL
            .iter()
            .find(|(d, _)| *d == self)
            .expect("listed")
            .1
    }

    /// The controls it has, or `None` for keys, which are any `KeyboardEvent.code` name.
    fn controls(self) -> Option<&'static [&'static str]> {
        match self {
            Self::Key => None,
            Self::Mouse => Some(&["x", "y", "left", "right", "middle"]),
            Self::Gamepad => Some(&[
                "left_x", "left_y", "right_x", "right_y", "south", "east", "west", "north", "l1",
                "r1", "l2", "r2", "select", "start", "l3", "r3", "up", "down", "left", "right",
            ]),
            Self::Touch => Some(&["stick_x", "stick_y", "look_x", "look_y", "tap"]),
        }
    }

    /// Whether a control gives movement in pixels, rather than a value between -1 and 1.
    fn is_movement(self, control: &str) -> bool {
        matches!(
            (self, control),
            (Self::Mouse, "x" | "y") | (Self::Touch, "look_x" | "look_y")
        )
    }
}

/// One control, feeding one action.
#[derive(Clone, Debug, PartialEq)]
pub struct Binding {
    pub action: String,
    pub device: Device,
    pub control: String,
    pub scale: f32,
}

/// How the devices' controls become actions.
#[derive(Clone, Debug, PartialEq)]
pub struct InputMap {
    pub bindings: Vec<Binding>,
}

impl Default for InputMap {
    fn default() -> Self {
        Self::built_in(true)
    }
}

impl InputMap {
    /// The built-in actions, with those for driving if `driving`. A scene with no car leaves
    /// them out, so its keys don't also press pedals it hasn't got.
    pub fn built_in(driving: bool) -> Self {
        let text = match driving {
            true => format!("{DEFAULT_INPUT}{DRIVING_INPUT}"),
            false => DEFAULT_INPUT.to_owned(),
        };
        Self::parse(&text).expect("the built-in actions read")
    }
}

impl InputMap {
    pub fn parse(text: &str) -> Result<Self, FileError> {
        let mut bindings = Vec::new();
        for (line, words) in lines(text) {
            let error = |message: String| FileError { line, message };
            if !(3..=4).contains(&words.len()) {
                return Err(error("expected `ACTION DEVICE CONTROL [SCALE]`".to_owned()));
            }
            let action = words[0];
            if !is_name(action) {
                return Err(error(format!(
                    "`{action}` can't be an action's name: use letters, digits and _"
                )));
            }
            let device = Device::ALL
                .iter()
                .find(|(_, name)| *name == words[1])
                .map(|(device, _)| *device)
                .ok_or_else(|| {
                    error(format!(
                        "`{}` is not a device: use key, mouse, gamepad or touch",
                        words[1]
                    ))
                })?;
            let control = words[2];
            let known = match device.controls() {
                Some(controls) => controls.contains(&control),
                None => control.chars().all(|c| c.is_ascii_alphanumeric()),
            };
            if !known {
                return Err(error(match device.controls() {
                    Some(controls) => format!(
                        "a {} has no `{control}`: it has {}",
                        device.name(),
                        controls.join(", ")
                    ),
                    None => format!("`{control}` is not a key's code, such as KeyW or Space"),
                }));
            }
            let scale = words.get(3).map_or(Ok(1.0), |s| number(s)).map_err(error)?;
            bindings.push(Binding {
                action: action.to_owned(),
                device,
                control: control.to_owned(),
                scale,
            });
        }
        Ok(Self { bindings })
    }

    /// The file's text, which [`InputMap::parse`] reads back as it is.
    pub fn to_text(&self) -> String {
        let mut text = String::new();
        for b in &self.bindings {
            let _ = write!(text, "{} {} {}", b.action, b.device.name(), b.control);
            if b.scale != 1.0 {
                let _ = write!(text, " {}", b.scale);
            }
            text.push('\n');
        }
        text
    }

    /// Each action's value this frame, `seconds` long, from what the devices are doing.
    pub fn actions(&self, devices: &Devices, seconds: f32) -> Input {
        let mut held: HashMap<&str, f32> = HashMap::new();
        let mut moved: HashMap<&str, f32> = HashMap::new();
        let mut input = Input::new();
        for b in &self.bindings {
            let value = devices.value(b.device, &b.control);
            if b.device.is_movement(&b.control) {
                if seconds > 0.0 {
                    *moved.entry(&b.action).or_default() += value / seconds * b.scale;
                }
            } else {
                *held.entry(&b.action).or_default() += value * b.scale;
            }
            if devices.was_pressed(b.device, &b.control) {
                input.press(&b.action);
            }
        }
        for (action, value) in held {
            input.set(
                action,
                value.clamp(-1.0, 1.0) + moved.remove(action).unwrap_or(0.0),
            );
        }
        for (action, value) in moved {
            input.set(action, value);
        }
        input
    }
}

/// Below this, a stick counts as centred: worn sticks rarely rest at exactly zero.
pub const STICK_DEAD_ZONE: f32 = 0.15;

/// What the input devices are doing this frame, filled in by a viewer from its events.
#[derive(Clone, Debug, Default)]
pub struct Devices {
    values: HashMap<(Device, String), f32>,
    pressed: HashSet<(Device, String)>,
    /// How many times as fast as it was made to a controller's right stick looks round: as
    /// it was made, if this is nothing. What is bound to the stick is a rate, so more of
    /// the stick is more of the rate.
    pub look: f32,
}

impl Devices {
    pub fn new() -> Self {
        Self::default()
    }

    fn value(&self, device: Device, control: &str) -> f32 {
        // Allocation-free lookups would need a borrowed key type; there are a few dozen a frame.
        self.values
            .get(&(device, control.to_owned()))
            .copied()
            .unwrap_or(0.0)
    }

    fn was_pressed(&self, device: Device, control: &str) -> bool {
        self.pressed.contains(&(device, control.to_owned()))
    }

    fn set(&mut self, device: Device, control: &str, value: f32) {
        let key = (device, control.to_owned());
        let before = self.values.get(&key).copied().unwrap_or(0.0);
        // Either way: a stick pushed to -1 presses an action bound to it with a scale of -1.
        if value.abs() >= 0.5 && before.abs() < 0.5 {
            self.pressed.insert(key.clone());
        }
        if value == 0.0 {
            self.values.remove(&key);
        } else {
            self.values.insert(key, value);
        }
    }

    fn add(&mut self, device: Device, control: &str, amount: f32) {
        *self.values.entry((device, control.to_owned())).or_default() += amount;
    }

    /// A key, by its `KeyboardEvent.code` name, going down or up.
    pub fn key(&mut self, code: &str, down: bool) {
        self.set(Device::Key, code, f32::from(u8::from(down)));
    }

    /// A mouse button, `left`, `right` or `middle`, going down or up.
    pub fn mouse_button(&mut self, button: &str, down: bool) {
        self.set(Device::Mouse, button, f32::from(u8::from(down)));
    }

    /// The mouse moving by this many pixels, right and down.
    pub fn mouse_motion(&mut self, x: f32, y: f32) {
        self.add(Device::Mouse, "x", x);
        self.add(Device::Mouse, "y", y);
    }

    /// A gamepad button's value, from 0 to 1; triggers are in between.
    pub fn gamepad_button(&mut self, button: &str, value: f32) {
        self.set(Device::Gamepad, button, value);
    }

    /// A stick's axis, from -1 to 1, with the dead zone taken out.
    pub fn gamepad_axis(&mut self, axis: &str, value: f32) {
        let size = value.abs();
        let value = if size < STICK_DEAD_ZONE {
            0.0
        } else {
            value.signum() * ((size - STICK_DEAD_ZONE) / (1.0 - STICK_DEAD_ZONE)).min(1.0)
        };
        let faster = axis.starts_with("right_") && self.look > 0.0;
        let value = if faster { value * self.look } else { value };
        self.set(Device::Gamepad, axis, value);
    }

    /// Where the touch stick is pushed, each axis from -1 to 1, or 0 and 0 when let go.
    pub fn touch_stick(&mut self, x: f32, y: f32) {
        self.set(Device::Touch, "stick_x", x.clamp(-1.0, 1.0));
        self.set(Device::Touch, "stick_y", y.clamp(-1.0, 1.0));
    }

    /// A drag on the look side of the screen, by this many pixels.
    pub fn touch_look(&mut self, x: f32, y: f32) {
        self.add(Device::Touch, "look_x", x);
        self.add(Device::Touch, "look_y", y);
    }

    /// A quick tap on the screen.
    pub fn touch_tap(&mut self) {
        self.pressed.insert((Device::Touch, "tap".to_owned()));
    }

    /// Lets everything go, for when the window loses focus.
    pub fn release_all(&mut self) {
        self.values.clear();
        self.pressed.clear();
    }

    /// Ends a frame: presses start again from nothing; what is held stays held, and movement
    /// builds up until [`Devices::clear_movement`].
    pub fn end_frame(&mut self) {
        self.pressed.clear();
    }

    /// Forgets the movement so far, once a step has used it.
    pub fn clear_movement(&mut self) {
        self.values
            .retain(|(device, control), _| !device.is_movement(control));
    }

    /// Whether anything of a device's was pressed this frame: a key or a button going down,
    /// a trigger pulled or a stick pushed past half way. What is only resting off its
    /// middle, a worn stick, never is; nor is the mouse being moved. So it says which
    /// device a hand is on, for whoever names its buttons.
    pub fn pressed_on(&self, device: Device) -> bool {
        self.pressed.iter().any(|(of, _)| *of == device)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_built_in_actions_read_back_what_they_wrote() {
        let map = InputMap::default();
        assert!(map.bindings.len() > 20);
        assert_eq!(InputMap::parse(&map.to_text()).unwrap(), map);
    }

    #[test]
    fn mistakes_in_input_files_are_explained() {
        let cases = [
            ("forward key", 1, "expected `ACTION DEVICE CONTROL"),
            ("\nfor-ward key KeyW", 2, "can't be an action's name"),
            ("forward keyboard KeyW", 1, "not a device"),
            ("forward key Key-W", 1, "not a key's code"),
            (
                "forward gamepad left_z",
                1,
                "has no `left_z`: it has left_x",
            ),
            ("forward touch swipe", 1, "a touch has no `swipe`"),
            ("forward key KeyW fast", 1, "`fast` is not a number"),
            ("forward key KeyW 1 2", 1, "expected"),
        ];
        for (text, line, part) in cases {
            let error = InputMap::parse(text).unwrap_err();
            assert_eq!(error.line, line, "{text:?}: {error}");
            assert!(error.message.contains(part), "{text:?}: {error}");
        }
    }

    #[test]
    fn controls_add_up_and_buttons_are_limited() {
        let map =
            InputMap::parse("forward key KeyW\nforward gamepad left_y -1\nback key KeyS -1\n")
                .unwrap();
        let mut devices = Devices::new();
        devices.key("KeyW", true);
        devices.gamepad_axis("left_y", -1.0);
        devices.key("KeyS", true);
        let input = map.actions(&devices, 1.0 / 60.0);
        assert_eq!(
            input.value("forward"),
            1.0,
            "two at once still count as one"
        );
        assert_eq!(input.value("back"), -1.0);
        assert!(input.pressed("forward"));
        // Held, not pressed again, in the next frame.
        devices.end_frame();
        let input = map.actions(&devices, 1.0 / 60.0);
        assert!(input.held("forward") && !input.pressed("forward"));
    }

    #[test]
    fn movement_is_in_pixels_a_second_and_starts_again_once_used() {
        let map = InputMap::parse("turn mouse x -0.5\nturn gamepad right_x 2\n").unwrap();
        let mut devices = Devices::new();
        devices.mouse_motion(10.0, 0.0);
        devices.mouse_motion(20.0, 4.0);
        devices.gamepad_axis("right_x", 1.0);
        let input = map.actions(&devices, 0.5);
        // 30 pixels in half a second, times -0.5, plus the stick's 2 (limited to 1).
        assert_eq!(input.value("turn"), 30.0 / 0.5 * -0.5 + 1.0);
        devices.clear_movement();
        assert_eq!(map.actions(&devices, 0.5).value("turn"), 1.0);
        // A frame of no time moves nothing, rather than dividing by zero.
        devices.mouse_motion(5.0, 0.0);
        assert_eq!(map.actions(&devices, 0.0).value("turn"), 1.0);
    }

    #[test]
    fn sticks_have_a_dead_zone_and_reach_one() {
        let map = InputMap::parse("right gamepad left_x").unwrap();
        let mut devices = Devices::new();
        devices.gamepad_axis("left_x", 0.1);
        assert_eq!(map.actions(&devices, 0.1).value("right"), 0.0);
        devices.gamepad_axis("left_x", -1.0);
        assert_eq!(map.actions(&devices, 0.1).value("right"), -1.0);
        devices.gamepad_axis("left_x", 0.575);
        assert!((map.actions(&devices, 0.1).value("right") - 0.5).abs() < 1.0e-6);
    }

    #[test]
    fn a_stick_pushed_either_way_presses() {
        // Up on the left stick is -1, and forward's binding to it has a scale of -1.
        let map = InputMap::default();
        let mut devices = Devices::new();
        devices.gamepad_axis("left_y", -1.0);
        let input = map.actions(&devices, 0.1);
        assert!(input.pressed("forward") && input.held("forward"));
        devices.end_frame();
        devices.gamepad_axis("left_y", -0.9);
        assert!(
            !map.actions(&devices, 0.1).pressed("forward"),
            "still held, not pressed again"
        );
    }

    #[test]
    fn a_button_read_again_each_frame_is_pressed_only_once() {
        // The web page reads gamepads every frame, so each button is set again and again.
        let map = InputMap::default();
        let mut devices = Devices::new();
        let mut presses = 0;
        for frame in 0..5 {
            devices.gamepad_button("south", if frame < 3 { 1.0 } else { 0.0 });
            presses += u32::from(map.actions(&devices, 0.1).pressed("jump"));
            devices.end_frame();
        }
        devices.gamepad_button("south", 1.0);
        presses += u32::from(map.actions(&devices, 0.1).pressed("jump"));
        assert_eq!(presses, 2, "pressed, held, released, pressed again");
    }

    #[test]
    fn the_device_in_hand_is_the_one_that_was_pressed() {
        let mut devices = Devices::new();
        // A worn stick resting off its middle, and the mouse moving, are nobody's hand.
        devices.gamepad_axis("left_x", 0.3);
        devices.mouse_motion(40.0, 0.0);
        assert!(!devices.pressed_on(Device::Gamepad) && !devices.pressed_on(Device::Mouse));
        // A trigger pulled past half way is, once: held, it is not pressed again.
        devices.gamepad_button("r2", 0.8);
        assert!(devices.pressed_on(Device::Gamepad) && !devices.pressed_on(Device::Key));
        devices.end_frame();
        devices.gamepad_button("r2", 0.9);
        assert!(!devices.pressed_on(Device::Gamepad));
        devices.key("KeyW", true);
        devices.mouse_button("left", true);
        assert!(devices.pressed_on(Device::Key) && devices.pressed_on(Device::Mouse));
    }

    #[test]
    fn a_tap_presses_without_holding_and_focus_loss_lets_go() {
        let map = InputMap::default();
        let mut devices = Devices::new();
        devices.touch_tap();
        devices.key("KeyW", true);
        let input = map.actions(&devices, 0.1);
        assert!(input.pressed("interact") && !input.held("interact"));
        devices.release_all();
        let input = map.actions(&devices, 0.1);
        assert!(!input.pressed("interact") && input.value("forward") == 0.0);
    }
}
