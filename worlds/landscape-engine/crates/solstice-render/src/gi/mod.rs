//! Dynamic global illumination, in the style of a software-traced surface cache.
//!
//! Hardware ray tracing is not available in WebGPU, so rays trace a simplified copy of
//! the scene kept on the GPU:
//!
//! * [`volume`] voxelizes nearby geometry into clipmap levels, stores a distance field for
//!   fast ray marching, and keeps a radiance value on every surface voxel. Those radiance
//!   values gather light from one another over time, giving unlimited bounces.
//! * [`screen`] traces rays from a sparse grid of screen probes into the volume, filters
//!   them, and interpolates per pixel. It also traces glossy reflections.
//! * [`depth_pyramid`] keeps the nearest depth of each screen tile, so screen traces can
//!   skip empty space and reach far across the screen in a few steps.

mod depth_pyramid;
mod screen;
mod volume;

pub use screen::{ScreenGi, ScreenGiBindings};
pub use volume::{GiVolume, LEVEL_COUNT};
