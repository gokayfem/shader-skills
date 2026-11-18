//! What the player is doing, as named actions, and a recording of it, step by step.
//!
//! Keys, buttons and touches are mapped to actions before they reach the world (step 4 of
//! docs/gameplay.md). The world sees each action's value: 1 for a button held, 0 for released,
//! anything between for a stick or a trigger. It also sees which actions were pressed since the
//! step before, so a tap that comes and goes between two steps isn't lost.
//!
//! Gameplay reads the input of the step it is in, never of the frame, so a recording of each
//! step's input replays a run exactly. Where the game is watched from is part of it for the same
//! reason: a door that opens to whoever stands by it must open in the replay too. So is whether
//! the player has left whoever they play, to fly the camera.

use glam::Vec3;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

/// Marks the entity the player plays: the one whose script hears nothing while the player is
/// [away](Input::away) from it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Played;

/// Marks one the player can play and is not playing now: a car whose driver has got out of
/// it, or the driver while they are in it. Its script hears nothing, and a car is asked for
/// nothing but its handbrake.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Idle;

/// Each action's value, which were pressed since the step before, where the game is watched
/// from, and whether the player is away. Actions not mentioned are 0.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Input {
    values: BTreeMap<Arc<str>, f32>,
    pressed: BTreeSet<Arc<str>>,
    viewer: Vec3,
    away: bool,
}

impl Input {
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets an action's value. Setting it to 0, or to not-a-number, forgets it.
    pub fn set(&mut self, action: &str, value: f32) {
        if value == 0.0 || value.is_nan() {
            self.values.remove(action);
        } else {
            self.values.insert(Arc::from(action), value);
        }
    }

    /// Notes that an action was pressed, even if it has been released again since: for a tap
    /// between two frames, which the frames' values alone would miss.
    pub fn press(&mut self, action: &str) {
        self.pressed.insert(Arc::from(action));
    }

    /// An action's value, or 0.
    pub fn value(&self, action: &str) -> f32 {
        self.values.get(action).copied().unwrap_or(0.0)
    }

    /// Whether an action is held: its value is at least a half.
    pub fn held(&self, action: &str) -> bool {
        self.value(action) >= 0.5
    }

    /// Whether an action was pressed since the step before.
    pub fn pressed(&self, action: &str) -> bool {
        self.pressed.contains(action)
    }

    /// Says where the game is watched from: the camera, whoever moves it. A place that isn't
    /// finite is ignored.
    pub fn set_viewer(&mut self, position: Vec3) {
        if position.is_finite() {
            self.viewer = position;
        }
    }

    /// Where the game is watched from, or the origin if nobody said.
    pub fn viewer(&self) -> Vec3 {
        self.viewer
    }

    /// Says whether the player has left whoever they play, to fly the camera: the keys then
    /// move the camera, and must not walk the one left standing.
    pub fn set_away(&mut self, away: bool) {
        self.away = away;
    }

    /// Whether the player is away from whoever they play. The input is all still here, for
    /// everything else that listens: a door opens to a camera flown up to it.
    pub fn away(&self) -> bool {
        self.away
    }

    /// Every action that isn't 0, in name order.
    pub fn values(&self) -> impl Iterator<Item = (&str, f32)> {
        self.values.iter().map(|(name, &value)| (&**name, value))
    }

    /// Notes as pressed every action held here and not in `before`, and every action pressed
    /// here.
    pub(crate) fn presses_since(&self, before: &Input, into: &mut BTreeSet<Arc<str>>) {
        for (name, &value) in &self.values {
            if value >= 0.5 && !before.held(name) {
                into.insert(name.clone());
            }
        }
        into.extend(self.pressed.iter().cloned());
    }

    /// This input, with `pressed` as what was pressed.
    pub(crate) fn with_pressed(&self, pressed: &BTreeSet<Arc<str>>) -> Input {
        Input {
            values: self.values.clone(),
            pressed: pressed.clone(),
            viewer: self.viewer,
            away: self.away,
        }
    }
}

/// The input of every step of a run, in order, and the step it starts after. Runs of steps with
/// the same input are stored once, since input changes far less often than steps go by.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Recording {
    /// The step the world had reached when recording began. A replay must start from a world
    /// at that step, set up as the recorded one was.
    pub start: u64,
    /// Each input, with how many steps in a row it lasted.
    runs: Vec<(Input, u64)>,
}

impl Recording {
    pub fn new(start: u64) -> Self {
        Self {
            start,
            runs: Vec::new(),
        }
    }

    /// Adds the input of the next step.
    pub fn push(&mut self, input: &Input) {
        match self.runs.last_mut() {
            Some((last, count)) if last == input => *count += 1,
            _ => self.runs.push((input.clone(), 1)),
        }
    }

    /// How many steps it holds.
    pub fn len(&self) -> u64 {
        self.runs.iter().map(|(_, count)| count).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.runs.is_empty()
    }

    /// Each step's input, in order.
    pub fn steps(&self) -> impl Iterator<Item = &Input> {
        self.runs
            .iter()
            .flat_map(|(input, count)| std::iter::repeat_n(input, *count as usize))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn actions_not_set_are_zero_and_zero_is_forgotten() {
        let mut input = Input::new();
        assert_eq!(input.value("jump"), 0.0);
        input.set("jump", 1.0);
        input.set("throttle", 0.25);
        input.set("broken", f32::NAN);
        assert!(input.held("jump") && !input.held("throttle"));
        input.set("jump", 0.0);
        assert_eq!(input.values().collect::<Vec<_>>(), vec![("throttle", 0.25)]);
    }

    #[test]
    fn a_press_is_a_change_to_held_or_a_tap() {
        let (mut before, mut now) = (Input::new(), Input::new());
        before.set("fire", 1.0);
        now.set("fire", 1.0);
        now.set("jump", 1.0);
        now.set("crouch", 0.3);
        now.press("use");
        let mut pressed = BTreeSet::new();
        now.presses_since(&before, &mut pressed);
        let pressed: Vec<&str> = pressed.iter().map(|p| &**p).collect();
        assert_eq!(pressed, ["jump", "use"]);
    }

    #[test]
    fn the_viewer_is_part_of_a_steps_input() {
        let mut input = Input::new();
        assert_eq!(input.viewer(), Vec3::ZERO);
        input.set_viewer(Vec3::new(1.0, 2.0, 3.0));
        input.set_viewer(Vec3::new(f32::NAN, 0.0, 0.0));
        assert_eq!(input.viewer(), Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(
            input.with_pressed(&BTreeSet::new()).viewer(),
            input.viewer()
        );
        // So is whether the player is away, flying the camera.
        assert!(!input.away());
        input.set_away(true);
        assert!(input.with_pressed(&BTreeSet::new()).away());
        assert_ne!(input, Input::new());
        input.set_away(false);
        // A step watched from elsewhere is another input, so a recording keeps both.
        let mut recording = Recording::new(0);
        recording.push(&Input::new());
        recording.push(&input);
        assert_eq!(recording.runs.len(), 2);
    }

    #[test]
    fn a_recording_gives_back_every_step_in_order() {
        let (mut idle, mut walk) = (Input::new(), Input::new());
        idle.set("look", 0.1);
        walk.set("forward", 1.0);
        let steps = [&idle, &idle, &walk, &walk, &walk, &idle];
        let mut recording = Recording::new(3);
        for input in steps {
            recording.push(input);
        }
        assert_eq!(recording.len(), 6);
        // Three runs, not six inputs.
        assert_eq!(recording.runs.len(), 3);
        assert!(recording.steps().eq(steps));
    }
}
