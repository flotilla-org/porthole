//! A live check that the input executor drives a real window and drains a
//! stream of events promptly, i.e. without re-focusing per event. Ignored:
//! needs a macOS desktop session with Accessibility granted to the test host.
#![cfg(all(target_os = "macos", unix))]

use std::{
    os::unix::net::UnixStream,
    sync::Arc,
    time::{Duration, Instant},
};

use jackstay::input::{Action, Event, Key, Mode, Position, transport::Client};
use porthole_adapter_macos::MacOsAdapter;
use porthole_core::{
    adapter::{Adapter, ProcessLaunchSpec, RequireConfidence},
    handle::HandleStore,
    input_pipeline::InputPipeline,
};
use portholed::input_executor::InputExecutor;

fn textedit_spec() -> ProcessLaunchSpec {
    ProcessLaunchSpec {
        app: "/System/Applications/TextEdit.app".to_string(),
        args: vec![],
        cwd: None,
        env: vec![],
        timeout: Duration::from_secs(10),
        require_confidence: RequireConfidence::Strong,
        require_fresh_surface: true,
        force_place: false,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires a real macOS desktop session + Accessibility"]
async fn a_stream_of_events_drains_without_per_event_focus() {
    let adapter = Arc::new(MacOsAdapter::new());
    let outcome = adapter.launch_process(&textedit_spec()).await.expect("launch");
    let info = outcome.surface;
    let surface = info.id.clone();
    let handles = HandleStore::new();
    handles.insert(info.clone()).await;
    let input = Arc::new(InputPipeline::new(adapter.clone(), handles));
    // Frame dimensions equal to the window's logical size, so the ratio is 1
    // and coordinates map straight onto the window.
    let (w, h) = input.window_logical_size(&surface).await.expect("window size");
    let (fw, fh) = (w as u32, h as u32);

    let dir = std::env::temp_dir().join(format!("porthole-exec-live-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("i");
    let _executor = InputExecutor::start(&path, surface.clone(), input, tokio::runtime::Handle::current(), fw, fh).unwrap();

    let (elapsed, completed) = tokio::task::spawn_blocking(move || {
        let client = Client::connect(UnixStream::connect(&path).unwrap(), Mode::Cooperative).unwrap();
        let rev = client.welcome().config.geometry.revision;
        let pos = |x: f64, y: f64| Position { revision: rev, x, y };
        // Focus once with a button down at the centre, then stream 30 motions
        // with no pause: with per-event focus this would take ~30 s; without,
        // it is a handful of milliseconds each.
        client
            .send(Event::Button {
                button: 1,
                action: Action::Down,
                position: pos(w / 2.0, h / 2.0),
            })
            .unwrap();
        client
            .send(Event::Button {
                button: 1,
                action: Action::Up,
                position: pos(w / 2.0, h / 2.0),
            })
            .unwrap();
        let start = Instant::now();
        for i in 0..30 {
            let x = 40.0 + (i as f64) * (w - 80.0) / 30.0;
            client.send(Event::Motion(pos(x, h / 2.0))).unwrap();
        }
        // Also a key with press identity, to exercise that path.
        client
            .send(Event::Key {
                press: 1,
                action: Action::Down,
                key: Key::Physical("KeyH".into()),
                modifiers: 0,
            })
            .unwrap();
        client
            .send(Event::Key {
                press: 1,
                action: Action::Up,
                key: Key::Physical("KeyH".into()),
                modifiers: 0,
            })
            .unwrap();
        let mut completed = 0;
        let deadline = Instant::now() + Duration::from_secs(20);
        while completed < 34 && Instant::now() < deadline {
            while let Some(status) = client.poll() {
                if let jackstay::input::Status::Completed { .. } = status {
                    completed += 1;
                }
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let elapsed = start.elapsed();
        client.close();
        (elapsed, completed)
    })
    .await
    .unwrap();

    let _ = adapter.close(&info).await;
    let _ = std::fs::remove_dir_all(&dir);

    assert_eq!(completed, 34, "every event should execute");
    // 34 events, well under a second without per-event focus; the old
    // per-event focus took roughly a second each.
    assert!(
        elapsed < Duration::from_secs(5),
        "stream took {elapsed:?}; per-event focus regressed"
    );
}
