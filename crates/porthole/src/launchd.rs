//! Thin wrappers over `launchctl` for the onboard flow.
//!
//! Used from `commands::onboard` to kickstart the daemon between permission
//! grants so its cached AX/SR trust state refreshes. The daemon itself is
//! registered as a launchd LaunchAgent by the macOS helper via
//! `SMAppService.agent` (see `apps/macos`), not from here. These operations
//! target the per-user GUI session domain (`gui/$UID`), which is the right
//! scope for a TCC-bound daemon — a system LaunchDaemon would have no per-user
//! identity for grants to attach to.

#[cfg(target_os = "macos")]
use std::{io, process::Command};

/// launchd job label of the portholed daemon agent. Must match the `Label` in
/// the bundled plist (`apps/macos/bundle/LaunchAgents/...daemon.plist`)
/// registered via SMAppService, so onboard's kickstart targets the right job.
pub const LAUNCH_AGENT_LABEL: &str = "work.flotilla.porthole.daemon";

#[derive(Debug, thiserror::Error)]
pub enum LaunchctlError {
    #[cfg(target_os = "macos")]
    #[error("launchctl exec failed: {0}")]
    Exec(#[from] io::Error),
    #[cfg(target_os = "macos")]
    #[error("launchctl {action} exit {code:?}: {stderr}")]
    NonZero {
        action: &'static str,
        code: Option<i32>,
        stderr: String,
    },
    #[cfg(not(target_os = "macos"))]
    #[error("launchd service management is not supported on this platform")]
    Unsupported,
}

#[cfg(target_os = "macos")]
fn current_uid() -> u32 {
    // SAFETY: getuid() has no preconditions and always succeeds on POSIX.
    unsafe { libc_getuid() }
}

#[cfg(target_os = "macos")]
unsafe extern "C" {
    #[link_name = "getuid"]
    fn libc_getuid() -> u32;
}

#[cfg(target_os = "macos")]
fn service_target() -> String {
    format!("gui/{}/{LAUNCH_AGENT_LABEL}", current_uid())
}

/// Restart the daemon: `launchctl kickstart -k gui/$UID/<label>` to kill,
/// then plain `launchctl kickstart gui/$UID/<label>` to guarantee it comes
/// back up.
///
/// Used by onboard between permission grants: AX and SR trust state is
/// loaded once per process and not refreshed, so a restart is the only way
/// to make the daemon see a freshly granted permission.
///
/// Why two steps: `kickstart -k` sends SIGTERM and then defers to the plist's
/// KeepAlive policy. The bundled daemon plist sets `KeepAlive=true`, so launchd
/// relaunches the daemon on its own after the SIGTERM. The trailing plain
/// `kickstart` is a belt-and-suspenders no-op — documented to "run the
/// specified service immediately, regardless of its configured launch
/// conditions" — that also covers a KeepAlive policy change or a fast race
/// where the daemon is already back up.
pub fn kickstart_kill() -> Result<(), LaunchctlError> {
    #[cfg(not(target_os = "macos"))]
    {
        Err(LaunchctlError::Unsupported)
    }
    #[cfg(target_os = "macos")]
    {
        let target = service_target();
        run_launchctl_kickstart("kickstart -k", &["kickstart", "-k", &target])?;
        run_launchctl_kickstart("kickstart", &["kickstart", &target])?;
        Ok(())
    }
}

/// Run a `launchctl kickstart[ -k]` invocation. `action` is the human-meaningful
/// step label that goes into the error on failure — distinct values for the two
/// invocations in `kickstart_kill` so a failure in the safety-net second step
/// doesn't look identical to a failure in the kill step.
#[cfg(target_os = "macos")]
fn run_launchctl_kickstart(action: &'static str, args: &[&str]) -> Result<(), LaunchctlError> {
    let output = Command::new("launchctl").args(args).output()?;
    if !output.status.success() {
        return Err(LaunchctlError::NonZero {
            action,
            code: output.status.code(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        });
    }
    Ok(())
}

/// True if the agent is currently loaded under launchd. Used by onboard to
/// decide whether daemon-restart is a thing it can do.
pub fn is_loaded() -> bool {
    #[cfg(not(target_os = "macos"))]
    {
        false
    }
    #[cfg(target_os = "macos")]
    {
        Command::new("launchctl")
            .args(["print", &service_target()])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }
}
