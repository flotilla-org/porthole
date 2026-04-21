#![forbid(unsafe_code)]

pub mod adapter;
pub mod error;
pub mod handle;
pub mod in_memory;
pub mod input;
pub mod key_names;
pub mod launch;
pub mod surface;
pub mod wait;

pub use error::{ErrorCode, PortholeError};
pub use input::{ClickButton, ClickSpec, KeyEvent, Modifier, ScrollSpec};
pub use surface::{SurfaceId, SurfaceInfo, SurfaceKind, SurfaceState};
pub use wait::{LastObserved, WaitCondition, WaitOutcome, DEFAULT_WAIT_TIMEOUT, WAIT_SAMPLE_INTERVAL};
