#![cfg(any(target_os = "macos", target_os = "linux"))]

use std::{sync::Arc, time::Duration};

use porthole::client::DaemonClient;
use porthole_core::in_memory::InMemoryAdapter;
use porthole_protocol::capture_sessions::{CaptureSessionResponse, CreateCaptureSessionResponse};

/// Synthetic process-boundary check; does not exercise desktop permissions,
/// ScreenCaptureKit or GPU completion. Build Jackstay's SDL viewer first.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires JACKSTAY_VIEWER pointing to a built SDL reference viewer"]
async fn cpu_reference_viewer_reads_host_session_and_disconnects_cleanly() {
    let viewer = std::env::var_os("JACKSTAY_VIEWER").expect("set JACKSTAY_VIEWER to the built executable");
    let directory = tempfile::tempdir_in("/tmp").unwrap();
    let socket = directory.path().join("control");
    let served_socket = socket.clone();
    let server = tokio::spawn(async move { portholed::server::serve(Arc::new(InMemoryAdapter::new()), served_socket).await });
    for _ in 0..200 {
        if socket.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(socket.exists(), "test host did not listen");
    let client = DaemonClient::new(&socket);
    let created: CreateCaptureSessionResponse = client
        .post_json("/capture-sessions/synthetic", &serde_json::json!({}))
        .await
        .unwrap();
    // Run twice: process restart must negotiate a fresh incarnation and leave
    // no old holding reservation that prevents the next viewer from starting.
    // The second process holds each frame while the host keeps publishing.
    for (frames, hold_ms) in [("60", "0"), ("8", "250")] {
        let mut command = tokio::process::Command::new(&viewer);
        command
            .env("SDL_VIDEODRIVER", "dummy")
            .arg("--porthole-socket")
            .arg(&socket)
            .arg("--session-id")
            .arg(&created.session_id)
            .args(["--frames", frames, "--hold-ms", hold_ms])
            .kill_on_drop(true);
        let output = tokio::time::timeout(Duration::from_secs(15), command.output())
            .await
            .unwrap()
            .unwrap();
        assert!(
            output.status.success(),
            "viewer failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains(&format!("acquired_frames={frames}")));
    }
    client
        .delete_empty(&format!("/capture-sessions/{}", created.session_id))
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let session: CaptureSessionResponse = client.get_json(&format!("/capture-sessions/{}", created.session_id)).await.unwrap();
            if session.status == "closed" {
                break;
            }
            assert_eq!(session.status, "draining", "unexpected teardown: {session:?}");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    server.abort();
    assert!(server.await.unwrap_err().is_cancelled());
}
