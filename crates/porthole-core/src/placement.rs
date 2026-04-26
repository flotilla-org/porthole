use serde::{Deserialize, Serialize};

use crate::display::{DisplayId, Rect};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PlacementSpec {
    #[serde(default)]
    pub on_display: Option<DisplayTarget>,
    #[serde(default)]
    pub geometry: Option<Rect>,
    #[serde(default)]
    pub anchor: Option<Anchor>,
}

impl PlacementSpec {
    /// True when the spec has no effective field — PlacementOutcome::NotRequested applies.
    pub fn is_effectively_empty(&self) -> bool {
        self.on_display.is_none() && self.geometry.is_none() && self.anchor.is_none()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum DisplayTarget {
    Focused,
    Primary,
    Id(DisplayId),
}

impl Serialize for DisplayTarget {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            DisplayTarget::Focused => s.serialize_str("focused"),
            DisplayTarget::Primary => s.serialize_str("primary"),
            DisplayTarget::Id(id) => s.serialize_str(id.as_str()),
        }
    }
}

impl<'de> Deserialize<'de> for DisplayTarget {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Ok(match s.as_str() {
            "focused" => DisplayTarget::Focused,
            "primary" => DisplayTarget::Primary,
            _ => DisplayTarget::Id(DisplayId::new(s)),
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Anchor {
    FocusedDisplay,
    Cursor,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PlacementOutcome {
    NotRequested,
    Applied,
    SkippedPreexisting,
    Failed { reason: String },
}

/// Snapshot of a window's current geometry, display-local.
/// Used by ReplacePipeline to inject inherited placement into the
/// replacement launch.
#[derive(Clone, Debug, PartialEq)]
pub struct GeometrySnapshot {
    pub display_id: DisplayId,
    pub display_local: Rect,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placement_spec_empty_by_default() {
        let p = PlacementSpec::default();
        assert!(p.is_effectively_empty());
    }

    #[test]
    fn placement_spec_with_any_field_not_empty() {
        let p = PlacementSpec {
            on_display: Some(DisplayTarget::Primary),
            ..Default::default()
        };
        assert!(!p.is_effectively_empty());
    }

    #[test]
    fn placement_outcome_roundtrip() {
        let o = PlacementOutcome::Applied;
        let s = serde_json::to_string(&o).unwrap();
        assert_eq!(s, r#"{"type":"applied"}"#);

        let o = PlacementOutcome::Failed {
            reason: "AX denied".into(),
        };
        let s = serde_json::to_string(&o).unwrap();
        assert_eq!(s, r#"{"type":"failed","reason":"AX denied"}"#);
    }

    #[test]
    fn display_target_id_serializes_as_plain_string() {
        let t = DisplayTarget::Id(DisplayId::new("disp_1"));
        let s = serde_json::to_string(&t).unwrap();
        assert_eq!(s, r#""disp_1""#);
    }

    #[test]
    fn display_target_focused_serializes_as_focused_string() {
        let t = DisplayTarget::Focused;
        let s = serde_json::to_string(&t).unwrap();
        assert_eq!(s, r#""focused""#);
    }

    #[test]
    fn display_target_deserializes_plain_string() {
        let t: DisplayTarget = serde_json::from_str(r#""disp_1""#).unwrap();
        assert_eq!(t, DisplayTarget::Id(DisplayId::new("disp_1")));
        let t: DisplayTarget = serde_json::from_str(r#""focused""#).unwrap();
        assert_eq!(t, DisplayTarget::Focused);
    }
}
