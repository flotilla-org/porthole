use std::time::{SystemTime, UNIX_EPOCH};

use porthole_core::adapter::{Rect, Screenshot};
use porthole_core::surface::SurfaceInfo;
use porthole_core::{ErrorCode, PortholeError};

pub async fn screenshot(surface: &SurfaceInfo) -> Result<Screenshot, PortholeError> {
    #[cfg(not(target_os = "macos"))]
    {
        let _ = surface;
        return Err(PortholeError::new(ErrorCode::AdapterUnsupported, "macOS adapter on non-macOS"));
    }

    #[cfg(target_os = "macos")]
    {
        use core_graphics::geometry::{CGPoint, CGRect, CGSize};
        use core_graphics::window::{
            create_image, kCGWindowImageBoundsIgnoreFraming, kCGWindowImageDefault,
            kCGWindowListOptionIncludingWindow,
        };

        let pid = surface.pid.ok_or_else(|| {
            PortholeError::new(ErrorCode::CapabilityMissing, "surface has no pid; cannot locate CGWindowID")
        })? as i32;

        // Resolve geometry first (before holding any non-Send CG types across an await).
        // Look up the backing scale for the display the window is on.
        // The macOS display ID encoding is "disp_<cgid>".
        let snap = crate::snapshot::snapshot_geometry(surface).await;
        let (pre_snap, pre_scale) = match &snap {
            Ok(s) => {
                let cg_id: u32 = s.display_id.as_str()
                    .strip_prefix("disp_")
                    .and_then(|x| x.parse().ok())
                    .unwrap_or(0);
                let scale = if cg_id != 0 {
                    crate::nsscreen::backing_scale_factor_for(cg_id)
                } else {
                    1.0
                };
                (Some(s.display_local), scale)
            }
            Err(_) => (None, 1.0),
        };

        // Prefer the stored CGWindowID for precise targeting; fall back to
        // PID+title heuristic for surfaces created before slice-A.
        let cg_window_id = if let Some(id) = surface.cg_window_id {
            id
        } else {
            locate_cg_window_id(pid, surface.title.as_deref())?
        };

        // An empty rect tells CG to use the window's own bounds when combined with
        // kCGWindowListOptionIncludingWindow.
        let zero_rect = CGRect::new(&CGPoint::new(0.0, 0.0), &CGSize::new(0.0, 0.0));
        let image = match create_image(
            zero_rect,
            kCGWindowListOptionIncludingWindow,
            cg_window_id,
            kCGWindowImageBoundsIgnoreFraming | kCGWindowImageDefault,
        ) {
            Some(img) => img,
            None => {
                return Err(PortholeError::new(
                    ErrorCode::PermissionNeeded,
                    "CGWindowListCreateImage returned null — likely missing Screen Recording permission",
                ));
            }
        };

        let width = image.width() as u32;
        let height = image.height() as u32;
        let bytes_per_row = image.bytes_per_row();
        let data = image.data();

        let bgra: &[u8] = data.bytes();
        let mut rgba = Vec::with_capacity((width * height * 4) as usize);
        for row in 0..height as usize {
            let row_start = row * bytes_per_row;
            for col in 0..width as usize {
                let px = row_start + col * 4;
                let b = bgra[px];
                let g = bgra[px + 1];
                let r = bgra[px + 2];
                let a = bgra[px + 3];
                rgba.extend_from_slice(&[r, g, b, a]);
            }
        }
        drop(data);
        drop(image);

        let mut png_bytes = Vec::new();
        {
            use image::codecs::png::PngEncoder;
            use image::{ColorType, ImageEncoder};
            let encoder = PngEncoder::new(&mut png_bytes);
            encoder
                .write_image(&rgba, width, height, ColorType::Rgba8.into())
                .map_err(|e| PortholeError::new(ErrorCode::CapabilityMissing, format!("png encode failed: {e}")))?;
        }

        let (window_bounds_points, scale) = match pre_snap {
            Some(bounds) => (bounds, pre_scale),
            None => {
                // snapshot_geometry failed; fall back to pixel dimensions with scale 1.
                tracing::warn!(
                    "snapshot_geometry failed during screenshot; reporting pixel bounds with scale 1"
                );
                (Rect { x: 0.0, y: 0.0, w: width as f64, h: height as f64 }, 1.0)
            }
        };

        Ok(Screenshot {
            png_bytes,
            window_bounds_points,
            content_bounds_points: None,
            scale,
            captured_at_unix_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
        })
    }
}

#[cfg(target_os = "macos")]
fn locate_cg_window_id(pid: i32, title: Option<&str>) -> Result<u32, PortholeError> {
    let windows = crate::enumerate::list_windows()?;
    let mut matching: Vec<_> = windows.iter().filter(|w| w.owner_pid == pid).collect();
    if let Some(t) = title {
        if matching.iter().any(|w| w.title.as_deref() == Some(t)) {
            matching.retain(|w| w.title.as_deref() == Some(t));
        }
    }
    match matching.first() {
        Some(w) => Ok(w.cg_window_id),
        None => Err(PortholeError::new(ErrorCode::SurfaceDead, format!("no live window found for pid {pid}"))),
    }
}
