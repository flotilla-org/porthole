use serde::{Deserialize, Serialize};

pub use porthole_core::attention::{AttentionInfo, CursorPos};
pub use porthole_core::display::DisplayInfo;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DisplaysResponse {
    pub displays: Vec<DisplayInfo>,
}
