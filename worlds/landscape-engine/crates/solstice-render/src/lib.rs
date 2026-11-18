//! A WebGPU renderer built around virtualized geometry and dynamic global illumination.
//!
//! Start with [`Renderer`]: give it a [`solstice_asset::LoadedScene`], and each frame a
//! [`Camera`], an [`Environment`] and a texture to draw to. [`Renderer::render`] is written
//! to read top to bottom as the frame's recipe; each pass lives in its own module.
//!
//! * [`vg`]: cluster LOD selection, culling and rasterization.
//! * [`streaming`]: moving pages of geometry onto and off the GPU.
//! * [`shadow`]: cascaded sun shadows.
//! * [`sky`]: the physically based atmosphere.
//! * [`clouds`]: volumetric clouds, driven by the weather.
//! * [`time_of_day`]: where the sun and moon are, and how they light the scene.
//! * [`gi`]: indirect lighting and reflections.
//! * [`lighting`]: combining everything into HDR colour.
//! * [`particles`]: effects simulated on the GPU and drawn over the lit scene.
//! * [`post`]: temporal resolve, exposure, bloom and tone mapping.

// glam 0.33 moved its camera matrices (`look_to_rh` and friends) to new names. The old ones still
// work; the renames can wait for a quiet moment in the passes they touch.
#![allow(deprecated)]

pub mod cards;
pub mod clouds;
pub mod frame;
pub mod gi;
pub mod gpu;
pub mod lighting;
pub mod lightning;
pub mod motion;
pub mod ocean;
pub mod particles;
pub mod pick;
pub mod post;
pub mod profiler;
pub mod rain;
pub mod regions;
pub mod renderer;
pub mod ruts;
pub mod scene;
pub mod shaders;
pub mod shadow;
pub mod skin;
pub mod sky;
pub mod streaming;
mod text;
pub mod time_of_day;
pub mod vg;
pub mod wind;

pub use clouds::{Observation, Weather, WeatherCycle};
pub use frame::{Camera, Coat, DebugView, Environment, Lamp};
pub use gpu::{Fallbacks, Gpu, GpuError};
pub use lightning::Thunder;
pub use ocean::{Ocean, Pond, Swell};
pub use particles::{Effect, EffectId, ParticleStats};
pub use renderer::{FrameStats, Quality, Renderer, RendererSettings, SceneMemory, StreamingBudget};
pub use ruts::RutSource;
pub use streaming::{StreamRequest, StreamingStats};
pub use time_of_day::{Place, TimeOfDay};
