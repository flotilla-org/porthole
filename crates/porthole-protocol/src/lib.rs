#![forbid(unsafe_code)]

pub mod attention;
pub mod close_focus;
pub mod error;
pub mod info;
pub mod input;
pub mod launches;
pub mod placement;
pub mod screenshot;
pub mod search;
pub mod system_permission;
pub mod wait;

pub use porthole_core::surface::{SurfaceId, SurfaceKind, SurfaceState};
