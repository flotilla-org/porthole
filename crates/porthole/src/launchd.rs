//! Thin wrappers over `launchctl` for the install / onboard flows.
//!
//! Used from `commands::install` (load + unload the LaunchAgent) and
//! `commands::onboard` (kickstart between permission grants so the daemon's
//! cached AX/SR trust state refreshes). All three operations target the
//! per-user GUI session domain (`gui/$UID`), which is the right scope for a
//! TCC-bound daemon — a system LaunchDaemon would have no per-user identity
//! for grants to attach to.

use std::{io, path::Path, process::Command};

pub const LAUNCH_AGENT_LABEL: &str = "org.flotilla.porthole";

#[derive(Debug, thiserror::Error)]
pub enum LaunchctlError {
    #[error("launchctl exec failed: {0}")]
    Exec(#[from] io::Error),
    #[error("launchctl {action} exit {code:?}: {stderr}")]
    NonZero {
        action: &'static str,
        code: Option<i32>,
        stderr: String,
    },
}

fn current_uid() -> u32 {
    // SAFETY: getuid() has no preconditions and always succeeds on POSIX.
    unsafe { libc_getuid() }
}

unsafe extern "C" {
    #[link_name = "getuid"]
    fn libc_getuid() -> u32;
}

fn target() -> String {
    format!("gui/{}", current_uid())
}

fn service_target() -> String {
    format!("gui/{}/{LAUNCH_AGENT_LABEL}", current_uid())
}

/// `launchctl bootstrap gui/$UID <plist>`. Loads and starts the agent.
pub fn bootstrap(plist_path: &Path) -> Result<(), LaunchctlError> {
    let output = Command::new("launchctl").args(["bootstrap", &target()]).arg(plist_path).output()?;
    if !output.status.success() {
        return Err(LaunchctlError::NonZero {
            action: "bootstrap",
            code: output.status.code(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        });
    }
    Ok(())
}

/// `launchctl bootout gui/$UID <plist>`. Idempotent: non-zero exit (typically
/// 113/EALREADY when the service isn't loaded) is the expected case on
/// fresh installs and is treated as success. Exec failure (launchctl missing
/// entirely) still surfaces as `Exec`. Other non-zero exits (permission
/// errors, malformed plist, etc.) are non-fatal — bootout is best-effort
/// before we re-write the plist anyway — but they're logged at warn so
/// operators have a signal something unexpected happened.
pub fn bootout(plist_path: &Path) -> Result<(), LaunchctlError> {
    let output = Command::new("launchctl").args(["bootout", &target()]).arg(plist_path).output()?;
    if !output.status.success() {
        // launchctl bootout exits 113 (EALREADY in some macOS versions, or a
        // generic "service not loaded" code in others) when there's nothing
        // to unload. That's the common path. Log everything else.
        let code = output.status.code().unwrap_or(-1);
        if code != 113 {
            tracing::warn!(
                exit_code = code,
                stderr = %String::from_utf8_lossy(&output.stderr).trim_end(),
                plist = %plist_path.display(),
                "launchctl bootout returned non-zero exit; continuing",
            );
        }
    }
    Ok(())
}

/// `launchctl kickstart -k gui/$UID/<label>`. Kills the running daemon and
/// restarts it. The `-k` flag is the bit that does the kill — without it
/// kickstart only starts an already-loaded service (no-op if running).
///
/// Used by onboard between permission grants: AX and SR trust state is
/// loaded once per process and not refreshed, so a restart is the only way
/// to make the daemon see a freshly granted permission.
pub fn kickstart_kill() -> Result<(), LaunchctlError> {
    let output = Command::new("launchctl").args(["kickstart", "-k", &service_target()]).output()?;
    if !output.status.success() {
        return Err(LaunchctlError::NonZero {
            action: "kickstart",
            code: output.status.code(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        });
    }
    Ok(())
}

/// True if the agent is currently loaded under launchd. Used by onboard to
/// decide whether daemon-restart is a thing it can do.
pub fn is_loaded() -> bool {
    Command::new("launchctl")
        .args(["print", &service_target()])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}
