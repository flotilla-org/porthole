use std::time::{Duration, Instant};

use async_trait::async_trait;
use porthole_protocol::{error::WireError, info::InfoResponse, system_permission::SystemPermissionPromptOutcome};

use crate::client::{ClientError, DaemonClient};

pub struct OnboardOptions {
    pub wait_seconds: u64,
    pub no_wait: bool,
}

impl Default for OnboardOptions {
    fn default() -> Self {
        Self {
            wait_seconds: 60,
            no_wait: false,
        }
    }
}

/// Return value carries the exit code the main binary should use.
pub struct OnboardResult {
    pub exit_code: i32,
}

#[async_trait]
pub trait OnboardClient: Send + Sync {
    async fn get_info(&self) -> Result<InfoResponse, ClientError>;
    async fn request_prompt(&self, name: &str) -> Result<SystemPermissionPromptOutcome, ClientError>;
}

#[async_trait]
impl OnboardClient for DaemonClient {
    async fn get_info(&self) -> Result<InfoResponse, ClientError> {
        self.get_json("/info").await
    }
    async fn request_prompt(&self, name: &str) -> Result<SystemPermissionPromptOutcome, ClientError> {
        self.post_json("/system-permissions/request", &serde_json::json!({ "name": name }))
            .await
    }
}

pub async fn run(client: &dyn OnboardClient, opts: OnboardOptions) -> Result<OnboardResult, ClientError> {
    // 1. Read initial /info.
    let info: InfoResponse = client.get_info().await?;
    let Some(adapter) = info.adapters.into_iter().next() else {
        println!("no adapters loaded");
        return Ok(OnboardResult { exit_code: 0 });
    };
    let perms = adapter.system_permissions;
    if perms.is_empty() {
        println!("adapter {} advertises no system permissions; nothing to onboard", adapter.name);
        return Ok(OnboardResult { exit_code: 0 });
    }

    let granted_before: Vec<(String, bool)> = perms.iter().map(|p| (p.name.clone(), p.granted)).collect();
    let ungranted: Vec<String> = perms.iter().filter(|p| !p.granted).map(|p| p.name.clone()).collect();

    if ungranted.is_empty() {
        for p in &perms {
            println!("  system permission {}: granted", p.name);
        }
        return Ok(OnboardResult { exit_code: 0 });
    }

    // 2. Request prompts for each ungranted permission.
    let mut had_request_error = false;
    let mut restart_required_seen = false;
    for name in &ungranted {
        match client.request_prompt(name).await {
            Ok(out) => {
                if out.requires_daemon_restart {
                    restart_required_seen = true;
                }
                if out.prompt_triggered {
                    println!(
                        "  prompt requested for {name} — a dialog should appear unless previously denied (in which case grant via {})",
                        settings_path_fallback(name)
                    );
                } else {
                    println!(
                        "  prompt already fired earlier this daemon session for {name} — grant via {} (re-arm dialog by restarting the daemon)",
                        settings_path_fallback(name)
                    );
                }
            }
            Err(ClientError::Api(wire)) => {
                had_request_error = true;
                print_request_error(name, &wire);
            }
            Err(e) => {
                return Err(e);
            }
        }
    }

    // 3. Optionally skip polling.
    if opts.no_wait || opts.wait_seconds == 0 {
        return Ok(OnboardResult { exit_code: 3 });
    }

    // 4. Poll /info until all granted or timeout.
    let deadline = Instant::now() + Duration::from_secs(opts.wait_seconds);
    let final_info = poll_until_granted(client, deadline).await?;
    let final_perms = final_info
        .adapters
        .into_iter()
        .next()
        .map(|a| a.system_permissions)
        .unwrap_or_default();

    // 5. Summarise.
    let mut any_transition_requires_restart = false;
    for p in &final_perms {
        let before = granted_before.iter().find(|(n, _)| n == &p.name).map(|(_, b)| *b).unwrap_or(false);
        let transitioned = !before && p.granted;
        let status = if p.granted { "granted" } else { "MISSING" };
        println!(
            "  system permission {}: {}{}",
            p.name,
            status,
            if transitioned { " (granted this session)" } else { "" }
        );
        if transitioned && restart_required_flag_for(&p.name) {
            any_transition_requires_restart = true;
        }
    }

    let any_still_missing = final_perms.iter().any(|p| !p.granted);

    // 6. Exit code.
    let exit_code = if any_still_missing || had_request_error {
        if any_still_missing {
            println!("\nAt least one permission is still ungranted. Grant in Settings and re-run `porthole onboard`.");
        }
        1
    } else if any_transition_requires_restart
        || (restart_required_seen && granted_before.iter().any(|(n, b)| !b && restart_required_flag_for(n)))
    {
        println!("\nAll permissions granted. Restart the daemon before using Accessibility-dependent features.");
        2
    } else {
        0
    };

    Ok(OnboardResult { exit_code })
}

async fn poll_until_granted(client: &dyn OnboardClient, deadline: Instant) -> Result<InfoResponse, ClientError> {
    loop {
        let info: InfoResponse = client.get_info().await?;
        let all_granted = info
            .adapters
            .first()
            .map(|a| a.system_permissions.iter().all(|p| p.granted))
            .unwrap_or(true);
        if all_granted || Instant::now() >= deadline {
            return Ok(info);
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

fn restart_required_flag_for(name: &str) -> bool {
    matches!(name, "accessibility")
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

    // ClientError isn't Clone, so the fake stores WireError (which is Clone)
    // and reconstructs ClientError::Api on demand.
    struct FakeClient {
        info_sequence: Mutex<Vec<InfoResponse>>,
        prompt_results: Mutex<Vec<Result<SystemPermissionPromptOutcome, WireError>>>,
    }

    #[async_trait]
    impl OnboardClient for FakeClient {
        async fn get_info(&self) -> Result<InfoResponse, ClientError> {
            let mut q = self.info_sequence.lock().unwrap();
            Ok(if q.len() > 1 { q.remove(0) } else { q[0].clone() })
        }
        async fn request_prompt(&self, _name: &str) -> Result<SystemPermissionPromptOutcome, ClientError> {
            let mut q = self.prompt_results.lock().unwrap();
            let item = if q.len() > 1 { q.remove(0) } else { q[0].clone() };
            item.map_err(ClientError::Api)
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
    async fn all_granted_at_start_exits_zero() {
        let client = FakeClient {
            info_sequence: Mutex::new(vec![info_with(vec![("accessibility", true), ("screen_recording", true)])]),
            prompt_results: Mutex::new(vec![]),
        };
        let res = run(
            &client,
            OnboardOptions {
                wait_seconds: 1,
                no_wait: false,
            },
        )
        .await
        .unwrap();
        assert_eq!(res.exit_code, 0);
    }

    #[tokio::test]
    async fn ax_transition_to_granted_exits_two() {
        let client = FakeClient {
            info_sequence: Mutex::new(vec![
                info_with(vec![("accessibility", false), ("screen_recording", true)]),
                info_with(vec![("accessibility", true), ("screen_recording", true)]),
            ]),
            prompt_results: Mutex::new(vec![Ok(outcome("accessibility", false, true, true))]),
        };
        let res = run(
            &client,
            OnboardOptions {
                wait_seconds: 1,
                no_wait: false,
            },
        )
        .await
        .unwrap();
        assert_eq!(res.exit_code, 2);
    }

    #[tokio::test]
    async fn screen_recording_transition_exits_zero() {
        let client = FakeClient {
            info_sequence: Mutex::new(vec![
                info_with(vec![("screen_recording", false)]),
                info_with(vec![("screen_recording", true)]),
            ]),
            prompt_results: Mutex::new(vec![Ok(outcome("screen_recording", false, true, false))]),
        };
        let res = run(
            &client,
            OnboardOptions {
                wait_seconds: 1,
                no_wait: false,
            },
        )
        .await
        .unwrap();
        assert_eq!(res.exit_code, 0);
    }

    #[tokio::test]
    async fn still_ungranted_after_poll_exits_one() {
        let client = FakeClient {
            info_sequence: Mutex::new(vec![info_with(vec![("accessibility", false)])]),
            prompt_results: Mutex::new(vec![Ok(outcome("accessibility", false, true, true))]),
        };
        let res = run(
            &client,
            OnboardOptions {
                wait_seconds: 1,
                no_wait: false,
            },
        )
        .await
        .unwrap();
        assert_eq!(res.exit_code, 1);
    }

    #[tokio::test]
    async fn no_wait_exits_three_without_polling() {
        let client = FakeClient {
            info_sequence: Mutex::new(vec![info_with(vec![("accessibility", false)])]),
            prompt_results: Mutex::new(vec![Ok(outcome("accessibility", false, true, true))]),
        };
        let res = run(
            &client,
            OnboardOptions {
                wait_seconds: 1,
                no_wait: true,
            },
        )
        .await
        .unwrap();
        assert_eq!(res.exit_code, 3);
    }

    #[tokio::test]
    async fn wait_zero_exits_three_without_polling() {
        let client = FakeClient {
            info_sequence: Mutex::new(vec![info_with(vec![("accessibility", false)])]),
            prompt_results: Mutex::new(vec![Ok(outcome("accessibility", false, true, true))]),
        };
        let res = run(
            &client,
            OnboardOptions {
                wait_seconds: 0,
                no_wait: false,
            },
        )
        .await
        .unwrap();
        assert_eq!(res.exit_code, 3);
    }

    #[tokio::test]
    async fn request_error_forces_exit_one_even_if_info_shows_granted() {
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
        let client = FakeClient {
            info_sequence: Mutex::new(vec![
                info_with(vec![("accessibility", false)]),
                info_with(vec![("accessibility", true)]),
            ]),
            prompt_results: Mutex::new(vec![Err(wire)]),
        };
        let res = run(
            &client,
            OnboardOptions {
                wait_seconds: 1,
                no_wait: false,
            },
        )
        .await
        .unwrap();
        assert_eq!(res.exit_code, 1);
    }
}
