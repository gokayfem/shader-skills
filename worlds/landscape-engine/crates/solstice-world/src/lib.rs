//! The gameplay framework (docs/gameplay.md): a world of entities with components, systems run
//! in tick groups, events and timers between them, and a fixed clock that makes every run
//! repeatable. It has no GPU, physics or scripting of its own; those come as systems added to
//! a [`Schedule`], each from its own crate.
//!
//! * [`World`]: entities and their components (on `hecs`), events, timers, and the step's input.
//! * [`Game`]: a world, its [`Schedule`] of systems, and the [`FixedClock`] that runs them.
//! * [`transform`]: each entity's place, relative to its parent and in the world.
//! * [`Input`] and [`Recording`]: actions, step by step, to replay a run exactly.

pub mod clock;
pub mod events;
pub mod game;
pub mod input;
pub mod name;
pub mod transform;
pub mod world;

pub use clock::{FixedClock, FrameSteps, MAX_STEPS_PER_FRAME, STEPS_PER_SECOND, STEP_SECONDS};
pub use events::{steps_for, TimerId};
pub use game::{Game, Schedule, TickGroup};
pub use hecs::{self, Entity};
pub use input::{Idle, Input, Played, Recording};
pub use name::Name;
pub use transform::{GlobalTransform, Parent, Snap, Transform, Walk};
pub use world::World;
