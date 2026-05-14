use serde::{Deserialize, Serialize};

use crate::display::Rect;

/// Which path the adapter's descent took to find the content child.
///
/// Surfaced on the wire so harnesses can diagnose surprising results without
/// daemon logs — see CONTEXT.md (Content rect) and `docs/adr/0001-...`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Descent {
    /// The OS accessibility tree returned a non-empty `contents` attribute and
    /// we used its first element.
    Contents,
    /// The accessibility tree had no usable `contents`; we fell back to the
    /// largest non-zero-area child.
    LargestChild,
}

/// Adapter-returned content-rect payload, in **window-local logical** units.
///
/// The pipeline converts to physical pixels when the caller asked for them;
/// adapters do not see `CoordUnits`.
#[derive(Clone, Debug, PartialEq)]
pub struct ContentRectInfo {
    pub rect: Rect,
    pub ax_role: String,
    pub descent: Descent,
}
