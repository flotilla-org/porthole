use std::{
    collections::HashMap,
    io::Cursor,
    mem::{size_of, zeroed},
    os::windows::io::{AsHandle, AsRawHandle, FromRawHandle},
    process::{Command, Stdio},
    ptr::null_mut,
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use porthole_core::{
    ErrorCode, PortholeError,
    adapter::*,
    attention::AttentionInfo,
    content_rect::ContentRectInfo,
    display::DisplayInfo,
    input::*,
    permission::*,
    placement::GeometrySnapshot,
    search::{Candidate, SearchQuery, encode_ref},
    surface::*,
    wait::*,
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    Storage::Xps::PrintWindow,
    System::{Diagnostics::ToolHelp::*, RemoteDesktop::ProcessIdToSessionId, StationsAndDesktops::*, Threading::*},
    UI::{HiDpi::*, Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
};

type Result<T> = std::result::Result<T, PortholeError>;
fn unsupported(operation: &str) -> PortholeError {
    PortholeError::new(
        ErrorCode::AdapterUnsupported,
        format!("Windows adapter does not support {operation}"),
    )
}
fn failure(code: ErrorCode, operation: &str) -> PortholeError {
    PortholeError::new(code, format!("{operation}: {}", std::io::Error::last_os_error()))
}
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

/// Cookies are window properties: Windows discards them when the HWND dies.
/// The random property namespace separates daemon lifetimes. Never trust HWND alone.
pub struct WindowsAdapter {
    property: Vec<u16>,
    cookies: Mutex<HashMap<usize, usize>>,
    input: tokio::sync::Mutex<()>,
    capture_slot: Arc<tokio::sync::Semaphore>,
}

impl Default for WindowsAdapter {
    fn default() -> Self {
        Self::new()
    }
}
impl WindowsAdapter {
    pub fn new() -> Self {
        Self {
            property: wide(&format!("work.flotilla.porthole.{}", uuid::Uuid::new_v4())),
            cookies: Mutex::new(HashMap::new()),
            input: tokio::sync::Mutex::new(()),
            capture_slot: Arc::new(tokio::sync::Semaphore::new(1)),
        }
    }

    fn desktop(&self) -> Result<()> {
        // Verify this process belongs to a non-service session and can access
        // the current input desktop. This does not create or switch desktops.
        unsafe {
            let mut session = 0;
            if ProcessIdToSessionId(GetCurrentProcessId(), &mut session) == 0 || session == 0 {
                return Err(PortholeError::new(
                    ErrorCode::SystemPermissionNeeded,
                    "run portholed inside the existing interactive GUI login (not Session 0)",
                ));
            }
            let desktop = OpenInputDesktop(0, 0, DESKTOP_READOBJECTS);
            if desktop.is_null() {
                return Err(failure(
                    ErrorCode::SystemPermissionNeeded,
                    "interactive input desktop unavailable; unlock the GUI session",
                ));
            }
            let input_name = desktop_name(desktop);
            let thread_name = desktop_name(GetThreadDesktop(GetCurrentThreadId()));
            CloseDesktop(desktop);
            if input_name? != thread_name? {
                return Err(PortholeError::new(
                    ErrorCode::SystemPermissionNeeded,
                    "daemon is not on the current input desktop; unlock the original GUI session",
                ));
            }
        }
        Ok(())
    }

    fn identify(&self, hwnd: HWND) -> Result<SurfaceInfo> {
        unsafe {
            let mut pid = 0;
            if IsWindow(hwnd) == 0 || GetWindowThreadProcessId(hwnd, &mut pid) == 0 {
                return Err(PortholeError::new(ErrorCode::SurfaceDead, "window disappeared"));
            }
            let mut cookies = self.cookies.lock().unwrap();
            let existing = GetPropW(hwnd, self.property.as_ptr()) as usize;
            let cookie = if existing != 0 && cookies.get(&(hwnd as usize)) == Some(&existing) {
                existing
            } else {
                let cookie = (uuid::Uuid::new_v4().as_u128() as usize) | 1;
                if SetPropW(hwnd, self.property.as_ptr(), cookie as HANDLE) == 0 {
                    return Err(failure(
                        ErrorCode::SystemPermissionNeeded,
                        "cannot mark window identity (window may have higher integrity)",
                    ));
                }
                cookies.insert(hwnd as usize, cookie);
                cookie
            };
            let mut title = vec![0u16; 32768];
            let len = GetWindowTextW(hwnd, title.as_mut_ptr(), title.len() as i32);
            let mut info = SurfaceInfo::window(SurfaceId::new(), pid);
            info.title = Some(String::from_utf16_lossy(&title[..len as usize]));
            info.app_name = process_name(pid);
            info.platform_ref = Some(PlatformSurfaceRef::Windows {
                hwnd: hwnd as u64,
                window_cookie: cookie as u64,
            });
            Ok(info)
        }
    }

    fn resolve(&self, surface: &SurfaceInfo) -> Result<HWND> {
        let Some(PlatformSurfaceRef::Windows { hwnd, window_cookie }) = surface.platform_ref else {
            return Err(unsupported("non-Windows surface"));
        };
        let hwnd = hwnd as HWND;
        unsafe {
            let mut pid = 0;
            GetWindowThreadProcessId(hwnd, &mut pid);
            if window_cookie == 0
                || IsWindow(hwnd) == 0
                || Some(pid) != surface.pid
                || GetPropW(hwnd, self.property.as_ptr()) as u64 != window_cookie
            {
                return Err(PortholeError::new(
                    ErrorCode::SurfaceDead,
                    "tracked window disappeared or its identity changed",
                ));
            }
        }
        Ok(hwnd)
    }

    fn search_windows(&self, query: &SearchQuery, handles: impl IntoIterator<Item = usize>) -> Result<Vec<Candidate>> {
        let regex = query
            .title_pattern
            .as_ref()
            .map(|s| regex::Regex::new(s))
            .transpose()
            .map_err(|e| PortholeError::new(ErrorCode::InvalidArgument, e.to_string()))?;
        let mut candidates = Vec::new();
        for hwnd in handles {
            let info = match self.identify(hwnd as HWND) {
                Ok(info) => info,
                Err(error) if matches!(error.code, ErrorCode::SurfaceDead | ErrorCode::SystemPermissionNeeded) => continue,
                Err(error) => return Err(error),
            };
            let pid = info.pid.unwrap();
            let platform_ref = info.platform_ref.unwrap();
            if !query.pids.is_empty() && !query.pids.contains(&pid) {
                continue;
            }
            if !query.platform_refs.is_empty() && !query.platform_refs.contains(&platform_ref) {
                continue;
            }
            if query
                .app_name
                .as_ref()
                .is_some_and(|n| !info.app_name.as_ref().is_some_and(|a| a.eq_ignore_ascii_case(n)))
            {
                continue;
            }
            if regex.as_ref().is_some_and(|r| !r.is_match(info.title.as_deref().unwrap_or(""))) {
                continue;
            }
            if query
                .frontmost
                .is_some_and(|front| front != unsafe { GetForegroundWindow() as usize == hwnd })
            {
                continue;
            }
            candidates.push(Candidate {
                ref_: encode_ref(pid, platform_ref.clone()),
                app_name: info.app_name,
                title: info.title,
                pid,
                platform_ref,
            });
        }
        Ok(candidates)
    }

    async fn focus_window(&self, surface: &SurfaceInfo) -> Result<()> {
        self.desktop()?;
        unsafe {
            let hwnd = self.resolve(surface)?;
            if IsIconic(hwnd) != 0 {
                ShowWindowAsync(hwnd, SW_RESTORE);
            }
            SetForegroundWindow(hwnd);
        }
        // Keep input serialized while yielding the worker during activation.
        for _ in 0..50 {
            unsafe {
                let hwnd = self.resolve(surface)?;
                if GetForegroundWindow() == hwnd {
                    return Ok(());
                }
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        Err(PortholeError::new(
            ErrorCode::SystemPermissionNeeded,
            "Windows denied foreground activation; activate the test window in the GUI session and retry",
        ))
    }

    fn send(&self, surface: &SurfaceInfo, inputs: &[INPUT]) -> Result<()> {
        let hwnd = self.resolve(surface)?;
        unsafe {
            if GetForegroundWindow() != hwnd {
                return Err(PortholeError::new(
                    ErrorCode::SystemPermissionNeeded,
                    "target lost foreground before input",
                ));
            }
            if !inputs.is_empty() && SendInput(inputs.len() as u32, inputs.as_ptr(), size_of::<INPUT>() as i32) != inputs.len() as u32 {
                // Release our keys after a partial insertion; do not leave modifiers stuck.
                let releases: Vec<_> = inputs
                    .iter()
                    .copied()
                    .filter(|i| i.Anonymous.ki.dwFlags & KEYEVENTF_KEYUP != 0)
                    .collect();
                SendInput(releases.len() as u32, releases.as_ptr(), size_of::<INPUT>() as i32);
                return Err(failure(
                    ErrorCode::SystemPermissionNeeded,
                    "SendInput incomplete (UIPI or desktop restriction)",
                ));
            }
        }
        Ok(())
    }
}

impl Drop for WindowsAdapter {
    fn drop(&mut self) {
        for (&hwnd, &cookie) in self.cookies.get_mut().unwrap().iter() {
            unsafe {
                if GetPropW(hwnd as HWND, self.property.as_ptr()) as usize == cookie {
                    RemovePropW(hwnd as HWND, self.property.as_ptr());
                }
            }
        }
    }
}

fn process_name(pid: u32) -> Option<String> {
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process.is_null() {
            return None;
        }
        let mut path = vec![0u16; 32768];
        let mut len = path.len() as u32;
        let ok = QueryFullProcessImageNameW(process, 0, path.as_mut_ptr(), &mut len);
        CloseHandle(process);
        if ok == 0 {
            return None;
        }
        let path = String::from_utf16_lossy(&path[..len as usize]);
        Some(std::path::Path::new(&path).file_name()?.to_string_lossy().into_owned())
    }
}

fn desktop_name(desktop: HDESK) -> Result<Vec<u16>> {
    let mut name = vec![0u16; 256];
    let mut needed = 0;
    unsafe {
        if GetUserObjectInformationW(desktop, UOI_NAME, name.as_mut_ptr().cast(), (name.len() * 2) as u32, &mut needed) == 0 {
            return Err(failure(ErrorCode::SystemPermissionNeeded, "identify current desktop"));
        }
    }
    name.truncate(needed as usize / 2);
    Ok(name)
}

fn windows() -> Result<Vec<usize>> {
    unsafe extern "system" fn collect(hwnd: HWND, param: LPARAM) -> i32 {
        unsafe {
            if IsWindowVisible(hwnd) != 0 && GetWindow(hwnd, GW_OWNER).is_null() {
                (&mut *(param as *mut Vec<usize>)).push(hwnd as usize);
            }
        }
        1
    }
    let mut result = Vec::new();
    unsafe {
        if EnumWindows(Some(collect), &mut result as *mut Vec<usize> as LPARAM) == 0 {
            return Err(failure(ErrorCode::InternalError, "EnumWindows"));
        }
    }
    Ok(result)
}

fn keyboard(vk: u16, scan: u16, flags: u32) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: scan,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

// Keep every verified process object alive for the entire launch observation.
// A PID/parent-PID snapshot alone is not an identity: the parent may have exited
// and its PID may already belong to an unrelated process.
struct LaunchProcess {
    handle: std::os::windows::io::OwnedHandle,
    created: u64,
}

fn filetime(value: FILETIME) -> u64 {
    (u64::from(value.dwHighDateTime) << 32) | u64::from(value.dwLowDateTime)
}

fn process_times(handle: HANDLE) -> Result<(u64, u64)> {
    unsafe {
        let (mut created, mut exited, mut kernel, mut user) = (zeroed(), zeroed(), zeroed(), zeroed());
        if GetProcessTimes(handle, &mut created, &mut exited, &mut kernel, &mut user) == 0 {
            return Err(failure(ErrorCode::LaunchCorrelationFailed, "read launch process identity"));
        }
        Ok((filetime(created), filetime(exited)))
    }
}

impl LaunchProcess {
    fn new(handle: std::os::windows::io::OwnedHandle) -> Result<Self> {
        let created = process_times(handle.as_raw_handle())?.0;
        Ok(Self { handle, created })
    }

    fn alive(&self) -> bool {
        unsafe { WaitForSingleObject(self.handle.as_raw_handle(), 0) == WAIT_TIMEOUT }
    }
}

#[derive(Clone, Copy)]
struct ProcessLink {
    pid: u32,
    parent: u32,
}

// The snapshot's parent link is usable only for the process incarnation that
// existed before the snapshot started. A later OpenProcess must not resolve a
// recycled child PID. Equal timestamps cannot prove ordering, so fail closed.
fn valid_birth(parent_created: u64, parent_exited: u64, child_created: u64, snapshot_started: u64) -> bool {
    parent_created < child_created && child_created < snapshot_started && (parent_exited == 0 || child_created < parent_exited)
}

fn process_links() -> Result<(u64, Vec<ProcessLink>)> {
    unsafe {
        let mut now = zeroed();
        windows_sys::Win32::System::SystemInformation::GetSystemTimePreciseAsFileTime(&mut now);
        let raw = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if raw == INVALID_HANDLE_VALUE {
            return Err(failure(ErrorCode::LaunchCorrelationFailed, "snapshot launch descendants"));
        }
        let snapshot = std::os::windows::io::OwnedHandle::from_raw_handle(raw);
        let mut entry: PROCESSENTRY32W = zeroed();
        entry.dwSize = size_of::<PROCESSENTRY32W>() as u32;
        let mut links = Vec::new();
        let mut ok = Process32FirstW(snapshot.as_raw_handle(), &mut entry);
        while ok != 0 {
            links.push(ProcessLink {
                pid: entry.th32ProcessID,
                parent: entry.th32ParentProcessID,
            });
            ok = Process32NextW(snapshot.as_raw_handle(), &mut entry);
        }
        if GetLastError() != ERROR_NO_MORE_FILES {
            return Err(failure(ErrorCode::LaunchCorrelationFailed, "enumerate launch descendants"));
        }
        Ok((filetime(now), links))
    }
}

struct LaunchTree {
    processes: HashMap<u32, LaunchProcess>,
}

impl LaunchTree {
    fn new(child: &std::process::Child) -> Result<Self> {
        // Clone the original spawn handle, never reopen the root by numeric PID.
        let handle = child
            .as_handle()
            .try_clone_to_owned()
            .map_err(|e| PortholeError::new(ErrorCode::LaunchCorrelationFailed, format!("retain launch process: {e}")))?;
        Ok(Self {
            processes: HashMap::from([(child.id(), LaunchProcess::new(handle)?)]),
        })
    }

    fn discover(&mut self) -> Result<()> {
        let (started, links) = process_links()?;
        self.observe(started, &links)
    }

    fn observe(&mut self, started: u64, links: &[ProcessLink]) -> Result<()> {
        // Repeat over one snapshot so enumeration order cannot lose grandchildren.
        loop {
            let mut added = false;
            for link in links {
                if self.processes.contains_key(&link.pid) {
                    continue;
                }
                let Some(parent) = self.processes.get(&link.parent) else { continue };
                let raw = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE, 0, link.pid) };
                if raw.is_null() {
                    // An inaccessible or already-reaped child cannot be proven ours.
                    continue;
                }
                let handle = unsafe { std::os::windows::io::OwnedHandle::from_raw_handle(raw) };
                let Ok(child) = LaunchProcess::new(handle) else { continue };
                // Query after opening the child, including a parent's recorded exit.
                let (_, exited) = process_times(parent.handle.as_raw_handle())?;
                if valid_birth(parent.created, exited, child.created, started) {
                    self.processes.insert(link.pid, child);
                    added = true;
                }
            }
            if !added {
                return Ok(());
            }
        }
    }

    fn owns_window(&self, hwnd: usize) -> bool {
        let mut pid = 0;
        unsafe {
            GetWindowThreadProcessId(hwnd as HWND, &mut pid);
        }
        self.processes.get(&pid).is_some_and(LaunchProcess::alive)
    }

    fn unique_window(&self, handles: impl IntoIterator<Item = usize>) -> Result<Option<usize>> {
        let mut owned = handles.into_iter().filter(|&hwnd| self.owns_window(hwnd));
        let first = owned.next();
        if owned.next().is_some() {
            return Err(PortholeError::new(
                ErrorCode::LaunchCorrelationAmbiguous,
                "launch process tree owns multiple visible windows; refusing to choose",
            ));
        }
        Ok(first)
    }
}

fn launch_candidate(candidate: Result<SurfaceInfo>) -> Result<Option<SurfaceInfo>> {
    match candidate {
        Ok(surface) => Ok(Some(surface)),
        Err(error) if error.code == ErrorCode::SurfaceDead => Ok(None),
        Err(error) => Err(error),
    }
}

fn spawn_process(spec: &ProcessLaunchSpec) -> Result<std::process::Child> {
    let mut command = Command::new(&spec.app);
    command
        .args(&spec.args)
        .envs(spec.env.iter().cloned())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(cwd) = &spec.cwd {
        command.current_dir(cwd);
    }
    command
        .spawn()
        .map_err(|e| PortholeError::new(ErrorCode::LaunchCorrelationFailed, format!("process launch failed: {e}")))
}

#[async_trait]
impl Adapter for WindowsAdapter {
    fn name(&self) -> &'static str {
        "windows"
    }
    async fn launch_process(&self, spec: &ProcessLaunchSpec) -> Result<LaunchOutcome> {
        self.desktop()?;
        let child = spawn_process(spec)?;
        let pid = child.id();
        let mut tree = LaunchTree::new(&child)?;
        let deadline = Instant::now() + spec.timeout;
        loop {
            self.desktop()?;
            tree.discover()?;
            if let Some(hwnd) = tree.unique_window(windows()?)? {
                // A startup window may disappear between enumeration, marking,
                // and cookie validation. Retry only SurfaceDead; permission and
                // other failures still abort. Fall through to the deadline and
                // sleep below even when candidates repeatedly disappear.
                let candidate = self.identify(hwnd as HWND).and_then(|surface| {
                    self.resolve(&surface)?;
                    Ok(surface)
                });
                if let Some(surface) = launch_candidate(candidate)?
                    && tree.owns_window(hwnd)
                    && unsafe { IsWindowVisible(hwnd as HWND) != 0 && GetWindow(hwnd as HWND, GW_OWNER).is_null() }
                {
                    return Ok(LaunchOutcome {
                        surface,
                        confidence: Confidence::Strong,
                        correlation: Correlation::PidTree,
                        surface_was_preexisting: false,
                    });
                }
            }
            // An exited wrapper does not end the launch window: a verified child
            // can still create its UI, or appear in the next process snapshot.
            if Instant::now() >= deadline {
                let alive = tree.processes.values().any(LaunchProcess::alive);
                return Err(PortholeError::new(
                    if alive {
                        ErrorCode::LaunchTimeout
                    } else {
                        ErrorCode::LaunchCorrelationFailed
                    },
                    format!(
                        "process {pid} and verified descendants have no unique visible window before deadline; brokered windows are unsupported; processes are left running"
                    ),
                ));
            }
            tokio::time::sleep(Duration::from_millis(50).min(deadline.saturating_duration_since(Instant::now()))).await;
        }
    }
    async fn screenshot(&self, surface: &SurfaceInfo) -> Result<Screenshot> {
        self.desktop()?;
        let hwnd = self.resolve(surface)? as usize;
        let permit = self
            .capture_slot
            .clone()
            .try_acquire_owned()
            .map_err(|_| unsupported("capture while a previous PrintWindow call is still running"))?;
        let (send, receive) = tokio::sync::oneshot::channel();
        // PrintWindow is synchronous and has no cancellation API. At most one
        // worker may outlive a request; retain its DC until Windows returns.
        // A detached OS thread also cannot stall Tokio's runtime shutdown.
        std::thread::Builder::new()
            .name("porthole-screenshot".into())
            .spawn(move || {
                let _permit = permit;
                let _ = send.send(capture(hwnd as HWND));
            })
            .map_err(|e| PortholeError::new(ErrorCode::InternalError, e.to_string()))?;
        let shot = tokio::time::timeout(Duration::from_secs(3), receive)
            .await
            .map_err(|_| unsupported("window did not complete PrintWindow within three seconds"))?
            .map_err(|_| PortholeError::new(ErrorCode::InternalError, "capture worker exited"))??;
        self.resolve(surface)?;
        Ok(shot)
    }
    async fn focus(&self, surface: &SurfaceInfo) -> Result<()> {
        let _guard = self.input.lock().await;
        self.focus_window(surface).await.map(|_| ())
    }
    async fn text(&self, surface: &SurfaceInfo, text: &str) -> Result<()> {
        let _guard = self.input.lock().await;
        self.focus_window(surface).await?;
        // Keep a UTF-16 surrogate pair in the same atomic SendInput batch.
        for character in text.chars() {
            let mut units = [0; 2];
            let events: Vec<_> = character
                .encode_utf16(&mut units)
                .iter()
                .flat_map(|&unit| {
                    [
                        keyboard(0, unit, KEYEVENTF_UNICODE),
                        keyboard(0, unit, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP),
                    ]
                })
                .collect();
            self.send(surface, &events)?;
        }
        Ok(())
    }
    async fn key(&self, surface: &SurfaceInfo, events: &[KeyEvent]) -> Result<()> {
        // Validate the entire request before any input side effects.
        let mapped = events
            .iter()
            .map(|event| crate::keys::virtual_key(&event.key))
            .collect::<Result<Vec<_>>>()?;
        let _guard = self.input.lock().await;
        self.focus_window(surface).await?;
        for (event, (vk, extended)) in events.iter().zip(mapped) {
            let modifiers: Vec<_> = event
                .modifiers
                .iter()
                .map(|m| match m {
                    Modifier::Cmd => VK_LWIN,
                    Modifier::Ctrl => VK_CONTROL,
                    Modifier::Alt => VK_MENU,
                    Modifier::Shift => VK_SHIFT,
                })
                .collect();
            let mut inputs: Vec<_> = modifiers
                .iter()
                .map(|&vk| keyboard(vk, 0, if vk == VK_LWIN { KEYEVENTF_EXTENDEDKEY } else { 0 }))
                .collect();
            let flags = if extended { KEYEVENTF_EXTENDEDKEY } else { 0 };
            inputs.extend([keyboard(vk, 0, flags), keyboard(vk, 0, flags | KEYEVENTF_KEYUP)]);
            inputs.extend(
                modifiers
                    .iter()
                    .rev()
                    .map(|&vk| keyboard(vk, 0, KEYEVENTF_KEYUP | if vk == VK_LWIN { KEYEVENTF_EXTENDEDKEY } else { 0 })),
            );
            self.send(surface, &inputs)?;
        }
        Ok(())
    }
    async fn close(&self, surface: &SurfaceInfo) -> Result<()> {
        let hwnd = self.resolve(surface)?;
        unsafe {
            if PostMessageW(hwnd, WM_CLOSE, 0, 0) == 0 {
                return Err(failure(ErrorCode::CloseFailed, "WM_CLOSE"));
            }
        }
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            if self.resolve(surface).is_err() {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        Err(PortholeError::new(
            ErrorCode::CloseFailed,
            "window remains open (possibly an unsaved-document prompt); no process was killed",
        ))
    }
    async fn surface_alive(&self, pid: u32, platform_ref: &PlatformSurfaceRef) -> Result<Option<SurfaceInfo>> {
        let mut surface = SurfaceInfo::window(SurfaceId::new(), pid);
        surface.platform_ref = Some(platform_ref.clone());
        match self.resolve(&surface) {
            Ok(hwnd) => self.identify(hwnd).map(Some),
            Err(e) if e.code == ErrorCode::SurfaceDead => Ok(None),
            Err(e) => Err(e),
        }
    }
    async fn search(&self, query: &SearchQuery) -> Result<Vec<Candidate>> {
        self.desktop()?;
        self.search_windows(query, windows()?)
    }
    async fn system_permissions(&self) -> Result<Vec<SystemPermissionStatus>> {
        Ok(vec![SystemPermissionStatus { name: "interactive_desktop".into(), granted: self.desktop().is_ok(), purpose: "desktop operations require an unlocked interactive GUI session; Windows also enforces foreground and integrity restrictions per operation".into() }])
    }
    async fn ensure_system_permission(&self, _name: &str) -> Result<()> {
        self.desktop()
    }
    async fn request_system_permission_prompt(&self, _name: &str) -> Result<SystemPermissionPromptOutcome> {
        Err(unsupported("permission prompts; unlock the existing GUI session manually"))
    }
    async fn focused_platform_surface_ref(&self) -> Result<Option<PlatformSurfaceRef>> {
        self.desktop()?;
        let hwnd = unsafe { GetForegroundWindow() };
        if hwnd.is_null() {
            Ok(None)
        } else {
            Ok(self.identify(hwnd)?.platform_ref)
        }
    }
    async fn click(&self, _: &SurfaceInfo, _: &ClickSpec) -> Result<()> {
        Err(unsupported("pointer click"))
    }
    async fn scroll(&self, _: &SurfaceInfo, _: &ScrollSpec) -> Result<()> {
        Err(unsupported("pointer scroll"))
    }
    async fn pointer_move(&self, _: &SurfaceInfo, _: &PointerMoveSpec) -> Result<()> {
        Err(unsupported("pointer movement"))
    }
    async fn attention(&self) -> Result<AttentionInfo> {
        Err(unsupported("attention"))
    }
    async fn displays(&self) -> Result<Vec<DisplayInfo>> {
        Err(unsupported("display enumeration"))
    }
    async fn launch_artifact(&self, _: &ArtifactLaunchSpec) -> Result<LaunchOutcome> {
        Err(unsupported("artifact launch"))
    }
    async fn place_surface(&self, _: &SurfaceInfo, _: Rect) -> Result<()> {
        Err(unsupported("placement"))
    }
    async fn snapshot_geometry(&self, _: &SurfaceInfo) -> Result<GeometrySnapshot> {
        Err(unsupported("placement geometry"))
    }
    async fn content_rect(&self, _: &SurfaceInfo) -> Result<ContentRectInfo> {
        Err(unsupported("UIAutomation content rect"))
    }
    fn validate_wait(&self, condition: &WaitCondition) -> Result<()> {
        match condition {
            WaitCondition::Stable { .. } | WaitCondition::Dirty { .. } => Err(unsupported("pixel-difference waits")),
            _ => Ok(()),
        }
    }
    async fn wait(
        &self,
        surface: &SurfaceInfo,
        condition: &WaitCondition,
        deadline: Instant,
    ) -> std::result::Result<WaitOutcome, WaitTimeout> {
        let start = Instant::now();
        let title_regex = match condition {
            WaitCondition::TitleMatches { pattern } => regex::Regex::new(pattern).ok(),
            _ => None,
        };
        loop {
            let live = self.resolve(surface).ok().and_then(|hwnd| self.identify(hwnd).ok());
            let (name, satisfied, observed) = match condition {
                WaitCondition::Exists => ("exists", live.is_some(), LastObserved::Presence { alive: live.is_some() }),
                WaitCondition::Gone => ("gone", live.is_none(), LastObserved::Presence { alive: live.is_some() }),
                WaitCondition::TitleMatches { .. } => {
                    let title = live.and_then(|s| s.title);
                    let matches = title_regex.as_ref().is_some_and(|r| r.is_match(title.as_deref().unwrap_or("")));
                    ("title_matches", matches, LastObserved::Title { title })
                }
                _ => {
                    return Err(WaitTimeout {
                        last_observed: LastObserved::Presence { alive: live.is_some() },
                        elapsed_ms: start.elapsed().as_millis() as u64,
                    });
                }
            };
            let elapsed_ms = start.elapsed().as_millis() as u64;
            if satisfied {
                return Ok(WaitOutcome {
                    condition: name.into(),
                    elapsed_ms,
                });
            }
            if Instant::now() >= deadline {
                return Err(WaitTimeout {
                    last_observed: observed,
                    elapsed_ms,
                });
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
    fn capabilities(&self) -> Vec<&'static str> {
        vec![
            "launch_process",
            "search",
            "wait",
            "focus",
            "input_key",
            "input_text",
            "screenshot",
            "close",
        ]
    }
}

// GDI resources are always released, including timeout/error paths.
struct CaptureDc {
    window: HWND,
    source: HDC,
    memory: HDC,
    bitmap: HBITMAP,
    previous: HGDIOBJ,
}
impl Drop for CaptureDc {
    fn drop(&mut self) {
        unsafe {
            if !self.previous.is_null() {
                SelectObject(self.memory, self.previous);
            }
            if !self.bitmap.is_null() {
                DeleteObject(self.bitmap);
            }
            if !self.memory.is_null() {
                DeleteDC(self.memory);
            }
            if !self.source.is_null() {
                ReleaseDC(self.window, self.source);
            }
        }
    }
}
struct DpiContext(DPI_AWARENESS_CONTEXT);
impl Drop for DpiContext {
    fn drop(&mut self) {
        unsafe {
            SetThreadDpiAwarenessContext(self.0);
        }
    }
}

fn capture(hwnd: HWND) -> Result<Screenshot> {
    unsafe {
        let _dpi = DpiContext(SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2));
        if IsIconic(hwnd) != 0 {
            return Err(unsupported("capturing minimized windows; restore first"));
        }
        let mut rect: RECT = zeroed();
        if GetWindowRect(hwnd, &mut rect) == 0 {
            return Err(failure(ErrorCode::SurfaceDead, "GetWindowRect"));
        }
        let width = rect.right - rect.left;
        let height = rect.bottom - rect.top;
        if width <= 0 || height <= 0 || (width as u64 * height as u64) > 64 * 1024 * 1024 {
            return Err(unsupported("invalid or excessively large capture dimensions"));
        }
        let mut dc = CaptureDc {
            window: hwnd,
            source: GetWindowDC(hwnd),
            memory: null_mut(),
            bitmap: null_mut(),
            previous: null_mut(),
        };
        dc.memory = CreateCompatibleDC(dc.source);
        let mut header: BITMAPINFO = zeroed();
        header.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
        header.bmiHeader.biWidth = width;
        header.bmiHeader.biHeight = -height;
        header.bmiHeader.biPlanes = 1;
        header.bmiHeader.biBitCount = 32;
        header.bmiHeader.biCompression = BI_RGB;
        let mut bits = null_mut();
        dc.bitmap = CreateDIBSection(dc.source, &header, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
        if dc.source.is_null() || dc.memory.is_null() || dc.bitmap.is_null() || bits.is_null() {
            return Err(failure(ErrorCode::InternalError, "allocate GDI capture"));
        }
        dc.previous = SelectObject(dc.memory, dc.bitmap);
        if dc.previous.is_null() || dc.previous as isize == -1 {
            dc.previous = null_mut();
            return Err(failure(ErrorCode::InternalError, "select capture bitmap"));
        }
        std::ptr::write_bytes(bits as *mut u8, 0, width as usize * height as usize * 4);
        if PrintWindow(hwnd, dc.memory, PW_RENDERFULLCONTENT) == 0 {
            return Err(failure(ErrorCode::AdapterUnsupported, "PrintWindow failed"));
        }
        GdiFlush();
        let mut rgba = std::slice::from_raw_parts(bits as *const u8, width as usize * height as usize * 4).to_vec();
        if rgba.iter().all(|b| *b == 0) {
            return Err(unsupported(
                "window does not render through PrintWindow (GPU/window capture is a later slice)",
            ));
        }
        for pixel in rgba.chunks_exact_mut(4) {
            pixel.swap(0, 2);
            pixel[3] = 255;
        }
        let image = image::RgbaImage::from_raw(width as u32, height as u32, rgba).unwrap();
        let mut png = Cursor::new(Vec::new());
        image
            .write_to(&mut png, image::ImageFormat::Png)
            .map_err(|e| PortholeError::new(ErrorCode::InternalError, e.to_string()))?;
        let scale = GetDpiForWindow(hwnd) as f64 / 96.0;
        if scale == 0.0 {
            return Err(PortholeError::new(ErrorCode::SurfaceDead, "window disappeared during capture"));
        }
        Ok(Screenshot {
            png_bytes: png.into_inner(),
            window_bounds_points: Rect {
                x: rect.left as f64 / scale,
                y: rect.top as f64 / scale,
                w: width as f64 / scale,
                h: height as f64 / scale,
            },
            content_bounds_points: None,
            scale,
            captured_at_unix_ms: SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nonexistent_executable_returns_launch_failure_without_desktop_dependency() {
        let spec = ProcessLaunchSpec {
            app: format!(r"C:\porthole-nonexistent-{}\missing.exe", uuid::Uuid::new_v4()),
            args: vec![],
            cwd: None,
            env: vec![],
            timeout: Duration::from_millis(10),
            require_confidence: RequireConfidence::Strong,
            require_fresh_surface: true,
            force_place: false,
        };
        assert_eq!(spawn_process(&spec).unwrap_err().code, ErrorCode::LaunchCorrelationFailed);
    }

    struct Window(HWND);
    impl Window {
        fn new() -> Self {
            let hwnd = unsafe {
                CreateWindowExW(
                    0,
                    wide("STATIC").as_ptr(),
                    wide("identity-test").as_ptr(),
                    WS_OVERLAPPED,
                    0,
                    0,
                    32,
                    32,
                    null_mut(),
                    null_mut(),
                    null_mut(),
                    null_mut(),
                )
            };
            assert!(!hwnd.is_null());
            Self(hwnd)
        }
    }
    impl Drop for Window {
        fn drop(&mut self) {
            unsafe {
                DestroyWindow(self.0);
            }
        }
    }

    #[test]
    fn search_keeps_live_candidates_when_an_enumerated_window_disappears() {
        let adapter = WindowsAdapter::new();
        let window = Window::new();
        let candidates = adapter.search_windows(&SearchQuery::default(), [0, window.0 as usize, 0]).unwrap();
        assert_eq!(candidates.len(), 1);
        assert_eq!(
            candidates[0].platform_ref,
            adapter.identify(window.0).unwrap().platform_ref.unwrap()
        );
        assert!(adapter.capabilities().contains(&"wait"));
    }

    #[test]
    fn identity_is_stable_and_rejects_cookie_or_pid_reuse() {
        let adapter = WindowsAdapter::new();
        let window = Window::new(); // Hidden: no GUI focus or input required on CI.
        let surface = adapter.identify(window.0).unwrap();
        assert_eq!(adapter.identify(window.0).unwrap().platform_ref, surface.platform_ref);
        assert_eq!(adapter.resolve(&surface).unwrap(), window.0);
        let mut wrong_pid = surface.clone();
        wrong_pid.pid = Some(0);
        assert_eq!(adapter.resolve(&wrong_pid).unwrap_err().code, ErrorCode::SurfaceDead);
        unsafe {
            RemovePropW(window.0, adapter.property.as_ptr());
        }
        let replacement = adapter.identify(window.0).unwrap();
        assert_ne!(surface.platform_ref, replacement.platform_ref);
        assert_eq!(adapter.resolve(&surface).unwrap_err().code, ErrorCode::SurfaceDead);
        drop(window);
        assert_eq!(adapter.resolve(&replacement).unwrap_err().code, ErrorCode::SurfaceDead);
    }

    #[test]
    fn daemon_lifetimes_do_not_share_identity_and_drop_removes_only_own_property() {
        let first = WindowsAdapter::new();
        let second = WindowsAdapter::new();
        let window = Window::new();
        let surface = first.identify(window.0).unwrap();
        let property = first.property.clone();
        let other_surface = second.identify(window.0).unwrap();
        assert_eq!(second.resolve(&surface).unwrap_err().code, ErrorCode::SurfaceDead);
        drop(first);
        assert!(unsafe { GetPropW(window.0, property.as_ptr()) }.is_null());
        assert!(second.resolve(&other_surface).is_ok());
    }

    #[tokio::test]
    async fn unsupported_pixel_wait_reaches_shared_pipeline_as_unsupported() {
        use std::sync::Arc;

        use porthole_core::{
            handle::HandleStore,
            wait_pipeline::{WaitPipeline, WaitPipelineError},
        };
        let adapter = Arc::new(WindowsAdapter::new());
        let handles = HandleStore::new();
        let surface = SurfaceInfo::window(SurfaceId::new(), 123);
        handles.insert(surface.clone()).await;
        let pipeline = WaitPipeline::new(adapter, handles);
        let error = pipeline
            .wait(&surface.id, &WaitCondition::Dirty { threshold_pct: 1.0 }, Duration::from_secs(1))
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            WaitPipelineError::Porthole(PortholeError {
                code: ErrorCode::AdapterUnsupported,
                ..
            })
        ));
    }

    #[tokio::test]
    async fn absent_window_is_dead_and_foreign_platform_is_unsupported() {
        let adapter = WindowsAdapter::new();
        let missing = PlatformSurfaceRef::Windows { hwnd: 0, window_cookie: 1 };
        assert!(adapter.surface_alive(0, &missing).await.unwrap().is_none());
        assert_eq!(
            adapter.surface_alive(0, &PlatformSurfaceRef::macos(1)).await.unwrap_err().code,
            ErrorCode::AdapterUnsupported
        );
        let mut surface = SurfaceInfo::window(SurfaceId::new(), 0);
        surface.platform_ref = Some(missing);
        assert_eq!(adapter.close(&surface).await.unwrap_err().code, ErrorCode::SurfaceDead);
    }
}

#[cfg(test)]
#[path = "launch_tests.rs"]
mod launch_tests;
