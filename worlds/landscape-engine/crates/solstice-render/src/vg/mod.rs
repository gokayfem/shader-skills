//! Virtualized geometry: per-view LOD selection, culling and rasterization.
//!
//! Each frame the main view runs two occlusion phases:
//!
//! ```text
//!  cull (old depth) -> raster -> build depth pyramid
//!  cull (retries against new depth) -> raster -> build depth pyramid
//!  resolve visibility buffer to G-buffer
//! ```
//!
//! Other views (shadow cascades, GI voxelization) reuse the same culling with their
//! own [`PassView`], without occlusion and with hardware rasterization only.

mod culling;
mod hzb;
mod raster;
#[cfg(test)]
mod raster_stress;
mod resolve;

pub use culling::{
    CullPhase, CullPipelines, CullStats, CullWork, InstanceFilter, PassView, ViewSlots,
};
pub use hzb::Hzb;
pub use raster::{
    DepthBuffer, RasterBindings, Rasterizer, VisibilityBuffer, DEPTH_FORMAT, DEPTH_STAMPS,
};
pub use resolve::{GBuffer, Resolve, ResolveBindings};
