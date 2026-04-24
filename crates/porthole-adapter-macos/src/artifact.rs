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
use crate::MacOsAdapter;
use crate::permissions::ensure_accessibility_granted;

const SAMPLE_INTERVAL: Duration = Duration::from_millis(150);

/// Resolve the name of the default handler application for the given file path
/// by invoking `NSWorkspace.URLForApplicationToOpenURL:` via objc2-app-kit.
/// Returns the `.app` bundle's stem (e.g. "Preview") or `None` on failure.
fn resolve_handler_app_name(path: &str) -> Option<String> {
    use objc2_app_kit::NSWorkspace;
    use objc2_foundation::{NSString, NSURL};

    unsafe {
        let ns_path = NSString::from_str(path);
        let file_url = NSURL::fileURLWithPath(&ns_path);
        let workspace = NSWorkspace::sharedWorkspace();
        let app_url = workspace.URLForApplicationToOpenURL(&file_url)?;
        // `path()` returns the file-system path string for the app URL.
        let path_ns = app_url.path()?;
        let s = path_ns.to_string();
        // ".../Preview.app" -> "Preview"
        s.rsplit('/').next()?.strip_suffix(".app").map(str::to_string)
    }
}

pub async fn launch_artifact(adapter: &MacOsAdapter, spec: &ArtifactLaunchSpec) -> Result<LaunchOutcome, PortholeError> {
    ensure_accessibility_granted(adapter)?;
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

    // Resolve the handler app name so we can narrow fallbacks to it.
    let handler_app = resolve_handler_app_name(&path_str);

    // Record the frontmost window of the handler app before `open`, for
    // FrontmostChanged correlation (plausible tier, spec §4.3).
    let before_frontmost = handler_app.as_deref().and_then(|app| {
        before
            .iter()
            .find(|w| w.app_name.as_deref() == Some(app))
            .map(|w| w.cg_window_id)
    });

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

    // FrontmostChanged — plausible tier (spec §4.3).
    // Check whether the frontmost window of the handler app changed since `open`.
    if let Some(app) = &handler_app {
        let after = list_windows()?;
        let current_frontmost = after
            .iter()
            .find(|w| w.app_name.as_deref() == Some(app.as_str()))
            .map(|w| w.cg_window_id);
        if current_frontmost != before_frontmost {
            if let Some(w) = after
                .into_iter()
                .find(|w| Some(w.cg_window_id) == current_frontmost)
            {
                return Ok(LaunchOutcome {
                    surface: make_surface(&w),
                    confidence: Confidence::Plausible,
                    correlation: Correlation::FrontmostChanged,
                    surface_was_preexisting: before_ids.contains(&w.cg_window_id),
                });
            }
        }
    }

    // Temporal fallback — first new window of the handler app (spec §4.3).
    // If the handler app could not be resolved, fall back to any new window.
    let after = list_windows()?;
    let new_windows: Vec<_> = after
        .into_iter()
        .filter(|w| !before_ids.contains(&w.cg_window_id))
        .filter(|w| {
            handler_app
                .as_deref()
                .is_none_or(|app| w.app_name.as_deref() == Some(app))
        })
        .collect();
    if let Some(w) = new_windows.into_iter().next() {
        return Ok(LaunchOutcome {
            surface: make_surface(&w),
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
