//! Test-owned Win32 window for native capture checks (#186). It paints its
//! whole client area one solid colour, cycling red, green and blue, so a
//! consumer can verify frames pixel by pixel. It is shown without activation
//! (it never takes the keyboard focus), can resize its own client area once
//! after a delay (the Windows adapter has no placement yet), and exits on
//! WM_CLOSE or after `--max-seconds`.
//!
//! With `--log PATH` (#189) it appends one JSON line per event, each with wall
//! time (`wall`, Unix seconds) and QPC time (`qpc_ns`, the clock of a WGC
//! frame's `SystemRelativeTime`), so captured frames can be matched against
//! what was on screen: `start`; `cycle` (the colour changed and the window was
//! invalidated); `paint` (colour, client size and painted rectangle); `size`,
//! `move`, `dpi` and `display` changes; `session` (WTS lock, unlock, remote
//! connect or disconnect as this window was told); `cursor` (the pointer
//! entered, moved within or left the window rectangle, polled every 50 ms,
//! since a captured cursor is drawn into the frame); and `exit`.
//!
//! ```text
//! capture_fixture [--width 320] [--height 200] [--resize-after-ms 6000 --resize 480x300]
//!                 [--cycle-ms 1000] [--max-seconds 600] [--log fixture-events.jsonl]
//! ```
#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(not(windows))]
fn main() {}

#[cfg(windows)]
fn main() {
    fixture::main();
}

#[cfg(windows)]
mod fixture {
    use std::{
        fs::File,
        io::Write,
        ptr::null_mut,
        sync::{
            Mutex,
            atomic::{AtomicUsize, Ordering},
        },
        time::{SystemTime, UNIX_EPOCH},
    };

    use windows_sys::Win32::{
        Foundation::*,
        Graphics::{
            Dwm::{DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND, DwmSetWindowAttribute},
            Gdi::*,
        },
        System::{
            Performance::{QueryPerformanceCounter, QueryPerformanceFrequency},
            RemoteDesktop::{NOTIFY_FOR_THIS_SESSION, WTSRegisterSessionNotification, WTSUnRegisterSessionNotification},
        },
        UI::{HiDpi::*, WindowsAndMessaging::*},
    };

    /// Red, green, blue as `0x00BBGGRR`.
    const COLORS: [u32; 3] = [0x0000_00ff, 0x0000_a000, 0x00ff_0000];
    const COLOR_NAMES: [&str; 3] = ["red", "green", "blue"];
    const CYCLE_TIMER: usize = 1;
    const RESIZE_TIMER: usize = 2;
    const EXIT_TIMER: usize = 3;
    const CURSOR_TIMER: usize = 4;

    static COLOR: AtomicUsize = AtomicUsize::new(0);
    static RESIZE: AtomicUsize = AtomicUsize::new(0);
    static LOG: Mutex<Option<File>> = Mutex::new(None);
    /// The last logged cursor position relative to the window rectangle, or
    /// `None` while outside it.
    static CURSOR: Mutex<Option<(i32, i32)>> = Mutex::new(None);

    fn qpc_ns() -> u64 {
        let (mut counter, mut frequency) = (0i64, 0i64);
        // SAFETY: both out pointers are live locals.
        unsafe {
            QueryPerformanceCounter(&mut counter);
            QueryPerformanceFrequency(&mut frequency);
        }
        if counter <= 0 || frequency <= 0 {
            return 0;
        }
        (counter as u128 * 1_000_000_000 / frequency as u128) as u64
    }

    /// Append `{"event": ..., "wall": ..., "qpc_ns": ..., <fields>}`; `fields`
    /// is already-formatted JSON members (numbers and plain strings only).
    fn event(name: &str, fields: &str) {
        let Ok(mut log) = LOG.lock() else { return };
        let Some(file) = log.as_mut() else { return };
        let wall = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0.0, |time| time.as_secs_f64());
        let separator = if fields.is_empty() { "" } else { ", " };
        let _ = writeln!(
            file,
            "{{\"event\": \"{name}\", \"wall\": {wall:.6}, \"qpc_ns\": {}{separator}{fields}}}",
            qpc_ns()
        );
        let _ = file.flush();
    }

    fn color_name() -> &'static str {
        COLOR_NAMES[COLOR.load(Ordering::SeqCst) % COLORS.len()]
    }

    fn session_change(code: u32) -> &'static str {
        match code {
            WTS_CONSOLE_CONNECT => "console_connect",
            WTS_CONSOLE_DISCONNECT => "console_disconnect",
            WTS_REMOTE_CONNECT => "remote_connect",
            WTS_REMOTE_DISCONNECT => "remote_disconnect",
            WTS_SESSION_LOGON => "logon",
            WTS_SESSION_LOGOFF => "logoff",
            WTS_SESSION_LOCK => "lock",
            WTS_SESSION_UNLOCK => "unlock",
            WTS_SESSION_REMOTE_CONTROL => "remote_control",
            _ => "other",
        }
    }

    /// Log the pointer entering, moving within or leaving the window
    /// rectangle (including its frame: WGC captures the frame bounds).
    ///
    /// SAFETY: `hwnd` is this thread's live window.
    unsafe fn poll_cursor(hwnd: HWND) {
        let mut point = POINT { x: 0, y: 0 };
        let mut window = RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        // SAFETY: out pointers are live locals.
        let known = unsafe { GetCursorPos(&mut point) != 0 && GetWindowRect(hwnd, &mut window) != 0 };
        let inside = known && point.x >= window.left && point.x < window.right && point.y >= window.top && point.y < window.bottom;
        let now = inside.then_some((point.x - window.left, point.y - window.top));
        let Ok(mut last) = CURSOR.lock() else { return };
        if *last == now {
            return;
        }
        let mut client = point;
        // SAFETY: out pointer is a live local.
        unsafe { ScreenToClient(hwnd, &mut client) };
        let state = match (*last, now) {
            (None, Some(_)) => "enter",
            (Some(_), None) => "leave",
            _ => "move",
        };
        *last = now;
        drop(last);
        event(
            "cursor",
            &format!(
                "\"state\": \"{state}\", \"screen\": [{}, {}], \"client\": [{}, {}], \"window\": [{}, {}, {}, {}]",
                point.x, point.y, client.x, client.y, window.left, window.top, window.right, window.bottom
            ),
        );
    }

    fn outer_size(width: i32, height: i32) -> (i32, i32) {
        let mut frame = RECT {
            left: 0,
            top: 0,
            right: width,
            bottom: height,
        };
        // SAFETY: frame is a live local.
        unsafe { AdjustWindowRectEx(&mut frame, WS_OVERLAPPEDWINDOW, 0, 0) };
        (frame.right - frame.left, frame.bottom - frame.top)
    }

    unsafe extern "system" fn procedure(hwnd: HWND, message: u32, w: WPARAM, l: LPARAM) -> LRESULT {
        // SAFETY: standard window procedure calls on this thread's own window.
        unsafe {
            match message {
                WM_PAINT => {
                    let mut paint: PAINTSTRUCT = std::mem::zeroed();
                    let dc = BeginPaint(hwnd, &mut paint);
                    let mut client: RECT = std::mem::zeroed();
                    GetClientRect(hwnd, &mut client);
                    let brush = CreateSolidBrush(COLORS[COLOR.load(Ordering::SeqCst) % COLORS.len()]);
                    FillRect(dc, &client, brush);
                    DeleteObject(brush);
                    EndPaint(hwnd, &paint);
                    let dirty = paint.rcPaint;
                    event(
                        "paint",
                        &format!(
                            "\"colour\": \"{}\", \"client\": [{}, {}], \"dirty\": [{}, {}, {}, {}]",
                            color_name(),
                            client.right - client.left,
                            client.bottom - client.top,
                            dirty.left,
                            dirty.top,
                            dirty.right,
                            dirty.bottom
                        ),
                    );
                    0
                }
                WM_TIMER if w == CYCLE_TIMER => {
                    COLOR.fetch_add(1, Ordering::SeqCst);
                    event("cycle", &format!("\"colour\": \"{}\"", color_name()));
                    InvalidateRect(hwnd, null_mut(), 0);
                    0
                }
                WM_TIMER if w == CURSOR_TIMER => {
                    poll_cursor(hwnd);
                    0
                }
                WM_TIMER if w == RESIZE_TIMER => {
                    KillTimer(hwnd, RESIZE_TIMER);
                    let packed = RESIZE.load(Ordering::SeqCst);
                    let (width, height) = outer_size((packed >> 16) as i32, (packed & 0xffff) as i32);
                    event("resize_request", &format!("\"outer\": [{width}, {height}]"));
                    SetWindowPos(hwnd, null_mut(), 0, 0, width, height, SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE);
                    InvalidateRect(hwnd, null_mut(), 0);
                    0
                }
                WM_TIMER if w == EXIT_TIMER => {
                    event("exit", "\"reason\": \"max-seconds\"");
                    DestroyWindow(hwnd);
                    0
                }
                WM_SIZE => {
                    event(
                        "size",
                        &format!(
                            "\"kind\": {}, \"client\": [{}, {}]",
                            w,
                            l as u32 & 0xffff,
                            (l as u32 >> 16) & 0xffff
                        ),
                    );
                    DefWindowProcW(hwnd, message, w, l)
                }
                WM_MOVE => {
                    event(
                        "move",
                        &format!(
                            "\"client_origin\": [{}, {}]",
                            (l as u32 & 0xffff) as i16,
                            ((l as u32 >> 16) & 0xffff) as i16
                        ),
                    );
                    DefWindowProcW(hwnd, message, w, l)
                }
                WM_DPICHANGED => {
                    event("dpi", &format!("\"dpi\": {}", w & 0xffff));
                    DefWindowProcW(hwnd, message, w, l)
                }
                WM_DISPLAYCHANGE => {
                    event(
                        "display",
                        &format!("\"bpp\": {}, \"size\": [{}, {}]", w, l as u32 & 0xffff, (l as u32 >> 16) & 0xffff),
                    );
                    DefWindowProcW(hwnd, message, w, l)
                }
                WM_WTSSESSION_CHANGE => {
                    event(
                        "session",
                        &format!("\"change\": \"{}\", \"code\": {}, \"session\": {}", session_change(w as u32), w, l),
                    );
                    0
                }
                WM_CLOSE => {
                    event("exit", "\"reason\": \"WM_CLOSE\"");
                    DestroyWindow(hwnd);
                    0
                }
                WM_DESTROY => {
                    WTSUnRegisterSessionNotification(hwnd);
                    PostQuitMessage(0);
                    0
                }
                _ => DefWindowProcW(hwnd, message, w, l),
            }
        }
    }

    fn dimensions(value: &str) -> (u32, u32) {
        let (width, height) = value.split_once('x').expect("WIDTHxHEIGHT");
        let size = (width.parse().expect("width"), height.parse().expect("height"));
        // The pending resize is packed as two 16-bit halves.
        assert!(size.0 <= 0xffff && size.1 <= 0xffff, "--resize dimensions must fit 16 bits");
        size
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }

    pub fn main() {
        let mut width = 320;
        let mut height = 200;
        let mut resize_after_ms = None;
        let mut resize = None;
        let mut cycle_ms = 1000;
        let mut max_seconds = 600;
        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            let mut value = || args.next().expect("option value");
            match arg.as_str() {
                "--width" => width = value().parse().expect("width"),
                "--height" => height = value().parse().expect("height"),
                "--resize-after-ms" => resize_after_ms = Some(value().parse::<u32>().expect("delay")),
                "--resize" => resize = Some(dimensions(&value())),
                "--cycle-ms" => cycle_ms = value().parse().expect("cycle"),
                "--max-seconds" => max_seconds = value().parse::<u32>().expect("seconds"),
                "--log" => {
                    let file = std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(value())
                        .expect("fixture log");
                    *LOG.lock().unwrap() = Some(file);
                }
                other => panic!("unknown argument {other}"),
            }
        }
        // SAFETY: plain Win32 window creation and a message loop, all on this
        // thread.
        unsafe {
            SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
            let class = wide("PortholeCaptureFixture");
            let wc = WNDCLASSW {
                style: CS_HREDRAW | CS_VREDRAW,
                lpfnWndProc: Some(procedure),
                lpszClassName: class.as_ptr(),
                ..std::mem::zeroed()
            };
            assert_ne!(RegisterClassW(&wc), 0);
            let (outer_width, outer_height) = outer_size(width, height);
            let hwnd = CreateWindowExW(
                0,
                class.as_ptr(),
                wide("Porthole native capture fixture").as_ptr(),
                WS_OVERLAPPEDWINDOW,
                40,
                40,
                outer_width,
                outer_height,
                null_mut(),
                null_mut(),
                null_mut(),
                null_mut(),
            );
            assert!(!hwnd.is_null());
            event(
                "start",
                &format!(
                    "\"pid\": {}, \"hwnd\": {}, \"client\": [{width}, {height}], \"cycle_ms\": {cycle_ms}",
                    std::process::id(),
                    hwnd as usize
                ),
            );
            // Windows 11 rounds top-level corners, which would make the
            // client area's bottom corners differ from its colour.
            let corners = DWMWCP_DONOTROUND;
            DwmSetWindowAttribute(
                hwnd,
                DWMWA_WINDOW_CORNER_PREFERENCE as u32,
                (&raw const corners).cast(),
                std::mem::size_of_val(&corners) as u32,
            );
            WTSRegisterSessionNotification(hwnd, NOTIFY_FOR_THIS_SESSION);
            SetTimer(hwnd, CYCLE_TIMER, cycle_ms, None);
            SetTimer(hwnd, CURSOR_TIMER, 50, None);
            if let (Some(delay), Some((width, height))) = (resize_after_ms, resize) {
                RESIZE.store(((width as usize) << 16) | height as usize, Ordering::SeqCst);
                SetTimer(hwnd, RESIZE_TIMER, delay, None);
            }
            SetTimer(hwnd, EXIT_TIMER, max_seconds.saturating_mul(1000), None);
            ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            UpdateWindow(hwnd);
            let mut message = std::mem::zeroed();
            while GetMessageW(&mut message, null_mut(), 0, 0) > 0 {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
    }
}
