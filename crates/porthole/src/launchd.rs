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

/// Restart the daemon: `launchctl kickstart -k gui/$UID/<label>` to kill,
/// then plain `launchctl kickstart gui/$UID/<label>` to guarantee it comes
/// back up.
///
/// Used by onboard between permission grants: AX and SR trust state is
/// loaded once per process and not refreshed, so a restart is the only way
/// to make the daemon see a freshly granted permission.
///
/// Why two steps: on macOS Tahoe (26 / Darwin 25), `kickstart -k` sends
/// SIGTERM and then defers to the plist's KeepAlive policy. Our plist sets
/// `KeepAlive={Crashed:true}`, which per `launchd.plist(5)` excludes
/// SIGTERM from the crash set — so the daemon stays down after `-k` alone.
/// Plain `kickstart` is documented to "run the specified service
/// immediately, regardless of its configured launch conditions", which
/// brings it back up. No-op if `-k` did restart on its own (older launchd
/// versions, or a fast race where the daemon's already up again).
pub fn kickstart_kill() -> Result<(), LaunchctlError> {
    let target = service_target();
    run_launchctl_kickstart("kickstart -k", &["kickstart", "-k", &target])?;
    run_launchctl_kickstart("kickstart", &["kickstart", &target])?;
    Ok(())
}

/// Run a `launchctl kickstart[ -k]` invocation. `action` is the human-meaningful
/// step label that goes into the error on failure — distinct values for the two
/// invocations in `kickstart_kill` so a failure in the safety-net second step
/// doesn't look identical to a failure in the kill step.
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
    Command::new("launchctl")
        .args(["print", &service_target()])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}
