use jackstay::{
    acquisition::arena::{ArenaConfig, ArenaProducer},
    native::windows::{AdapterInfo, AdapterLuid, capture::DesktopUnavailable},
};

use super::*;

fn status(state: CaptureState, epoch: u64) -> CaptureStatus {
    CaptureStatus {
        state,
        epoch,
        mode: PublicationMode::D3d11 {
            adapter: AdapterInfo {
                luid: AdapterLuid(0x9fe5),
                description: "Test adapter".to_owned(),
                vendor_id: 0,
                device_id: 0,
                software: false,
            },
        },
        size: (320, 200),
        frames_published: 3,
        frames_dropped: 1,
        border_shown: Some(false),
        notes: Vec::new(),
        revision: epoch,
    }
}

fn cpu_publication() -> Publication {
    Publication::Cpu(Arc::new(Mutex::new(
        ArenaProducer::new(ArenaConfig {
            resource_capacity: 2,
            retained_history: 1,
            producer_reserve: 1,
            payload_capacity: 64,
            memory_budget: 1024 * 1024,
            max_incarnations: 2,
            drain_timeout: Duration::from_secs(1),
        })
        .unwrap(),
    )))
}

#[test]
fn request_policy_maps_onto_jackstay_with_porthole_arena_limits() {
    let policy = capture_policy(&CaptureSessionRequest::default(), None).unwrap();
    assert!(policy.cursor);
    assert_eq!(policy.border, BorderPolicy::PreferHidden);
    assert_eq!(policy.min_update_interval, None);
    assert_eq!(policy.output_size, OutputSize::Source);
    assert_eq!(policy.arena.resource_capacity, CAPTURE_RESOURCE_CAPACITY);
    assert_eq!(policy.arena.memory_budget, CAPTURE_MEMORY_BUDGET);
    assert!(policy.desktop.is_none(), "production uses the session desktop monitor");

    let policy = capture_policy(
        &CaptureSessionRequest {
            cursor: Some(false),
            border: Some(CaptureBorderPolicy::RequireHidden),
            min_update_interval_ms: Some(20),
            output: Some(CaptureOutputPolicy::Fixed { width: 640, height: 360 }),
        },
        None,
    )
    .unwrap();
    assert!(!policy.cursor);
    assert_eq!(policy.border, BorderPolicy::RequireHidden);
    assert_eq!(policy.min_update_interval, Some(Duration::from_millis(20)));
    assert_eq!(policy.output_size, OutputSize::Fixed { width: 640, height: 360 });
    let fit = capture_policy(
        &CaptureSessionRequest {
            border: Some(CaptureBorderPolicy::Show),
            output: Some(CaptureOutputPolicy::Fit { width: 800, height: 600 }),
            ..CaptureSessionRequest::default()
        },
        None,
    )
    .unwrap();
    assert_eq!(fit.border, BorderPolicy::Show);
    assert_eq!(
        fit.output_size,
        OutputSize::Fit {
            max_width: 800,
            max_height: 600
        }
    );

    for bad in [
        CaptureSessionRequest {
            min_update_interval_ms: Some(0),
            ..CaptureSessionRequest::default()
        },
        CaptureSessionRequest {
            min_update_interval_ms: Some(MAX_MIN_UPDATE_INTERVAL_MS + 1),
            ..CaptureSessionRequest::default()
        },
        CaptureSessionRequest {
            output: Some(CaptureOutputPolicy::Fit { width: 0, height: 10 }),
            ..CaptureSessionRequest::default()
        },
        CaptureSessionRequest {
            output: Some(CaptureOutputPolicy::Fixed {
                width: 16384,
                height: 16384,
            }),
            ..CaptureSessionRequest::default()
        },
    ] {
        assert!(capture_policy(&bad, None).is_err(), "{bad:?}");
    }
}

#[test]
fn capture_states_map_to_porthole_lifecycle() {
    let mut state = State::default();
    assert_eq!(state.snapshot().status, "starting");
    state.observe(status(CaptureState::Running, 1), None);
    let ready = state.snapshot();
    assert_eq!((ready.status, ready.width, ready.height), ("ready", 320, 200));
    assert!(
        ready
            .message
            .unwrap()
            .contains("epoch=1, publication=d3d11 on adapter 00000000:00009fe5")
    );

    // Lock and RDP disconnect pause and report the desktop unavailable;
    // neither is a failure.
    state.observe(status(CaptureState::Paused(DesktopUnavailable::Locked), 1), None);
    let paused = state.snapshot();
    assert_eq!(paused.status, "paused");
    assert!(paused.message.unwrap().starts_with("desktop unavailable: session locked"));
    state.observe(status(CaptureState::Paused(DesktopUnavailable::Disconnected), 1), None);
    assert!(
        state
            .snapshot()
            .message
            .unwrap()
            .starts_with("desktop unavailable: session disconnected")
    );
    assert!(!state.should_stop());

    state.observe(
        status(
            CaptureState::Recovering {
                reason: "device removed".to_owned(),
            },
            1,
        ),
        None,
    );
    assert_eq!(state.snapshot().status, "recovering");
    state.observe(status(CaptureState::Running, 2), None);
    assert!(state.snapshot().message.unwrap().contains("epoch=2"));
    assert!(!state.should_stop());

    // The window closing fails the session; nothing restarts it.
    let mut closed = status(CaptureState::Closed, 2);
    closed.notes.push("closed: window destroyed (IsWindow)".to_owned());
    state.observe(closed, None);
    assert!(state.should_stop());
    let failed = state.snapshot();
    assert_eq!(failed.status, "failed");
    assert!(
        failed
            .message
            .unwrap()
            .starts_with("captured window closed (closed: window destroyed")
    );
    state.stopped = Some(Instant::now());
    state.maintain();
    assert!(state.drained);
    let retired = state.snapshot();
    assert_eq!(retired.status, "failed");
    assert!(retired.message.unwrap().contains("native resources retired"));
}

#[test]
fn a_host_close_is_not_a_failure() {
    let mut state = State::default();
    state.observe(status(CaptureState::Running, 1), None);
    state.closing = Some(Instant::now());
    assert_eq!(state.snapshot().status, "draining");
    state.observe(status(CaptureState::Stopped, 1), None);
    assert!(state.failure.is_none());
    state.stopped = Some(Instant::now());
    state.maintain();
    assert_eq!(state.snapshot().status, "closed");
    // A capture that reports Closed after the host's own close is not a
    // failure either.
    let mut state = State {
        closing: Some(Instant::now()),
        ..State::default()
    };
    state.observe(status(CaptureState::Closed, 1), None);
    assert!(state.failure.is_none());
}

#[test]
fn attach_requires_this_sessions_token() {
    let line = |session: &str, token: &str| {
        serde_json::to_vec(&WindowsNativeAttachRequest::OpenNativeCapture {
            session_id: session.to_owned(),
            attach_token: token.to_owned(),
        })
        .unwrap()
    };
    assert!(authorize(&line("s1", "ptas_secret"), "s1", "ptas_secret").is_ok());
    assert!(authorize(&line("s1", "ptas_secreT"), "s1", "ptas_secret").is_err());
    assert!(authorize(&line("s1", "ptas_secret_longer"), "s1", "ptas_secret").is_err());
    assert!(authorize(&line("s2", "ptas_secret"), "s1", "ptas_secret").is_err());
    assert!(authorize(b"{\"op\":\"open_native_capture\"}", "s1", "ptas_secret").is_err());
    assert!(authorize(b"not json", "s1", "ptas_secret").is_err());
}

#[test]
fn connections_are_bounded_follow_epochs_and_drain() {
    let mut state = State::default();
    state.observe(status(CaptureState::Running, 1), Some((1, cpu_publication())));
    let mut streams = Vec::new();
    for expected in 1..=MAX_CONNECTIONS as u64 {
        let (stream, peer) = local::pipe_pair().unwrap();
        let (id, epoch, _) = state.open_connection(local::shutdown_handle(&stream).unwrap()).unwrap();
        assert_eq!((id, epoch), (expected, 1));
        streams.push((stream, peer));
    }
    let (extra, _extra_peer) = local::pipe_pair().unwrap();
    let refused = state
        .open_connection(local::shutdown_handle(&extra).unwrap())
        .map(|(id, ..)| id)
        .unwrap_err();
    assert!(refused.contains("connection limit"), "{refused}");

    // Device loss: a new epoch replaces the publication and closes the old
    // epoch's setup connections so their consumers attach again.
    state.observe(status(CaptureState::Running, 2), Some((2, cpu_publication())));
    assert_eq!(state.retired.len(), 1);
    assert!(streams.iter().all(|(stream, _)| !local::is_alive(stream)));
    state.connections.clear();
    let (stream, _peer) = local::pipe_pair().unwrap();
    let (_, epoch, publication) = state.open_connection(local::shutdown_handle(&stream).unwrap()).unwrap();
    assert_eq!(epoch, 2);
    drop(publication);
    state.connections.clear();

    // Window closed: new connections are refused with the reason, and the
    // publications drain once no owner remains.
    state.observe(status(CaptureState::Closed, 2), None);
    let refused = state
        .open_connection(local::shutdown_handle(&stream).unwrap())
        .map(|(id, ..)| id)
        .unwrap_err();
    assert!(refused.starts_with("captured window closed"), "{refused}");
    state.stop_publications();
    state.maintain();
    assert!(state.drained, "{:?}", state.snapshot().message);
}

/// Ignored: needs an interactive, unlocked desktop. It captures only a
/// window it creates, and injects lock, disconnect and device loss through
/// Jackstay's `DesktopMonitor` and `simulate_device_loss`; it never locks
/// the workstation or disconnects RDP.
mod live {
    use std::sync::{
        atomic::{AtomicU32, Ordering},
        mpsc,
    };

    use jackstay::{
        acquisition::arena::{AcquireOutcome, ArenaConsumer},
        native::windows::{AdapterSelection, D3d11Device, SharedFenceHandle, SharedTextureHandle, setup::D3d11SetupClient},
    };
    use porthole_core::{adapter::Adapter, search::SearchQuery};
    use windows::{
        Win32::{
            Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM},
            Graphics::{
                Dwm::{DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND, DwmSetWindowAttribute},
                Gdi::{BeginPaint, CreateSolidBrush, DeleteObject, EndPaint, FillRect, InvalidateRect, PAINTSTRUCT, UpdateWindow},
            },
            UI::{
                HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetThreadDpiAwarenessContext},
                WindowsAndMessaging::{
                    AdjustWindowRectEx, CS_HREDRAW, CS_VREDRAW, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW,
                    GWLP_USERDATA, GetClientRect, GetMessageW, GetWindowLongPtrW, MSG, PostMessageW, PostQuitMessage, RegisterClassW,
                    SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOZORDER, SetWindowLongPtrW, SetWindowPos, ShowWindow,
                    TranslateMessage, WINDOW_EX_STYLE, WM_CLOSE, WM_DESTROY, WM_PAINT, WNDCLASSW, WS_OVERLAPPEDWINDOW, WS_VISIBLE,
                },
            },
        },
        core::{HSTRING, w},
    };

    use super::*;

    const RED: [u8; 3] = [255, 0, 0];
    const GREEN: [u8; 3] = [0, 160, 0];
    const BLUE: [u8; 3] = [0, 0, 255];

    fn colorref(rgb: [u8; 3]) -> COLORREF {
        COLORREF(u32::from(rgb[0]) | (u32::from(rgb[1]) << 8) | (u32::from(rgb[2]) << 16))
    }

    unsafe extern "system" fn procedure(window: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        // SAFETY: standard window procedure calls on this thread's own window.
        unsafe {
            match message {
                WM_PAINT => {
                    let mut paint = PAINTSTRUCT::default();
                    let dc = BeginPaint(window, &mut paint);
                    let mut client = RECT::default();
                    let _ = GetClientRect(window, &mut client);
                    let brush = CreateSolidBrush(COLORREF(GetWindowLongPtrW(window, GWLP_USERDATA) as u32));
                    FillRect(dc, &client, brush);
                    let _ = DeleteObject(brush.into());
                    let _ = EndPaint(window, &paint);
                    LRESULT(0)
                }
                WM_CLOSE => {
                    let _ = DestroyWindow(window);
                    LRESULT(0)
                }
                WM_DESTROY => {
                    PostQuitMessage(0);
                    LRESULT(0)
                }
                _ => DefWindowProcW(window, message, wparam, lparam),
            }
        }
    }

    static CLASS: AtomicU32 = AtomicU32::new(0);

    /// A small window this test owns, shown without activation.
    struct TestWindow {
        window: isize,
        thread: Option<std::thread::JoinHandle<()>>,
    }

    impl TestWindow {
        fn open(title: &str, width: i32, height: i32, rgb: [u8; 3]) -> Self {
            let (sender, receiver) = mpsc::channel();
            let title = HSTRING::from(title);
            // SAFETY: plain Win32 window creation and a message loop, all on
            // this thread.
            let thread = std::thread::spawn(move || unsafe {
                SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
                if CLASS.fetch_add(1, Ordering::SeqCst) == 0 {
                    let class = WNDCLASSW {
                        style: CS_HREDRAW | CS_VREDRAW,
                        lpfnWndProc: Some(procedure),
                        lpszClassName: w!("PortholeNativeCaptureTest"),
                        ..Default::default()
                    };
                    assert_ne!(RegisterClassW(&class), 0);
                }
                let mut frame = RECT {
                    left: 0,
                    top: 0,
                    right: width,
                    bottom: height,
                };
                AdjustWindowRectEx(&mut frame, WS_OVERLAPPEDWINDOW, false, WINDOW_EX_STYLE(0)).unwrap();
                let window = CreateWindowExW(
                    WINDOW_EX_STYLE(0),
                    w!("PortholeNativeCaptureTest"),
                    &title,
                    WS_OVERLAPPEDWINDOW & !WS_VISIBLE,
                    60,
                    60,
                    frame.right - frame.left,
                    frame.bottom - frame.top,
                    None,
                    None,
                    None,
                    None,
                )
                .unwrap();
                SetWindowLongPtrW(window, GWLP_USERDATA, colorref(rgb).0 as isize);
                let corners = DWMWCP_DONOTROUND;
                let _ = DwmSetWindowAttribute(
                    window,
                    DWMWA_WINDOW_CORNER_PREFERENCE,
                    (&raw const corners).cast(),
                    std::mem::size_of_val(&corners) as u32,
                );
                let _ = ShowWindow(window, SW_SHOWNOACTIVATE);
                let _ = UpdateWindow(window);
                sender.send(window.0 as isize).unwrap();
                let mut message = MSG::default();
                while GetMessageW(&mut message, None, 0, 0).as_bool() {
                    let _ = TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            });
            Self {
                window: receiver.recv().unwrap(),
                thread: Some(thread),
            }
        }

        fn hwnd(&self) -> HWND {
            HWND(self.window as *mut _)
        }

        fn set_color(&self, rgb: [u8; 3]) {
            // SAFETY: the window lives until close; both calls are thread-safe.
            unsafe {
                SetWindowLongPtrW(self.hwnd(), GWLP_USERDATA, colorref(rgb).0 as isize);
                let _ = InvalidateRect(Some(self.hwnd()), None, true);
            }
        }

        fn resize(&self, width: i32, height: i32) {
            let mut frame = RECT {
                left: 0,
                top: 0,
                right: width,
                bottom: height,
            };
            // SAFETY: plain Win32 calls on a live window.
            unsafe {
                let previous = SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
                AdjustWindowRectEx(&mut frame, WS_OVERLAPPEDWINDOW, false, WINDOW_EX_STYLE(0)).unwrap();
                SetWindowPos(
                    self.hwnd(),
                    None,
                    0,
                    0,
                    frame.right - frame.left,
                    frame.bottom - frame.top,
                    SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
                )
                .unwrap();
                SetThreadDpiAwarenessContext(previous);
                let _ = InvalidateRect(Some(self.hwnd()), None, true);
            }
        }

        fn close(&mut self) {
            if let Some(thread) = self.thread.take() {
                // SAFETY: posting to a window owned by the loop thread.
                unsafe {
                    let _ = PostMessageW(Some(self.hwnd()), WM_CLOSE, WPARAM(0), LPARAM(0));
                }
                thread.join().unwrap();
            }
        }
    }

    impl Drop for TestWindow {
        fn drop(&mut self) {
            self.close();
        }
    }

    /// Injected desktop availability (Jackstay's `DesktopMonitor`).
    #[derive(Debug, Default)]
    struct InjectedDesktop(Mutex<Option<DesktopUnavailable>>);

    impl DesktopMonitor for InjectedDesktop {
        fn unavailable(&self) -> Option<DesktopUnavailable> {
            *self.0.lock().unwrap()
        }
    }

    /// A consumer over the session's attach endpoint, as a separate client
    /// would use it.
    struct TestConsumer {
        client: D3d11SetupClient,
        consumer: ArenaConsumer,
        device: D3d11Device,
        epoch: u64,
        after: u64,
    }

    impl TestConsumer {
        fn attach(session_id: &str, native: &NativeCaptureInfo) -> Result<Self, porthole::native_capture::OpenError> {
            let opened = porthole::native_capture::open(session_id, native)?;
            assert_eq!(opened.publication, WindowsNativePublication::D3d11);
            // SAFETY: `open` verified the endpoint's server; this process is
            // the sole recipient of its grants.
            let mut client = unsafe { D3d11SetupClient::from_stream(opened.stream) };
            let producer = client.describe().unwrap();
            let device = D3d11Device::new(AdapterSelection::Luid(producer.adapter.luid)).unwrap();
            let consumer = client.attach(1, &device).unwrap();
            Ok(Self {
                client,
                consumer,
                device,
                epoch: opened.epoch,
                after: 0,
            })
        }

        /// The size of the next frame that is `rgb` throughout; None on
        /// timeout or when the publication closes.
        fn frame_of(&mut self, rgb: [u8; 3], timeout: Duration) -> Option<(u32, u32)> {
            let deadline = Instant::now() + timeout;
            while Instant::now() < deadline {
                match self.consumer.acquire_latest(self.after).unwrap() {
                    AcquireOutcome::Frame(frame) => {
                        self.after = frame.cursor();
                        let descriptor = *frame.descriptor();
                        let native = frame.native_resources::<SharedTextureHandle, SharedFenceHandle>().unwrap();
                        let texture = self.device.open_texture(native.surface).unwrap();
                        let ready = self.device.open_fence(native.sync_handle).unwrap();
                        let pixels = self
                            .device
                            .read_pixels(&texture, &[(&ready, descriptor.fence_value)], Duration::from_secs(2))
                            .unwrap();
                        if pixels.chunks_exact(4).all(|pixel| pixel == [rgb[2], rgb[1], rgb[0], 255]) {
                            return Some((descriptor.width, descriptor.height));
                        }
                    }
                    AcquireOutcome::Reconfiguration => {
                        self.client.install_configuration(&mut self.consumer).unwrap();
                    }
                    AcquireOutcome::Closed => return None,
                    _ => std::thread::sleep(Duration::from_millis(10)),
                }
            }
            None
        }
    }

    fn wait_status(registry: &CaptureRegistry, session_id: &str, want: impl Fn(&str, &str) -> bool) -> (String, String) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let response = registry.get_session(session_id).unwrap();
            let message = response.status_message.unwrap_or_default();
            if want(&response.status, &message) {
                return (response.status, message);
            }
            assert!(Instant::now() < deadline, "status stayed {}: {message}", response.status);
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    fn published(message: &str) -> u64 {
        message
            .split("published=")
            .nth(1)
            .and_then(|rest| rest.split(',').next())
            .and_then(|count| count.parse().ok())
            .unwrap()
    }

    fn regex_escape(text: &str) -> String {
        text.chars()
            .flat_map(|c| {
                let escape = "\\.+*?()|[]{}^$#".contains(c);
                escape.then_some('\\').into_iter().chain(Some(c))
            })
            .collect()
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "needs an interactive, unlocked Windows desktop; captures only a window it creates"]
    async fn native_capture_follows_lock_disconnect_device_loss_resize_and_close() {
        let title = format!("Porthole 186 native capture test {}", Uuid::new_v4().simple());
        let mut window = TestWindow::open(&title, 240, 160, RED);
        let adapter = Arc::new(WindowsAdapter::new());
        let candidates = adapter
            .search(&SearchQuery {
                pids: vec![std::process::id()],
                title_pattern: Some(format!("^{}$", regex_escape(&title))),
                ..SearchQuery::default()
            })
            .await
            .unwrap();
        assert_eq!(candidates.len(), 1, "exactly this test's window");
        let surface = adapter
            .surface_alive(candidates[0].pid, &candidates[0].platform_ref)
            .await
            .unwrap()
            .unwrap();
        let desktop = Arc::new(InjectedDesktop::default());
        let registry = CaptureRegistry::disabled();
        let response = create(
            &registry,
            adapter,
            surface,
            AgentId::from("agent_native_capture_test"),
            &CaptureSessionRequest {
                cursor: Some(false),
                ..CaptureSessionRequest::default()
            },
            Some(desktop.clone()),
        )
        .await
        .unwrap();
        let session_id = response.session_id.clone();
        let native = response.native.clone().unwrap();
        assert_eq!(native.transport_kind, NATIVE_ATTACH_TRANSPORT_WINDOWS_LOCAL_ENDPOINT);

        let checks = tokio::task::spawn_blocking(move || {
            let mut consumer = TestConsumer::attach(&session_id, &native).unwrap();
            assert_eq!(consumer.epoch, 1);
            assert_eq!(consumer.frame_of(RED, Duration::from_secs(5)), Some((240, 160)));

            // Lock, then RDP disconnect: paused with the desktop unavailable;
            // frames stop publishing and resume with the desktop.
            for reason in [DesktopUnavailable::Locked, DesktopUnavailable::Disconnected] {
                *desktop.0.lock().unwrap() = Some(reason);
                let (_, message) = wait_status(&registry, &session_id, |status, _| status == "paused");
                assert!(message.starts_with(&format!("desktop unavailable: {reason}")), "{message}");
                let before = published(&message);
                window.set_color(GREEN);
                assert_eq!(
                    consumer.frame_of(GREEN, Duration::from_millis(800)),
                    None,
                    "published while {reason}"
                );
                let (status, message) = wait_status(&registry, &session_id, |_, _| true);
                assert_eq!(status, "paused");
                assert_eq!(published(&message), before, "published while {reason}");
                *desktop.0.lock().unwrap() = None;
                wait_status(&registry, &session_id, |status, _| status == "ready");
                // Jackstay drops frames that arrive while paused without
                // keeping them for resume, so a static window publishes
                // again only when it next repaints (jackstay#47).
                window.set_color(BLUE);
                assert!(
                    consumer.frame_of(BLUE, Duration::from_secs(5)).is_some(),
                    "no frame after {reason} ended"
                );
                window.set_color(RED);
                assert!(consumer.frame_of(RED, Duration::from_secs(5)).is_some());
            }

            // Device loss recovers as a new epoch; the old consumer's
            // publication closes and a new attach gets epoch 2.
            registry
                .inner
                .lock()
                .unwrap()
                .native_holds
                .get(&session_id)
                .unwrap()
                .runtime
                .lock()
                .inject_device_loss = Some("injected by the test".to_owned());
            wait_status(&registry, &session_id, |status, message| {
                status == "ready" && message.contains("epoch=2")
            });
            window.set_color(BLUE);
            assert_eq!(
                consumer.frame_of(BLUE, Duration::from_secs(2)),
                None,
                "the old epoch still published"
            );
            // The old epoch's consumer has seen its publication close; it
            // leaves, so that publication can drain.
            drop(consumer);
            let mut consumer = TestConsumer::attach(&session_id, &native).unwrap();
            assert_eq!(consumer.epoch, 2);
            assert!(consumer.frame_of(BLUE, Duration::from_secs(5)).is_some());

            // Resize: a new pool generation at the new size.
            window.resize(320, 200);
            window.set_color(RED);
            assert_eq!(consumer.frame_of(RED, Duration::from_secs(5)), Some((320, 200)));
            wait_status(&registry, &session_id, |status, _| status == "ready");
            assert_eq!(registry.get_session(&session_id).unwrap().width, 320);

            // Window closed: the session fails, nothing restarts it, and a new
            // consumer is told why.
            window.close();
            let (_, message) = wait_status(&registry, &session_id, |status, _| status == "failed");
            assert!(message.starts_with("captured window closed"), "{message}");
            match TestConsumer::attach(&session_id, &native) {
                Err(porthole::native_capture::OpenError::Rejected(reason)) => {
                    assert!(reason.starts_with("captured window closed"), "{reason}");
                }
                Err(other) => panic!("expected a refusal, got {other}"),
                Ok(_) => panic!("a closed session admitted a consumer"),
            }
            drop(consumer);
            wait_status(&registry, &session_id, |status, message| {
                status == "failed" && message.contains("native resources retired")
            });
            registry.close_session(&session_id).unwrap();
        });
        checks.await.unwrap();
    }
}
