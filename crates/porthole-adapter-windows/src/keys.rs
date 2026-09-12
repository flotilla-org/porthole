//! Pure mapping seam, tested without a desktop.
#![cfg_attr(not(windows), allow(dead_code))]
use porthole_core::{ErrorCode, PortholeError};

pub fn virtual_key(name: &str) -> Result<(u16, bool), PortholeError> {
    let code = match name {
        "Enter" => 0x0d,
        "Escape" => 0x1b,
        "Space" => 0x20,
        "Tab" => 9,
        "Backspace" => 8,
        "Delete" => 0x2e,
        "ArrowUp" => 0x26,
        "ArrowDown" => 0x28,
        "ArrowLeft" => 0x25,
        "ArrowRight" => 0x27,
        "Home" => 0x24,
        "End" => 0x23,
        "PageUp" => 0x21,
        "PageDown" => 0x22,
        "Minus" => 0xbd,
        "Equal" => 0xbb,
        "Comma" => 0xbc,
        "Period" => 0xbe,
        "Slash" => 0xbf,
        "Semicolon" => 0xba,
        "Quote" => 0xde,
        "Backquote" => 0xc0,
        "BracketLeft" => 0xdb,
        "BracketRight" => 0xdd,
        "Backslash" => 0xdc,
        _ => {
            if let Some(s) = name
                .strip_prefix("Key")
                .filter(|s| s.len() == 1 && s.as_bytes()[0].is_ascii_uppercase())
            {
                return Ok((s.as_bytes()[0] as u16, false));
            }
            if let Some(s) = name
                .strip_prefix("Digit")
                .filter(|s| s.len() == 1 && s.as_bytes()[0].is_ascii_digit())
            {
                return Ok((s.as_bytes()[0] as u16, false));
            }
            if let Some(n) = name
                .strip_prefix('F')
                .and_then(|s| s.parse::<u16>().ok())
                .filter(|n| (1..=12).contains(n))
            {
                return Ok((0x70 + n - 1, false));
            }
            return Err(PortholeError::new(ErrorCode::UnknownKey, format!("unsupported key: {name}")));
        }
    };
    Ok((code, matches!(code, 0x21..=0x28 | 0x2e)))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn covers_shared_key_vocabulary() {
        for key in porthole_core::key_names::supported() {
            assert!(virtual_key(key).is_ok(), "{key}");
        }
        assert_eq!(virtual_key("ArrowLeft").unwrap(), (0x25, true));
        assert_eq!(virtual_key("KeyA").unwrap(), (0x41, false));
        assert_eq!(virtual_key("F12").unwrap(), (0x7b, false));
        assert_eq!(virtual_key("KeyAA").unwrap_err().code, ErrorCode::UnknownKey);
    }
}
