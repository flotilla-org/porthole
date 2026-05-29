use std::{
    collections::{HashMap, HashSet},
    path::Path,
    time::{Duration, Instant},
};

use porthole_core::{
    ErrorCode, PortholeError,
    adapter::{Confidence, Correlation, LaunchOutcome, ProcessLaunchSpec},
};
use tokio::{process::Command, time::sleep};

use crate::{
    KWinAdapter,
    snapshot::{KWinSnapshotPayload, KWinWindow},
    surface_from_window,
};

const LAUNCH_POLL: Duration = Duration::from_millis(100);

pub(crate) async fn launch_process(adapter: &KWinAdapter, spec: &ProcessLaunchSpec) -> Result<LaunchOutcome, PortholeError> {
    let before = adapter.snapshot().await.ok();
    let preexisting = before.as_ref().map(window_ids).unwrap_or_default();
    let preexisting_active_window_id = before.as_ref().and_then(active_window_id);
    let launched_pid = spawn_process(spec)?;
    let started = Instant::now();
    let deadline = started + spec.timeout;
    let app_hint = app_hint(&spec.app);
    let mut saw_multiple_pid_tree_matches = false;

    loop {
        let _ = adapter.refresh_snapshot().await;
        let snapshot = adapter.snapshot().await?;
        let pids = pid_tree(launched_pid).await;
        let new_pid_matches = new_pid_tree_matches(&snapshot, &preexisting, &pids);
        if new_pid_matches.len() == 1 {
            return Ok(LaunchOutcome {
                surface: surface_from_window(new_pid_matches[0]),
                confidence: Confidence::Strong,
                correlation: Correlation::PidTree,
                surface_was_preexisting: false,
            });
        }
        if new_pid_matches.len() > 1 {
            saw_multiple_pid_tree_matches = true;
        }

        if let Some(window) = active_app_match(&snapshot, &app_hint, preexisting_active_window_id.as_deref(), &preexisting) {
            return Ok(LaunchOutcome {
                surface: surface_from_window(window),
                confidence: Confidence::Plausible,
                correlation: Correlation::FrontmostChanged,
                surface_was_preexisting: preexisting.contains(&window.window_id),
            });
        }

        if Instant::now() >= deadline {
            if saw_multiple_pid_tree_matches {
                return Err(PortholeError::new(
                    ErrorCode::LaunchCorrelationAmbiguous,
                    format!("multiple new KWin windows matched launched pid tree rooted at {launched_pid}"),
                ));
            }
            return Err(PortholeError::new(
                ErrorCode::LaunchCorrelationFailed,
                format!(
                    "no KWin window matched launched pid tree rooted at {launched_pid} or active app hint {app_hint:?} within {:?}",
                    started.elapsed()
                ),
            ));
        }
        sleep(LAUNCH_POLL).await;
    }
}

fn spawn_process(spec: &ProcessLaunchSpec) -> Result<u32, PortholeError> {
    let mut command = Command::new(&spec.app);
    command.args(&spec.args);
    if let Some(cwd) = &spec.cwd {
        command.current_dir(cwd);
    }
    for (key, value) in &spec.env {
        command.env(key, value);
    }
    let mut child = command
        .spawn()
        .map_err(|error| PortholeError::new(ErrorCode::CapabilityMissing, format!("failed to launch {}: {error}", spec.app)))?;
    let pid = child.id().ok_or_else(|| {
        PortholeError::new(
            ErrorCode::LaunchCorrelationFailed,
            format!("launched process {} did not expose a pid", spec.app),
        )
    })?;
    tokio::spawn(async move {
        let _ = child.wait().await;
    });
    Ok(pid)
}

fn window_ids(snapshot: &KWinSnapshotPayload) -> HashSet<String> {
    snapshot.windows.iter().map(|window| window.window_id.clone()).collect()
}

fn active_window_id(snapshot: &KWinSnapshotPayload) -> Option<String> {
    snapshot
        .active_window
        .as_ref()
        .or_else(|| snapshot.windows.iter().find(|window| window.active))
        .map(|window| window.window_id.clone())
}

fn new_pid_tree_matches<'a>(snapshot: &'a KWinSnapshotPayload, preexisting: &HashSet<String>, pids: &HashSet<u32>) -> Vec<&'a KWinWindow> {
    snapshot
        .windows
        .iter()
        .filter(|window| window.normal_window && !preexisting.contains(&window.window_id) && pids.contains(&window.pid))
        .collect()
}

fn active_app_match<'a>(
    snapshot: &'a KWinSnapshotPayload,
    app_hint: &str,
    preexisting_active_window_id: Option<&str>,
    preexisting: &HashSet<String>,
) -> Option<&'a KWinWindow> {
    if app_hint.is_empty() {
        return None;
    }
    snapshot
        .active_window
        .as_ref()
        .or_else(|| snapshot.windows.iter().find(|window| window.active))
        .filter(|window| {
            window.normal_window
                && window_matches_app_hint(window, app_hint)
                && (!preexisting.contains(&window.window_id) || Some(window.window_id.as_str()) != preexisting_active_window_id)
        })
}

fn app_hint(app: &str) -> String {
    Path::new(app)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or(app)
        .trim_end_matches(".desktop")
        .to_lowercase()
}

fn window_matches_app_hint(window: &KWinWindow, app_hint: &str) -> bool {
    [
        window.desktop_file_name.as_deref(),
        window.resource_class.as_deref(),
        window.resource_name.as_deref(),
        window.caption.as_deref(),
    ]
    .into_iter()
    .flatten()
    .map(|value| value.to_lowercase())
    .any(|value| value.contains(app_hint) || app_hint.contains(&value))
}

async fn pid_tree(root: u32) -> HashSet<u32> {
    let parent_by_pid = process_parent_map().await.unwrap_or_default();
    let mut out = HashSet::from([root]);
    let mut changed = true;
    while changed {
        changed = false;
        for (pid, ppid) in &parent_by_pid {
            if !out.contains(pid) && out.contains(ppid) {
                out.insert(*pid);
                changed = true;
            }
        }
    }
    out
}

async fn process_parent_map() -> Result<HashMap<u32, u32>, PortholeError> {
    let output = Command::new("ps").args(["-eo", "pid=,ppid="]).output().await.map_err(|error| {
        PortholeError::new(
            ErrorCode::CapabilityMissing,
            format!("failed to inspect process tree with ps: {error}"),
        )
    })?;
    if !output.status.success() {
        return Err(PortholeError::new(
            ErrorCode::CapabilityMissing,
            format!("ps exited with status {}", output.status),
        ));
    }
    Ok(parse_ps_parent_map(&String::from_utf8_lossy(&output.stdout)))
}

fn parse_ps_parent_map(output: &str) -> HashMap<u32, u32> {
    output
        .lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let pid = parts.next()?.parse().ok()?;
            let ppid = parts.next()?.parse().ok()?;
            Some((pid, ppid))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SNAPSHOT: &str = r#"{
        "schemaVersion": 1,
        "activeWindow": {
            "windowId": "win-b",
            "caption": "Konsole",
            "desktopFileName": "org.kde.konsole",
            "pid": 20,
            "normalWindow": true,
            "active": true
        },
        "windows": [
            { "windowId": "win-a", "caption": "Old", "resourceClass": "old", "pid": 10, "normalWindow": true },
            { "windowId": "win-b", "caption": "Konsole", "desktopFileName": "org.kde.konsole", "pid": 20, "normalWindow": true, "active": true }
        ]
    }"#;

    #[test]
    fn app_hint_uses_executable_stem() {
        assert_eq!(app_hint("/usr/bin/konsole"), "konsole");
        assert_eq!(app_hint("org.kde.konsole.desktop"), "org.kde.konsole");
    }

    #[test]
    fn pid_tree_match_excludes_preexisting_windows() {
        let snapshot: KWinSnapshotPayload = serde_json::from_str(SNAPSHOT).unwrap();
        let preexisting = HashSet::from(["win-a".to_string()]);
        let pids = HashSet::from([10, 20]);

        let matches = new_pid_tree_matches(&snapshot, &preexisting, &pids);

        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].window_id, "win-b");
    }

    #[test]
    fn active_app_match_uses_desktop_file_and_resource_names() {
        let snapshot: KWinSnapshotPayload = serde_json::from_str(SNAPSHOT).unwrap();

        let window = active_app_match(&snapshot, "konsole", Some("win-a"), &HashSet::from(["win-a".to_string()])).unwrap();

        assert_eq!(window.window_id, "win-b");
    }

    #[test]
    fn active_app_match_rejects_unchanged_preexisting_active_window() {
        let snapshot: KWinSnapshotPayload = serde_json::from_str(SNAPSHOT).unwrap();

        let window = active_app_match(&snapshot, "konsole", Some("win-b"), &HashSet::from(["win-b".to_string()]));

        assert!(window.is_none());
    }

    #[test]
    fn parses_ps_parent_map() {
        let map = parse_ps_parent_map(
            r#"
                1       0
              100       1
              101     100
            "#,
        );

        assert_eq!(map.get(&100), Some(&1));
        assert_eq!(map.get(&101), Some(&100));
    }
}
