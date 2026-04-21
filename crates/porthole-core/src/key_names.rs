//! DOM KeyboardEvent.code-style key names supported by porthole input.
//!
//! Agent-facing callers pass these strings on the wire. The adapter
//! implementation maps them to platform-native keycodes.

use std::collections::HashSet;
use std::sync::OnceLock;

/// Returns the full set of supported key names.
pub fn supported() -> &'static HashSet<&'static str> {
    static SET: OnceLock<HashSet<&'static str>> = OnceLock::new();
    SET.get_or_init(|| {
        let mut s: HashSet<&'static str> = HashSet::new();
        // Letters
        for c in b'A'..=b'Z' {
            // SAFETY: ASCII upper letters are valid str, values live for 'static via intern table below.
            s.insert(intern(&format!("Key{}", c as char)));
        }
        // Digits
        for d in 0..=9 {
            s.insert(intern(&format!("Digit{d}")));
        }
        // Function keys
        for n in 1..=12u8 {
            s.insert(intern(&format!("F{n}")));
        }
        // Named keys
        for name in [
            "Enter", "Escape", "Space", "Tab", "Backspace", "Delete",
            "ArrowUp", "ArrowDown", "ArrowLeft", "ArrowRight",
            "Home", "End", "PageUp", "PageDown",
            "Minus", "Equal", "Comma", "Period", "Slash",
            "Semicolon", "Quote", "Backquote", "BracketLeft", "BracketRight",
            "Backslash",
        ] {
            s.insert(name);
        }
        s
    })
}

/// Returns true if `name` is a supported key name.
pub fn is_supported(name: &str) -> bool {
    supported().contains(name)
}

/// Leaks a string to obtain a `&'static str`. Only used for the key-name set,
/// which is populated exactly once at program start.
fn intern(s: &str) -> &'static str {
    Box::leak(s.to_string().into_boxed_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letters_are_supported() {
        assert!(is_supported("KeyA"));
        assert!(is_supported("KeyZ"));
    }

    #[test]
    fn digits_are_supported() {
        assert!(is_supported("Digit0"));
        assert!(is_supported("Digit9"));
    }

    #[test]
    fn named_keys_are_supported() {
        assert!(is_supported("Enter"));
        assert!(is_supported("ArrowUp"));
        assert!(is_supported("F5"));
    }

    #[test]
    fn unsupported_names_return_false() {
        assert!(!is_supported("KeyAA"));
        assert!(!is_supported("Ctrl"));
        assert!(!is_supported(""));
    }
}
