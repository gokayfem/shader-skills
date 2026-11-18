//! `self`: the actor a handler is called for, as the script sees it.
//!
//! A handler works on a copy of the actor, made on the call's heap: its properties, its
//! `self.vars`, and where it stands. When the handler returns, the copy is written back to the
//! world. When it fails, nothing is: a failing handler changes nothing, so an actor is never
//! left half-changed.

use crate::value::{EntityValue, ScriptValue, Vec3Value};
use allocative::Allocative;
use glam::Vec3;
use solstice_world::Entity;
use starlark::any::ProvidesStaticType;
use starlark::collections::SmallMap;
use starlark::values::float::StarlarkFloat;
use starlark::values::{Heap, NoSerialize, StarlarkValue, Trace, Value, ValueLike};
use starlark_derive::starlark_value;
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::fmt;

/// An actor's own values: `self.vars.name`. Any name can be set; reading one never set is an
/// error, as for any missing attribute.
#[derive(Debug, Trace, ProvidesStaticType, NoSerialize, Allocative)]
pub struct VarsValue<'v>(RefCell<SmallMap<String, Value<'v>>>);

impl fmt::Display for VarsValue<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "vars(")?;
        for (i, (name, value)) in self.0.borrow().iter().enumerate() {
            write!(f, "{}{name} = {value}", if i == 0 { "" } else { ", " })?;
        }
        write!(f, ")")
    }
}

#[starlark_value(type = "vars")]
impl<'v> StarlarkValue<'v> for VarsValue<'v> {
    fn get_attr(&self, attribute: &str, _heap: Heap<'v>) -> Option<Value<'v>> {
        self.0.borrow().get(attribute).copied()
    }

    fn has_attr(&self, attribute: &str, _heap: Heap<'v>) -> bool {
        self.0.borrow().contains_key(attribute)
    }

    fn dir_attr(&self) -> Vec<String> {
        self.0.borrow().keys().cloned().collect()
    }

    fn set_attr(&self, attribute: &str, new_value: Value<'v>) -> starlark::Result<()> {
        self.0.borrow_mut().insert(attribute.to_owned(), new_value);
        Ok(())
    }
}

/// The actor a handler is called for.
#[derive(Debug, Trace, ProvidesStaticType, NoSerialize, Allocative)]
pub struct ActorValue<'v> {
    #[allocative(skip)]
    #[trace(static)]
    entity: Entity,
    /// The properties its actor file declares: they can be read and set, but not added.
    properties: RefCell<SmallMap<String, Value<'v>>>,
    vars: Value<'v>,
    #[allocative(skip)]
    #[trace(static)]
    position: Cell<Vec3>,
    #[allocative(skip)]
    #[trace(static)]
    yaw: Cell<f32>,
    /// How the handler wants a walking actor to move, if it said.
    #[allocative(skip)]
    #[trace(static)]
    walk: Cell<Option<Vec3>>,
    /// How it is walking already, which `self.walk` reads until the handler sets it.
    #[allocative(skip)]
    #[trace(static)]
    walking: Vec3,
}

impl fmt::Display for ActorValue<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "actor({})", self.entity.id())
    }
}

/// What a handler left an actor as, to write back to the world.
pub struct ActorState {
    pub properties: BTreeMap<String, ScriptValue>,
    pub vars: BTreeMap<String, ScriptValue>,
    pub position: Vec3,
    pub yaw: f32,
    /// How it should walk, if the handler said.
    pub walk: Option<Vec3>,
}

impl<'v> ActorValue<'v> {
    /// The script's view of an actor, on `heap`.
    pub fn alloc(
        heap: Heap<'v>,
        entity: Entity,
        properties: &BTreeMap<String, ScriptValue>,
        vars: &BTreeMap<String, ScriptValue>,
        position: Vec3,
        yaw: f32,
        walking: Vec3,
    ) -> Value<'v> {
        let to_map = |values: &BTreeMap<String, ScriptValue>| {
            values
                .iter()
                .map(|(name, value)| (name.clone(), value.to_starlark(heap)))
                .collect::<SmallMap<_, _>>()
        };
        let vars = heap.alloc_complex_no_freeze(VarsValue(RefCell::new(to_map(vars))));
        heap.alloc_complex_no_freeze(ActorValue {
            entity,
            properties: RefCell::new(to_map(properties)),
            vars,
            position: Cell::new(position),
            yaw: Cell::new(yaw),
            walk: Cell::new(None),
            walking,
        })
    }

    /// What the handler left the actor as. Fails if it stored something that can't be kept.
    pub fn state(value: Value<'v>) -> Result<ActorState, String> {
        let actor = value
            .downcast_ref::<ActorValue>()
            .expect("an actor value made by `alloc`");
        let vars = actor
            .vars
            .downcast_ref::<VarsValue>()
            .expect("an actor's vars are a vars value");
        let to_map = |values: &SmallMap<String, Value<'v>>, kind: &str| {
            values
                .iter()
                .map(|(name, value)| {
                    ScriptValue::from_starlark(*value)
                        .map(|v| (name.clone(), v))
                        .map_err(|e| format!("{kind}{name}: {e}"))
                })
                .collect::<Result<BTreeMap<_, _>, _>>()
        };
        Ok(ActorState {
            properties: to_map(&actor.properties.borrow(), "self.")?,
            vars: to_map(&vars.0.borrow(), "self.vars.")?,
            position: actor.position.get(),
            yaw: actor.yaw.get(),
            walk: actor.walk.get(),
        })
    }
}

#[starlark_value(type = "actor")]
impl<'v> StarlarkValue<'v> for ActorValue<'v> {
    fn get_attr(&self, attribute: &str, heap: Heap<'v>) -> Option<Value<'v>> {
        match attribute {
            "vars" => Some(self.vars),
            "entity" => Some(heap.alloc(EntityValue(self.entity))),
            "position" => Some(heap.alloc(Vec3Value(self.position.get()))),
            "yaw" => Some(heap.alloc(StarlarkFloat(f64::from(self.yaw.get())))),
            "walk" => Some(heap.alloc(Vec3Value(self.walk.get().unwrap_or(self.walking)))),
            _ => self.properties.borrow().get(attribute).copied(),
        }
    }

    fn has_attr(&self, attribute: &str, _heap: Heap<'v>) -> bool {
        matches!(attribute, "vars" | "entity" | "position" | "yaw" | "walk")
            || self.properties.borrow().contains_key(attribute)
    }

    fn dir_attr(&self) -> Vec<String> {
        let mut names: Vec<String> = ["vars", "entity", "position", "yaw", "walk"]
            .map(String::from)
            .to_vec();
        names.extend(self.properties.borrow().keys().cloned());
        names
    }

    fn set_attr(&self, attribute: &str, new_value: Value<'v>) -> starlark::Result<()> {
        match attribute {
            "position" => {
                let Some(position) = new_value.downcast_ref::<Vec3Value>() else {
                    return Err(
                        anyhow::anyhow!("self.position must be a vec3, not {new_value}").into(),
                    );
                };
                self.position.set(position.0);
            }
            "yaw" => {
                let Some(yaw) = crate::value::number(new_value).map(|n| n as f32) else {
                    return Err(
                        anyhow::anyhow!("self.yaw must be a number, not {new_value}").into(),
                    );
                };
                self.yaw.set(yaw);
            }
            "walk" => {
                let Some(walk) = new_value.downcast_ref::<Vec3Value>() else {
                    return Err(anyhow::anyhow!("self.walk must be a vec3, not {new_value}").into());
                };
                self.walk.set(Some(walk.0));
            }
            "vars" | "entity" => {
                return Err(anyhow::anyhow!("self.{attribute} can't be replaced").into());
            }
            _ => {
                let mut properties = self.properties.borrow_mut();
                let Some(slot) = properties.get_mut(attribute) else {
                    return Err(anyhow::anyhow!(
                        "the actor has no property `{attribute}`: declare it, or use self.vars.{attribute}"
                    )
                    .into());
                };
                *slot = new_value;
            }
        }
        Ok(())
    }
}
