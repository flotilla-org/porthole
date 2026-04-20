#![cfg(target_os = "macos")]

use std::time::Duration;

use porthole_adapter_macos::MacOsAdapter;
use porthole_core::adapter::{Adapter, ProcessLaunchSpec, RequireConfidence};

#[tokio::test]
#[ignore = "requires a real macOS desktop session with Screen Recording permission"]
async fn launch_textedit_and_capture() {
    let adapter = MacOsAdapter::new();
    let spec = ProcessLaunchSpec {
        app: "/System/Applications/TextEdit.app".to_string(),
        args: vec![],
        cwd: None,
        env: vec![],
        timeout: Duration::from_secs(10),
        require_confidence: RequireConfidence::Strong,
    };
    let outcome = adapter.launch_process(&spec).await.expect("launch");
    assert!(outcome.surface.pid.is_some());

    let shot = adapter.screenshot(&outcome.surface).await.expect("screenshot");
    assert!(shot.png_bytes.starts_with(&[0x89, 0x50, 0x4e, 0x47]), "not a PNG");
    assert!(shot.window_bounds_points.w > 0.0);
}
