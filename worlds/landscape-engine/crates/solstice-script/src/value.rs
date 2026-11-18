//! Values that outlive a script call, and the Starlark types for vectors and entities.
//!
//! Starlark values live on a heap that is dropped when a step's script calls end. Anything a
//! script keeps, in a property or in `self.vars`, or sends with an event, is stored as a
//! [`ScriptValue`] instead: plain data, `Send + Sync`, which the world can hold as a component.

use allocative::Allocative;
use glam::Vec3;
use solstice_world::Entity;
use starlark::any::ProvidesStaticType;
use starlark::collections::StarlarkHasher;
use starlark::starlark_simple_value;
use starlark::values::dict::{AllocDict, DictRef};
use starlark::values::float::StarlarkFloat;
use starlark::values::list::{AllocList, ListRef};
use starlark::values::tuple::TupleRef;
use starlark::values::{Heap, NoSerialize, StarlarkValue, Value, ValueLike};
use starlark_derive::starlark_value;
use std::fmt;
use std::hash::Hasher;
use std::sync::Arc;

/// A value a script keeps from one step to the next.
#[derive(Clone, Debug, PartialEq)]
pub enum ScriptValue {
    None,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(Arc<str>),
    /// Kept in single precision, as places are: a kept `0.1` comes back a little off.
    Vec3(Vec3),
    Entity(Entity),
    List(Vec<ScriptValue>),
    /// A dict with string keys, in the order its keys were added, as Starlark keeps them.
    Dict(Vec<(String, ScriptValue)>),
}

/// How deeply a kept value may nest lists and dicts. Deeper, or one that contains itself, is
/// refused rather than followed until the stack runs out.
pub const MAX_DEPTH: u32 = 64;
/// How many values one kept value may hold in all, counting everything nested.
pub const MAX_VALUES: u32 = 100_000;

impl ScriptValue {
    /// The value on a Starlark heap.
    pub fn to_starlark<'v>(&self, heap: Heap<'v>) -> Value<'v> {
        match self {
            Self::None => Value::new_none(),
            Self::Bool(b) => Value::new_bool(*b),
            Self::Int(i) => heap.alloc(*i),
            Self::Float(f) => heap.alloc(StarlarkFloat(*f)),
            Self::Str(s) => heap.alloc(&**s),
            Self::Vec3(v) => heap.alloc(Vec3Value(*v)),
            Self::Entity(e) => heap.alloc(EntityValue(*e)),
            Self::List(items) => heap.alloc(AllocList(items.iter().map(|i| i.to_starlark(heap)))),
            Self::Dict(entries) => heap.alloc(AllocDict(
                entries
                    .iter()
                    .map(|(k, v)| (heap.alloc(k.as_str()), v.to_starlark(heap))),
            )),
        }
    }

    /// The value a Starlark value holds, if it is one a script can keep: none, a bool, an int
    /// that fits 64 bits, a float, a string, a vector, an entity, or a list, tuple or dict of
    /// those, with string keys, nested at most [`MAX_DEPTH`] deep and holding at most
    /// [`MAX_VALUES`] in all.
    pub fn from_starlark(value: Value) -> Result<Self, String> {
        let mut budget = MAX_VALUES;
        Self::convert(value, 0, &mut budget)
    }

    fn convert(value: Value, depth: u32, budget: &mut u32) -> Result<Self, String> {
        if depth > MAX_DEPTH {
            return Err(format!(
                "a kept value can nest at most {MAX_DEPTH} deep, and can't contain itself"
            ));
        }
        if *budget == 0 {
            return Err(format!("a kept value can hold at most {MAX_VALUES} values"));
        }
        *budget -= 1;
        if value.is_none() {
            return Ok(Self::None);
        }
        if let Some(b) = value.unpack_bool() {
            return Ok(Self::Bool(b));
        }
        if let Some(i) = value.unpack_i32() {
            return Ok(Self::Int(i64::from(i)));
        }
        if let Some(f) = value.downcast_ref::<StarlarkFloat>() {
            return Ok(Self::Float(f.0));
        }
        if let Some(s) = value.unpack_str() {
            return Ok(Self::Str(Arc::from(s)));
        }
        if let Some(v) = value.downcast_ref::<Vec3Value>() {
            return Ok(Self::Vec3(v.0));
        }
        if let Some(e) = value.downcast_ref::<EntityValue>() {
            return Ok(Self::Entity(e.0));
        }
        let items = ListRef::from_value(value)
            .map(|list| list.content())
            .or_else(|| TupleRef::from_value(value).map(|tuple| tuple.content()));
        if let Some(items) = items {
            return items
                .iter()
                .map(|&item| Self::convert(item, depth + 1, budget))
                .collect::<Result<_, _>>()
                .map(Self::List);
        }
        if let Some(dict) = DictRef::from_value(value) {
            let mut entries = Vec::with_capacity(dict.len());
            for (key, item) in dict.iter() {
                let key = key.unpack_str().ok_or_else(|| {
                    format!("a dict kept by a script needs string keys, not {key}")
                })?;
                entries.push((key.to_owned(), Self::convert(item, depth + 1, budget)?));
            }
            return Ok(Self::Dict(entries));
        }
        // An int too large for 32 bits but not for 64.
        if value.get_type() == "int" {
            if let Ok(i) = value.to_str().parse::<i64>() {
                return Ok(Self::Int(i));
            }
            return Err("an int kept by a script must fit in 64 bits".to_owned());
        }
        Err(format!(
            "{} can't be kept from one step to the next",
            with_article(value.get_type())
        ))
    }
}

/// "a list", "an int".
fn with_article(noun: &str) -> String {
    let article = if noun.starts_with(['a', 'e', 'i', 'o', 'u']) {
        "an"
    } else {
        "a"
    };
    format!("{article} {noun}")
}

impl From<f64> for ScriptValue {
    fn from(value: f64) -> Self {
        Self::Float(value)
    }
}

impl From<i64> for ScriptValue {
    fn from(value: i64) -> Self {
        Self::Int(value)
    }
}

impl From<bool> for ScriptValue {
    fn from(value: bool) -> Self {
        Self::Bool(value)
    }
}

impl From<&str> for ScriptValue {
    fn from(value: &str) -> Self {
        Self::Str(Arc::from(value))
    }
}

impl From<Vec3> for ScriptValue {
    fn from(value: Vec3) -> Self {
        Self::Vec3(value)
    }
}

impl From<Entity> for ScriptValue {
    fn from(value: Entity) -> Self {
        Self::Entity(value)
    }
}

/// A number, as scripts give them: an int of any size, or a float.
pub(crate) fn number(value: Value) -> Option<f64> {
    if let Some(i) = value.unpack_i32() {
        return Some(f64::from(i));
    }
    if let Some(f) = value.downcast_ref::<StarlarkFloat>() {
        return Some(f.0);
    }
    (value.get_type() == "int")
        .then(|| value.to_str().parse::<f64>().ok())
        .flatten()
}

/// A vector, as scripts see it: `v.x`, `v.y`, `v.z`, `+`, `-`, and `*` or `/` by a number.
/// Vectors can't be dict keys: as in Python, floats that look equal often aren't.
#[derive(Clone, Copy, Debug, PartialEq, ProvidesStaticType, NoSerialize, Allocative)]
pub struct Vec3Value(#[allocative(skip)] pub Vec3);

starlark_simple_value!(Vec3Value);

impl fmt::Display for Vec3Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "vec3({}, {}, {})", self.0.x, self.0.y, self.0.z)
    }
}

#[starlark_value(type = "vec3")]
impl<'v> StarlarkValue<'v> for Vec3Value {
    fn get_attr(&self, attribute: &str, heap: Heap<'v>) -> Option<Value<'v>> {
        let component = match attribute {
            "x" => self.0.x,
            "y" => self.0.y,
            "z" => self.0.z,
            _ => return None,
        };
        Some(heap.alloc(StarlarkFloat(f64::from(component))))
    }

    fn dir_attr(&self) -> Vec<String> {
        ["x", "y", "z"].map(String::from).to_vec()
    }

    fn equals(&self, other: Value<'v>) -> starlark::Result<bool> {
        Ok(other.downcast_ref::<Self>().is_some_and(|o| o.0 == self.0))
    }

    fn add(&self, rhs: Value<'v>, heap: Heap<'v>) -> Option<starlark::Result<Value<'v>>> {
        let rhs = rhs.downcast_ref::<Self>()?;
        Some(Ok(heap.alloc(Self(self.0 + rhs.0))))
    }

    fn sub(&self, rhs: Value<'v>, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        match rhs.downcast_ref::<Self>() {
            Some(rhs) => Ok(heap.alloc(Self(self.0 - rhs.0))),
            None => starlark::values::ValueError::unsupported_with(self, "-", rhs),
        }
    }

    fn mul(&self, rhs: Value<'v>, heap: Heap<'v>) -> Option<starlark::Result<Value<'v>>> {
        number(rhs).map(|n| Ok(heap.alloc(Self(self.0 * n as f32))))
    }

    fn rmul(&self, lhs: Value<'v>, heap: Heap<'v>) -> Option<starlark::Result<Value<'v>>> {
        number(lhs).map(|n| Ok(heap.alloc(Self(self.0 * n as f32))))
    }

    fn div(&self, rhs: Value<'v>, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        match number(rhs) {
            Some(n) => Ok(heap.alloc(Self(self.0 / n as f32))),
            None => starlark::values::ValueError::unsupported_with(self, "/", rhs),
        }
    }

    fn minus(&self, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        Ok(heap.alloc(Self(-self.0)))
    }
}

/// An entity, as scripts see it: something to send events to, or to look up. Entities can be
/// dict keys, for sets of them such as what is in range.
#[derive(Clone, Copy, Debug, PartialEq, ProvidesStaticType, NoSerialize, Allocative)]
pub struct EntityValue(#[allocative(skip)] pub Entity);

starlark_simple_value!(EntityValue);

impl fmt::Display for EntityValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "entity({})", self.0.id())
    }
}

#[starlark_value(type = "entity")]
impl<'v> StarlarkValue<'v> for EntityValue {
    fn equals(&self, other: Value<'v>) -> starlark::Result<bool> {
        Ok(other.downcast_ref::<Self>().is_some_and(|o| o.0 == self.0))
    }

    fn write_hash(&self, hasher: &mut StarlarkHasher) -> starlark::Result<()> {
        hasher.write_u64(self.0.to_bits().get());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use starlark::environment::Module;

    #[test]
    fn values_come_back_as_they_went_in() {
        let dict = vec![
            ("speed".to_owned(), ScriptValue::Float(2.5)),
            (
                "path".to_owned(),
                ScriptValue::List(vec![ScriptValue::Vec3(Vec3::X)]),
            ),
            // Out of order: dicts keep the order keys were added.
            ("alpha".to_owned(), ScriptValue::None),
        ];
        let values = [
            ScriptValue::None,
            ScriptValue::Bool(true),
            ScriptValue::Int(-7),
            ScriptValue::Int(1 << 40),
            ScriptValue::Float(0.25),
            ScriptValue::from("gate"),
            ScriptValue::Vec3(Vec3::new(1.0, 2.0, 3.0)),
            ScriptValue::List(vec![ScriptValue::Int(1), ScriptValue::from("two")]),
            ScriptValue::Dict(dict),
        ];
        Module::with_temp_heap(|module| {
            for value in &values {
                let back = ScriptValue::from_starlark(value.to_starlark(module.heap()));
                assert_eq!(back.as_ref(), Ok(value));
            }
        });
    }

    #[test]
    fn deep_or_huge_values_are_refused_not_followed() {
        Module::with_temp_heap(|module| {
            let heap = module.heap();
            let mut deep = ScriptValue::None;
            for _ in 0..=MAX_DEPTH {
                deep = ScriptValue::List(vec![deep]);
            }
            let err = ScriptValue::from_starlark(deep.to_starlark(heap)).unwrap_err();
            assert!(err.contains("deep"), "{err}");
            let wide = ScriptValue::List(vec![ScriptValue::Int(1); MAX_VALUES as usize]);
            let err = ScriptValue::from_starlark(wide.to_starlark(heap)).unwrap_err();
            assert!(err.contains("at most"), "{err}");
        });
    }

    #[test]
    fn numbers_of_every_size_scale_vectors() {
        Module::with_temp_heap(|module| {
            let big = module.heap().alloc(3_000_000_000_i64);
            assert_eq!(number(big), Some(3.0e9));
            assert_eq!(with_article("int"), "an int");
            assert_eq!(with_article("list"), "a list");
        });
    }
}
