//! Composition-level tests for the stateful MemoryAdapter. The whole point of
//! the new fake is that operations are observable through state — these tests
//! demonstrate that shape.

use std::time::{Duration, Instant};

use super::{MemoryAdapter, WindowSpec};
use crate::{
    ErrorCode,
    adapter::{Adapter, ProcessLaunchSpec, RequireConfidence},
    content_rect::Descent,
    display::{DisplayId, DisplayInfo, Rect},
    input::{ClickButton, ClickSpec, PointerMoveSpec, ScrollSpec},
    search::SearchQuery,
    surface::SurfaceInfo,
    wait::WaitCondition,
};

fn one_display(scale: f64) -> DisplayInfo {
    DisplayInfo {
        id: DisplayId::new("d0"),
        bounds: Rect {
            x: 0.0,
            y: 0.0,
            w: 1920.0,
            h: 1080.0,
        },
        scale,
        primary: true,
        focused: true,
    }
}

fn rect(x: f64, y: f64, w: f64, h: f64) -> Rect {
    Rect { x, y, w, h }
}

fn surface_info(pid: u32, cg: u32, id: crate::surface::SurfaceId) -> SurfaceInfo {
    SurfaceInfo {
        id,
        kind: crate::surface::SurfaceKind::Window,
        state: crate::surface::SurfaceState::Alive,
        title: None,
        app_name: None,
        pid: Some(pid),
        parent_surface_id: None,
        cg_window_id: Some(cg),
    }
}

#[tokio::test]
async fn place_then_snapshot_reflects_new_rect() {
    let mut b = MemoryAdapter::builder().display(one_display(1.0));
    let id = b.window(WindowSpec::new(100, 1, rect(0.0, 0.0, 800.0, 600.0)));
    let adapter = b.build();
    let surface = surface_info(100, 1, id.clone());

    adapter.place_surface(&surface, rect(50.0, 60.0, 1200.0, 900.0)).await.unwrap();

    let snap = adapter.window(&id).unwrap();
    assert_eq!(snap.outer_rect, rect(50.0, 60.0, 1200.0, 900.0));
    // snapshot_geometry should also reflect it (display-local since the
    // display starts at 0,0 they match exactly here).
    let geom = adapter.snapshot_geometry(&surface).await.unwrap();
    assert_eq!(geom.display_local, rect(50.0, 60.0, 1200.0, 900.0));
}

#[tokio::test]
async fn focus_then_attention_reports_focused_surface() {
    let mut b = MemoryAdapter::builder().display(one_display(1.0));
    let id1 = b.window(WindowSpec::new(100, 1, rect(0.0, 0.0, 800.0, 600.0)).with_app_name("App1"));
    let id2 = b.window(WindowSpec::new(200, 2, rect(0.0, 0.0, 800.0, 600.0)).with_app_name("App2"));
    let adapter = b.build();
    let surface2 = surface_info(200, 2, id2.clone());

    // Initially no focus.
    let att = adapter.attention().await.unwrap();
    assert!(att.focused_surface_id.is_none());

    adapter.focus(&surface2).await.unwrap();
    let att = adapter.attention().await.unwrap();
    assert_eq!(att.focused_surface_id, Some(id2.clone()));
    assert_eq!(att.focused_app_name, Some("App2".to_string()));
    assert_eq!(adapter.frontmost_window_id().await.unwrap(), Some(2));
    // id1 is still tracked but unfocused.
    let _ = id1;
}

#[tokio::test]
async fn close_marks_dead_and_subsequent_ops_return_surface_dead() {
    let mut b = MemoryAdapter::builder().display(one_display(1.0));
    let id = b.window(WindowSpec::new(100, 1, rect(0.0, 0.0, 800.0, 600.0)));
    let adapter = b.build();
    let surface = surface_info(100, 1, id.clone());

    adapter.close(&surface).await.unwrap();
    assert!(!adapter.window(&id).unwrap().alive);
    assert_eq!(adapter.live_window_count(), 0);

    let err = adapter.place_surface(&surface, rect(0.0, 0.0, 100.0, 100.0)).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::SurfaceDead);
}

#[tokio::test]
async fn close_focused_window_clears_focus() {
    let mut b = MemoryAdapter::builder().display(one_display(1.0));
    let id = b.window(WindowSpec::new(100, 1, rect(0.0, 0.0, 800.0, 600.0)));
    let adapter = b.focus(100).build();
    let surface = surface_info(100, 1, id);

    assert_eq!(adapter.focused_pid(), Some(100));
    adapter.close(&surface).await.unwrap();
    assert_eq!(adapter.focused_pid(), None);
}

#[tokio::test]
async fn pointer_move_updates_cursor_in_screen_global() {
    let mut b = MemoryAdapter::builder().display(one_display(1.0));
    let id = b.window(WindowSpec::new(100, 1, rect(200.0, 100.0, 800.0, 600.0)));
    let adapter = b.build();
    let surface = surface_info(100, 1, id);

    adapter.pointer_move(&surface, &PointerMoveSpec { x: 50.0, y: 25.0 }).await.unwrap();
    let cursor = adapter.cursor();
    // Window outer at (200, 100); window-local (50, 25) → global (250, 125).
    assert_eq!(cursor.x, 250.0);
    assert_eq!(cursor.y, 125.0);
    assert_eq!(cursor.display_id, Some(DisplayId::new("d0")));
}

#[tokio::test]
async fn click_and_scroll_also_move_cursor() {
    let mut b = MemoryAdapter::builder().display(one_display(1.0));
    let id = b.window(WindowSpec::new(100, 1, rect(0.0, 0.0, 800.0, 600.0)));
    let adapter = b.build();
    let surface = surface_info(100, 1, id);

    adapter
        .click(
            &surface,
            &ClickSpec {
                x: 100.0,
                y: 200.0,
                button: ClickButton::Left,
                count: 1,
                modifiers: vec![],
            },
        )
        .await
        .unwrap();
    assert_eq!(adapter.cursor().x, 100.0);

    adapter
        .scroll(
            &surface,
            &ScrollSpec {
                x: 300.0,
                y: 400.0,
                delta_x: 0.0,
                delta_y: -3.0,
            },
        )
        .await
        .unwrap();
    assert_eq!(adapter.cursor().x, 300.0);
    assert_eq!(adapter.cursor().y, 400.0);
}

#[tokio::test]
async fn launch_process_synthesizes_window_and_focuses_it() {
    let adapter = MemoryAdapter::builder().display(one_display(1.0)).build();
    let spec = ProcessLaunchSpec {
        app: "/Applications/Terminal.app".to_string(),
        args: vec![],
        cwd: None,
        env: vec![],
        timeout: Duration::from_secs(5),
        require_confidence: RequireConfidence::Strong,
        require_fresh_surface: false,
    };
    let outcome = adapter.launch_process(&spec).await.unwrap();
    assert_eq!(outcome.surface.title.as_deref(), Some("Terminal.app"));
    assert_eq!(outcome.surface.app_name.as_deref(), Some("Terminal.app"));
    assert_eq!(adapter.live_window_count(), 1);
    assert_eq!(adapter.focused_pid(), outcome.surface.pid);
    // attention now reports the launched window.
    let att = adapter.attention().await.unwrap();
    assert_eq!(att.focused_surface_id.as_ref(), Some(&outcome.surface.id));
}

#[tokio::test]
async fn launch_twice_mints_distinct_ids() {
    let adapter = MemoryAdapter::builder().display(one_display(1.0)).build();
    let spec = ProcessLaunchSpec {
        app: "/Applications/A.app".into(),
        args: vec![],
        cwd: None,
        env: vec![],
        timeout: Duration::from_secs(5),
        require_confidence: RequireConfidence::Strong,
        require_fresh_surface: false,
    };
    let a = adapter.launch_process(&spec).await.unwrap();
    let b = adapter.launch_process(&spec).await.unwrap();
    assert_ne!(a.surface.id, b.surface.id);
    assert_ne!(a.surface.pid, b.surface.pid);
    assert_ne!(a.surface.cg_window_id, b.surface.cg_window_id);
}

#[tokio::test]
async fn content_rect_derives_from_window_state() {
    let mut b = MemoryAdapter::builder().display(one_display(1.0)).title_bar_h(40.0);
    let id = b.window(WindowSpec::new(100, 1, rect(0.0, 0.0, 1400.0, 900.0)));
    let adapter = b.build();
    let surface = surface_info(100, 1, id);

    let cr = adapter.content_rect(&surface).await.unwrap();
    assert_eq!(cr.rect, rect(0.0, 40.0, 1400.0, 860.0));
    assert_eq!(cr.role, "AXScrollArea");
    assert!(matches!(cr.descent, Descent::Contents));
}

#[tokio::test]
async fn content_rect_overrides_apply() {
    let mut b = MemoryAdapter::builder().display(one_display(1.0));
    let id = b.window(
        WindowSpec::new(100, 1, rect(0.0, 0.0, 800.0, 600.0))
            .with_role("AXGroup")
            .with_descent(Descent::LargestChild)
            .with_content_rect_override(rect(10.0, 20.0, 100.0, 200.0)),
    );
    let adapter = b.build();
    let surface = surface_info(100, 1, id);

    let cr = adapter.content_rect(&surface).await.unwrap();
    assert_eq!(cr.rect, rect(10.0, 20.0, 100.0, 200.0));
    assert_eq!(cr.role, "AXGroup");
    assert!(matches!(cr.descent, Descent::LargestChild));
}

#[tokio::test]
async fn search_filters_by_app_name() {
    let mut b = MemoryAdapter::builder().display(one_display(1.0));
    let _ = b.window(WindowSpec::new(1, 1, rect(0.0, 0.0, 100.0, 100.0)).with_app_name("Kitty"));
    let _ = b.window(WindowSpec::new(2, 2, rect(0.0, 0.0, 100.0, 100.0)).with_app_name("Ghostty"));
    let adapter = b.build();

    let results = adapter
        .search(&SearchQuery {
            app_name: Some("Kitty".into()),
            ..SearchQuery::default()
        })
        .await
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].app_name.as_deref(), Some("Kitty"));
}

#[tokio::test]
async fn search_frontmost_returns_only_focused() {
    let mut b = MemoryAdapter::builder().display(one_display(1.0));
    let _ = b.window(WindowSpec::new(1, 1, rect(0.0, 0.0, 100.0, 100.0)));
    let _ = b.window(WindowSpec::new(2, 2, rect(0.0, 0.0, 100.0, 100.0)));
    let adapter = b.focus(2).build();

    let results = adapter
        .search(&SearchQuery {
            frontmost: Some(true),
            ..SearchQuery::default()
        })
        .await
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].pid, 2);
}

#[tokio::test]
async fn ensure_system_permission_respects_grant_flag() {
    let granted = MemoryAdapter::builder().accessibility_granted(true).build();
    assert!(granted.ensure_system_permission("accessibility").await.is_ok());

    let denied = MemoryAdapter::builder().accessibility_granted(false).build();
    let err = denied.ensure_system_permission("accessibility").await.unwrap_err();
    assert_eq!(err.code, ErrorCode::SystemPermissionNeeded);
}

#[tokio::test]
async fn wait_returns_condition_tag_on_alive_surface() {
    let mut b = MemoryAdapter::builder().display(one_display(1.0));
    let id = b.window(WindowSpec::new(100, 1, rect(0.0, 0.0, 100.0, 100.0)));
    let adapter = b.build();
    let surface = surface_info(100, 1, id);

    let out = adapter
        .wait(&surface, &WaitCondition::Exists, Instant::now() + Duration::from_secs(5))
        .await
        .unwrap();
    assert_eq!(out.condition, "exists");
}

#[tokio::test]
async fn capabilities_advertise_attention_focused_surface() {
    let adapter = MemoryAdapter::builder().build();
    let caps = adapter.capabilities();
    // Real improvement over InMemoryAdapter: MemoryAdapter can actually
    // resolve focused surface from state, so it advertises the capability.
    assert!(caps.contains(&"attention_focused_surface"));
    assert!(caps.contains(&"content_rect"));
    assert!(caps.contains(&"input_pointer_move"));
}
