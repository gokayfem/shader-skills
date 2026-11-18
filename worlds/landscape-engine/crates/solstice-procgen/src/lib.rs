//! Generates content from a seed instead of storing it, offline or in the browser.
//!
//! Each layer builds on the one before, and can be used without the ones after:
//!
//! * [`noise`] is the raw randomness.
//! * [`field`] combines noise into seamless values across a square.
//! * [`cracks`] grows a crack network the way drying really cracks a surface.
//! * [`surface`] turns fields into material textures.
//! * [`surfaces`] is a library of ready-made surfaces, each fully described by a name and a seed.
//! * [`bush`] adds the ground of the Australian bush to it: clay, mud, gum litter and straw.
//!
//! Nothing here touches files or threads directly, and the same seed gives the same result on
//! every platform, so a scene can hold a recipe in place of the data it produces.

pub mod bush;
pub mod cracks;
pub mod field;
pub mod foliage;
pub mod noise;
pub mod surface;
pub mod surfaces;

pub use field::Field;
pub use surface::{Relief, Shade, Surface, Texel};
