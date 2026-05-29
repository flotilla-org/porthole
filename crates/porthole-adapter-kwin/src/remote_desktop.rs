use std::{collections::HashMap, ops::BitOr};

use futures_util::StreamExt;
use porthole_core::{
    ErrorCode, PortholeError,
    input::{KeyEvent, Modifier},
};
use serde_json::json;
use uuid::Uuid;
use zbus::{
    Connection, Proxy,
    zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value},
};

const PORTAL_DEST: &str = "org.freedesktop.portal.Desktop";
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
const REMOTE_DESKTOP_IFACE: &str = "org.freedesktop.portal.RemoteDesktop";
const REQUEST_IFACE: &str = "org.freedesktop.portal.Request";

const RESPONSE_SUCCESS: u32 = 0;
const RESPONSE_CANCELLED: u32 = 1;

const DEVICE_KEYBOARD: u32 = 1;
const DEVICE_POINTER: u32 = 2;

pub(crate) const BTN_LEFT: i32 = 0x110;
pub(crate) const BTN_RIGHT: i32 = 0x111;
pub(crate) const BTN_MIDDLE: i32 = 0x112;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RemoteDesktopDevice {
    Keyboard,
    Pointer,
}

impl RemoteDesktopDevice {
    pub(crate) const KEYBOARD: RemoteDesktopDevices = RemoteDesktopDevices(DEVICE_KEYBOARD);
    pub(crate) const POINTER: RemoteDesktopDevices = RemoteDesktopDevices(DEVICE_POINTER);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RemoteDesktopDevices(u32);

impl RemoteDesktopDevices {
    fn has(self, required: RemoteDesktopDevice) -> bool {
        let bit = match required {
            RemoteDesktopDevice::Keyboard => DEVICE_KEYBOARD,
            RemoteDesktopDevice::Pointer => DEVICE_POINTER,
        };
        self.0 & bit != 0
    }
}

impl BitOr for RemoteDesktopDevices {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct RemoteDesktopPortal;

impl RemoteDesktopPortal {
    pub(crate) fn new() -> Self {
        Self
    }

    pub(crate) async fn start_session(&self, devices: RemoteDesktopDevices) -> Result<RemoteDesktopSession, PortholeError> {
        let connection = Connection::session()
            .await
            .map_err(|error| portal_unavailable(format!("cannot connect to session bus: {error}")))?;
        let proxy = remote_desktop_proxy(&connection).await?;

        let create_handle: OwnedObjectPath = proxy
            .call(
                "CreateSession",
                &options([
                    ("handle_token", token_value("create")),
                    ("session_handle_token", token_value("session")),
                ]),
            )
            .await
            .map_err(portal_call_failed)?;
        let create_results = wait_response(&connection, create_handle).await?;
        let session_handle = string_result(&create_results, "session_handle")?;
        let session_path = OwnedObjectPath::try_from(session_handle.as_str()).map_err(|error| {
            PortholeError::new(
                ErrorCode::InternalError,
                format!("portal returned invalid session_handle {session_handle:?}: {error}"),
            )
        })?;

        let select_handle: OwnedObjectPath = proxy
            .call(
                "SelectDevices",
                &(
                    ObjectPath::try_from(session_path.as_str()).map_err(portal_variant_error)?,
                    options([
                        ("handle_token", token_value("select")),
                        ("types", Value::U32(devices.0)),
                        ("persist_mode", Value::U32(1)),
                    ]),
                ),
            )
            .await
            .map_err(portal_call_failed)?;
        wait_response(&connection, select_handle).await?;

        let start_handle: OwnedObjectPath = proxy
            .call(
                "Start",
                &(
                    ObjectPath::try_from(session_path.as_str()).map_err(portal_variant_error)?,
                    "",
                    options([("handle_token", token_value("start"))]),
                ),
            )
            .await
            .map_err(portal_call_failed)?;
        let start_results = wait_response(&connection, start_handle).await?;
        let granted = u32_result(&start_results, "devices").unwrap_or(devices.0);
        Ok(RemoteDesktopSession {
            connection,
            session_path,
            devices: RemoteDesktopDevices(granted),
        })
    }
}

#[derive(Clone, Debug)]
pub(crate) struct RemoteDesktopSession {
    connection: Connection,
    session_path: OwnedObjectPath,
    devices: RemoteDesktopDevices,
}

impl RemoteDesktopSession {
    pub(crate) fn has(&self, required: RemoteDesktopDevice) -> bool {
        self.devices.has(required)
    }

    pub(crate) async fn key_event(&self, event: &KeyEvent) -> Result<(), PortholeError> {
        let proxy = remote_desktop_proxy(&self.connection).await?;
        let modifiers = modifier_keycodes(&event.modifiers);
        for modifier in &modifiers {
            notify_keyboard_keycode(&proxy, &self.session_path, *modifier, true).await?;
        }
        let keycode = linux_keycode(&event.key)
            .ok_or_else(|| PortholeError::new(ErrorCode::UnknownKey, format!("no Linux keycode for '{}'", event.key)))?;
        notify_keyboard_keycode(&proxy, &self.session_path, keycode, true).await?;
        notify_keyboard_keycode(&proxy, &self.session_path, keycode, false).await?;
        for modifier in modifiers.iter().rev() {
            notify_keyboard_keycode(&proxy, &self.session_path, *modifier, false).await?;
        }
        Ok(())
    }

    pub(crate) async fn text(&self, text: &str) -> Result<(), PortholeError> {
        let proxy = remote_desktop_proxy(&self.connection).await?;
        for ch in text.chars() {
            let keysym = keysym_for_char(ch)
                .ok_or_else(|| PortholeError::new(ErrorCode::UnknownKey, format!("no X keysym mapping for U+{:04X}", ch as u32)))?;
            notify_keyboard_keysym(&proxy, &self.session_path, keysym, true).await?;
            notify_keyboard_keysym(&proxy, &self.session_path, keysym, false).await?;
        }
        Ok(())
    }

    pub(crate) async fn pointer_motion(&self, dx: f64, dy: f64) -> Result<(), PortholeError> {
        let proxy = remote_desktop_proxy(&self.connection).await?;
        proxy
            .call::<_, _, ()>(
                "NotifyPointerMotion",
                &(
                    ObjectPath::try_from(self.session_path.as_str()).map_err(portal_variant_error)?,
                    empty_options(),
                    dx,
                    dy,
                ),
            )
            .await
            .map_err(portal_call_failed)
    }

    pub(crate) async fn pointer_button(&self, button: i32, pressed: bool) -> Result<(), PortholeError> {
        let proxy = remote_desktop_proxy(&self.connection).await?;
        proxy
            .call::<_, _, ()>(
                "NotifyPointerButton",
                &(
                    ObjectPath::try_from(self.session_path.as_str()).map_err(portal_variant_error)?,
                    empty_options(),
                    button,
                    if pressed { 1_u32 } else { 0_u32 },
                ),
            )
            .await
            .map_err(portal_call_failed)
    }

    pub(crate) async fn pointer_axis(&self, dx: f64, dy: f64) -> Result<(), PortholeError> {
        let proxy = remote_desktop_proxy(&self.connection).await?;
        proxy
            .call::<_, _, ()>(
                "NotifyPointerAxis",
                &(
                    ObjectPath::try_from(self.session_path.as_str()).map_err(portal_variant_error)?,
                    options([("finish", Value::Bool(true))]),
                    dx,
                    dy,
                ),
            )
            .await
            .map_err(portal_call_failed)
    }
}

async fn remote_desktop_proxy(connection: &Connection) -> Result<Proxy<'_>, PortholeError> {
    Proxy::new(connection, PORTAL_DEST, PORTAL_PATH, REMOTE_DESKTOP_IFACE)
        .await
        .map_err(|error| portal_unavailable(format!("RemoteDesktop portal is unavailable: {error}")))
}

async fn wait_response(connection: &Connection, handle: OwnedObjectPath) -> Result<HashMap<String, OwnedValue>, PortholeError> {
    let proxy = Proxy::new(connection, PORTAL_DEST, handle.as_str(), REQUEST_IFACE)
        .await
        .map_err(portal_call_failed)?;
    let mut responses = proxy.receive_signal("Response").await.map_err(portal_call_failed)?;
    let Some(message) = responses.next().await else {
        return Err(portal_unavailable("portal request closed before Response signal"));
    };
    let (response, results): (u32, HashMap<String, OwnedValue>) = message.body().deserialize().map_err(portal_call_failed)?;
    match response {
        RESPONSE_SUCCESS => Ok(results),
        RESPONSE_CANCELLED => Err(permission_needed("RemoteDesktop portal consent was cancelled")),
        other => Err(PortholeError::new(
            ErrorCode::SystemPermissionRequestFailed,
            format!("RemoteDesktop portal request failed with response code {other}"),
        )
        .with_details(json!({
            "permission": "remote_desktop",
            "reason": format!("portal response code {other}"),
            "settings_path": "KDE System Settings -> Security & Privacy -> Application Permissions",
            "binary_path": current_exe(),
        }))),
    }
}

fn options<'a, const N: usize>(entries: [(&'static str, Value<'a>); N]) -> HashMap<&'static str, Value<'a>> {
    entries.into_iter().collect()
}

fn empty_options() -> HashMap<&'static str, Value<'static>> {
    HashMap::new()
}

fn token_value(prefix: &str) -> Value<'static> {
    Value::from(format!("{prefix}_{}", Uuid::new_v4().simple()))
}

fn string_result(results: &HashMap<String, OwnedValue>, key: &str) -> Result<String, PortholeError> {
    let value = results
        .get(key)
        .ok_or_else(|| PortholeError::new(ErrorCode::InternalError, format!("portal response missing {key}")))?;
    String::try_from(value.try_clone().map_err(portal_variant_error)?).map_err(portal_variant_error)
}

fn u32_result(results: &HashMap<String, OwnedValue>, key: &str) -> Option<u32> {
    results
        .get(key)
        .and_then(|value| value.try_clone().ok())
        .and_then(|value| u32::try_from(value).ok())
}

async fn notify_keyboard_keycode(
    proxy: &Proxy<'_>,
    session_path: &OwnedObjectPath,
    keycode: i32,
    pressed: bool,
) -> Result<(), PortholeError> {
    proxy
        .call::<_, _, ()>(
            "NotifyKeyboardKeycode",
            &(
                ObjectPath::try_from(session_path.as_str()).map_err(portal_variant_error)?,
                empty_options(),
                keycode,
                if pressed { 1_u32 } else { 0_u32 },
            ),
        )
        .await
        .map_err(portal_call_failed)
}

async fn notify_keyboard_keysym(
    proxy: &Proxy<'_>,
    session_path: &OwnedObjectPath,
    keysym: i32,
    pressed: bool,
) -> Result<(), PortholeError> {
    proxy
        .call::<_, _, ()>(
            "NotifyKeyboardKeysym",
            &(
                ObjectPath::try_from(session_path.as_str()).map_err(portal_variant_error)?,
                empty_options(),
                keysym,
                if pressed { 1_u32 } else { 0_u32 },
            ),
        )
        .await
        .map_err(portal_call_failed)
}

fn modifier_keycodes(modifiers: &[Modifier]) -> Vec<i32> {
    let mut out = Vec::new();
    for modifier in modifiers {
        let keycode = match modifier {
            Modifier::Cmd => 125,  // KEY_LEFTMETA
            Modifier::Ctrl => 29,  // KEY_LEFTCTRL
            Modifier::Alt => 56,   // KEY_LEFTALT
            Modifier::Shift => 42, // KEY_LEFTSHIFT
        };
        if !out.contains(&keycode) {
            out.push(keycode);
        }
    }
    out
}

fn linux_keycode(name: &str) -> Option<i32> {
    let code = match name {
        "KeyA" => 30,
        "KeyB" => 48,
        "KeyC" => 46,
        "KeyD" => 32,
        "KeyE" => 18,
        "KeyF" => 33,
        "KeyG" => 34,
        "KeyH" => 35,
        "KeyI" => 23,
        "KeyJ" => 36,
        "KeyK" => 37,
        "KeyL" => 38,
        "KeyM" => 50,
        "KeyN" => 49,
        "KeyO" => 24,
        "KeyP" => 25,
        "KeyQ" => 16,
        "KeyR" => 19,
        "KeyS" => 31,
        "KeyT" => 20,
        "KeyU" => 22,
        "KeyV" => 47,
        "KeyW" => 17,
        "KeyX" => 45,
        "KeyY" => 21,
        "KeyZ" => 44,
        "Digit1" => 2,
        "Digit2" => 3,
        "Digit3" => 4,
        "Digit4" => 5,
        "Digit5" => 6,
        "Digit6" => 7,
        "Digit7" => 8,
        "Digit8" => 9,
        "Digit9" => 10,
        "Digit0" => 11,
        "Enter" => 28,
        "Escape" => 1,
        "Space" => 57,
        "Tab" => 15,
        "Backspace" => 14,
        "Delete" => 111,
        "ArrowLeft" => 105,
        "ArrowRight" => 106,
        "ArrowDown" => 108,
        "ArrowUp" => 103,
        "Home" => 102,
        "End" => 107,
        "PageUp" => 104,
        "PageDown" => 109,
        "Minus" => 12,
        "Equal" => 13,
        "Comma" => 51,
        "Period" => 52,
        "Slash" => 53,
        "Semicolon" => 39,
        "Quote" => 40,
        "Backquote" => 41,
        "BracketLeft" => 26,
        "BracketRight" => 27,
        "Backslash" => 43,
        _ => {
            if let Some(rest) = name.strip_prefix('F')
                && let Ok(n) = rest.parse::<i32>()
                && (1..=12).contains(&n)
            {
                return Some(58 + n);
            }
            return None;
        }
    };
    Some(code)
}

fn keysym_for_char(ch: char) -> Option<i32> {
    match ch {
        '\n' => Some(0xff0d),     // XK_Return
        '\t' => Some(0xff09),     // XK_Tab
        '\u{08}' => Some(0xff08), // XK_BackSpace
        '\u{1b}' => Some(0xff1b), // XK_Escape
        '\u{20}'..='\u{7e}' | '\u{a0}'..='\u{ff}' => Some(ch as i32),
        _ => Some((0x0100_0000_u32 | ch as u32) as i32),
    }
}

pub(crate) fn permission_needed(reason: impl Into<String>) -> PortholeError {
    PortholeError::new(ErrorCode::SystemPermissionNeeded, reason.into()).with_details(json!({
        "permission": "remote_desktop",
        "remediation": {
            "cli_command": "porthole onboard",
            "requires_daemon_restart": false,
            "settings_path": "KDE System Settings -> Security & Privacy -> Application Permissions",
            "binary_path": current_exe(),
        }
    }))
}

fn portal_unavailable(reason: impl Into<String>) -> PortholeError {
    PortholeError::new(ErrorCode::SystemPermissionRequestFailed, reason.into()).with_details(json!({
        "permission": "remote_desktop",
        "reason": "RemoteDesktop portal is unavailable",
        "settings_path": "KDE System Settings -> Security & Privacy -> Application Permissions",
        "binary_path": current_exe(),
    }))
}

fn portal_call_failed(error: impl std::fmt::Display) -> PortholeError {
    PortholeError::new(
        ErrorCode::SystemPermissionRequestFailed,
        format!("RemoteDesktop portal call failed: {error}"),
    )
}

fn portal_variant_error(error: impl std::fmt::Display) -> PortholeError {
    PortholeError::new(ErrorCode::InternalError, format!("RemoteDesktop portal value error: {error}"))
}

fn current_exe() -> String {
    std::env::current_exe()
        .map(|path| path.to_string_lossy().to_string())
        .unwrap_or_else(|_| "<unknown>".to_string())
}

#[cfg(test)]
mod tests {
    use porthole_core::input::Modifier;

    use super::*;

    #[test]
    fn linux_keycodes_cover_common_dom_names() {
        assert_eq!(linux_keycode("KeyA"), Some(30));
        assert_eq!(linux_keycode("Digit1"), Some(2));
        assert_eq!(linux_keycode("Enter"), Some(28));
        assert_eq!(linux_keycode("ArrowLeft"), Some(105));
        assert_eq!(linux_keycode("F12"), Some(70));
        assert_eq!(linux_keycode("Nope"), None);
    }

    #[test]
    fn modifier_keycodes_dedupe_in_order() {
        assert_eq!(modifier_keycodes(&[Modifier::Ctrl, Modifier::Shift, Modifier::Ctrl]), vec![29, 42]);
    }

    #[test]
    fn keysyms_cover_ascii_controls_and_unicode() {
        assert_eq!(keysym_for_char('A'), Some(65));
        assert_eq!(keysym_for_char('\n'), Some(0xff0d));
        assert_eq!(keysym_for_char('é'), Some(0xe9));
        assert_eq!(keysym_for_char('λ'), Some(0x0100_03bb));
    }
}
