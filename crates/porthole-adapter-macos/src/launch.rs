use std::time::{Duration, Instant};

use porthole_core::adapter::{Confidence, Correlation, LaunchOutcome, ProcessLaunchSpec};
use porthole_core::surface::{SurfaceId, SurfaceInfo, SurfaceKind, SurfaceState};
use porthole_core::{ErrorCode, PortholeError};
use tokio::process::Command;
use tokio::time::sleep;

use crate::correlation::{new_launch_tag, PORTHOLE_LAUNCH_TAG_ENV};
use crate::enumerate::list_windows;
use crate::MacOsAdapter;
use crate::permissions::ensure_accessibility_granted;

pub async fn launch_process(adapter: &MacOsAdapter, spec: &ProcessLaunchSpec) -> Result<LaunchOutcome, PortholeError> {
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (adapter, spec);
        return Err(PortholeError::new(ErrorCode::AdapterUnsupported, "macOS adapter on non-macOS"));
    }

    #[cfg(target_os = "macos")]
    {
        ensure_accessibility_granted(adapter)?;
        let tag = new_launch_tag();
        let child = build_and_spawn(spec, &tag)?;
        let deadline = Instant::now() + spec.timeout;
        loop {
            if let Some(window) = find_window_with_tag(&tag).await? {
                let surface = SurfaceInfo {
                    id: SurfaceId::new(),
                    kind: SurfaceKind::Window,
                    state: SurfaceState::Alive,
                    title: window.title,
                    app_name: window.app_name,
                    pid: Some(window.owner_pid as u32),
                    parent_surface_id: None,
                    cg_window_id: Some(window.cg_window_id),
                };
                return Ok(LaunchOutcome {
                    surface,
                    confidence: Confidence::Strong,
                    correlation: Correlation::Tag,
                    surface_was_preexisting: false,
                });
            }
            if Instant::now() >= deadline {
                // Clean up our launcher child if it's still around.
                drop(child);
                return Err(PortholeError::new(
                    ErrorCode::LaunchCorrelationFailed,
                    "no window found carrying the launch tag within the timeout",
                ));
            }
            sleep(Duration::from_millis(100)).await;
        }
    }
}

#[cfg(target_os = "macos")]
fn build_and_spawn(spec: &ProcessLaunchSpec, tag: &str) -> Result<tokio::process::Child, PortholeError> {
    let mut cmd = Command::new("/usr/bin/open");
    cmd.arg("-n").arg("-a").arg(&spec.app);
    cmd.arg("--env").arg(format!("{PORTHOLE_LAUNCH_TAG_ENV}={tag}"));
    for (k, v) in &spec.env {
        cmd.arg("--env").arg(format!("{k}={v}"));
    }
    if !spec.args.is_empty() {
        cmd.arg("--args");
        for a in &spec.args {
            cmd.arg(a);
        }
    }
    if let Some(cwd) = &spec.cwd {
        cmd.current_dir(cwd);
    }
    cmd.kill_on_drop(true);
    cmd.spawn().map_err(|e| PortholeError::new(ErrorCode::CapabilityMissing, format!("failed to spawn open: {e}")))
}

#[cfg(target_os = "macos")]
async fn find_window_with_tag(tag: &str) -> Result<Option<crate::enumerate::WindowRecord>, PortholeError> {
    let windows = list_windows()?;
    for window in windows {
        if pid_has_env(window.owner_pid, PORTHOLE_LAUNCH_TAG_ENV, tag).await {
            return Ok(Some(window));
        }
    }
    Ok(None)
}

#[cfg(target_os = "macos")]
async fn pid_has_env(pid: i32, key: &str, expected: &str) -> bool {
    let out = Command::new("/bin/ps").args(["eww", "-o", "command=", "-p", &pid.to_string()]).output().await;
    let Ok(out) = out else { return false };
    let text = String::from_utf8_lossy(&out.stdout);
    let needle = format!("{key}={expected}");
    text.contains(&needle)
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    #[tokio::test]
    async fn launch_missing_app_fails_with_correlation_failed() {
        let adapter = crate::MacOsAdapter::new();
        let spec = ProcessLaunchSpec {
            app: "/Applications/__definitely_not_installed__.app".to_string(),
            args: vec![],
            cwd: None,
            env: vec![],
            timeout: Duration::from_millis(500),
            require_confidence: porthole_core::adapter::RequireConfidence::Strong,
            require_fresh_surface: false,
        };
        let err = launch_process(&adapter, &spec).await.unwrap_err();
        // `open` will exit nonzero but our poll loop still hits the deadline.
        // If accessibility is not granted, we may get SystemPermissionNeeded instead.
        assert!(matches!(
            err.code,
            ErrorCode::LaunchCorrelationFailed | ErrorCode::CapabilityMissing | ErrorCode::SystemPermissionNeeded | ErrorCode::SystemPermissionRequestFailed
        ));
    }
}
