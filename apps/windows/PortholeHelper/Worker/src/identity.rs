use std::{ffi::OsString, io, os::windows::ffi::OsStringExt, path::PathBuf};

use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE, LocalFree, WAIT_TIMEOUT},
    Security::{
        Authorization::ConvertSidToStringSidW, GetTokenInformation, TOKEN_ELEVATION, TOKEN_GROUPS, TOKEN_INFORMATION_CLASS, TOKEN_QUERY,
        TOKEN_USER, TokenElevation, TokenLogonSid, TokenUser,
    },
    System::{
        RemoteDesktop::ProcessIdToSessionId,
        Threading::{
            OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE, QueryFullProcessImageNameW,
            WaitForSingleObject,
        },
    },
};

pub struct Handle(pub HANDLE);
impl Drop for Handle {
    fn drop(&mut self) {
        // This type owns exactly one successfully opened Windows handle.
        unsafe { CloseHandle(self.0) };
    }
}

#[derive(Debug, PartialEq)]
pub struct Identity {
    pub user: String,
    pub logon: String,
    pub session: u32,
    pub elevated: bool,
}

pub struct Peer {
    pub handle: Handle,
    pub pid: u32,
    pub identity: Identity,
    pub image: PathBuf,
}

fn token_data(token: HANDLE, class: TOKEN_INFORMATION_CLASS) -> io::Result<Vec<usize>> {
    let mut size = 0;
    unsafe { GetTokenInformation(token, class, std::ptr::null_mut(), 0, &mut size) };
    if size == 0 || size > 65536 {
        return Err(io::Error::other("Invalid token information size"));
    }
    // usize storage supplies the alignment required by TOKEN_USER/TOKEN_GROUPS.
    let mut data = vec![0usize; (size as usize).div_ceil(std::mem::size_of::<usize>())];
    if unsafe { GetTokenInformation(token, class, data.as_mut_ptr().cast(), size, &mut size) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(data)
}

unsafe fn sid_string(sid: *mut std::ffi::c_void) -> io::Result<String> {
    let mut text = std::ptr::null_mut();
    if unsafe { ConvertSidToStringSidW(sid, &mut text) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let mut length = 0;
    unsafe {
        while *text.add(length) != 0 {
            length += 1;
        }
        let result = String::from_utf16_lossy(std::slice::from_raw_parts(text, length));
        LocalFree(text.cast());
        Ok(result)
    }
}

impl Peer {
    pub fn open(pid: u32) -> io::Result<Self> {
        let raw = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE, 0, pid) };
        if raw.is_null() {
            return Err(io::Error::last_os_error());
        }
        let handle = Handle(raw);
        let mut token = std::ptr::null_mut();
        if unsafe { OpenProcessToken(raw, TOKEN_QUERY, &mut token) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let token = Handle(token);
        let user = token_data(token.0, TokenUser)?;
        let logon = token_data(token.0, TokenLogonSid)?;
        let elevation = token_data(token.0, TokenElevation)?;
        let identity = unsafe {
            let groups = &*logon.as_ptr().cast::<TOKEN_GROUPS>();
            if groups.GroupCount != 1 {
                return Err(io::Error::other("Exactly one logon SID required"));
            }
            let mut session = 0;
            if ProcessIdToSessionId(pid, &mut session) == 0 || session == 0 {
                return Err(io::Error::other("Interactive session required"));
            }
            Identity {
                user: sid_string((*user.as_ptr().cast::<TOKEN_USER>()).User.Sid)?,
                logon: sid_string(groups.Groups[0].Sid)?,
                session,
                elevated: (*elevation.as_ptr().cast::<TOKEN_ELEVATION>()).TokenIsElevated != 0,
            }
        };
        let mut name = vec![0u16; 32768];
        let mut size = name.len() as u32;
        if unsafe { QueryFullProcessImageNameW(raw, 0, name.as_mut_ptr(), &mut size) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let image = PathBuf::from(OsString::from_wide(&name[..size as usize])).canonicalize()?;
        Ok(Self {
            handle,
            pid,
            identity,
            image,
        })
    }

    pub fn alive(&self) -> bool {
        unsafe { WaitForSingleObject(self.handle.0, 0) == WAIT_TIMEOUT }
    }

    pub fn authorize_helper(&self, worker: &Self, expected: &std::path::Path) -> io::Result<()> {
        if !self.alive()
            || self.identity.elevated
            || !worker.identity.elevated
            || self.identity.user != worker.identity.user
            || self.identity.logon != worker.identity.logon
            || self.identity.session != worker.identity.session
            || self.image != expected.canonicalize()?
        {
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, "Helper identity mismatch"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peer_policy_rejects_user_logon_session_elevation_and_image_mismatch() {
        let helper = Peer::open(std::process::id()).unwrap();
        assert!(!helper.identity.elevated, "Run this policy test unelevated");
        let mut worker = Peer::open(std::process::id()).unwrap();
        // Model the elevated half of the same user's token pair. Actual UAC
        // token pairing is a separate native acceptance test, not simulated here.
        worker.identity.elevated = true;
        assert!(helper.authorize_helper(&worker, &helper.image).is_ok());
        worker.identity.session += 1;
        assert!(helper.authorize_helper(&worker, &helper.image).is_err());
        worker.identity.session = helper.identity.session;
        worker.identity.user.push('0');
        assert!(helper.authorize_helper(&worker, &helper.image).is_err());
        worker.identity.user = helper.identity.user.clone();
        worker.identity.logon.push('0');
        assert!(helper.authorize_helper(&worker, &helper.image).is_err());
        worker.identity.logon = helper.identity.logon.clone();
        assert!(helper.authorize_helper(&worker, helper.image.parent().unwrap()).is_err());
        worker.identity.elevated = false;
        assert!(helper.authorize_helper(&worker, &helper.image).is_err());
    }
}
