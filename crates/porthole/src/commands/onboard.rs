use std::time::{Duration, Instant};

use porthole_protocol::error::WireError;
use porthole_protocol::info::InfoResponse;
use porthole_protocol::system_permission::SystemPermissionPromptOutcome;

use crate::client::{ClientError, DaemonClient};

pub struct OnboardOptions {
    pub wait_seconds: u64,
    pub no_wait: bool,
}

impl Default for OnboardOptions {
    fn default() -> Self {
        Self { wait_seconds: 60, no_wait: false }
    }
}

/// Return value carries the exit code the main binary should use.
pub struct OnboardResult {
    pub exit_code: i32,
}

pub async fn run(client: &DaemonClient, opts: OnboardOptions) -> Result<OnboardResult, ClientError> {
    // 1. Read initial /info.
    let info: InfoResponse = client.get_json("/info").await?;
    let Some(adapter) = info.adapters.into_iter().next() else {
        println!("no adapters loaded");
        return Ok(OnboardResult { exit_code: 0 });
    };
    let perms = adapter.system_permissions;
    if perms.is_empty() {
        println!("adapter {} advertises no system permissions; nothing to onboard", adapter.name);
        return Ok(OnboardResult { exit_code: 0 });
    }

    let granted_before: Vec<(String, bool)> =
        perms.iter().map(|p| (p.name.clone(), p.granted)).collect();
    let ungranted: Vec<String> =
        perms.iter().filter(|p| !p.granted).map(|p| p.name.clone()).collect();

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
        match client
            .post_json::<serde_json::Value, SystemPermissionPromptOutcome>(
                "/system-permissions/request",
                &serde_json::json!({ "name": name }),
            )
            .await
        {
            Ok(out) => {
                if out.requires_daemon_restart {
                    restart_required_seen = true;
                }
                if out.prompt_triggered {
                    println!("  dialog opened for {name}");
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
    if opts.no_wait {
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
        let before = granted_before
            .iter()
            .find(|(n, _)| n == &p.name)
            .map(|(_, b)| *b)
            .unwrap_or(false);
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
            println!(
                "\nAt least one permission is still ungranted. Grant in Settings and re-run `porthole onboard`."
            );
        }
        1
    } else if any_transition_requires_restart
        || (restart_required_seen
            && granted_before.iter().any(|(n, b)| !b && restart_required_flag_for(n)))
    {
        println!(
            "\nAll permissions granted. Restart the daemon before using Accessibility-dependent features."
        );
        2
    } else {
        0
    };

    Ok(OnboardResult { exit_code })
}

async fn poll_until_granted(
    client: &DaemonClient,
    deadline: Instant,
) -> Result<InfoResponse, ClientError> {
    #[allow(unused_assignments)]
    let mut last_seen: Option<InfoResponse> = None;
    loop {
        let info: InfoResponse = client.get_json("/info").await?;
        let all_granted = info
            .adapters
            .first()
            .map(|a| a.system_permissions.iter().all(|p| p.granted))
            .unwrap_or(true);
        last_seen = Some(info);
        if all_granted || Instant::now() >= deadline {
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    Ok(last_seen.expect("at least one /info read"))
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
