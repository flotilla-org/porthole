#![forbid(unsafe_code)]

pub mod adapter;
pub mod error;
pub mod handle;
pub mod in_memory;
pub mod input;
pub mod key_names;
pub mod launch;
pub mod surface;

pub use error::{ErrorCode, PortholeError};
pub use input::{ClickButton, ClickSpec, KeyEvent, Modifier, ScrollSpec};
pub use surface::{SurfaceId, SurfaceInfo, SurfaceKind, SurfaceState};
