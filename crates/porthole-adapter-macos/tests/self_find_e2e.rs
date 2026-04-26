#![cfg(target_os = "macos")]

use std::{path::PathBuf, process::Stdio, sync::Arc, time::Duration};

use porthole_core::{
    in_memory::InMemoryAdapter,
    search::Candidate,
    surface::{SurfaceId, SurfaceInfo},
};
use portholed::server::serve;

/// Exercises the full path from `porthole attach --containing-pid $$
/// --frontmost` back to a scripted candidate that matches the test
/// process's own PID. Uses the in-memory adapter so no real macOS
/// desktop is required — this asserts the ancestry walk + CLI flag
/// wiring + daemon round-trip, not the real AX integration.
#[tokio::test]
async fn attach_containing_pid_self_finds_scripted_candidate() {
    let test_pid = std::process::id();

    let tmp = tempfile::tempdir().unwrap();
    let socket = tmp.path().join("porthole.sock");

    let adapter = Arc::new(InMemoryAdapter::new());
    // Script a single candidate whose PID equals this test's own PID.
    // The CLI will include this PID in its ancestry-derived pids list,
    // so the in-memory adapter returning any candidate is sufficient:
    // search doesn't actually filter inside the in-memory adapter
    // (it just returns whatever is scripted).
    let candidate = Candidate {
        ref_: porthole_core::search::encode_ref(test_pid, 42),
        app_name: Some("TestHarness".into()),
        title: Some("self-find".into()),
        pid: test_pid,
        cg_window_id: 42,
    };
    adapter.set_next_search_result(Ok(vec![candidate])).await;

    let mut info = SurfaceInfo::window(SurfaceId::new(), test_pid);
    info.cg_window_id = Some(42);
    info.app_name = Some("TestHarness".into());
    adapter.set_next_window_alive_result(Ok(Some(info))).await;

    let socket_for_serve = socket.clone();
    let adapter_for_serve: Arc<dyn porthole_core::adapter::Adapter> = adapter.clone();
    let server_task = tokio::spawn(async move { serve(adapter_for_serve, socket_for_serve).await });

    for _ in 0..200 {
        if socket.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(socket.exists(), "socket did not appear");

    // CARGO_BIN_EXE_porthole is set by Cargo for integration tests of the
    // crate that owns the binary, but not for external crates. We fall back
    // to resolving the path from CARGO_MANIFEST_DIR / the target directory.
    let cli = std::env::var("CARGO_BIN_EXE_porthole").map(PathBuf::from).unwrap_or_else(|_| {
        // In a workspace, CARGO_MANIFEST_DIR points to this crate.
        // Walk up to the workspace root and then into target/debug.
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        // CARGO_MANIFEST_DIR = .../porthole/crates/porthole-adapter-macos
        // parent() × 2 = .../porthole  (the workspace root)
        let workspace_root = manifest_dir
            .parent() // .../porthole/crates
            .and_then(|p| p.parent()) // .../porthole
            .expect("workspace root");
        workspace_root.join("target").join("debug").join("porthole")
    });
    let runtime_dir = tmp.path().to_path_buf();
    let output = tokio::task::spawn_blocking(move || {
        std::process::Command::new(cli)
            .env("PORTHOLE_RUNTIME_DIR", runtime_dir)
            .args(["attach", "--containing-pid", &test_pid.to_string(), "--frontmost", "--json"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
    })
    .await
    .expect("join")
    .expect("spawn");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "CLI exit {:?}\nstdout: {stdout}\nstderr: {stderr}",
        output.status.code()
    );
    let parsed: serde_json::Value = serde_json::from_str(stdout.trim()).expect("CLI stdout is not JSON");
    assert!(parsed.get("surface_id").is_some(), "response missing surface_id");
    assert_eq!(parsed.get("cg_window_id").and_then(|v| v.as_u64()), Some(42));

    server_task.abort();

    // keep tmp alive through the command run
    drop(tmp);
}
