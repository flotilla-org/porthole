//! Test-owned Win32 window for native capture checks (#186). It paints its
//! whole client area one solid colour, cycling red, green and blue, so a
//! consumer can verify frames pixel by pixel. It is shown without activation
//! (it never takes the keyboard focus), can resize its own client area once
//! after a delay (the Windows adapter has no placement yet), and exits on
//! WM_CLOSE or after `--max-seconds`.
//!
//! ```text
//! capture_fixture [--width 320] [--height 200] [--resize-after-ms 6000 --resize 480x300]
//!                 [--cycle-ms 1000] [--max-seconds 600]
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
        ptr::null_mut,
        sync::atomic::{AtomicUsize, Ordering},
    };

    use windows_sys::Win32::{
        Foundation::*,
        Graphics::{
            Dwm::{DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND, DwmSetWindowAttribute},
            Gdi::*,
        },
        UI::{HiDpi::*, WindowsAndMessaging::*},
    };

    /// Red, green, blue as `0x00BBGGRR`.
    const COLORS: [u32; 3] = [0x0000_00ff, 0x0000_a000, 0x00ff_0000];
    const CYCLE_TIMER: usize = 1;
    const RESIZE_TIMER: usize = 2;
    const EXIT_TIMER: usize = 3;

    static COLOR: AtomicUsize = AtomicUsize::new(0);
    static RESIZE: AtomicUsize = AtomicUsize::new(0);

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
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
                    let mut paint = std::mem::zeroed();
                    let dc = BeginPaint(hwnd, &mut paint);
                    let mut client = std::mem::zeroed();
                    GetClientRect(hwnd, &mut client);
                    let brush = CreateSolidBrush(COLORS[COLOR.load(Ordering::SeqCst) % COLORS.len()]);
                    FillRect(dc, &client, brush);
                    DeleteObject(brush);
                    EndPaint(hwnd, &paint);
                    0
                }
                WM_TIMER if w == CYCLE_TIMER => {
                    COLOR.fetch_add(1, Ordering::SeqCst);
                    InvalidateRect(hwnd, null_mut(), 0);
                    0
                }
                WM_TIMER if w == RESIZE_TIMER => {
                    KillTimer(hwnd, RESIZE_TIMER);
                    let packed = RESIZE.load(Ordering::SeqCst);
                    let (width, height) = outer_size((packed >> 16) as i32, (packed & 0xffff) as i32);
                    SetWindowPos(hwnd, null_mut(), 0, 0, width, height, SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE);
                    InvalidateRect(hwnd, null_mut(), 0);
                    0
                }
                WM_TIMER if w == EXIT_TIMER => {
                    DestroyWindow(hwnd);
                    0
                }
                WM_CLOSE => {
                    DestroyWindow(hwnd);
                    0
                }
                WM_DESTROY => {
                    PostQuitMessage(0);
                    0
                }
                _ => DefWindowProcW(hwnd, message, w, l),
            }
        }
    }

    fn dimensions(value: &str) -> (u32, u32) {
        let (width, height) = value.split_once('x').expect("WIDTHxHEIGHT");
        (width.parse().expect("width"), height.parse().expect("height"))
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
            // Windows 11 rounds top-level corners, which would make the
            // client area's bottom corners differ from its colour.
            let corners = DWMWCP_DONOTROUND;
            DwmSetWindowAttribute(
                hwnd,
                DWMWA_WINDOW_CORNER_PREFERENCE as u32,
                (&raw const corners).cast(),
                std::mem::size_of_val(&corners) as u32,
            );
            SetTimer(hwnd, CYCLE_TIMER, cycle_ms, None);
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
