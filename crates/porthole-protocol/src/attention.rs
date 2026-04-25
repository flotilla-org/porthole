pub use porthole_core::{
    attention::{AttentionInfo, CursorPos},
    display::DisplayInfo,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DisplaysResponse {
    pub displays: Vec<DisplayInfo>,
}
