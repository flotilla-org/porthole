use std::{
    collections::HashMap,
    io::Read,
    os::{fd::OwnedFd as StdOwnedFd, unix::net::UnixStream},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use porthole_core::{
    ErrorCode, PortholeError,
    adapter::{Rect, Screenshot},
    surface::{PlatformSurfaceRef, SurfaceInfo},
};
use zbus::{
    Connection, Proxy,
    zvariant::{OwnedFd, OwnedValue, Value},
};

use crate::KWinAdapter;

const KWIN_DEST: &str = "org.kde.KWin";
const SCREENSHOT_PATH: &str = "/org/kde/KWin/ScreenShot2";
const SCREENSHOT_IFACE: &str = "org.kde.KWin.ScreenShot2";

pub(crate) async fn screenshot(adapter: &KWinAdapter, surface: &SurfaceInfo) -> Result<Screenshot, PortholeError> {
    let Some(PlatformSurfaceRef::Kwin { window_id }) = &surface.platform_ref else {
        return Err(PortholeError::new(ErrorCode::InvalidArgument, "surface is not a KWin surface"));
    };
    let snapshot = adapter.snapshot().await?;
    let window = snapshot
        .windows
        .iter()
        .find(|window| &window.window_id == window_id)
        .ok_or_else(|| PortholeError::new(ErrorCode::SurfaceDead, "KWin surface is no longer alive"))?;
    let frame = window
        .frame_geometry
        .ok_or_else(|| PortholeError::new(ErrorCode::CapabilityMissing, "KWin snapshot did not include frame geometry"))?;
    let scale = window
        .output
        .as_ref()
        .and_then(|name| snapshot.outputs.iter().find(|output| &output.name == name))
        .map_or(1.0, |output| output.scale);

    let png_bytes = capture_window_png(window_id).await?;

    Ok(Screenshot {
        png_bytes,
        window_bounds_points: Rect {
            x: frame.x,
            y: frame.y,
            w: frame.width,
            h: frame.height,
        },
        content_bounds_points: None,
        scale,
        captured_at_unix_ms: unix_ms(),
    })
}

async fn capture_window_png(window_id: &str) -> Result<Vec<u8>, PortholeError> {
    let connection = Connection::session()
        .await
        .map_err(|error| screenshot_unavailable(format!("cannot connect to session bus: {error}")))?;
    let proxy = Proxy::new(&connection, KWIN_DEST, SCREENSHOT_PATH, SCREENSHOT_IFACE)
        .await
        .map_err(|error| screenshot_unavailable(format!("cannot create KWin ScreenShot2 proxy: {error}")))?;

    let (mut reader, writer) = UnixStream::pair()
        .map_err(|error| PortholeError::new(ErrorCode::InternalError, format!("cannot create screenshot pipe: {error}")))?;
    reader
        .set_read_timeout(Some(Duration::from_secs(2)))
        .map_err(|error| PortholeError::new(ErrorCode::InternalError, format!("cannot set screenshot pipe timeout: {error}")))?;
    let writer_fd = OwnedFd::from(StdOwnedFd::from(writer));
    let options = screenshot_options();

    let results: HashMap<String, OwnedValue> = proxy
        .call("CaptureWindow", &(window_id, options, writer_fd))
        .await
        .map_err(screenshot_call_failed)?;

    let raw_bytes = tokio::task::spawn_blocking(move || {
        let mut bytes = Vec::new();
        reader
            .read_to_end(&mut bytes)
            .map_err(|error| PortholeError::new(ErrorCode::InternalError, format!("failed reading KWin screenshot pipe: {error}")))?;
        Ok(bytes)
    })
    .await
    .map_err(|error| PortholeError::new(ErrorCode::InternalError, format!("screenshot pipe reader task failed: {error}")))??;
    encode_kwin_raw_png(raw_bytes, &results)
}

fn screenshot_options() -> HashMap<&'static str, Value<'static>> {
    HashMap::from([
        ("include-cursor", Value::Bool(false)),
        ("include-decoration", Value::Bool(true)),
        ("native-resolution", Value::Bool(false)),
    ])
}

fn encode_kwin_raw_png(raw: Vec<u8>, results: &HashMap<String, OwnedValue>) -> Result<Vec<u8>, PortholeError> {
    let image_type = string_result(results, "type")?;
    if image_type != "raw" {
        return Err(PortholeError::new(
            ErrorCode::InternalError,
            format!("unsupported KWin screenshot type {image_type:?}"),
        ));
    }
    let width = u32_result(results, "width")?;
    let height = u32_result(results, "height")?;
    let stride = u32_result(results, "stride")?;
    let format = u32_result(results, "format")?;
    let rgba = kwin_raw_to_rgba(&raw, width, height, stride, format)?;

    let mut png_bytes = Vec::new();
    {
        use image::{ColorType, ImageEncoder, codecs::png::PngEncoder};
        let encoder = PngEncoder::new(&mut png_bytes);
        encoder
            .write_image(&rgba, width, height, ColorType::Rgba8.into())
            .map_err(|error| PortholeError::new(ErrorCode::InternalError, format!("png encode failed: {error}")))?;
    }
    Ok(png_bytes)
}

fn kwin_raw_to_rgba(raw: &[u8], width: u32, height: u32, stride: u32, format: u32) -> Result<Vec<u8>, PortholeError> {
    let width = width as usize;
    let height = height as usize;
    let stride = stride as usize;
    if width == 0 || height == 0 {
        return Err(PortholeError::new(
            ErrorCode::InternalError,
            "KWin screenshot returned an empty image",
        ));
    }
    let required = stride
        .checked_mul(height)
        .ok_or_else(|| PortholeError::new(ErrorCode::InternalError, "KWin screenshot dimensions overflow"))?;
    if raw.len() < required {
        return Err(PortholeError::new(
            ErrorCode::InternalError,
            format!(
                "KWin screenshot pipe returned too few bytes: got {}, need at least {required}",
                raw.len()
            ),
        ));
    }

    match format {
        // QImage::Format_RGB32, ARGB32, ARGB32_Premultiplied. On little-endian
        // Linux these are stored as B,G,R,A bytes.
        4..=6 => convert_rows(raw, width, height, stride, 4, |pixel| {
            let alpha = if format == 4 { 255 } else { pixel[3] };
            [pixel[2], pixel[1], pixel[0], alpha]
        }),
        // QImage::Format_RGB888.
        13 => convert_rows(raw, width, height, stride, 3, |pixel| [pixel[0], pixel[1], pixel[2], 255]),
        // QImage::Format_RGBX8888, RGBA8888, RGBA8888_Premultiplied.
        16..=18 => convert_rows(raw, width, height, stride, 4, |pixel| {
            let alpha = if format == 16 { 255 } else { pixel[3] };
            [pixel[0], pixel[1], pixel[2], alpha]
        }),
        // QImage::Format_BGR888.
        29 => convert_rows(raw, width, height, stride, 3, |pixel| [pixel[2], pixel[1], pixel[0], 255]),
        other => Err(PortholeError::new(
            ErrorCode::CapabilityMissing,
            format!("unsupported KWin screenshot QImage format {other}"),
        )),
    }
}

fn convert_rows(
    raw: &[u8],
    width: usize,
    height: usize,
    stride: usize,
    bytes_per_pixel: usize,
    convert: impl Fn(&[u8]) -> [u8; 4],
) -> Result<Vec<u8>, PortholeError> {
    let row_bytes = width
        .checked_mul(bytes_per_pixel)
        .ok_or_else(|| PortholeError::new(ErrorCode::InternalError, "KWin screenshot row width overflow"))?;
    if stride < row_bytes {
        return Err(PortholeError::new(
            ErrorCode::InternalError,
            format!("KWin screenshot stride {stride} is smaller than row bytes {row_bytes}"),
        ));
    }
    let mut rgba = Vec::with_capacity(width * height * 4);
    for y in 0..height {
        let start = y * stride;
        let row = &raw[start..start + row_bytes];
        for pixel in row.chunks_exact(bytes_per_pixel) {
            rgba.extend_from_slice(&convert(pixel));
        }
    }
    Ok(rgba)
}

fn string_result(results: &HashMap<String, OwnedValue>, key: &str) -> Result<String, PortholeError> {
    let value = results
        .get(key)
        .ok_or_else(|| PortholeError::new(ErrorCode::InternalError, format!("KWin screenshot response missing {key}")))?;
    String::try_from(value.try_clone().map_err(screenshot_variant_error)?).map_err(screenshot_variant_error)
}

fn u32_result(results: &HashMap<String, OwnedValue>, key: &str) -> Result<u32, PortholeError> {
    let value = results
        .get(key)
        .ok_or_else(|| PortholeError::new(ErrorCode::InternalError, format!("KWin screenshot response missing {key}")))?;
    u32::try_from(value.try_clone().map_err(screenshot_variant_error)?).map_err(screenshot_variant_error)
}

fn screenshot_variant_error(error: zbus::zvariant::Error) -> PortholeError {
    PortholeError::new(ErrorCode::InternalError, format!("invalid KWin screenshot response value: {error}"))
}

fn screenshot_unavailable(reason: String) -> PortholeError {
    PortholeError::new(ErrorCode::CapabilityMissing, reason)
}

fn screenshot_call_failed(error: zbus::Error) -> PortholeError {
    let message = error.to_string();
    if message.contains("not authorized") || message.contains("denied") || message.contains("not allowed") {
        return PortholeError::new(
            ErrorCode::SystemPermissionNeeded,
            "KWin ScreenShot2 access is restricted to trusted desktop entries",
        )
        .with_details(serde_json::json!({
            "permission": "kwin_screenshot",
            "remediation": {
                "cli_command": "porthole kwin install-desktop-entry",
                "requires_daemon_restart": true,
                "settings_path": "KDE desktop entry X-KDE-DBUS-Restricted-Interfaces",
                "binary_path": std::env::current_exe().map(|path| path.display().to_string()).unwrap_or_default()
            },
            "reason": message
        }));
    }
    PortholeError::new(
        ErrorCode::SystemPermissionRequestFailed,
        format!("KWin ScreenShot2 call failed: {error}"),
    )
}

fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| u64::try_from(duration.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screenshot_options_request_png_window_image() {
        let options = screenshot_options();

        assert_eq!(options.get("include-cursor"), Some(&Value::Bool(false)));
        assert_eq!(options.get("include-decoration"), Some(&Value::Bool(true)));
    }

    #[test]
    fn kwin_bgra_raw_converts_to_rgba() {
        let raw = [10, 20, 30, 40, 50, 60, 70, 80];

        let rgba = kwin_raw_to_rgba(&raw, 2, 1, 8, 5).expect("rgba");

        assert_eq!(rgba, vec![30, 20, 10, 40, 70, 60, 50, 80]);
    }

    #[test]
    fn kwin_xrgb_raw_converts_to_opaque_rgba() {
        let raw = [10, 20, 30, 0, 50, 60, 70, 0];

        let rgba = kwin_raw_to_rgba(&raw, 2, 1, 8, 4).expect("rgba");

        assert_eq!(rgba, vec![30, 20, 10, 255, 70, 60, 50, 255]);
    }

    #[test]
    fn kwin_rgba8888_raw_preserves_channel_order() {
        let raw = [10, 20, 30, 40, 50, 60, 70, 80];

        let rgba = kwin_raw_to_rgba(&raw, 2, 1, 8, 17).expect("rgba");

        assert_eq!(rgba, vec![10, 20, 30, 40, 50, 60, 70, 80]);
    }

    #[test]
    fn kwin_raw_conversion_rejects_short_pipe() {
        let error = kwin_raw_to_rgba(&[0; 3], 1, 1, 4, 5).expect_err("short raw buffer");

        assert_eq!(error.code, ErrorCode::InternalError);
    }
}
