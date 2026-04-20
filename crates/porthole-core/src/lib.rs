#![forbid(unsafe_code)]

pub mod adapter;
pub mod error;
pub mod handle;
pub mod in_memory;
pub mod launch;
pub mod surface;

pub use error::{ErrorCode, PortholeError};
pub use surface::{SurfaceId, SurfaceInfo, SurfaceKind, SurfaceState};
