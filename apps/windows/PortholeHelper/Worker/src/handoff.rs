use std::{
    ffi::OsString,
    io,
    os::windows::{ffi::OsStringExt, process::CommandExt},
    path::PathBuf,
    process::Command,
    time::Duration,
};

use windows_sys::Win32::System::{
    RemoteDesktop::{WTSClientProtocolType, WTSConnectState, WTSFreeMemory, WTSQuerySessionInformationW},
    StationsAndDesktops::{CloseDesktop, DESKTOP_READOBJECTS, GetUserObjectInformationW, OpenInputDesktop, UOI_NAME},
    SystemInformation::GetSystemDirectoryW,
};

pub fn require_rdp(session: u32) -> io::Result<()> {
    unsafe {
        let mut data = std::ptr::null_mut();
        let mut size = 0;
        if WTSQuerySessionInformationW(std::ptr::null_mut(), session, WTSClientProtocolType, &mut data, &mut size) == 0 {
            return Err(io::Error::last_os_error());
        }
        let rdp = size >= 2 && *data.cast::<u16>() == 2;
        WTSFreeMemory(data.cast());
        if !rdp {
            return Err(io::Error::other("Active RDP session required"));
        }
        if WTSQuerySessionInformationW(std::ptr::null_mut(), session, WTSConnectState, &mut data, &mut size) == 0 {
            return Err(io::Error::last_os_error());
        }
        let active = size >= 4 && *data.cast::<u32>() == 0;
        WTSFreeMemory(data.cast());
        if !active {
            return Err(io::Error::other("Session is not active"));
        }
        let desktop = OpenInputDesktop(0, 0, DESKTOP_READOBJECTS);
        if desktop.is_null() {
            return Err(io::Error::last_os_error());
        }
        let mut name = [0u16; 256];
        let ok = GetUserObjectInformationW(
            desktop,
            UOI_NAME,
            name.as_mut_ptr().cast(),
            std::mem::size_of_val(&name) as u32,
            &mut size,
        );
        CloseDesktop(desktop);
        let end = name.iter().position(|&c| c == 0).unwrap_or(name.len());
        if ok == 0 || String::from_utf16_lossy(&name[..end]) != "Default" {
            return Err(io::Error::other("Unlocked input desktop required"));
        }
    }
    Ok(())
}

// The session comes exclusively from the worker's OS token/process identity.
// Neither the executable nor its arguments can be supplied over IPC.
pub async fn transfer(session: u32) -> io::Result<[u8; 4]> {
    require_rdp(session)?;
    let mut buffer = vec![0u16; 32768];
    let length = unsafe { GetSystemDirectoryW(buffer.as_mut_ptr(), buffer.len() as u32) } as usize;
    if length == 0 || length >= buffer.len() {
        return Err(io::Error::last_os_error());
    }
    let system = PathBuf::from(OsString::from_wide(&buffer[..length]));
    let mut child = Command::new(system.join("tscon.exe"))
        .arg(session.to_string())
        .arg("/dest:console")
        .current_dir(&system)
        .env_clear()
        .env(
            "SystemRoot",
            system.parent().ok_or_else(|| io::Error::other("Invalid system directory"))?,
        )
        .creation_flags(0x08000000)
        .spawn()?;
    let expires = tokio::time::Instant::now() + Duration::from_secs(15);
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(if status.success() { *b"DONE" } else { *b"FAIL" });
        }
        if tokio::time::Instant::now() >= expires {
            // The child may still complete. Do not pretend timeout means no effect.
            return Ok(*b"UNKN");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[cfg(test)]
mod tests {
    #[test]
    #[ignore = "requires active unlocked RDP; read-only, never invokes tscon"]
    fn active_rdp_preflight() {
        let peer = crate::identity::Peer::open(std::process::id()).unwrap();
        super::require_rdp(peer.identity.session).unwrap();
    }
}
