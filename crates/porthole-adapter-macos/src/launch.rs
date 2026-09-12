use std::{collections::HashSet, sync::Mutex, time::Duration};

use block2::RcBlock;
use objc2::rc::Retained;
use objc2_app_kit::{NSRunningApplication, NSWorkspace, NSWorkspaceOpenConfiguration};
use objc2_foundation::{NSArray, NSDictionary, NSError, NSString, NSURL};
use porthole_core::{
    ErrorCode, PortholeError,
    adapter::{Confidence, Correlation, LaunchOutcome, ProcessLaunchSpec},
    surface::{PlatformSurfaceRef, SurfaceId, SurfaceInfo, SurfaceKind, SurfaceState},
};
use tokio::{
    sync::oneshot,
    time::{Instant, sleep, timeout_at},
};

use crate::{
    MacOsAdapter,
    correlation::select_window,
    enumerate::{list_all_windows, list_windows},
    permissions::ensure_accessibility_granted,
};

pub async fn launch_process(adapter: &MacOsAdapter, spec: &ProcessLaunchSpec) -> Result<LaunchOutcome, PortholeError> {
    validate_spec(spec)?;
    ensure_accessibility_granted(adapter)?;
    let deadline = Instant::now() + spec.timeout;
    let before: HashSet<_> = list_all_windows()?.into_iter().map(|w| (w.owner_pid, w.cg_window_id)).collect();
    // The callback converts AppKit objects to an owned Rust result. No ObjC
    // objects or borrowed callback arguments cross an await point.
    let completion = start_application(spec)?;
    let pid = timeout_at(deadline, completion)
        .await
        .map_err(|_| {
            PortholeError::new(
                ErrorCode::LaunchTimeout,
                "timed out waiting for LaunchServices to launch the application",
            )
        })?
        .map_err(|_| PortholeError::new(ErrorCode::InternalError, "application launch completion was dropped"))??;
    loop {
        let at_deadline = Instant::now() >= deadline;
        if let Some((window, surface_was_preexisting)) = select_window(pid, &before, &list_windows()?, at_deadline)? {
            return Ok(LaunchOutcome {
                surface: SurfaceInfo {
                    id: SurfaceId::new(),
                    kind: SurfaceKind::Window,
                    state: SurfaceState::Alive,
                    title: window.title,
                    app_name: window.app_name,
                    pid: Some(window.owner_pid as u32),
                    parent_surface_id: None,
                    platform_ref: Some(PlatformSurfaceRef::macos(window.cg_window_id)),
                },
                confidence: Confidence::Strong,
                // Exact root-PID ownership is the simplest PID-tree match.
                correlation: Correlation::PidTree,
                surface_was_preexisting,
            });
        }
        if at_deadline {
            return Err(PortholeError::new(
                ErrorCode::LaunchCorrelationFailed,
                format!("no visible window owned by launched application PID {pid} within the timeout"),
            ));
        }
        sleep(Duration::from_millis(100).min(deadline.saturating_duration_since(Instant::now()))).await;
    }
}

fn validate_spec(spec: &ProcessLaunchSpec) -> Result<(), PortholeError> {
    if spec.cwd.is_some() && !is_executable_path(&spec.app) {
        return Err(PortholeError::new(
            ErrorCode::AdapterUnsupported,
            "macOS application bundle launch does not support cwd; use application-specific arguments",
        ));
    }
    if spec.app.is_empty() {
        return Err(PortholeError::new(ErrorCode::InvalidArgument, "application name or path is empty"));
    }
    Ok(())
}

fn application_url(workspace: &NSWorkspace, app: &str) -> Result<Retained<NSURL>, PortholeError> {
    if app.contains('/') {
        let path = std::fs::canonicalize(app)
            .map_err(|error| PortholeError::new(ErrorCode::CapabilityMissing, format!("cannot resolve application {app}: {error}")))?;
        let path = path
            .to_str()
            .ok_or_else(|| PortholeError::new(ErrorCode::InvalidArgument, "application path is not UTF-8"))?;
        return Ok(unsafe { NSURL::fileURLWithPath(&NSString::from_str(path)) });
    }
    let name = NSString::from_str(app);
    // Bundle identifiers have a modern resolver; retain LaunchServices name
    // lookup for the CLI's existing --app TextEdit/Terminal interface.
    unsafe {
        if let Some(url) = workspace.URLForApplicationWithBundleIdentifier(&name) {
            return Ok(url);
        }
        #[allow(deprecated)]
        if let Some(path) = workspace.fullPathForApplication(&name) {
            return Ok(NSURL::fileURLWithPath(&path));
        }
    }
    Err(PortholeError::new(
        ErrorCode::CapabilityMissing,
        format!("application not found: {app}"),
    ))
}

fn launch_configuration(spec: &ProcessLaunchSpec) -> Retained<NSWorkspaceOpenConfiguration> {
    let arguments = NSArray::from_vec(spec.args.iter().map(|s| NSString::from_str(s)).collect());
    let env: std::collections::BTreeMap<_, _> = spec.env.iter().cloned().collect();
    let keys: Vec<_> = env.keys().map(|key| NSString::from_str(key)).collect();
    let keys: Vec<_> = keys.iter().map(|key| &**key).collect();
    let values = env.values().map(|value| NSString::from_str(value)).collect();
    let environment = NSDictionary::from_vec(&keys, values);
    unsafe {
        let config = NSWorkspaceOpenConfiguration::configuration();
        config.setCreatesNewApplicationInstance(true);
        config.setArguments(&arguments);
        config.setEnvironment(&environment);
        config
    }
}

fn is_executable_path(app: &str) -> bool {
    app.contains('/') && std::path::Path::new(app).is_file()
}

fn start_application(spec: &ProcessLaunchSpec) -> Result<oneshot::Receiver<Result<i32, PortholeError>>, PortholeError> {
    let (sender, receiver) = oneshot::channel();
    if is_executable_path(&spec.app) {
        let executable = std::fs::canonicalize(&spec.app)
            .map_err(|error| PortholeError::new(ErrorCode::LaunchCorrelationFailed, format!("cannot resolve executable: {error}")))?;
        let mut command = tokio::process::Command::new(executable);
        command.args(&spec.args).envs(spec.env.iter().cloned());
        if let Some(cwd) = &spec.cwd {
            command.current_dir(cwd);
        }
        let mut child = command
            .spawn()
            .map_err(|error| PortholeError::new(ErrorCode::LaunchCorrelationFailed, format!("failed to launch executable: {error}")))?;
        let pid = child.id().expect("newly spawned child has a PID") as i32;
        // Reap the child without tying application lifetime to the request.
        tokio::spawn(async move {
            let _ = child.wait().await;
        });
        let _ = sender.send(Ok(pid));
        return Ok(receiver);
    }
    let sender = Mutex::new(Some(sender));
    let callback = RcBlock::new(move |app: *mut NSRunningApplication, error: *mut NSError| {
        // AppKit keeps these objects valid for the duration of the callback.
        let result = unsafe {
            if let Some(error) = error.as_ref() {
                Err(PortholeError::new(
                    ErrorCode::LaunchCorrelationFailed,
                    format!("application launch failed: {}", error.localizedDescription()),
                ))
            } else if let Some(app) = app.as_ref() {
                let pid = app.processIdentifier();
                if pid > 0 {
                    Ok(pid)
                } else {
                    Err(PortholeError::new(
                        ErrorCode::LaunchCorrelationFailed,
                        "LaunchServices returned no running application PID",
                    ))
                }
            } else {
                Err(PortholeError::new(
                    ErrorCode::LaunchCorrelationFailed,
                    "LaunchServices returned neither an application nor an error",
                ))
            }
        };
        if let Some(sender) = sender.lock().expect("launch completion lock poisoned").take() {
            // A timeout may have dropped the receiver; the app may still launch.
            let _ = sender.send(result);
        }
    });
    unsafe {
        let workspace = NSWorkspace::sharedWorkspace();
        let url = application_url(&workspace, &spec.app)?;
        let config = launch_configuration(spec);
        // AppKit copies the escaping block and invokes it on a concurrent queue.
        workspace.openApplicationAtURL_configuration_completionHandler(&url, &config, Some(&callback));
    }
    Ok(receiver)
}

#[cfg(test)]
mod tests {
    use porthole_core::adapter::RequireConfidence;

    use super::*;

    fn spec() -> ProcessLaunchSpec {
        ProcessLaunchSpec {
            app: "TextEdit".into(),
            args: vec!["one argument".into()],
            cwd: None,
            env: vec![("TEST_LAUNCH_VALUE".into(), "one value".into())],
            timeout: Duration::from_secs(2),
            require_confidence: RequireConfidence::Strong,
            require_fresh_surface: false,
            force_place: false,
        }
    }

    #[test]
    fn native_configuration_preserves_arguments_and_environment() {
        let config = launch_configuration(&spec());
        unsafe {
            assert!(config.createsNewApplicationInstance());
            assert_eq!(config.arguments().objectAtIndex(0).to_string(), "one argument");
            assert_eq!(
                config
                    .environment()
                    .objectForKey(&NSString::from_str("TEST_LAUNCH_VALUE"))
                    .unwrap()
                    .to_string(),
                "one value"
            );
        }
    }

    #[test]
    fn working_directory_is_rejected_instead_of_silently_ignored() {
        let mut spec = spec();
        spec.cwd = Some("/tmp".into());
        assert_eq!(validate_spec(&spec).unwrap_err().code, ErrorCode::AdapterUnsupported);
    }

    #[test]
    fn executable_launch_accepts_working_directory() {
        let mut spec = spec();
        spec.app = "/bin/sleep".into();
        spec.cwd = Some("/tmp".into());
        assert!(validate_spec(&spec).is_ok());
    }

    #[test]
    fn nonexistent_application_fails_during_resolution() {
        let workspace = unsafe { NSWorkspace::sharedWorkspace() };
        assert_eq!(
            application_url(&workspace, "/Applications/__porthole_not_installed__.app")
                .unwrap_err()
                .code,
            ErrorCode::CapabilityMissing
        );
    }
}
