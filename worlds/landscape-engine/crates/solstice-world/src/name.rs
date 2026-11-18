//! An entity's name, to find it by: what scripts call `find("gate")` with, and what scenes name
//! their actors.

use std::sync::Arc;

/// An entity's name. Names need not be unique.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Name(pub Arc<str>);

impl Name {
    pub fn new(name: &str) -> Self {
        Self(Arc::from(name))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}
