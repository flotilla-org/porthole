//! `porthole onboard` — interactive permission grant flow.
//!
//! macOS's AX/SR trust state is loaded once per process and cached
//! indefinitely; calling `AXIsProcessTrusted` or `CGPreflightScreenCaptureAccess`
//! at any point poisons the daemon's view, so a freshly granted permission
//! looks MISSING until the daemon restarts. TCC also coalesces back-to-back
//! prompt requests from the same process, so firing two prompts in quick
//! succession only surfaces one dialog.
//!
//! Both pathologies force the same shape: serialise prompts, restart the
//! daemon between each grant, verify the post-restart state.

use std::{
    io::{self, BufRead},
    time::Duration,
};

use async_trait::async_trait;
use porthole_protocol::{error::WireError, info::InfoResponse, system_permission::SystemPermissionPromptOutcome};

use crate::{
    client::{ClientError, DaemonClient},
    launchd,
};

pub struct OnboardOptions {
    /// Skip the per-permission Enter wait + auto-restart. Equivalent to "fire
    /// prompts and exit immediately" — caller does the rest manually.
    pub no_wait: bool,
    /// How long to wait for the daemon to come back up after kickstart, in
    /// seconds. Default 10.
    pub restart_timeout_seconds: u64,
}

impl Default for OnboardOptions {
    fn default() -> Self {
        Self {
            no_wait: false,
            restart_timeout_seconds: 10,
        }
    }
}

/// Return value carries the exit code the main binary should use.
pub struct OnboardResult {
    pub exit_code: i32,
}

/// True if `restart_daemon` actually restarted; false means the daemon is
/// not under launchd's control, so the caller has to handle restart manually.
pub type RestartHappened = bool;

#[async_trait]
pub trait OnboardClient: Send + Sync {
    async fn get_info(&self) -> Result<InfoResponse, ClientError>;
    async fn request_prompt(&self, name: &str) -> Result<SystemPermissionPromptOutcome, ClientError>;
    /// Restart the daemon so AX/SR cached trust state resets. Returns
    /// `Ok(true)` on actual restart, `Ok(false)` if not under launchd
    /// (caller surfaces a manual-restart hint).
    async fn restart_daemon(&self) -> Result<RestartHappened, ClientError>;
    /// Block until /info responds again. Implementations use polling with
    /// exponential backoff; tests no-op.
    async fn wait_until_ready(&self) -> Result<(), ClientError>;
    /// Block until the user signals to continue (typically by pressing Enter).
    /// Tests no-op.
    fn wait_for_user_continue(&self);
}

/// Real implementation: stdin-blocking Enter wait, launchctl-mediated restart,
/// HTTP polling for daemon readiness.
pub struct InteractiveOnboardClient<'a> {
    pub client: &'a DaemonClient,
    pub restart_timeout_seconds: u64,
}

#[async_trait]
impl OnboardClient for InteractiveOnboardClient<'_> {
    async fn get_info(&self) -> Result<InfoResponse, ClientError> {
        self.client.get_json("/info").await
    }
    async fn request_prompt(&self, name: &str) -> Result<SystemPermissionPromptOutcome, ClientError> {
        self.client
            .post_json("/system-permissions/request", &serde_json::json!({ "name": name }))
            .await
    }
    async fn restart_daemon(&self) -> Result<RestartHappened, ClientError> {
        if !launchd::is_loaded() {
            return Ok(false);
        }
        launchd::kickstart_kill().map_err(|e| ClientError::Local(format!("launchctl kickstart failed: {e}")))?;
        Ok(true)
    }
    async fn wait_until_ready(&self) -> Result<(), ClientError> {
        self.client
            .wait_until_ready(Duration::from_secs(self.restart_timeout_seconds))
            .await
    }
    fn wait_for_user_continue(&self) {
        let mut buf = String::new();
        let _ = io::stdin().lock().read_line(&mut buf);
    }
}

pub async fn run(client: &dyn OnboardClient, opts: OnboardOptions) -> Result<OnboardResult, ClientError> {
    let info = client.get_info().await?;
    let Some(adapter) = info.adapters.into_iter().next() else {
        println!("no adapters loaded");
        return Ok(OnboardResult { exit_code: 0 });
    };
    let perms = adapter.system_permissions;
    if perms.is_empty() {
        println!("adapter {} advertises no system permissions; nothing to onboard", adapter.name);
        return Ok(OnboardResult { exit_code: 0 });
    }

    let ungranted: Vec<String> = perms.iter().filter(|p| !p.granted).map(|p| p.name.clone()).collect();
    if ungranted.is_empty() {
        for p in &perms {
            println!("  system permission {}: granted", p.name);
        }
        return Ok(OnboardResult { exit_code: 0 });
    }

    let mut had_request_error = false;
    let mut still_missing: Vec<String> = vec![];

    for name in &ungranted {
        match client.request_prompt(name).await {
            Ok(_) => {
                println!();
                println!("  prompt fired for {name}");
                println!("  grant in: {}", settings_path_fallback(name));

                if opts.no_wait {
                    // Fire and forget; user resumes manually.
                    still_missing.push(name.clone());
                    continue;
                }

                println!("  press Enter when granted (or Ctrl+C to abort the rest of onboarding):");
                client.wait_for_user_continue();

                println!("  restarting daemon to refresh trust state...");
                let restarted = client.restart_daemon().await?;
                if !restarted {
                    eprintln!(
                        "  warning: daemon is not under launchd; auto-restart unavailable. Restart it manually, then re-run `porthole onboard` to verify."
                    );
                    eprintln!("  (Run `porthole install` to register the daemon with launchd.)");
                    still_missing.push(name.clone());
                    continue;
                }
                client.wait_until_ready().await?;

                let after = client.get_info().await?;
                let granted_now = after
                    .adapters
                    .first()
                    .and_then(|a| a.system_permissions.iter().find(|p| p.name == *name))
                    .map(|p| p.granted)
                    .unwrap_or(false);
                if granted_now {
                    println!("  ✓ {name}: granted");
                } else {
                    println!("  ✗ {name}: still missing — was the dialog dismissed without granting?");
                    still_missing.push(name.clone());
                }
            }
            Err(ClientError::Api(wire)) => {
                had_request_error = true;
                print_request_error(name, &wire);
            }
            Err(e) => return Err(e),
        }
    }

    if opts.no_wait {
        return Ok(OnboardResult { exit_code: 3 });
    }

    let exit_code = if had_request_error || !still_missing.is_empty() {
        if !still_missing.is_empty() {
            println!();
            println!("Still missing: {}.", still_missing.join(", "));
            println!("Grant in Settings and re-run `porthole onboard` to verify.");
        }
        1
    } else {
        0
    };
    Ok(OnboardResult { exit_code })
}

fn settings_path_fallback(name: &str) -> &'static str {
    match name {
        "accessibility" => "System Settings → Privacy & Security → Accessibility",
        "screen_recording" => "System Settings → Privacy & Security → Screen Recording",
        _ => "System Settings → Privacy & Security",
    }
}

fn print_request_error(name: &str, err: &WireError) {
    eprintln!("  request failed for {name}: {} ({})", err.message, err.code);
    if let Some(details) = &err.details {
        if let Some(settings) = details.get("settings_path").and_then(|v| v.as_str()) {
            eprintln!("    grant manually: {settings}");
        }
        if let Some(reason) = details.get("reason").and_then(|v| v.as_str()) {
            eprintln!("    os reason: {reason}");
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use porthole_core::ErrorCode;
    use porthole_protocol::info::{AdapterInfo, SystemPermissionStatus};

    use super::*;

    /// Scriptable fake. ClientError isn't Clone, so prompt outcomes are stored
    /// as Results-of-(SystemPermissionPromptOutcome, WireError) and rebuilt on demand.
    struct FakeClient {
        info_sequence: Mutex<Vec<InfoResponse>>,
        prompt_results: Mutex<Vec<Result<SystemPermissionPromptOutcome, WireError>>>,
        prompt_names_called: Mutex<Vec<String>>,
        restart_count: Mutex<u32>,
        restart_outcomes: Mutex<Vec<RestartHappened>>,
        continue_count: Mutex<u32>,
    }

    impl FakeClient {
        fn new(info_sequence: Vec<InfoResponse>, prompt_results: Vec<Result<SystemPermissionPromptOutcome, WireError>>) -> Self {
            Self {
                info_sequence: Mutex::new(info_sequence),
                prompt_results: Mutex::new(prompt_results),
                prompt_names_called: Mutex::new(vec![]),
                restart_count: Mutex::new(0),
                restart_outcomes: Mutex::new(vec![]),
                continue_count: Mutex::new(0),
            }
        }
        fn with_restart_outcomes(mut self, outcomes: Vec<RestartHappened>) -> Self {
            self.restart_outcomes = Mutex::new(outcomes);
            self
        }
    }

    #[async_trait]
    impl OnboardClient for FakeClient {
        async fn get_info(&self) -> Result<InfoResponse, ClientError> {
            let mut q = self.info_sequence.lock().unwrap();
            Ok(if q.len() > 1 { q.remove(0) } else { q[0].clone() })
        }
        async fn request_prompt(&self, name: &str) -> Result<SystemPermissionPromptOutcome, ClientError> {
            self.prompt_names_called.lock().unwrap().push(name.to_string());
            let mut q = self.prompt_results.lock().unwrap();
            let item = if q.len() > 1 { q.remove(0) } else { q[0].clone() };
            item.map_err(ClientError::Api)
        }
        async fn restart_daemon(&self) -> Result<RestartHappened, ClientError> {
            *self.restart_count.lock().unwrap() += 1;
            let mut q = self.restart_outcomes.lock().unwrap();
            Ok(if q.is_empty() {
                true
            } else if q.len() > 1 {
                q.remove(0)
            } else {
                q[0]
            })
        }
        async fn wait_until_ready(&self) -> Result<(), ClientError> {
            Ok(())
        }
        fn wait_for_user_continue(&self) {
            *self.continue_count.lock().unwrap() += 1;
        }
    }

    fn info_with(perms: Vec<(&str, bool)>) -> InfoResponse {
        InfoResponse {
            daemon_version: "test".into(),
            uptime_seconds: 0,
            adapters: vec![AdapterInfo {
                name: "fake".into(),
                loaded: true,
                capabilities: vec!["system_permission_prompt".into()],
                system_permissions: perms
                    .into_iter()
                    .map(|(n, g)| SystemPermissionStatus {
                        name: n.into(),
                        granted: g,
                        purpose: String::new(),
                    })
                    .collect(),
            }],
        }
    }

    fn outcome(name: &str, granted_after: bool, prompt_triggered: bool, requires_restart: bool) -> SystemPermissionPromptOutcome {
        SystemPermissionPromptOutcome {
            permission: name.into(),
            granted_before: false,
            granted_after,
            prompt_triggered,
            requires_daemon_restart: requires_restart,
            notes: String::new(),
        }
    }

    #[tokio::test]
    async fn all_granted_at_start_exits_zero_no_restart() {
        let client = FakeClient::new(vec![info_with(vec![("accessibility", true), ("screen_recording", true)])], vec![]);
        let res = run(&client, OnboardOptions::default()).await.unwrap();
        assert_eq!(res.exit_code, 0);
        assert_eq!(*client.restart_count.lock().unwrap(), 0);
    }

    #[tokio::test]
    async fn restart_called_once_per_grant_and_state_verified_post_restart() {
        // Initial: both missing.
        // After AX prompt + restart: AX granted, SR still missing.
        // After SR prompt + restart: both granted.
        let client = FakeClient::new(
            vec![
                info_with(vec![("accessibility", false), ("screen_recording", false)]),
                info_with(vec![("accessibility", true), ("screen_recording", false)]),
                info_with(vec![("accessibility", true), ("screen_recording", true)]),
            ],
            vec![
                Ok(outcome("accessibility", true, true, true)),
                Ok(outcome("screen_recording", true, true, false)),
            ],
        );
        let res = run(&client, OnboardOptions::default()).await.unwrap();
        assert_eq!(res.exit_code, 0);
        assert_eq!(*client.restart_count.lock().unwrap(), 2);
        assert_eq!(*client.continue_count.lock().unwrap(), 2);
    }

    #[tokio::test]
    async fn still_missing_after_restart_exits_one() {
        // User dismisses the dialog without granting; post-restart info still
        // shows MISSING for the prompted permission.
        let client = FakeClient::new(
            vec![info_with(vec![("accessibility", false)]), info_with(vec![("accessibility", false)])],
            vec![Ok(outcome("accessibility", false, true, true))],
        );
        let res = run(&client, OnboardOptions::default()).await.unwrap();
        assert_eq!(res.exit_code, 1);
    }

    #[tokio::test]
    async fn no_wait_skips_restart_and_exits_three() {
        let client = FakeClient::new(
            vec![info_with(vec![("accessibility", false)])],
            vec![Ok(outcome("accessibility", false, true, true))],
        );
        let res = run(
            &client,
            OnboardOptions {
                no_wait: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(res.exit_code, 3);
        assert_eq!(*client.restart_count.lock().unwrap(), 0);
    }

    #[tokio::test]
    async fn not_under_launchd_warns_and_marks_still_missing() {
        let client = FakeClient::new(
            vec![info_with(vec![("accessibility", false)])],
            vec![Ok(outcome("accessibility", false, true, true))],
        )
        .with_restart_outcomes(vec![false]);
        let res = run(&client, OnboardOptions::default()).await.unwrap();
        // Restart was attempted but reported "not under launchd"; permission
        // can't be auto-verified so it counts as still-missing → exit 1.
        assert_eq!(res.exit_code, 1);
        assert_eq!(*client.restart_count.lock().unwrap(), 1);
    }

    #[tokio::test]
    async fn request_error_forces_exit_one() {
        let wire = WireError {
            code: ErrorCode::SystemPermissionRequestFailed,
            message: "bundle missing".into(),
            details: Some(serde_json::json!({
                "permission": "accessibility",
                "reason": "not in bundle",
                "settings_path": "Settings → ...",
                "binary_path": "/x"
            })),
        };
        let client = FakeClient::new(
            vec![info_with(vec![("accessibility", false)]), info_with(vec![("accessibility", true)])],
            vec![Err(wire)],
        );
        let res = run(&client, OnboardOptions::default()).await.unwrap();
        assert_eq!(res.exit_code, 1);
        // Request error means we never reach the restart path.
        assert_eq!(*client.restart_count.lock().unwrap(), 0);
    }

    #[tokio::test]
    async fn prompts_serialise_one_at_a_time() {
        // Verifies the request-prompt order: AX before SR (not back-to-back).
        // The fake's prompt_results is a queue, so consuming both means each
        // request_prompt was called once in order.
        let client = FakeClient::new(
            vec![
                info_with(vec![("accessibility", false), ("screen_recording", false)]),
                info_with(vec![("accessibility", true), ("screen_recording", false)]),
                info_with(vec![("accessibility", true), ("screen_recording", true)]),
            ],
            vec![
                Ok(outcome("accessibility", true, true, true)),
                Ok(outcome("screen_recording", true, true, false)),
            ],
        );
        run(&client, OnboardOptions::default()).await.unwrap();
        let prompted = client.prompt_names_called.lock().unwrap().clone();
        assert_eq!(prompted, vec!["accessibility".to_string(), "screen_recording".to_string()]);
        assert_eq!(*client.restart_count.lock().unwrap(), 2);
    }
}
