//! The engine, as scripts see it: the functions every script can call.
//!
//! This one module is the engine's whole API for scripts. Starlark's own tools read it, with
//! its doc comments, for editors and for [`crate::host::api_reference`].
//!
//! A handler only reads the world while it runs. What it asks of the world (events to send,
//! timers to set or cancel) is held back, and done only if the handler succeeds, so a failing
//! handler changes nothing. Its log lines are kept either way: they are how one finds out why
//! it failed.

use crate::host::{ScriptEvent, RESERVED_EVENTS};
use crate::value::{number, EntityValue, ScriptValue, Vec3Value};
use solstice_world::{Entity, Idle, Input, Name, Played, TimerId, World};
use starlark::any::ProvidesStaticType;
use starlark::environment::GlobalsBuilder;
use starlark::eval::Evaluator;
use starlark::starlark_module;
use starlark::values::list::AllocList;
use starlark::values::none::NoneType;
use starlark::values::tuple::UnpackTuple;
use starlark::values::{Value, ValueLike};
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

/// The most lines one call may log, and the longest a line may be. More are dropped.
pub(crate) const MAX_LOG_LINES: usize = 100;
pub(crate) const MAX_LOG_CHARS: usize = 1000;

/// What a handler's calls into the engine reach: the world to read, the actor it runs for, and
/// what it has asked for so far.
#[derive(ProvidesStaticType)]
pub(crate) struct Context<'w> {
    pub world: &'w World,
    pub actor: Entity,
    pub timers: &'w Timers,
    pub effects: RefCell<Effects>,
}

/// What a script told an actor's animation to do, carried out in the step it was made.
#[derive(Clone, Debug, PartialEq)]
pub struct AnimCommand {
    pub actor: Entity,
    /// A parameter to set, and its value.
    pub param: Option<(String, f32)>,
    /// A state to fade to, and over how many seconds.
    pub state: Option<(String, f32)>,
}

/// What a handler asked of the world, to do if it succeeds.
#[derive(Default)]
pub(crate) struct Effects {
    pub sends: Vec<ScriptEvent>,
    pub animation: Vec<AnimCommand>,
    pub timers: Vec<TimerChange>,
    pub log: Vec<String>,
    /// Lines past [`MAX_LOG_LINES`], dropped.
    pub dropped_lines: usize,
}

pub(crate) enum TimerChange {
    Set {
        handle: i64,
        seconds: f64,
        repeat: bool,
        event: ScriptEvent,
    },
    Cancel(i64),
}

/// Timers scripts have set, by the handle they were given. Handles are an actor's own: one
/// actor can't cancel another's timers.
#[derive(Default)]
pub(crate) struct Timers {
    pub next: i64,
    pub set: HashMap<i64, ScriptTimer>,
}

pub(crate) struct ScriptTimer {
    pub id: TimerId,
    pub owner: Entity,
    /// The step a timer that fires once has fired by, after which its handle is forgotten.
    pub done_by: Option<u64>,
}

fn context<'a, 'e>(eval: &Evaluator<'_, 'a, 'e>) -> anyhow::Result<&'a Context<'e>> {
    eval.extra
        .and_then(|extra| extra.downcast_ref::<Context>())
        .ok_or_else(|| anyhow::anyhow!("engine functions can only be called from a handler"))
}

/// The step's input, as the handler's actor hears it: none of it for the one the player plays,
/// while they are away flying the camera, so the keys that fly it don't walk who was left.
fn heard<'a>(context: &'a Context) -> Option<&'a Input> {
    let input = context.world.input();
    let entities = &context.world.entities;
    // Nor for one the player could play and is not playing: the driver while they drive.
    let left = entities.get::<&Idle>(context.actor).is_ok()
        || (input.away() && entities.get::<&Played>(context.actor).is_ok());
    (!left).then_some(input)
}

/// Builds a script event from a name and Starlark arguments.
fn event(to: Option<Entity>, name: &str, args: &[Value]) -> anyhow::Result<ScriptEvent> {
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        anyhow::bail!("event names are made of letters, digits and _, not `{name}`");
    }
    if RESERVED_EVENTS.contains(&name) {
        anyhow::bail!("`{name}` is the engine's own, and can't be sent");
    }
    let args = args
        .iter()
        .map(|&arg| ScriptValue::from_starlark(arg).map_err(anyhow::Error::msg))
        .collect::<anyhow::Result<_>>()?;
    Ok(ScriptEvent {
        to,
        name: Arc::from(name),
        args,
    })
}

/// Holds back a timer for the handler's actor, and gives its handle.
fn set_timer<'v>(
    seconds: Value<'v>,
    name: &str,
    args: &[Value<'v>],
    repeat: bool,
    eval: &mut Evaluator<'v, '_, '_>,
) -> anyhow::Result<i64> {
    let seconds = number(seconds)
        .ok_or_else(|| anyhow::anyhow!("a timer's seconds must be a number, not {seconds}"))?;
    let context = context(eval)?;
    let event = event(Some(context.actor), name, args)?;
    let mut effects = context.effects.borrow_mut();
    // Handles count on from those already given, and are only taken if the handler succeeds.
    let handle = context.timers.next
        + 1
        + effects
            .timers
            .iter()
            .filter(|t| matches!(t, TimerChange::Set { .. }))
            .count() as i64;
    effects.timers.push(TimerChange::Set {
        handle,
        seconds,
        repeat,
        event,
    });
    Ok(handle)
}

fn angle(value: Value) -> anyhow::Result<f64> {
    number(value).ok_or_else(|| anyhow::anyhow!("an angle must be a number, not {value}"))
}

#[starlark_module]
pub(crate) fn engine(builder: &mut GlobalsBuilder) {
    /// A vector: `vec3(1, 0, 0)`.
    fn vec3<'v>(x: Value<'v>, y: Value<'v>, z: Value<'v>) -> anyhow::Result<Vec3Value> {
        let [x, y, z] = [x, y, z].map(number);
        match (x, y, z) {
            (Some(x), Some(y), Some(z)) => {
                Ok(Vec3Value(glam::Vec3::new(x as f32, y as f32, z as f32)))
            }
            _ => anyhow::bail!("vec3 takes three numbers"),
        }
    }

    /// The sine of an angle in radians. Starlark has no maths module, so the engine gives the
    /// few functions games need.
    fn sin<'v>(radians: Value<'v>) -> anyhow::Result<f64> {
        Ok(angle(radians)?.sin())
    }

    /// The cosine of an angle in radians.
    fn cos<'v>(radians: Value<'v>) -> anyhow::Result<f64> {
        Ok(angle(radians)?.cos())
    }

    /// The angle, in radians, from the +x axis to the point (x, y): `atan2(y, x)`.
    fn atan2<'v>(y: Value<'v>, x: Value<'v>) -> anyhow::Result<f64> {
        match (number(y), number(x)) {
            (Some(y), Some(x)) => Ok(y.atan2(x)),
            _ => anyhow::bail!("atan2 takes two numbers"),
        }
    }

    /// Sets a parameter of this actor's animation, which its state machine's transitions and
    /// blends read: `anim_set("speed", 1.4)`.
    fn anim_set<'v>(
        param: &str,
        value: Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<NoneType> {
        let value = number(value)
            .ok_or_else(|| anyhow::anyhow!("an animation parameter is a number, not {value}"))?;
        let context = context(eval)?;
        context.effects.borrow_mut().animation.push(AnimCommand {
            actor: context.actor,
            param: Some((param.to_owned(), value as f32)),
            state: None,
        });
        Ok(NoneType)
    }

    /// Fades this actor's animation to a state now, whatever its transitions say, over `fade`
    /// seconds: `anim_play("sit", 0.4)`.
    fn anim_play<'v>(
        state: &str,
        fade: Option<Value<'v>>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<NoneType> {
        let fade = fade
            .map_or(Some(0.2), number)
            .ok_or_else(|| anyhow::anyhow!("a fade is a number of seconds"))?;
        let context = context(eval)?;
        context.effects.borrow_mut().animation.push(AnimCommand {
            actor: context.actor,
            param: None,
            state: Some((state.to_owned(), fade as f32)),
        });
        Ok(NoneType)
    }

    /// The square root of a number, which must not be negative.
    fn sqrt<'v>(value: Value<'v>) -> anyhow::Result<f64> {
        match number(value) {
            Some(n) if n >= 0.0 => Ok(n.sqrt()),
            _ => anyhow::bail!("sqrt takes a number that isn't negative, not {value}"),
        }
    }

    /// Sends an event to an entity, which its script handles in `on_<name>(self, ...)`, or to
    /// every actor whose script handles it, if `to` is `None`. Handlers run next step.
    fn send<'v>(
        to: Value<'v>,
        name: &str,
        #[starlark(args)] args: UnpackTuple<Value<'v>>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<NoneType> {
        let to = if to.is_none() {
            None
        } else {
            Some(
                to.downcast_ref::<EntityValue>()
                    .ok_or_else(|| anyhow::anyhow!("send to an entity or None, not {to}"))?
                    .0,
            )
        };
        let event = event(to, name, &args.items)?;
        context(eval)?.effects.borrow_mut().sends.push(event);
        Ok(NoneType)
    }

    /// Sends an event to this actor after `seconds`, rounded up to whole steps. Returns a
    /// handle to cancel it.
    fn after<'v>(
        seconds: Value<'v>,
        name: &str,
        #[starlark(args)] args: UnpackTuple<Value<'v>>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<i64> {
        set_timer(seconds, name, &args.items, false, eval)
    }

    /// Sends an event to this actor every `seconds`, rounded up to whole steps, until
    /// cancelled. Returns a handle to cancel it.
    fn every<'v>(
        seconds: Value<'v>,
        name: &str,
        #[starlark(args)] args: UnpackTuple<Value<'v>>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<i64> {
        set_timer(seconds, name, &args.items, true, eval)
    }

    /// Stops one of this actor's timers, set by `after` or `every`. Returns whether it is one
    /// of its own.
    fn cancel(timer: i64, eval: &mut Evaluator) -> anyhow::Result<bool> {
        let context = context(eval)?;
        let mut effects = context.effects.borrow_mut();
        let own = context
            .timers
            .set
            .get(&timer)
            .is_some_and(|t| t.owner == context.actor)
            || effects
                .timers
                .iter()
                .any(|t| matches!(t, TimerChange::Set { handle, .. } if *handle == timer));
        if own {
            effects.timers.push(TimerChange::Cancel(timer));
        }
        Ok(own)
    }

    /// Every entity with this name, in the world's order.
    fn find<'v>(name: &str, eval: &mut Evaluator<'v, '_, '_>) -> anyhow::Result<Value<'v>> {
        let heap = eval.heap();
        let context = context(eval)?;
        let found: Vec<Entity> = context
            .world
            .entities
            .query::<(Entity, &Name)>()
            .iter()
            .filter(|(_, n)| &*n.0 == name)
            .map(|(entity, _)| entity)
            .collect();
        Ok(heap.alloc(AllocList(found.into_iter().map(EntityValue))))
    }

    /// An input action's value this step: 1 held, 0 released, between for a stick. The actor
    /// the player plays hears no input while the player is away from it, flying the camera.
    fn input(action: &str, eval: &mut Evaluator) -> anyhow::Result<f64> {
        Ok(heard(context(eval)?).map_or(0.0, |input| f64::from(input.value(action))))
    }

    /// Whether an input action is held this step.
    fn held(action: &str, eval: &mut Evaluator) -> anyhow::Result<bool> {
        Ok(heard(context(eval)?).is_some_and(|input| input.held(action)))
    }

    /// Whether an input action was pressed since the step before.
    fn pressed(action: &str, eval: &mut Evaluator) -> anyhow::Result<bool> {
        Ok(heard(context(eval)?).is_some_and(|input| input.pressed(action)))
    }

    /// Where the game is watched from this step: the camera, whether the player's or one flown
    /// freely. `viewer() - self.position` is how far off it is, and which way.
    fn viewer(eval: &mut Evaluator) -> anyhow::Result<Vec3Value> {
        Ok(Vec3Value(context(eval)?.world.input().viewer()))
    }

    /// Simulated seconds since the game started.
    fn now(eval: &mut Evaluator) -> anyhow::Result<f64> {
        Ok(context(eval)?.world.seconds())
    }

    /// Writes to the game's log, with the step and the actor. A call keeps its first hundred
    /// lines, of up to a thousand characters each.
    fn log<'v>(
        #[starlark(args)] args: UnpackTuple<Value<'v>>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<NoneType> {
        let context = context(eval)?;
        let mut effects = context.effects.borrow_mut();
        if effects.log.len() >= MAX_LOG_LINES {
            effects.dropped_lines += 1;
            return Ok(NoneType);
        }
        let mut text = String::new();
        for (i, value) in args.items.iter().enumerate() {
            if i > 0 {
                text.push(' ');
            }
            match value.unpack_str() {
                Some(s) => text.push_str(s),
                None => text.push_str(&value.to_str()),
            }
            if text.len() > MAX_LOG_CHARS {
                break;
            }
        }
        if text.len() > MAX_LOG_CHARS {
            let mut end = MAX_LOG_CHARS;
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            text.truncate(end);
            text.push('…');
        }
        effects.log.push(text);
        Ok(NoneType)
    }
}
