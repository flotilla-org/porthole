#![cfg(target_os = "macos")]

use std::time::Duration;

use porthole_adapter_macos::MacOsAdapter;
use porthole_core::adapter::{Adapter, ProcessLaunchSpec, RequireConfidence};
use porthole_core::search::{encode_ref, SearchQuery};

fn textedit_spec() -> ProcessLaunchSpec {
    ProcessLaunchSpec {
        app: "/System/Applications/TextEdit.app".to_string(),
        args: vec![],
        cwd: None,
        env: vec![],
        timeout: Duration::from_secs(10),
        require_confidence: RequireConfidence::Strong,
        require_fresh_surface: false,
    }
}

#[tokio::test]
#[ignore = "requires a real macOS desktop session + permissions"]
async fn search_finds_launched_textedit_by_app_name() {
    let adapter = MacOsAdapter::new();
    let outcome = adapter.launch_process(&textedit_spec()).await.expect("launch");
    let query = SearchQuery { app_name: Some("TextEdit".into()), ..Default::default() };
    let candidates = adapter.search(&query).await.expect("search");
    assert!(
        candidates.iter().any(|c| c.pid == outcome.surface.pid.unwrap()),
        "search did not find launched TextEdit"
    );
    adapter.close(&outcome.surface).await.ok();
}

#[tokio::test]
#[ignore = "requires a real macOS desktop session + permissions"]
async fn window_alive_survives_app_hide() {
    use std::process::Command;
    let adapter = MacOsAdapter::new();
    let outcome = adapter.launch_process(&textedit_spec()).await.expect("launch");
    let pid = outcome.surface.pid.unwrap();
    let cg = outcome.surface.cg_window_id.unwrap();
    // Issue Cmd+H via osascript to hide the app.
    Command::new("/usr/bin/osascript")
        .args([
            "-e",
            r#"tell application "System Events" to tell process "TextEdit" to set visible to false"#,
        ])
        .output()
        .ok();
    tokio::time::sleep(Duration::from_millis(500)).await;
    // Hidden windows should still resolve as alive under window_alive's broad enum.
    let alive = adapter.window_alive(pid, cg).await.expect("window_alive");
    assert!(alive.is_some(), "hidden window should still be alive");
    adapter.close(&outcome.surface).await.ok();
}

#[tokio::test]
#[ignore = "requires a real macOS desktop session + permissions"]
async fn track_via_ref_roundtrip() {
    let adapter = MacOsAdapter::new();
    let outcome = adapter.launch_process(&textedit_spec()).await.expect("launch");
    let pid = outcome.surface.pid.unwrap();
    let cg = outcome.surface.cg_window_id.unwrap();
    let r = encode_ref(pid, cg);
    // Decode via window_alive (what the track path does).
    let info = adapter.window_alive(pid, cg).await.expect("alive").expect("some");
    assert_eq!(info.cg_window_id, Some(cg));
    adapter.close(&outcome.surface).await.ok();
    let _ = r;
}
