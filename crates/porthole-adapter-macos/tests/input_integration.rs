#![cfg(target_os = "macos")]

use std::time::Duration;

use porthole_adapter_macos::MacOsAdapter;
use porthole_core::adapter::{Adapter, ProcessLaunchSpec, RequireConfidence};
use porthole_core::input::{ClickButton, ClickSpec, KeyEvent};
use porthole_core::wait::WaitCondition;

fn spec_textedit() -> ProcessLaunchSpec {
    ProcessLaunchSpec {
        app: "/System/Applications/TextEdit.app".to_string(),
        args: vec![],
        cwd: None,
        env: vec![],
        timeout: Duration::from_secs(10),
        require_confidence: RequireConfidence::Strong,
    }
}

#[tokio::test]
#[ignore = "requires a real macOS desktop session, Accessibility, and Screen Recording permissions"]
async fn text_types_into_textedit_and_wait_dirty_fires() {
    let adapter = MacOsAdapter::new();
    let outcome = adapter.launch_process(&spec_textedit()).await.expect("launch");
    let surface = outcome.surface;

    // Wait for the editor to be visible/stable first.
    adapter
        .wait(&surface, &WaitCondition::Stable { window_ms: 800, threshold_pct: 1.0 })
        .await
        .expect("initial stable");

    // Type text; expect the frame to go dirty.
    let baseline = adapter.screenshot(&surface).await.expect("baseline");
    adapter.text(&surface, "hello porthole\n").await.expect("text");
    let dirty = adapter
        .wait(&surface, &WaitCondition::Dirty { threshold_pct: 1.0 })
        .await
        .expect("dirty");
    assert_eq!(dirty.condition, "dirty");
    let _ = baseline;

    adapter.close(&surface).await.expect("close");
}

#[tokio::test]
#[ignore = "requires a real macOS desktop session + permissions"]
async fn key_event_triggers_dirty_after_typing() {
    let adapter = MacOsAdapter::new();
    let outcome = adapter.launch_process(&spec_textedit()).await.expect("launch");
    let surface = outcome.surface;

    adapter.text(&surface, "x").await.expect("text");
    // Pressing Enter should cause a frame change.
    adapter
        .key(&surface, &[KeyEvent { key: "Enter".into(), modifiers: vec![] }])
        .await
        .expect("key Enter");
    let dirty = adapter
        .wait(&surface, &WaitCondition::Dirty { threshold_pct: 1.0 })
        .await
        .expect("dirty");
    assert_eq!(dirty.condition, "dirty");

    adapter.close(&surface).await.expect("close");
}

#[tokio::test]
#[ignore = "requires a real macOS desktop session + permissions"]
async fn click_inside_window_is_accepted() {
    let adapter = MacOsAdapter::new();
    let outcome = adapter.launch_process(&spec_textedit()).await.expect("launch");
    let surface = outcome.surface;

    adapter
        .click(&surface, &ClickSpec { x: 100.0, y: 100.0, button: ClickButton::Left, count: 1, modifiers: vec![] })
        .await
        .expect("click");

    adapter.close(&surface).await.expect("close");
}

#[tokio::test]
#[ignore = "requires a real macOS desktop session + permissions"]
async fn attention_and_displays_return_non_empty() {
    let adapter = MacOsAdapter::new();
    let attention = adapter.attention().await.expect("attention");
    let displays = adapter.displays().await.expect("displays");
    assert!(!displays.is_empty(), "displays should list at least one monitor");
    // Cursor position should be inside some display bounds.
    let any_inside = displays.iter().any(|d| {
        attention.cursor.x >= d.bounds.x
            && attention.cursor.x < d.bounds.x + d.bounds.w
            && attention.cursor.y >= d.bounds.y
            && attention.cursor.y < d.bounds.y + d.bounds.h
    });
    assert!(any_inside, "cursor position should fall within some display");
}
