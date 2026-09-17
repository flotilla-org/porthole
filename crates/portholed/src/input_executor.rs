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
    time::{Duration, Instant},
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
/// How often the executor re-reads the surface's size to update the extent.
const GEOMETRY_POLL: Duration = Duration::from_secs(1);

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
    pub fn start(path: &Path, surface: SurfaceId, input: Arc<InputPipeline>, handle: tokio::runtime::Handle) -> std::io::Result<Self> {
        if path.exists() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                format!("{}: path exists; another export may own it", path.display()),
            ));
        }
        // Seed the extent from the surface's logical size so a controller maps
        // its window onto the right coordinates from the first frame.
        let geometry = initial_geometry(&handle, &input, &surface);
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
            std::thread::Builder::new()
                .name("porthole-input-accept".into())
                .spawn(move || accept_loop(listener, target, stop))?
        };
        let poll = {
            let stop = stop.clone();
            std::thread::Builder::new()
                .name("porthole-input-exec".into())
                .spawn(move || poll_loop(target, input, surface, handle, geometry, stop))?
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

fn initial_geometry(handle: &tokio::runtime::Handle, input: &Arc<InputPipeline>, surface: &SurfaceId) -> Geometry {
    // Run the async query on a thread of its own: `start` may be called from a
    // runtime worker (a test) or a blocking thread (create_export), and
    // `block_on` must not run on a worker.
    let (handle, input, surface) = (handle.clone(), input.clone(), surface.clone());
    let rect = std::thread::scope(|s| {
        s.spawn(|| handle.block_on(input.content_rect(&surface, CoordUnits::Logical)))
            .join()
            .ok()
    });
    match rect {
        Some(Ok(rect)) if rect.rect.w > 0.0 && rect.rect.h > 0.0 => Geometry {
            revision: 1,
            width: rect.rect.w,
            height: rect.rect.h,
        },
        _ => Geometry {
            revision: 1,
            width: 1.0,
            height: 1.0,
        },
    }
}

fn accept_loop(listener: UnixListener, target: Target, stop: Arc<AtomicBool>) {
    let mut servers: Vec<Server> = Vec::new();
    while !stop.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok((stream, _)) => {
                // The egress connects a blocking socket; the transport manages
                // its own nonblocking mode, but guard the macOS accept-inherit.
                let _ = stream.set_nonblocking(false);
                match Server::start(target.clone(), stream) {
                    Ok(server) => servers.push(server),
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
    mut geometry: Geometry,
    stop: Arc<AtomicBool>,
) {
    let mut last_geometry = Instant::now();
    while !stop.load(Ordering::Relaxed) {
        let mut idle = true;
        while let Some(work) = target.next() {
            idle = false;
            let outcome = match work.operation {
                Operation::Event(event) => execute(&handle, &input, &surface, event),
                Operation::Cleanup { .. } => {
                    let _ = handle.block_on(input.release_held(&surface));
                    Outcome::Executed
                }
            };
            if let Err(e) = target.complete(work.id, outcome) {
                eprintln!("input executor: complete: {e:?}");
            }
        }
        // Follow the surface's size: a controller learns the new extent as a
        // reset. Only bumped while a controller could be attached.
        if last_geometry.elapsed() >= GEOMETRY_POLL {
            last_geometry = Instant::now();
            if let Ok(rect) = handle.block_on(input.content_rect(&surface, CoordUnits::Logical)) {
                if rect.rect.w > 0.0 && rect.rect.h > 0.0 && (rect.rect.w != geometry.width || rect.rect.h != geometry.height) {
                    geometry = Geometry {
                        revision: geometry.revision + 1,
                        width: rect.rect.w,
                        height: rect.rect.h,
                    };
                    let _ = target.set_geometry(geometry);
                }
            }
        }
        if idle {
            std::thread::sleep(Duration::from_millis(2));
        }
    }
}

fn execute(handle: &tokio::runtime::Handle, input: &Arc<InputPipeline>, surface: &SurfaceId, event: Event) -> Outcome {
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
            handle.block_on(input.pointer_move(surface, &PointerMoveSpec { x, y }, CoordUnits::Logical))
        }
        Event::Button { button, action, position } => handle.block_on(input.button(
            surface,
            &ButtonSpec {
                x: position.x,
                y: position.y,
                button: click_button(button),
                action: press_action(action),
                modifiers: Vec::new(),
            },
            CoordUnits::Logical,
        )),
        Event::Scroll { x, y, unit, position } => {
            let (delta_x, delta_y) = scroll_lines(x, y, unit);
            handle.block_on(input.scroll(
                surface,
                &ScrollSpec {
                    x: position.x,
                    y: position.y,
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
    use std::os::unix::net::UnixStream;

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
        let executor = InputExecutor::start(&path, surface.clone(), input, tokio::runtime::Handle::current()).unwrap();

        // Drive the executor from a blocking thread: the jackstay Client is
        // synchronous and would otherwise stall the async test.
        let recorded = tokio::task::spawn_blocking(move || {
            let client = Client::connect(UnixStream::connect(&path).unwrap(), Mode::Cooperative).unwrap();
            let geo = client.welcome().config.geometry;
            let pos = Position {
                revision: geo.revision,
                x: 100.0,
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
}
