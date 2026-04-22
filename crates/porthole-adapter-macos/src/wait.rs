#![cfg(target_os = "macos")]

use std::time::{Duration, Instant};

use porthole_core::surface::SurfaceInfo;
use porthole_core::wait::{LastObserved, WaitCondition, WaitOutcome, WaitTimeout, WAIT_SAMPLE_INTERVAL};
use porthole_core::{ErrorCode, PortholeError};
use regex::Regex;
use tokio::time::sleep;

use crate::capture;
use crate::enumerate::list_windows;
use crate::frame_diff::Fingerprint;

pub async fn wait(
    surface: &SurfaceInfo,
    condition: &WaitCondition,
    deadline: Instant,
) -> Result<WaitOutcome, WaitTimeout> {
    let start = Instant::now();
    match condition {
        WaitCondition::Exists => loop {
            let alive = surface_is_alive(surface).map_err(|e| timeout_from_err(start, e))?;
            if alive {
                return Ok(outcome("exists", start));
            }
            if Instant::now() >= deadline {
                return Err(WaitTimeout {
                    last_observed: LastObserved::Presence { alive },
                    elapsed_ms: start.elapsed().as_millis() as u64,
                });
            }
            sleep(WAIT_SAMPLE_INTERVAL).await;
        },
        WaitCondition::Gone => loop {
            let alive = surface_is_alive(surface).map_err(|e| timeout_from_err(start, e))?;
            if !alive {
                return Ok(outcome("gone", start));
            }
            if Instant::now() >= deadline {
                return Err(WaitTimeout {
                    last_observed: LastObserved::Presence { alive },
                    elapsed_ms: start.elapsed().as_millis() as u64,
                });
            }
            sleep(WAIT_SAMPLE_INTERVAL).await;
        },
        WaitCondition::TitleMatches { pattern } => {
            let re = Regex::new(pattern).map_err(|e| {
                timeout_from_err(
                    start,
                    PortholeError::new(ErrorCode::InvalidArgument, format!("bad regex: {e}")),
                )
            })?;
            loop {
                let title =
                    current_title(surface).map_err(|e| timeout_from_err(start, e))?;
                if let Some(t) = &title {
                    if re.is_match(t) {
                        return Ok(outcome("title_matches", start));
                    }
                }
                if Instant::now() >= deadline {
                    return Err(WaitTimeout {
                        last_observed: LastObserved::Title { title },
                        elapsed_ms: start.elapsed().as_millis() as u64,
                    });
                }
                sleep(WAIT_SAMPLE_INTERVAL).await;
            }
        }
        WaitCondition::Stable { window_ms, threshold_pct } => {
            let mut last_fp = sample_fingerprint(surface)
                .await
                .map_err(|e| timeout_from_err(start, e))?;
            let mut last_change_at = Instant::now();
            let mut last_change_pct: f64 = 0.0;
            loop {
                if Instant::now() >= deadline {
                    return Err(WaitTimeout {
                        last_observed: LastObserved::FrameChange {
                            last_change_ms_ago: last_change_at.elapsed().as_millis() as u64,
                            last_change_pct,
                        },
                        elapsed_ms: start.elapsed().as_millis() as u64,
                    });
                }
                sleep(WAIT_SAMPLE_INTERVAL).await;
                let fp = sample_fingerprint(surface)
                    .await
                    .map_err(|e| timeout_from_err(start, e))?;
                let diff = fp.diff_pct(&last_fp);
                if diff > *threshold_pct {
                    last_change_at = Instant::now();
                    last_change_pct = diff;
                }
                last_fp = fp;
                if last_change_at.elapsed() >= Duration::from_millis(*window_ms) {
                    return Ok(outcome("stable", start));
                }
            }
        }
        WaitCondition::Dirty { threshold_pct } => {
            let initial = sample_fingerprint(surface)
                .await
                .map_err(|e| timeout_from_err(start, e))?;
            let mut last_pct: f64 = 0.0;
            loop {
                if Instant::now() >= deadline {
                    return Err(WaitTimeout {
                        last_observed: LastObserved::FrameChange {
                            last_change_ms_ago: 0,
                            last_change_pct: last_pct,
                        },
                        elapsed_ms: start.elapsed().as_millis() as u64,
                    });
                }
                sleep(WAIT_SAMPLE_INTERVAL).await;
                let fp = sample_fingerprint(surface)
                    .await
                    .map_err(|e| timeout_from_err(start, e))?;
                let diff = fp.diff_pct(&initial);
                last_pct = diff;
                if diff > *threshold_pct {
                    return Ok(outcome("dirty", start));
                }
            }
        }
    }
}

/// Convert a sampling error mid-wait into a `WaitTimeout` with coarse diagnostics.
/// Used when `surface_is_alive`, `current_title`, or `sample_fingerprint` fails
/// (e.g. permission revocation or catastrophic OS errors).
fn timeout_from_err(start: Instant, err: PortholeError) -> WaitTimeout {
    tracing::warn!(?err, "wait sampling failed; reporting as timeout");
    WaitTimeout {
        last_observed: LastObserved::FrameChange {
            last_change_ms_ago: 0,
            last_change_pct: 0.0,
        },
        elapsed_ms: start.elapsed().as_millis() as u64,
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
