//! Real, test-owned Win32 editor for the manual named-pipe acceptance script.
//! It has no porthole dependency or automation interface: input goes through Windows.
#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(not(windows))]
fn main() {}

#[cfg(windows)]
fn main() {
    if std::env::args().any(|arg| arg == "--exit-without-window") {
        return;
    }
    use std::ptr::null_mut;

    use windows_sys::Win32::{
        Foundation::*,
        Graphics::Gdi::COLOR_WINDOW,
        UI::{Input::KeyboardAndMouse::SetFocus, WindowsAndMessaging::*},
    };
    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }
    unsafe extern "system" fn procedure(hwnd: HWND, message: u32, w: WPARAM, l: LPARAM) -> LRESULT {
        unsafe {
            match message {
                WM_SETFOCUS => {
                    SetFocus(GetDlgItem(hwnd, 1));
                    0
                }
                WM_SIZE => {
                    MoveWindow(
                        GetDlgItem(hwnd, 1),
                        12,
                        12,
                        (l as u32 & 0xffff) as i32 - 24,
                        ((l as u32 >> 16) & 0xffff) as i32 - 24,
                        1,
                    );
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
    unsafe {
        let class = wide("Porthole117Editor");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(procedure),
            lpszClassName: class.as_ptr(),
            hbrBackground: (COLOR_WINDOW + 1) as _,
            ..std::mem::zeroed()
        };
        assert_ne!(RegisterClassW(&wc), 0);
        let hwnd = CreateWindowExW(
            0,
            class.as_ptr(),
            wide("Porthole #117 - test-owned editor").as_ptr(),
            WS_OVERLAPPEDWINDOW,
            120,
            120,
            900,
            420,
            null_mut(),
            null_mut(),
            null_mut(),
            null_mut(),
        );
        assert!(!hwnd.is_null());
        let edit = CreateWindowExW(
            WS_EX_CLIENTEDGE,
            wide("EDIT").as_ptr(),
            wide("").as_ptr(),
            WS_CHILD | WS_VISIBLE | WS_VSCROLL | ES_MULTILINE as u32 | ES_AUTOVSCROLL as u32,
            12,
            12,
            860,
            340,
            hwnd,
            1usize as HMENU,
            null_mut(),
            null_mut(),
        );
        assert!(!edit.is_null());
        ShowWindow(hwnd, SW_SHOW);
        SetFocus(edit);
        let mut message = std::mem::zeroed();
        while GetMessageW(&mut message, null_mut(), 0, 0) > 0 {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
}
