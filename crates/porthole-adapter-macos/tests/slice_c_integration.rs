#![cfg(target_os = "macos")]

use std::{path::PathBuf, time::Duration};

use porthole_adapter_macos::MacOsAdapter;
use porthole_core::{
    adapter::{Adapter, ArtifactLaunchSpec, RequireConfidence},
    display::Rect,
};

fn pdf_spec(path: &str) -> ArtifactLaunchSpec {
    ArtifactLaunchSpec {
        path: PathBuf::from(path),
        require_confidence: RequireConfidence::Plausible,
        require_fresh_surface: false,
        timeout: Duration::from_secs(10),
    }
}

#[tokio::test]
#[ignore = "requires a real macOS desktop session + permissions + a PDF file"]
async fn artifact_launch_pdf_and_screenshot() {
    // Requires /tmp/slice-c-test.pdf to exist. Build your own minimal PDF:
    //   printf "%%PDF-1.0\n%%EOF" > /tmp/slice-c-test.pdf
    let adapter = MacOsAdapter::new();
    let outcome = adapter.launch_artifact(&pdf_spec("/tmp/slice-c-test.pdf")).await.expect("launch");
    let shot = adapter.screenshot(&outcome.surface).await.expect("screenshot");
    assert!(shot.png_bytes.starts_with(&[0x89, 0x50, 0x4e, 0x47]));
    adapter.close(&outcome.surface).await.ok();
}

#[tokio::test]
#[ignore = "requires a real macOS desktop session + permissions"]
async fn place_surface_moves_textedit() {
    use porthole_core::adapter::{Adapter, ProcessLaunchSpec};
    let adapter = MacOsAdapter::new();
    let spec = ProcessLaunchSpec {
        app: "/System/Applications/TextEdit.app".to_string(),
        args: vec![],
        cwd: None,
        env: vec![],
        timeout: Duration::from_secs(10),
        require_confidence: RequireConfidence::Strong,
        require_fresh_surface: false,
    };
    let outcome = adapter.launch_process(&spec).await.expect("launch");
    adapter
        .place_surface(
            &outcome.surface,
            Rect {
                x: 200.0,
                y: 100.0,
                w: 800.0,
                h: 600.0,
            },
        )
        .await
        .expect("place");
    let snap = adapter.snapshot_geometry(&outcome.surface).await.expect("snapshot");
    assert!((snap.display_local.w - 800.0).abs() < 5.0);
    adapter.close(&outcome.surface).await.ok();
}

#[tokio::test]
#[ignore = "requires a real macOS desktop session + permissions"]
async fn snapshot_geometry_returns_display_id() {
    use porthole_core::adapter::{Adapter, ProcessLaunchSpec};
    let adapter = MacOsAdapter::new();
    let spec = ProcessLaunchSpec {
        app: "/System/Applications/TextEdit.app".to_string(),
        args: vec![],
        cwd: None,
        env: vec![],
        timeout: Duration::from_secs(10),
        require_confidence: RequireConfidence::Strong,
        require_fresh_surface: false,
    };
    let outcome = adapter.launch_process(&spec).await.expect("launch");
    let snap = adapter.snapshot_geometry(&outcome.surface).await.expect("snapshot");
    assert!(snap.display_id.as_str().starts_with("disp_"));
    adapter.close(&outcome.surface).await.ok();
}
