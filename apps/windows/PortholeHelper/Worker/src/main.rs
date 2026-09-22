#![windows_subsystem = "windows"]

mod handoff;
mod identity;
mod ipc;

use std::{ffi::OsString, os::windows::ffi::OsStringExt, path::PathBuf};

use windows_sys::Win32::{
    Foundation::CloseHandle,
    Security::{GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation},
    System::{
        Com::CoTaskMemFree,
        RemoteDesktop::ProcessIdToSessionId,
        Threading::{GetCurrentProcess, GetCurrentProcessId, OpenProcessToken},
    },
    UI::Shell::{FOLDERID_ProgramFiles, SHGetKnownFolderPath},
};

// Installation/elevation guard shared by the probe and fixed console operation.
fn check() -> Result<(), i32> {
    unsafe {
        let mut token = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return Err(41);
        }
        let mut elevation = TOKEN_ELEVATION { TokenIsElevated: 0 };
        let mut size = 0;
        let queried = GetTokenInformation(
            token,
            TokenElevation,
            (&mut elevation as *mut TOKEN_ELEVATION).cast(),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut size,
        );
        CloseHandle(token);
        if queried == 0 || elevation.TokenIsElevated == 0 {
            return Err(41);
        }

        // Resolve via the OS, never an inherited environment variable.
        let mut folder = std::ptr::null_mut();
        if SHGetKnownFolderPath(&FOLDERID_ProgramFiles, 0, std::ptr::null_mut(), &mut folder) < 0 {
            return Err(42);
        }
        let mut len = 0;
        while *folder.add(len) != 0 {
            len += 1;
        }
        let expected = PathBuf::from(OsString::from_wide(std::slice::from_raw_parts(folder, len)))
            .join("PortholeHelper")
            .join("Worker")
            .join("PortholeConsoleWorker.exe");
        CoTaskMemFree(folder.cast());
        let expected = expected.canonicalize().map_err(|_| 42)?;
        let actual = std::env::current_exe().and_then(|p| p.canonicalize()).map_err(|_| 42)?;
        if actual != expected {
            return Err(42);
        }
        let mut session = 0;
        if ProcessIdToSessionId(GetCurrentProcessId(), &mut session) == 0 || session == 0 {
            return Err(43);
        }
    }
    Ok(())
}

fn main() {
    std::process::exit(run().err().unwrap_or(0));
}

fn run() -> Result<(), i32> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() != 3 || args[0] != "--console-handoff" {
        return Err(40);
    }
    check()?;
    let pid: u32 = args[1].parse().map_err(|_| 40)?;
    ipc::pipe_name(&args[2]).map_err(|_| 40)?;
    let helper = identity::Peer::open(pid).map_err(|_| 44)?;
    let worker = identity::Peer::open(std::process::id()).map_err(|_| 44)?;
    let expected = worker.image.parent().and_then(|p| p.parent()).ok_or(42)?.join("PortholeHelper.exe");
    helper.authorize_helper(&worker, &expected).map_err(|_| 44)?;
    // Check input-desktop state at commit, after the UAC secure desktop closes.
    ipc::run(&helper, &worker, &expected, &args[2]).map_err(|_| 45)
}
