#![forbid(unsafe_code)]

pub mod adapter;
pub mod attach_pipeline;
pub mod attention;
pub mod display;
pub mod error;
pub mod handle;
pub mod in_memory;
pub mod input;
pub mod input_pipeline;
pub mod key_names;
pub mod launch;
pub mod permission;
pub mod placement;
pub mod replace_pipeline;
pub mod search;
pub mod surface;
pub mod wait;
pub mod wait_pipeline;

pub use attention::{AttentionInfo, CursorPos};
pub use display::{DisplayId, DisplayInfo, Rect as DisplayRect};
pub use error::{ErrorCode, PortholeError};
pub use input::{ClickButton, ClickSpec, KeyEvent, Modifier, ScrollSpec};
pub use permission::SystemPermissionStatus;
pub use placement::{Anchor, DisplayTarget, GeometrySnapshot, PlacementOutcome, PlacementSpec};
pub use search::{Candidate, SearchQuery};
pub use surface::{SurfaceId, SurfaceInfo, SurfaceKind, SurfaceState};
pub use wait::{LastObserved, WaitCondition, WaitOutcome, WaitTimeout, DEFAULT_WAIT_TIMEOUT, WAIT_SAMPLE_INTERVAL};
