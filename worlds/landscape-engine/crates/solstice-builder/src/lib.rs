//! Offline preprocessing for virtualized geometry.
//!
//! [`build_virtual_mesh`] converts an ordinary triangle mesh into a DAG of small
//! triangle clusters at many levels of detail. At runtime a *cut* through this
//! DAG is selected per view, so that triangle density tracks pixel density.
//!
//! The pipeline, repeated per level until nothing simplifies further:
//!
//! 1. `partition` splits triangles into compact clusters.
//! 2. `group` gathers neighbouring clusters into groups.
//! 3. `simplify` halves each group's triangles while locking its outer boundary.
//! 4. `partition` splits the result back into clusters, which become the parents
//!    of the group's clusters in the DAG.
//!
//! Because a group's boundary is locked, a group and its simplified replacement
//! are interchangeable *independently of their neighbours*. Because grouping
//! changes every level, the boundary locked at one level is interior at the next,
//! so no seam survives forever.

mod bounds;
pub mod collision;
mod dag;
mod group;
mod hierarchy;
mod mesh;
mod partition;
mod simplify;
mod texture;
mod thin;

mod paging;

pub use bounds::Sphere;
pub use dag::{build_skinned_mesh, build_virtual_mesh, BuildOptions};
pub use mesh::{Mesh, Vertex};
pub use paging::{paginate, paginate_from, ClusterRange, ClusterRecords, ClusterStore};
pub use texture::{compress_textures, masked_mip_chain, mip_chain, move_alpha_threshold};
