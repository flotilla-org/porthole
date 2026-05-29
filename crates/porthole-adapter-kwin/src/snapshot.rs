use std::collections::BTreeMap;

use porthole_core::display::{DisplayId, DisplayInfo, Rect};
use serde::Deserialize;

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct KWinSnapshotPayload {
    pub(crate) schema_version: u32,
    #[serde(default)]
    pub(crate) active_window: Option<KWinWindow>,
    #[serde(default)]
    pub(crate) cursor: Option<KWinCursor>,
    #[serde(default)]
    pub(crate) outputs: Vec<KWinOutput>,
    #[serde(default)]
    pub(crate) windows: Vec<KWinWindow>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct KWinCursor {
    pub(crate) x: f64,
    pub(crate) y: f64,
    #[serde(default)]
    pub(crate) output: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct KWinOutput {
    pub(crate) name: String,
    pub(crate) geometry: KWinRect,
    #[serde(default = "default_scale")]
    pub(crate) scale: f64,
    #[serde(default)]
    pub(crate) active: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct KWinRect {
    pub(crate) x: f64,
    pub(crate) y: f64,
    #[serde(alias = "w")]
    pub(crate) width: f64,
    #[serde(alias = "h")]
    pub(crate) height: f64,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct KWinWindow {
    pub(crate) window_id: String,
    #[serde(default)]
    pub(crate) caption: Option<String>,
    #[serde(default)]
    pub(crate) resource_class: Option<String>,
    #[serde(default)]
    pub(crate) resource_name: Option<String>,
    #[serde(default)]
    pub(crate) desktop_file_name: Option<String>,
    #[serde(default)]
    pub(crate) pid: u32,
    #[serde(default)]
    pub(crate) normal_window: bool,
    #[serde(default)]
    pub(crate) active: bool,
    #[serde(default)]
    pub(crate) minimized: bool,
    #[serde(default)]
    pub(crate) output: Option<String>,
    #[serde(default)]
    pub(crate) frame_geometry: Option<KWinRect>,
}

impl KWinSnapshotPayload {
    pub(crate) fn displays(&self) -> Vec<DisplayInfo> {
        if !self.outputs.is_empty() {
            return self
                .outputs
                .iter()
                .enumerate()
                .map(|(index, output)| DisplayInfo {
                    id: DisplayId::new(output.name.clone()),
                    bounds: Rect {
                        x: output.geometry.x,
                        y: output.geometry.y,
                        w: output.geometry.width,
                        h: output.geometry.height,
                    },
                    scale: output.scale,
                    primary: index == 0,
                    focused: output.active,
                })
                .collect();
        }

        let mut by_name = BTreeMap::<String, Rect>::new();
        for window in &self.windows {
            let (Some(output), Some(rect)) = (&window.output, window.frame_geometry) else {
                continue;
            };
            by_name
                .entry(output.clone())
                .and_modify(|bounds| {
                    let min_x = bounds.x.min(rect.x);
                    let min_y = bounds.y.min(rect.y);
                    let max_x = (bounds.x + bounds.w).max(rect.x + rect.width);
                    let max_y = (bounds.y + bounds.h).max(rect.y + rect.height);
                    *bounds = Rect {
                        x: min_x,
                        y: min_y,
                        w: max_x - min_x,
                        h: max_y - min_y,
                    };
                })
                .or_insert(Rect {
                    x: rect.x,
                    y: rect.y,
                    w: rect.width,
                    h: rect.height,
                });
        }
        by_name
            .into_iter()
            .enumerate()
            .map(|(index, (name, bounds))| DisplayInfo {
                id: DisplayId::new(name.clone()),
                bounds,
                scale: 1.0,
                primary: index == 0,
                focused: self.active_window.as_ref().and_then(|window| window.output.as_ref()) == Some(&name),
            })
            .collect()
    }
}

impl KWinWindow {
    pub(crate) fn app_name(&self) -> Option<String> {
        self.desktop_file_name
            .clone()
            .or_else(|| self.resource_class.clone())
            .or_else(|| self.resource_name.clone())
    }
}

fn default_scale() -> f64 {
    1.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_deserializes_window_identity() {
        let snapshot: KWinSnapshotPayload = serde_json::from_str(
            r#"{
                "schemaVersion": 1,
                "activeWindow": {
                    "windowId": "win-a",
                    "caption": "Terminal",
                    "resourceClass": "org.kde.konsole",
                    "pid": 123,
                    "normalWindow": true,
                    "active": true,
                    "output": "eDP-1",
                    "frameGeometry": { "x": 10, "y": 20, "width": 800, "height": 600 }
                },
                "windows": []
            }"#,
        )
        .unwrap();
        let window = snapshot.active_window.unwrap();
        assert_eq!(window.window_id, "win-a");
        assert_eq!(window.app_name().as_deref(), Some("org.kde.konsole"));
    }

    #[test]
    fn displays_prefer_output_geometry() {
        let snapshot: KWinSnapshotPayload = serde_json::from_str(
            r#"{
                "schemaVersion": 1,
                "outputs": [
                    { "name": "eDP-1", "geometry": { "x": 0, "y": 0, "width": 1920, "height": 1080 }, "scale": 1.25, "active": true }
                ],
                "windows": []
            }"#,
        )
        .unwrap();
        let displays = snapshot.displays();
        assert_eq!(displays.len(), 1);
        assert_eq!(displays[0].id.as_str(), "eDP-1");
        assert_eq!(displays[0].scale, 1.25);
        assert!(displays[0].focused);
    }
}
