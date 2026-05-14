use std::sync::Arc;

use crate::{
    ErrorCode, PortholeError,
    adapter::Adapter,
    display::Rect,
    handle::HandleStore,
    input::{ClickSpec, CoordUnits, KeyEvent, ScrollSpec},
    key_names,
    surface::{SurfaceId, SurfaceInfo},
};

pub struct InputPipeline {
    adapter: Arc<dyn Adapter>,
    handles: HandleStore,
}

impl InputPipeline {
    pub fn new(adapter: Arc<dyn Adapter>, handles: HandleStore) -> Self {
        Self { adapter, handles }
    }

    pub async fn key(&self, surface: &SurfaceId, events: &[KeyEvent]) -> Result<(), PortholeError> {
        for ev in events {
            if !key_names::is_supported(&ev.key) {
                return Err(PortholeError::new(
                    ErrorCode::UnknownKey,
                    format!("key '{}' is not in the supported set", ev.key),
                ));
            }
        }
        let info = self.handles.require_alive(surface).await?;
        self.adapter.key(&info, events).await
    }

    pub async fn text(&self, surface: &SurfaceId, text: &str) -> Result<(), PortholeError> {
        let info = self.handles.require_alive(surface).await?;
        self.adapter.text(&info, text).await
    }

    pub async fn click(&self, surface: &SurfaceId, spec: &ClickSpec, units: CoordUnits) -> Result<(), PortholeError> {
        if spec.count == 0 || spec.count > 3 {
            return Err(PortholeError::new(
                ErrorCode::InvalidArgument,
                format!("click count must be 1, 2, or 3 (got {})", spec.count),
            ));
        }
        let info = self.handles.require_alive(surface).await?;
        let spec = self.to_logical_click(spec, units, &info).await?;
        self.adapter.click(&info, &spec).await
    }

    pub async fn scroll(&self, surface: &SurfaceId, spec: &ScrollSpec, units: CoordUnits) -> Result<(), PortholeError> {
        let info = self.handles.require_alive(surface).await?;
        let spec = self.to_logical_scroll(spec, units, &info).await?;
        self.adapter.scroll(&info, &spec).await
    }

    pub async fn close(&self, surface: &SurfaceId) -> Result<(), PortholeError> {
        let info = self.handles.require_alive(surface).await?;
        match self.adapter.close(&info).await {
            Ok(()) => {
                self.handles.mark_dead(surface).await?;
                Ok(())
            }
            Err(e) if e.code == ErrorCode::CloseFailed => {
                // The window is still alive (e.g. a save dialog vetoed the close).
                // Do NOT mark the handle dead — the caller can retry or investigate.
                Err(e)
            }
            Err(e) => Err(e),
        }
    }

    pub async fn focus(&self, surface: &SurfaceId) -> Result<(), PortholeError> {
        let info = self.handles.require_alive(surface).await?;
        self.adapter.focus(&info).await
    }

    /// In-place resize/move. Surface identity is preserved — the same
    /// `surface_id` resolves before and after, the inner process keeps
    /// running. Use this for terminal-reflow tests and any other workflow
    /// where you'd otherwise reach for `/replace` but don't want the window
    /// closed and reopened.
    pub async fn place(&self, surface: &SurfaceId, rect: Rect, units: CoordUnits) -> Result<(), PortholeError> {
        // Negative x/y is legitimate (windows off the primary display can have
        // negative origins), but non-finite values would propagate to AX with
        // undefined behaviour. w/h additionally have to be > 0 — a zero-size
        // window is meaningless and AX would reject the size write anyway.
        if !rect.x.is_finite() || !rect.y.is_finite() || !rect.w.is_finite() || !rect.h.is_finite() {
            return Err(PortholeError::new(
                ErrorCode::InvalidArgument,
                format!(
                    "place: x, y, w, h must all be finite (got x={}, y={}, w={}, h={})",
                    rect.x, rect.y, rect.w, rect.h
                ),
            ));
        }
        if rect.w <= 0.0 || rect.h <= 0.0 {
            return Err(PortholeError::new(
                ErrorCode::InvalidArgument,
                format!("place: w and h must be > 0 (got w={}, h={})", rect.w, rect.h),
            ));
        }
        let info = self.handles.require_alive(surface).await?;
        let rect = self.to_logical_rect(rect, units, &info).await?;
        self.adapter.place_surface(&info, rect).await
    }

    /// Look up the surface's current display's backing scale factor.
    /// Used to convert physical-pixel coordinates to the logical points the
    /// adapter expects. Errors if the surface's display has been disconnected
    /// between snapshot and lookup — better to fail loudly than silently
    /// apply a wrong scale.
    async fn surface_scale(&self, info: &SurfaceInfo) -> Result<f64, PortholeError> {
        let snap = self.adapter.snapshot_geometry(info).await?;
        let displays = self.adapter.displays().await?;
        displays.iter().find(|d| d.id == snap.display_id).map(|d| d.scale).ok_or_else(|| {
            PortholeError::new(
                ErrorCode::CapabilityMissing,
                format!("display {} not found while resolving scale", snap.display_id.as_str()),
            )
        })
    }

    async fn to_logical_click(&self, spec: &ClickSpec, units: CoordUnits, info: &SurfaceInfo) -> Result<ClickSpec, PortholeError> {
        if matches!(units, CoordUnits::Logical) {
            return Ok(spec.clone());
        }
        let scale = self.surface_scale(info).await?;
        Ok(ClickSpec {
            x: spec.x / scale,
            y: spec.y / scale,
            button: spec.button,
            count: spec.count,
            modifiers: spec.modifiers.clone(),
        })
    }

    async fn to_logical_scroll(&self, spec: &ScrollSpec, units: CoordUnits, info: &SurfaceInfo) -> Result<ScrollSpec, PortholeError> {
        if matches!(units, CoordUnits::Logical) {
            return Ok(spec.clone());
        }
        let scale = self.surface_scale(info).await?;
        // Position scales; delta is a velocity/wheel-line count, not a pixel
        // distance, so it's left untouched. CGEventScrollWheel deltas are in
        // wheel "lines" and don't get the Retina treatment.
        Ok(ScrollSpec {
            x: spec.x / scale,
            y: spec.y / scale,
            delta_x: spec.delta_x,
            delta_y: spec.delta_y,
        })
    }

    async fn to_logical_rect(&self, rect: Rect, units: CoordUnits, info: &SurfaceInfo) -> Result<Rect, PortholeError> {
        if matches!(units, CoordUnits::Logical) {
            return Ok(rect);
        }
        let scale = self.surface_scale(info).await?;
        Ok(Rect {
            x: rect.x / scale,
            y: rect.y / scale,
            w: rect.w / scale,
            h: rect.h / scale,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{in_memory::InMemoryAdapter, surface::SurfaceInfo};

    async fn setup() -> (Arc<InMemoryAdapter>, HandleStore, SurfaceId) {
        let adapter = Arc::new(InMemoryAdapter::new());
        let handles = HandleStore::new();
        let info = SurfaceInfo::window(SurfaceId::new(), 1);
        let id = info.id.clone();
        handles.insert(info).await;
        (adapter, handles, id)
    }

    #[tokio::test]
    async fn key_rejects_unsupported_name() {
        let (adapter, handles, id) = setup().await;
        let pipeline = InputPipeline::new(adapter.clone(), handles);
        let err = pipeline
            .key(
                &id,
                &[KeyEvent {
                    key: "NotAKey".into(),
                    modifiers: vec![],
                }],
            )
            .await
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::UnknownKey);
    }

    #[tokio::test]
    async fn key_delegates_to_adapter() {
        let (adapter, handles, id) = setup().await;
        let pipeline = InputPipeline::new(adapter.clone(), handles);
        pipeline
            .key(
                &id,
                &[KeyEvent {
                    key: "Enter".into(),
                    modifiers: vec![],
                }],
            )
            .await
            .unwrap();
        assert_eq!(adapter.key_calls().await.len(), 1);
    }

    #[tokio::test]
    async fn click_rejects_count_zero() {
        let (adapter, handles, id) = setup().await;
        let pipeline = InputPipeline::new(adapter.clone(), handles);
        let err = pipeline
            .click(
                &id,
                &ClickSpec {
                    x: 0.0,
                    y: 0.0,
                    button: crate::input::ClickButton::Left,
                    count: 0,
                    modifiers: vec![],
                },
                CoordUnits::Logical,
            )
            .await
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidArgument);
    }

    #[tokio::test]
    async fn close_marks_handle_dead() {
        let (adapter, handles, id) = setup().await;
        let pipeline = InputPipeline::new(adapter.clone(), handles.clone());
        pipeline.close(&id).await.unwrap();
        let err = handles.require_alive(&id).await.unwrap_err();
        assert_eq!(err.code, ErrorCode::SurfaceDead);
    }

    #[tokio::test]
    async fn place_rejects_invalid_rect() {
        let (adapter, handles, id) = setup().await;
        let pipeline = InputPipeline::new(adapter.clone(), handles);
        let cases: &[Rect] = &[
            // non-positive size
            Rect {
                x: 0.0,
                y: 0.0,
                w: 0.0,
                h: 100.0,
            },
            Rect {
                x: 0.0,
                y: 0.0,
                w: 100.0,
                h: 0.0,
            },
            Rect {
                x: 0.0,
                y: 0.0,
                w: -1.0,
                h: 100.0,
            },
            // non-finite size
            Rect {
                x: 0.0,
                y: 0.0,
                w: f64::NAN,
                h: 100.0,
            },
            Rect {
                x: 0.0,
                y: 0.0,
                w: 100.0,
                h: f64::NAN,
            },
            Rect {
                x: 0.0,
                y: 0.0,
                w: f64::INFINITY,
                h: 100.0,
            },
            // non-finite position
            Rect {
                x: f64::NAN,
                y: 0.0,
                w: 100.0,
                h: 100.0,
            },
            Rect {
                x: 0.0,
                y: f64::INFINITY,
                w: 100.0,
                h: 100.0,
            },
        ];
        for rect in cases {
            let err = pipeline.place(&id, *rect, CoordUnits::Logical).await.unwrap_err();
            assert_eq!(err.code, ErrorCode::InvalidArgument, "expected InvalidArgument for {rect:?}");
        }
    }

    #[tokio::test]
    async fn place_accepts_negative_origin() {
        // A window on a secondary display can legitimately have a negative
        // global x/y origin.
        let (adapter, handles, id) = setup().await;
        let pipeline = InputPipeline::new(adapter.clone(), handles);
        pipeline
            .place(
                &id,
                Rect {
                    x: -100.0,
                    y: -50.0,
                    w: 800.0,
                    h: 600.0,
                },
                CoordUnits::Logical,
            )
            .await
            .unwrap();
        assert_eq!(adapter.place_surface_calls().await.len(), 1);
    }

    #[tokio::test]
    async fn place_records_call_on_adapter() {
        let (adapter, handles, id) = setup().await;
        let pipeline = InputPipeline::new(adapter.clone(), handles);
        pipeline
            .place(
                &id,
                Rect {
                    x: 10.0,
                    y: 20.0,
                    w: 800.0,
                    h: 600.0,
                },
                CoordUnits::Logical,
            )
            .await
            .unwrap();
        let calls = adapter.place_surface_calls().await;
        assert_eq!(calls.len(), 1);
        assert_eq!(
            calls[0].1,
            Rect {
                x: 10.0,
                y: 20.0,
                w: 800.0,
                h: 600.0
            }
        );
    }

    #[tokio::test]
    async fn place_propagates_dead_handle() {
        let (adapter, handles, id) = setup().await;
        handles.mark_dead(&id).await.unwrap();
        let pipeline = InputPipeline::new(adapter.clone(), handles);
        let err = pipeline
            .place(
                &id,
                Rect {
                    x: 0.0,
                    y: 0.0,
                    w: 100.0,
                    h: 100.0,
                },
                CoordUnits::Logical,
            )
            .await
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::SurfaceDead);
    }

    #[tokio::test]
    async fn place_physical_units_halve_on_2x_display() {
        // 2x Retina: caller passes physical pixels, adapter must see logical
        // points (half of physical). The InMemoryAdapter's snapshot_geometry
        // returns display_id "disp_test"; we seed `displays()` to claim that
        // display has scale 2.
        let (adapter, handles, id) = setup().await;
        adapter.set_test_scale_for_snapshot(2.0).await;
        let pipeline = InputPipeline::new(adapter.clone(), handles);
        pipeline
            .place(
                &id,
                Rect {
                    x: 100.0,
                    y: 200.0,
                    w: 1600.0,
                    h: 1200.0,
                },
                CoordUnits::Physical,
            )
            .await
            .unwrap();
        let calls = adapter.place_surface_calls().await;
        assert_eq!(calls.len(), 1);
        assert_eq!(
            calls[0].1,
            Rect {
                x: 50.0,
                y: 100.0,
                w: 800.0,
                h: 600.0
            },
            "physical 1600x1200 @ (100,200) on 2x display should become logical 800x600 @ (50,100)"
        );
    }

    #[tokio::test]
    async fn click_physical_units_scale_x_y() {
        let (adapter, handles, id) = setup().await;
        adapter.set_test_scale_for_snapshot(2.0).await;
        let pipeline = InputPipeline::new(adapter.clone(), handles);
        pipeline
            .click(
                &id,
                &ClickSpec {
                    x: 800.0,
                    y: 600.0,
                    button: crate::input::ClickButton::Left,
                    count: 1,
                    modifiers: vec![],
                },
                CoordUnits::Physical,
            )
            .await
            .unwrap();
        let calls = adapter.click_calls().await;
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].1.x, 400.0);
        assert_eq!(calls[0].1.y, 300.0);
    }

    #[tokio::test]
    async fn scroll_physical_units_scale_position_not_delta() {
        // x/y are position in window-local pixels — scale by display factor.
        // delta_x/delta_y are wheel-line counts — leave alone.
        let (adapter, handles, id) = setup().await;
        adapter.set_test_scale_for_snapshot(2.0).await;
        let pipeline = InputPipeline::new(adapter.clone(), handles);
        pipeline
            .scroll(
                &id,
                &ScrollSpec {
                    x: 400.0,
                    y: 200.0,
                    delta_x: 0.0,
                    delta_y: -3.0,
                },
                CoordUnits::Physical,
            )
            .await
            .unwrap();
        let calls = adapter.scroll_calls().await;
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].1.x, 200.0);
        assert_eq!(calls[0].1.y, 100.0);
        assert_eq!(calls[0].1.delta_y, -3.0, "delta is wheel lines, not pixels — must not scale");
    }

    #[tokio::test]
    async fn close_failed_keeps_handle_alive() {
        let (adapter, handles, id) = setup().await;
        adapter
            .set_next_close_result(Err(PortholeError::new(ErrorCode::CloseFailed, "save dialog vetoed")))
            .await;
        let pipeline = InputPipeline::new(adapter.clone(), handles.clone());
        let err = pipeline.close(&id).await.unwrap_err();
        assert_eq!(err.code, ErrorCode::CloseFailed);
        // Handle must still be alive — the window was not closed.
        handles
            .require_alive(&id)
            .await
            .expect("handle should remain alive after close_failed");
    }
}
