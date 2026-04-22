#![cfg(target_os = "macos")]

use std::collections::HashSet;
use std::time::{Duration, Instant};

use porthole_core::adapter::{ArtifactLaunchSpec, Confidence, Correlation, LaunchOutcome};
use porthole_core::surface::{SurfaceId, SurfaceInfo, SurfaceKind, SurfaceState};
use porthole_core::{ErrorCode, PortholeError};
use tokio::process::Command;
use tokio::time::sleep;

use crate::ax::AxElement;
use crate::close_focus::with_ax_window_by_cg_id;
use crate::enumerate::{list_windows, WindowRecord};

const SAMPLE_INTERVAL: Duration = Duration::from_millis(150);

pub async fn launch_artifact(spec: &ArtifactLaunchSpec) -> Result<LaunchOutcome, PortholeError> {
    let path_str = spec
        .path
        .to_str()
        .ok_or_else(|| {
            PortholeError::new(ErrorCode::InvalidArgument, "path is not valid UTF-8")
        })?
        .to_string();
    let file_url = format!("file://{path_str}");

    // Snapshot the set of existing window IDs (for surface_was_preexisting).
    let before = list_windows()?;
    let before_ids: HashSet<u32> = before.iter().map(|w| w.cg_window_id).collect();

    // Invoke `open <path>`.
    let status = Command::new("/usr/bin/open")
        .arg(&path_str)
        .status()
        .await
        .map_err(|e| {
            PortholeError::new(
                ErrorCode::CapabilityMissing,
                format!("failed to spawn `open`: {e}"),
            )
        })?;
    if !status.success() {
        return Err(PortholeError::new(
            ErrorCode::LaunchCorrelationFailed,
            format!("`open` exited with status {status}"),
        ));
    }

    // Poll until the deadline for a window whose AXDocument matches the file url.
    let deadline = Instant::now() + spec.timeout;
    loop {
        // DocumentMatch attempt.
        if let Some((record, preexisting)) =
            find_window_by_document(&file_url, &before_ids).await?
        {
            return Ok(LaunchOutcome {
                surface: make_surface(&record),
                confidence: Confidence::Strong,
                correlation: Correlation::DocumentMatch,
                surface_was_preexisting: preexisting,
            });
        }

        if Instant::now() >= deadline {
            break;
        }
        sleep(SAMPLE_INTERVAL).await;
    }

    // Fallback: temporal — first new window across all apps within the timeout window.
    let after = list_windows()?;
    let new_windows: Vec<_> = after
        .iter()
        .filter(|w| !before_ids.contains(&w.cg_window_id))
        .collect();
    if let Some(w) = new_windows.first() {
        return Ok(LaunchOutcome {
            surface: make_surface(w),
            confidence: Confidence::Weak,
            correlation: Correlation::Temporal,
            surface_was_preexisting: false,
        });
    }

    Err(PortholeError::new(
        ErrorCode::LaunchCorrelationFailed,
        "no window with matching document and no new windows after open",
    ))
}

async fn find_window_by_document(
    target_url: &str,
    before_ids: &HashSet<u32>,
) -> Result<Option<(WindowRecord, bool)>, PortholeError> {
    let windows = list_windows()?;
    for w in windows {
        let pid = w.owner_pid;
        let cg = w.cg_window_id;
        // Query AXDocument for this window.
        let doc = match with_ax_window_by_cg_id(pid, cg, |raw| Ok(ax_document_for(raw))) {
            Ok(Some(s)) => s,
            _ => continue,
        };
        if doc == target_url {
            let preexisting = before_ids.contains(&cg);
            return Ok(Some((w, preexisting)));
        }
    }
    Ok(None)
}

/// Read the `AXDocument` attribute from a borrowed window element.
/// Returns the document URL string if available.
///
/// Safety note: `copy_attribute_raw` returns a retained pointer.
/// `CFString::wrap_under_create_rule` takes ownership of that retain.
/// When the `CFString` wrapper drops, it calls `CFRelease` exactly once.
/// We must NOT call `cf_release` additionally.
fn ax_document_for(raw: crate::ax::AxElementRef) -> Option<String> {
    use core_foundation::base::TCFType;
    unsafe {
        AxElement::with_borrowed(raw, |elem| {
            let ptr = elem.copy_attribute_raw("AXDocument")?;
            // AXDocument returns a CFStringRef (retained via copy rule).
            // wrap_under_create_rule takes ownership: drop releases exactly once.
            let cfs = core_foundation::string::CFString::wrap_under_create_rule(
                ptr as core_foundation::string::CFStringRef,
            );
            Some(cfs.to_string())
            // cfs drops here, releasing ptr. Do NOT call cf_release on ptr.
        })
    }
}

fn make_surface(w: &WindowRecord) -> SurfaceInfo {
    SurfaceInfo {
        id: SurfaceId::new(),
        kind: SurfaceKind::Window,
        state: SurfaceState::Alive,
        title: w.title.clone(),
        app_name: w.app_name.clone(),
        pid: Some(w.owner_pid as u32),
        parent_surface_id: None,
        cg_window_id: Some(w.cg_window_id),
    }
}
