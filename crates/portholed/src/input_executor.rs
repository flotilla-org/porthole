//! The producer-side executor for a remote input channel.
//!
//! An export that carries input asks portholed to be the jackstay input
//! *executor* for the captured surface. This binds a Unix socket the egress
//! half connects a relayed controller to, runs `jackstay::input::transport`'s
//! `Server` for each connection against one shared `Target`, and drives the
//! events onto porthole's [`InputPipeline`] with press identity. The target
//! admits one controller at a time and issues a cleanup barrier when a
//! controller goes away, which the executor answers by releasing everything
//! the surface still holds. The socket is owner-only and unlinked on drop.

use std::{
    os::unix::{fs::PermissionsExt, net::UnixListener},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use jackstay::input::{CAP_ALL, Config, Event, Geometry, Key, Mode, Operation, Outcome, Position, ScrollUnit, Target, transport::Server};
use porthole_core::{
    CoordUnits, ErrorCode, PortholeError,
    input::{ButtonSpec, ClickButton, KeyStrokeSpec, Modifier, PointerMoveSpec, PressAction, ScrollSpec},
    input_pipeline::InputPipeline,
    surface::SurfaceId,
};

/// Link latency adds to the controller's idle timeout, so give it room beyond
/// the 5 s default.
const IDLE_TIMEOUT: Duration = Duration::from_secs(10);
/// A running executor. Dropping it stops the threads and unlinks the socket.
pub struct InputExecutor {
    path: PathBuf,
    stop: Arc<AtomicBool>,
    accept: Option<std::thread::JoinHandle<()>>,
    poll: Option<std::thread::JoinHandle<()>>,
}

impl InputExecutor {
    /// Binds `path` and starts serving controllers that drive `surface`.
    /// `handle` runs the pipeline's async calls from the executor threads.
    pub fn start(
        path: &Path,
        surface: SurfaceId,
        input: Arc<InputPipeline>,
        handle: tokio::runtime::Handle,
        frame_width: u32,
        frame_height: u32,
    ) -> std::io::Result<Self> {
        if path.exists() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                format!("{}: path exists; another export may own it", path.display()),
            ));
        }
        // The extent is the captured frame in pixels: a controller maps its
        // view of that frame onto the extent, and the events come back in
        // frame pixels, injected in physical units (divided by the surface's
        // scale at the daemon boundary). The frame origin is the window
        // origin, so no content-rect offset is needed.
        let geometry = Geometry {
            revision: 1,
            width: f64::from(frame_width.max(1)),
            height: f64::from(frame_height.max(1)),
        };
        // One mapping, computed once: frame pixels -> window-local points by
        // the capture's own frame-to-window ratio (not the display scale).
        // A controller sends coordinates in the frame it renders; the window
        // may be sampled at a different scale than the display it is on.
        let ratio = frame_to_window_ratio(&handle, &input, &surface, frame_width, frame_height);
        let target = Target::new(Config {
            modes: Mode::Cooperative.bit() | Mode::Physical.bit(),
            capabilities: CAP_ALL,
            idle_timeout: IDLE_TIMEOUT,
            geometry,
            ..Config::default()
        })
        .map_err(|e| std::io::Error::other(format!("input target: {e:?}")))?;

        let listener = UnixListener::bind(path)?;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        listener.set_nonblocking(true)?;
        let stop = Arc::new(AtomicBool::new(false));

        let accept = {
            let target = target.clone();
            let stop = stop.clone();
            let (input, surface, handle) = (input.clone(), surface.clone(), handle.clone());
            std::thread::Builder::new()
                .name("porthole-input-accept".into())
                .spawn(move || accept_loop(listener, target, input, surface, handle, stop))?
        };
        let poll = {
            let stop = stop.clone();
            std::thread::Builder::new()
                .name("porthole-input-exec".into())
                .spawn(move || poll_loop(target, input, surface, handle, ratio, stop))?
        };
        Ok(Self {
            path: path.to_path_buf(),
            stop,
            accept: Some(accept),
            poll: Some(poll),
        })
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for InputExecutor {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.accept.take() {
            let _ = t.join();
        }
        if let Some(t) = self.poll.take() {
            let _ = t.join();
        }
        let _ = std::fs::remove_file(&self.path);
    }
}

/// The frame-to-window ratio (window logical size / frame pixel size), so a
/// frame coordinate maps to a window-local point. Falls back to 1:1 when the
/// window size cannot be read.
fn frame_to_window_ratio(
    handle: &tokio::runtime::Handle,
    input: &Arc<InputPipeline>,
    surface: &SurfaceId,
    frame_width: u32,
    frame_height: u32,
) -> (f64, f64) {
    let (handle, input, surface) = (handle.clone(), input.clone(), surface.clone());
    let size = std::thread::scope(|s| s.spawn(|| handle.block_on(input.window_logical_size(&surface))).join().ok());
    match size {
        Some(Ok((w, h))) if w > 0.0 && h > 0.0 && frame_width > 0 && frame_height > 0 => {
            (w / f64::from(frame_width), h / f64::from(frame_height))
        }
        _ => (1.0, 1.0),
    }
}

fn accept_loop(
    listener: UnixListener,
    target: Target,
    input: Arc<InputPipeline>,
    surface: SurfaceId,
    handle: tokio::runtime::Handle,
    stop: Arc<AtomicBool>,
) {
    let mut servers: Vec<Server> = Vec::new();
    while !stop.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok((stream, _)) => {
                // The egress connects a blocking socket; the transport manages
                // its own nonblocking mode, but guard the macOS accept-inherit.
                let _ = stream.set_nonblocking(false);
                match Server::start(target.clone(), stream) {
                    Ok(server) => {
                        // Focus the surface once for this controller session;
                        // per-event injection then skips the (~1 s) focus. A
                        // focus failure is non-fatal — the events still post.
                        if let Err(e) = handle.block_on(input.begin_drive(&surface)) {
                            eprintln!("input executor: begin_drive: {e:?}");
                        }
                        servers.push(server);
                    }
                    Err(e) => eprintln!("input executor: server: {e}"),
                }
                servers.retain(|s| !s.finished());
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                servers.retain(|s| !s.finished());
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(e) => {
                eprintln!("input executor: accept: {e}");
                break;
            }
        }
    }
}

fn poll_loop(
    target: Target,
    input: Arc<InputPipeline>,
    surface: SurfaceId,
    handle: tokio::runtime::Handle,
    ratio: (f64, f64),
    stop: Arc<AtomicBool>,
) {
    // The extent is seeded once at start and deliberately not polled: a
    // geometry change ends a controller's pointer gesture, and the surface's
    // content rect twitches when the app reacts to input, so polling it would
    // cancel the very gesture that caused the twitch. A real resize should
    // drive set_geometry from the capture session's frame size; that is the
    // follow-up the design note records.
    while !stop.load(Ordering::Relaxed) {
        let mut idle = true;
        while let Some(work) = target.next() {
            idle = false;
            let outcome = match work.operation {
                Operation::Event(event) => execute(&handle, &input, &surface, event, ratio),
                Operation::Cleanup { .. } => {
                    let _ = handle.block_on(input.release_held(&surface));
                    // The controller is gone; per-event focus resumes for any
                    // later CLI use of this surface.
                    handle.block_on(input.end_drive(&surface));
                    Outcome::Executed
                }
            };
            if let Err(e) = target.complete(work.id, outcome) {
                eprintln!("input executor: complete: {e:?}");
            }
        }
        if idle {
            std::thread::sleep(Duration::from_millis(2));
        }
    }
}

fn execute(handle: &tokio::runtime::Handle, input: &Arc<InputPipeline>, surface: &SurfaceId, event: Event, ratio: (f64, f64)) -> Outcome {
    // Map a frame coordinate onto a window-local point.
    let map = |x: f64, y: f64| (x * ratio.0, y * ratio.1);
    let result: Result<(), PortholeError> = match event {
        Event::Key {
            press,
            action,
            key,
            modifiers,
        } => match key {
            Key::Physical(code) => handle.block_on(input.key_stroke(
                surface,
                &KeyStrokeSpec {
                    press,
                    action: press_action(action),
                    key: code,
                    modifiers: modifier_list(modifiers),
                },
            )),
            // Logical keys have no macOS mapping yet; katzensteg sends these.
            Key::Logical(_) => return Outcome::Unsupported,
        },
        Event::Text(text) => handle.block_on(input.text(surface, &text)),
        Event::Motion(Position { x, y, .. }) => {
            let (x, y) = map(x, y);
            handle.block_on(input.pointer_move(surface, &PointerMoveSpec { x, y }, CoordUnits::Logical))
        }
        Event::Button { button, action, position } => {
            // Same mapping as Motion/Scroll: a frame coordinate maps to a
            // window-local point by the ratio, injected as logical. Passing
            // the raw frame coordinate as physical would double-count scale
            // whenever the capture frame is not at display resolution.
            let (x, y) = map(position.x, position.y);
            handle.block_on(input.button(
                surface,
                &ButtonSpec {
                    x,
                    y,
                    button: click_button(button),
                    action: press_action(action),
                    modifiers: Vec::new(),
                },
                CoordUnits::Logical,
            ))
        }
        Event::Scroll { x, y, unit, position } => {
            let (delta_x, delta_y) = scroll_lines(x, y, unit);
            let (px, py) = map(position.x, position.y);
            handle.block_on(input.scroll(
                surface,
                &ScrollSpec {
                    x: px,
                    y: py,
                    delta_x,
                    delta_y,
                },
                CoordUnits::Logical,
            ))
        }
    };
    match result {
        Ok(()) => Outcome::Executed,
        Err(e) if e.code == ErrorCode::AdapterUnsupported => Outcome::Unsupported,
        Err(_) => Outcome::Rejected,
    }
}

fn press_action(action: jackstay::input::Action) -> PressAction {
    match action {
        jackstay::input::Action::Down => PressAction::Down,
        jackstay::input::Action::Up => PressAction::Up,
        jackstay::input::Action::Repeat => PressAction::Repeat,
    }
}

fn click_button(button: u32) -> ClickButton {
    match button {
        2 => ClickButton::Right,
        3 => ClickButton::Middle,
        _ => ClickButton::Left,
    }
}

fn modifier_list(modifiers: u32) -> Vec<Modifier> {
    // Bit values from jackstay_input.h: Shift 1, Control 2, Alt 4, Super 8.
    let mut out = Vec::new();
    if modifiers & 1 != 0 {
        out.push(Modifier::Shift);
    }
    if modifiers & 2 != 0 {
        out.push(Modifier::Ctrl);
    }
    if modifiers & 4 != 0 {
        out.push(Modifier::Alt);
    }
    if modifiers & 8 != 0 {
        out.push(Modifier::Cmd);
    }
    out
}

fn scroll_lines(x: f64, y: f64, unit: ScrollUnit) -> (f64, f64) {
    // porthole scrolls in wheel lines; approximate other units.
    let scale = match unit {
        ScrollUnit::Line => 1.0,
        ScrollUnit::Pixel => 1.0 / 10.0,
        ScrollUnit::Page => 3.0,
    };
    (x * scale, y * scale)
}

#[cfg(test)]
mod tests {
    use std::{os::unix::net::UnixStream, time::Instant};

    use jackstay::input::{Action, Event, Key, Mode, Position, transport::Client};
    use porthole_core::{handle::HandleStore, in_memory::InMemoryAdapter, surface::SurfaceInfo};

    use super::*;

    fn wait<T>(mut f: impl FnMut() -> Option<T>) -> T {
        let start = Instant::now();
        loop {
            if let Some(v) = f() {
                return v;
            }
            assert!(start.elapsed() < Duration::from_secs(3), "timed out");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn button_numbers_follow_the_canonical_scheme() {
        // The SDL reference viewer sends primary=1, secondary=2, auxiliary=3.
        assert_eq!(click_button(1), ClickButton::Left);
        assert_eq!(click_button(2), ClickButton::Right);
        assert_eq!(click_button(3), ClickButton::Middle);
        assert_eq!(click_button(9), ClickButton::Left);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn events_reach_the_pipeline_with_press_identity() {
        let adapter = std::sync::Arc::new(InMemoryAdapter::new());
        let handles = HandleStore::new();
        let info = SurfaceInfo::window(SurfaceId::new(), 1);
        let surface = info.id.clone();
        handles.insert(info).await;
        let input = Arc::new(InputPipeline::new(adapter.clone(), handles));

        let dir = std::env::temp_dir().join(format!("porthole-exec-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("i");
        // Frame is 800x600 pixels; the in-memory display is scale 1, so frame
        // pixels inject as the same window-local logical points.
        let executor = InputExecutor::start(&path, surface.clone(), input, tokio::runtime::Handle::current(), 800, 600).unwrap();

        // Drive the executor from a blocking thread: the jackstay Client is
        // synchronous and would otherwise stall the async test.
        let recorded = tokio::task::spawn_blocking(move || {
            let client = Client::connect(UnixStream::connect(&path).unwrap(), Mode::Cooperative).unwrap();
            let geo = client.welcome().config.geometry;
            assert_eq!((geo.width, geo.height), (800.0, 600.0));
            let pos = Position {
                revision: geo.revision,
                x: 200.0,
                y: 120.0,
            };
            client.send(Event::Motion(pos)).unwrap();
            client
                .send(Event::Button {
                    button: 1,
                    action: Action::Down,
                    position: pos,
                })
                .unwrap();
            client
                .send(Event::Button {
                    button: 1,
                    action: Action::Up,
                    position: pos,
                })
                .unwrap();
            client
                .send(Event::Key {
                    press: 7,
                    action: Action::Down,
                    key: Key::Physical("KeyA".into()),
                    modifiers: 1, // Shift
                })
                .unwrap();
            client.send(Event::Text("hi".into())).unwrap();
            // Every event should complete (executed).
            let mut completed = 0;
            wait(|| {
                while let Some(status) = client.poll() {
                    if let jackstay::input::Status::Completed { .. } = status {
                        completed += 1;
                    }
                }
                (completed >= 5).then_some(())
            });
            client.close();
        });
        recorded.await.unwrap();

        assert_eq!(adapter.pointer_move_calls().await.len(), 1);
        let buttons = adapter.button_calls().await;
        assert_eq!(buttons.len(), 2);
        assert_eq!(buttons[0].1.action, PressAction::Down);
        assert_eq!((buttons[0].1.x, buttons[0].1.y), (200.0, 120.0));
        assert_eq!(buttons[1].1.action, PressAction::Up);
        let keys = adapter.key_stroke_calls().await;
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0].1.press, 7);
        assert_eq!(keys[0].1.key, "KeyA");
        assert_eq!(keys[0].1.modifiers, vec![Modifier::Shift]);
        assert_eq!(adapter.text_calls().await.len(), 1);

        // A controller going away releases what it held.
        let start = Instant::now();
        while adapter.release_held_calls().await.is_empty() {
            assert!(start.elapsed() < Duration::from_secs(3), "no cleanup release");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        drop(executor);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn button_coordinates_go_through_the_frame_ratio() {
        use porthole_core::{DisplayId, GeometrySnapshot, adapter::Rect};

        // Window logical size 400x300 against an 800x600 frame gives ratio 0.5.
        // A button, like Motion and Scroll, must map its frame coordinate by
        // that ratio: frame (200,120) -> window-local (100,60). Passing the raw
        // frame coordinate as physical would instead divide by the display
        // scale and land the click somewhere else whenever frame != display.
        let adapter = std::sync::Arc::new(InMemoryAdapter::new());
        adapter
            .set_next_snapshot_geometry(Ok(GeometrySnapshot {
                display_id: DisplayId::new("ratio-test"),
                display_local: Rect {
                    x: 0.0,
                    y: 0.0,
                    w: 400.0,
                    h: 300.0,
                },
            }))
            .await;
        let handles = HandleStore::new();
        let info = SurfaceInfo::window(SurfaceId::new(), 1);
        let surface = info.id.clone();
        handles.insert(info).await;
        let input = Arc::new(InputPipeline::new(adapter.clone(), handles));

        let dir = std::env::temp_dir().join(format!("porthole-ratio-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("i");
        let executor = InputExecutor::start(&path, surface.clone(), input, tokio::runtime::Handle::current(), 800, 600).unwrap();

        tokio::task::spawn_blocking(move || {
            let client = Client::connect(UnixStream::connect(&path).unwrap(), Mode::Cooperative).unwrap();
            let rev = client.welcome().config.geometry.revision;
            client
                .send(Event::Button {
                    button: 1,
                    action: Action::Down,
                    position: Position {
                        revision: rev,
                        x: 200.0,
                        y: 120.0,
                    },
                })
                .unwrap();
            wait(|| {
                while let Some(status) = client.poll() {
                    if let jackstay::input::Status::Completed { .. } = status {
                        return Some(());
                    }
                }
                None
            });
            client.close();
        })
        .await
        .unwrap();

        let buttons = adapter.button_calls().await;
        assert_eq!(buttons.len(), 1);
        assert_eq!((buttons[0].1.x, buttons[0].1.y), (100.0, 60.0));

        drop(executor);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
