use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::{
    ErrorCode, PortholeError,
    adapter::Adapter,
    display::{DisplayId, Rect},
};

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

/// Validate display ids in a placement spec before attempting a move.
pub async fn validate_placement(spec: &PlacementSpec, adapter: &Arc<dyn Adapter>) -> Result<(), PortholeError> {
    if spec.is_effectively_empty() {
        return Ok(());
    }
    if let Some(DisplayTarget::Id(id)) = &spec.on_display {
        let displays = adapter.displays().await?;
        if !displays.iter().any(|d| &d.id == id) {
            let known: Vec<String> = displays.iter().map(|d| d.id.as_str().to_string()).collect();
            return Err(PortholeError::new(
                ErrorCode::InvalidArgument,
                format!("unknown on_display id '{}'; known ids: [{}]", id.as_str(), known.join(", ")),
            ));
        }
    }
    Ok(())
}

/// Resolve display-local placement vocabulary to a global logical rectangle.
pub async fn resolve_placement_rect(spec: &PlacementSpec, adapter: &Arc<dyn Adapter>) -> Result<Rect, String> {
    let displays = adapter.displays().await.map_err(|e| e.message)?;
    if displays.is_empty() {
        return Err("no displays enumerated".into());
    }

    let needs_attention = matches!(&spec.on_display, Some(DisplayTarget::Focused))
        || matches!(spec.anchor, Some(Anchor::Cursor) | Some(Anchor::FocusedDisplay));
    let attn_opt = if needs_attention {
        Some(adapter.attention().await.map_err(|e| e.message)?)
    } else {
        None
    };

    let target = match &spec.on_display {
        Some(DisplayTarget::Id(id)) => displays
            .iter()
            .find(|d| &d.id == id)
            .cloned()
            .ok_or_else(|| unknown_display_id_message(id, &displays))?,
        Some(DisplayTarget::Primary) => displays.iter().find(|d| d.primary).cloned().unwrap_or_else(|| displays[0].clone()),
        Some(DisplayTarget::Focused) => {
            let attn = attn_opt.as_ref().unwrap();
            match &attn.focused_display_id {
                Some(id) => displays
                    .iter()
                    .find(|d| &d.id == id)
                    .cloned()
                    .unwrap_or_else(|| displays[0].clone()),
                None => displays.iter().find(|d| d.primary).cloned().unwrap_or_else(|| displays[0].clone()),
            }
        }
        None => match spec.anchor {
            Some(Anchor::Cursor) => {
                let attn = attn_opt.as_ref().unwrap();
                displays
                    .iter()
                    .find(|d| {
                        attn.cursor.x >= d.bounds.x
                            && attn.cursor.x < d.bounds.x + d.bounds.w
                            && attn.cursor.y >= d.bounds.y
                            && attn.cursor.y < d.bounds.y + d.bounds.h
                    })
                    .cloned()
                    .unwrap_or_else(|| displays[0].clone())
            }
            Some(Anchor::FocusedDisplay) => {
                let attn = attn_opt.as_ref().unwrap();
                match &attn.focused_display_id {
                    Some(id) => displays
                        .iter()
                        .find(|d| &d.id == id)
                        .cloned()
                        .unwrap_or_else(|| displays[0].clone()),
                    None => displays.iter().find(|d| d.primary).cloned().unwrap_or_else(|| displays[0].clone()),
                }
            }
            None => displays.iter().find(|d| d.primary).cloned().unwrap_or_else(|| displays[0].clone()),
        },
    };

    if let Some(local) = &spec.geometry {
        Ok(Rect {
            x: target.bounds.x + local.x,
            y: target.bounds.y + local.y,
            w: local.w,
            h: local.h,
        })
    } else {
        let w = (target.bounds.w * 0.7).min(1400.0);
        let h = (target.bounds.h * 0.7).min(1000.0);
        let (cx, cy) = if matches!(spec.anchor, Some(Anchor::Cursor)) {
            let attn = attn_opt.as_ref().unwrap();
            (attn.cursor.x, attn.cursor.y)
        } else {
            (target.bounds.x + target.bounds.w / 2.0, target.bounds.y + target.bounds.h / 2.0)
        };
        let x = (cx - w / 2.0).clamp(target.bounds.x, target.bounds.x + target.bounds.w - w);
        let y = (cy - h / 2.0).clamp(target.bounds.y, target.bounds.y + target.bounds.h - h);
        Ok(Rect { x, y, w, h })
    }
}

fn unknown_display_id_message(id: &DisplayId, displays: &[crate::display::DisplayInfo]) -> String {
    let known: Vec<String> = displays.iter().map(|d| d.id.as_str().to_string()).collect();
    format!("unknown on_display id '{}'; known ids: [{}]", id.as_str(), known.join(", "))
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
