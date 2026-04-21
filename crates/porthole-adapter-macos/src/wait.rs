#![cfg(target_os = "macos")]

use std::time::{Duration, Instant};

use porthole_core::surface::SurfaceInfo;
use porthole_core::wait::{LastObserved, WaitCondition, WaitOutcome, WAIT_SAMPLE_INTERVAL};
use porthole_core::{ErrorCode, PortholeError};
use regex::Regex;
use tokio::time::sleep;

use crate::capture;
use crate::enumerate::list_windows;
use crate::frame_diff::Fingerprint;

pub async fn wait(surface: &SurfaceInfo, condition: &WaitCondition) -> Result<WaitOutcome, PortholeError> {
    let start = Instant::now();
    match condition {
        WaitCondition::Exists => {
            loop {
                if surface_is_alive(surface)? {
                    return Ok(outcome("exists", start));
                }
                sleep(WAIT_SAMPLE_INTERVAL).await;
                // Pipeline-level timeout aborts this loop via tokio::time::timeout.
            }
        }
        WaitCondition::Gone => {
            loop {
                if !surface_is_alive(surface)? {
                    return Ok(outcome("gone", start));
                }
                sleep(WAIT_SAMPLE_INTERVAL).await;
            }
        }
        WaitCondition::TitleMatches { pattern } => {
            let re = Regex::new(pattern)
                .map_err(|e| PortholeError::new(ErrorCode::InvalidArgument, format!("bad regex: {e}")))?;
            loop {
                if let Some(title) = current_title(surface)? {
                    if re.is_match(&title) {
                        return Ok(outcome("title_matches", start));
                    }
                }
                sleep(WAIT_SAMPLE_INTERVAL).await;
            }
        }
        WaitCondition::Stable { window_ms, threshold_pct } => {
            let mut last_fp = sample_fingerprint(surface).await?;
            let mut last_change_at = Instant::now();
            loop {
                sleep(WAIT_SAMPLE_INTERVAL).await;
                let fp = sample_fingerprint(surface).await?;
                let diff = fp.diff_pct(&last_fp);
                if diff > *threshold_pct {
                    last_change_at = Instant::now();
                }
                last_fp = fp;
                if last_change_at.elapsed() >= Duration::from_millis(*window_ms) {
                    return Ok(outcome("stable", start));
                }
            }
        }
        WaitCondition::Dirty { threshold_pct } => {
            let initial = sample_fingerprint(surface).await?;
            loop {
                sleep(WAIT_SAMPLE_INTERVAL).await;
                let fp = sample_fingerprint(surface).await?;
                if fp.diff_pct(&initial) > *threshold_pct {
                    return Ok(outcome("dirty", start));
                }
            }
        }
    }
}

pub async fn wait_last_observed(
    surface: &SurfaceInfo,
    condition: &WaitCondition,
) -> Result<LastObserved, PortholeError> {
    match condition {
        WaitCondition::Exists | WaitCondition::Gone => {
            Ok(LastObserved::Presence { alive: surface_is_alive(surface)? })
        }
        WaitCondition::TitleMatches { .. } => Ok(LastObserved::Title { title: current_title(surface)? }),
        WaitCondition::Stable { .. } | WaitCondition::Dirty { .. } => {
            // Best effort: report placeholder values; precise tracking across
            // timeout boundaries is a v0.1 improvement.
            Ok(LastObserved::FrameChange { last_change_ms_ago: 0, last_change_pct: 0.0 })
        }
    }
}

fn outcome(condition: &str, start: Instant) -> WaitOutcome {
    WaitOutcome { condition: condition.to_string(), elapsed_ms: start.elapsed().as_millis() as u64 }
}

fn surface_is_alive(surface: &SurfaceInfo) -> Result<bool, PortholeError> {
    let pid = surface.pid.unwrap_or(0) as i32;
    if pid == 0 {
        return Ok(false);
    }
    let windows = list_windows()?;
    if let Some(cg_id) = surface.cg_window_id {
        Ok(windows.iter().any(|w| w.cg_window_id == cg_id))
    } else {
        Ok(windows.iter().any(|w| w.owner_pid == pid && (surface.title.is_none() || w.title == surface.title)))
    }
}

fn current_title(surface: &SurfaceInfo) -> Result<Option<String>, PortholeError> {
    let pid = surface.pid.unwrap_or(0) as i32;
    if pid == 0 {
        return Ok(None);
    }
    let windows = list_windows()?;
    if let Some(cg_id) = surface.cg_window_id {
        Ok(windows.iter().find(|w| w.cg_window_id == cg_id).and_then(|w| w.title.clone()))
    } else {
        Ok(windows.iter().find(|w| w.owner_pid == pid).and_then(|w| w.title.clone()))
    }
}

async fn sample_fingerprint(surface: &SurfaceInfo) -> Result<Fingerprint, PortholeError> {
    let shot = capture::screenshot(surface).await?;
    Fingerprint::from_png(&shot.png_bytes)
        .map_err(|e| PortholeError::new(ErrorCode::CapabilityMissing, format!("frame decode failed: {e}")))
}
