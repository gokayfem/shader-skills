//! Running a world: tick groups, the systems in them, and the frame.
//!
//! ```text
//!  each frame:
//!    0 to 3 fixed steps of 1/60 s, each:
//!      (start)       the step's input; timers due on it send their events
//!      PrePhysics    gameplay: script ticks, Rust systems, root motion
//!      Physics       the physics step
//!      PostPhysics   reactions to hits and overlaps
//!      (transforms)  every entity's place in the world
//!    Animation       poses, at the frame's own time
//!    Late            the camera
//! ```
//!
//! Everything that decides what happens runs in the fixed steps, from input recorded per step,
//! so a run replays exactly at any frame rate: exactly, that is, with the same build on the same
//! platform. Native and web builds may round some maths differently. The per-frame groups only
//! work out what is drawn (see [`World`] for what they may do).

use crate::clock::{FixedClock, FrameSteps};
use crate::input::{Input, Recording};
use crate::transform::update_global_transforms;
use crate::world::World;
use std::collections::BTreeSet;
use std::sync::Arc;

/// When in a frame a system runs. Within a group, systems run in the order they were added.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum TickGroup {
    /// Each step, before physics: gameplay decides what happens.
    PrePhysics,
    /// Each step: the physics step.
    Physics,
    /// Each step, after physics: reactions to what it found.
    PostPhysics,
    /// Each frame, after the steps: poses for drawing.
    Animation,
    /// Each frame, last: the camera, and anything that follows what was drawn.
    Late,
}

impl TickGroup {
    pub const ALL: [Self; 5] = [
        Self::PrePhysics,
        Self::Physics,
        Self::PostPhysics,
        Self::Animation,
        Self::Late,
    ];
    const STEP: [Self; 3] = [Self::PrePhysics, Self::Physics, Self::PostPhysics];
    const FRAME: [Self; 2] = [Self::Animation, Self::Late];

    /// Whether it runs in each fixed step, rather than each frame.
    pub fn is_fixed(self) -> bool {
        Self::STEP.contains(&self)
    }

    fn index(self) -> usize {
        self as usize
    }
}

/// A system: what it does, and its name, to list a schedule by.
struct System {
    name: &'static str,
    run: Box<dyn FnMut(&mut World)>,
}

/// The systems of each tick group, in order.
#[derive(Default)]
pub struct Schedule {
    groups: [Vec<System>; TickGroup::ALL.len()],
}

impl Schedule {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a system to run in `group`, after those already there.
    pub fn add(
        &mut self,
        group: TickGroup,
        name: &'static str,
        run: impl FnMut(&mut World) + 'static,
    ) {
        self.groups[group.index()].push(System {
            name,
            run: Box::new(run),
        });
    }

    /// The names of a group's systems, in the order they run.
    pub fn names(&self, group: TickGroup) -> impl Iterator<Item = &'static str> + '_ {
        self.groups[group.index()].iter().map(|system| system.name)
    }

    fn run(&mut self, group: TickGroup, world: &mut World) {
        for system in &mut self.groups[group.index()] {
            (system.run)(world);
        }
    }
}

/// A world, its systems and its clock.
pub struct Game {
    pub world: World,
    pub schedule: Schedule,
    clock: FixedClock,
    /// Every step's input, while recording.
    recording: Option<Recording>,
    /// The latest frame's input, to tell what was pressed since.
    last_frame_input: Input,
    /// Actions pressed since the last step, for the next to see.
    presses: BTreeSet<Arc<str>>,
}

impl Game {
    pub fn new(world: World, schedule: Schedule) -> Self {
        Self {
            world,
            schedule,
            clock: FixedClock::new(),
            recording: None,
            last_frame_input: Input::new(),
            presses: BTreeSet::new(),
        }
    }

    /// Runs a frame `seconds` long, with `input` as the player's input during it: the fixed
    /// steps the frame holds, then the per-frame groups. Returns how many steps ran, and how far
    /// the frame is between the last step and the next, for drawing.
    ///
    /// Every step sees the frame's values. Actions pressed since the last step (held now and not
    /// in the frame before, or [`Input::press`]ed) are seen by the next step only, even if the
    /// frames between ran no step.
    pub fn frame(&mut self, seconds: f64, input: &Input) -> FrameSteps {
        input.presses_since(&self.last_frame_input, &mut self.presses);
        self.last_frame_input.clone_from(input);
        let steps = self.clock.advance(seconds);
        for _ in 0..steps.steps {
            let step_input = input.with_pressed(&self.presses);
            self.presses.clear();
            self.step(&step_input);
        }
        self.world.begin_frame_groups(input);
        for group in TickGroup::FRAME {
            self.schedule.run(group, &mut self.world);
        }
        self.world.end_frame_groups();
        steps
    }

    /// Runs one fixed step with `input`, as the step's input exactly.
    pub fn step(&mut self, input: &Input) {
        if let Some(recording) = &mut self.recording {
            recording.push(input);
        }
        self.world.begin_step(input);
        for (index, group) in TickGroup::STEP.into_iter().enumerate() {
            self.world.begin_step_group(index as u8);
            self.schedule.run(group, &mut self.world);
            self.world.end_step_group();
        }
        self.world.end_step();
        let world = &mut self.world;
        update_global_transforms(&mut world.entities, &mut world.propagation);
    }

    /// Starts recording every step's input, from the next step on.
    pub fn record(&mut self) {
        self.recording = Some(Recording::new(self.world.step()));
    }

    /// Stops recording, and gives back what was recorded.
    pub fn take_recording(&mut self) -> Option<Recording> {
        self.recording.take()
    }

    /// Runs the steps of a recording, one per input, with no frames between them. The world
    /// must be at the step the recording started from, set up as the recorded one was.
    pub fn replay(&mut self, recording: &Recording) {
        assert_eq!(
            self.world.step(),
            recording.start,
            "a recording replays from the step it started at"
        );
        for input in recording.steps() {
            self.step(input);
        }
    }
}
