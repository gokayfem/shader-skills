//! The world: every entity and its components, with the events, timers and input of the step
//! being run.

use crate::clock::STEP_SECONDS;
use crate::events::{steps_for, Events, Reader, Stamp, TimerId, Timers};
use crate::input::Input;
use crate::transform::Propagation;
use hecs::{CommandBuffer, Entity};

/// Everything gameplay works on.
///
/// Systems read and change components through [`Self::entities`] directly. Spawning, despawning,
/// and adding or removing components while a tick group runs go through [`Self::commands`]
/// instead, and happen when the group ends, so no system sees the world change shape under it.
///
/// Components are `Send + Sync`, as `hecs` requires. Values that can't be, such as a script's
/// own values, are converted into ones that can before they are stored.
///
/// **What the per-frame groups may do.** `Animation` and `Late` run as often as frames come,
/// which differs from run to run, and a replay doesn't run them at all. So they must not change
/// anything the fixed steps read: they send no events, set or cancel no timers, and spawn or
/// despawn nothing, and those are checked. They may write only components that exist for drawing
/// (poses, the camera), which nothing in the fixed steps reads; that is up to them.
pub struct World {
    /// Every entity and its components.
    pub entities: hecs::World,
    events: Events,
    timers: Timers,
    commands: CommandBuffer,
    /// Fixed steps begun so far.
    step: u64,
    /// Where in the step things happen now: 0 before its first group, then the groups' places.
    place: u8,
    input: Input,
    /// The latest frame's input, for the per-frame groups.
    frame_input: Input,
    /// Whether a per-frame tick group is running.
    in_frame_group: bool,
    /// The last event the per-frame groups have seen.
    frame_seen: Stamp,
    pub(crate) propagation: Propagation,
}

impl Default for World {
    fn default() -> Self {
        Self::new()
    }
}

impl World {
    pub fn new() -> Self {
        Self {
            entities: hecs::World::new(),
            events: Events::default(),
            timers: Timers::default(),
            commands: CommandBuffer::new(),
            step: 0,
            place: 0,
            input: Input::new(),
            frame_input: Input::new(),
            in_frame_group: false,
            frame_seen: Stamp { step: 0, place: 0 },
            propagation: Propagation::default(),
        }
    }

    fn not_in_frame_group(&self, what: &str) {
        assert!(!self.in_frame_group, "per-frame tick groups don't {what}");
    }

    /// Spawns an entity at once. While a tick group runs, use [`Self::spawn_later`] or
    /// [`Self::commands`].
    pub fn spawn(&mut self, components: impl hecs::DynamicBundle) -> Entity {
        self.not_in_frame_group("spawn");
        self.entities.spawn(components)
    }

    /// Spawns an entity when the current tick group ends, and names it now, so it can be given
    /// to others straight away.
    pub fn spawn_later(&mut self, components: impl hecs::DynamicBundle) -> Entity {
        self.not_in_frame_group("spawn");
        let entity = self.entities.reserve_entity();
        self.commands.insert(entity, components);
        entity
    }

    /// Changes to make when the current tick group ends: spawning, despawning, and adding or
    /// removing components.
    pub fn commands(&mut self) -> &mut CommandBuffer {
        self.not_in_frame_group("change the world's shape");
        &mut self.commands
    }

    /// Sends an event. Every fixed-step group sees it once, within a step: those after this one
    /// in this step, and the rest in the next. The per-frame groups see it after the frame's
    /// steps. An event sent before a step starts is seen by all of that step's groups.
    pub fn send<E: 'static>(&mut self, event: E) {
        self.not_in_frame_group("send events");
        // Outside a step (setting up, before the first), it counts as sent before the next.
        let stamp = if self.place == 0 {
            Stamp {
                step: self.step + 1,
                place: 0,
            }
        } else {
            Stamp {
                step: self.step,
                place: self.place,
            }
        };
        self.events.send(stamp, event);
    }

    /// The events of one type for the group running now to see, in the order they were sent.
    pub fn read<E: 'static>(&self) -> impl Iterator<Item = &E> {
        let reader = if self.in_frame_group {
            Reader::Frame {
                since: self.frame_seen,
            }
        } else {
            Reader::Step(Stamp {
                step: self.step,
                place: self.place,
            })
        };
        self.events.read(reader)
    }

    /// The same, copied, so that the world can be changed while going through them.
    pub fn read_cloned<E: Clone + 'static>(&self) -> Vec<E> {
        self.read().cloned().collect()
    }

    /// Sends `event` after `seconds`, rounded up to whole steps, as the step it falls on starts.
    pub fn after<E: Clone + 'static>(&mut self, seconds: f64, event: E) -> TimerId {
        self.not_in_frame_group("set timers");
        self.timers.after(self.step, steps_for(seconds), event)
    }

    /// Sends `event` every `seconds`, rounded up to whole steps, until cancelled.
    pub fn every<E: Clone + 'static>(&mut self, seconds: f64, event: E) -> TimerId {
        self.not_in_frame_group("set timers");
        self.timers.every(self.step, steps_for(seconds), event)
    }

    /// Stops a timer. Returns whether it was still waiting.
    pub fn cancel(&mut self, timer: TimerId) -> bool {
        self.not_in_frame_group("cancel timers");
        self.timers.cancel(timer)
    }

    /// The input of the step being run. The per-frame groups see the last step's; for the
    /// frame's own, such as a camera's look, see [`Self::frame_input`].
    pub fn input(&self) -> &Input {
        &self.input
    }

    /// The input of the latest frame, which changes even when a frame runs no step: for the
    /// per-frame groups, such as a camera following the mouse. The fixed steps must not read it.
    pub fn frame_input(&self) -> &Input {
        &self.frame_input
    }

    /// Fixed steps begun so far.
    pub fn step(&self) -> u64 {
        self.step
    }

    /// Simulated seconds so far: the steps run, times the step's length.
    pub fn seconds(&self) -> f64 {
        self.step as f64 * STEP_SECONDS
    }

    /// Starts a step: takes its input, and fires the timers due on it.
    pub(crate) fn begin_step(&mut self, input: &Input) {
        self.step += 1;
        self.place = 0;
        self.input.clone_from(input);
        self.events.prune(self.step, self.frame_seen);
        self.timers.fire(self.step, &mut self.events);
    }

    /// Starts a fixed-step tick group, the `index`th of the step.
    pub(crate) fn begin_step_group(&mut self, index: u8) {
        self.place = index + 1;
    }

    /// Ends a fixed-step tick group: applies its commands.
    pub(crate) fn end_step_group(&mut self) {
        self.commands.run_on(&mut self.entities);
    }

    /// Ends a step: what happens until the next is outside one.
    pub(crate) fn end_step(&mut self) {
        self.place = 0;
    }

    /// Starts the per-frame groups, with the frame's input.
    pub(crate) fn begin_frame_groups(&mut self, frame_input: &Input) {
        self.frame_input.clone_from(frame_input);
        self.in_frame_group = true;
    }

    /// Ends the per-frame groups: what they have seen is now everything sent so far.
    pub(crate) fn end_frame_groups(&mut self) {
        self.in_frame_group = false;
        self.frame_seen = Stamp {
            step: self.step,
            place: u8::MAX,
        };
    }
}
