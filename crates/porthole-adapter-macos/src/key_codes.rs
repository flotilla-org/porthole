//! DOM KeyboardEvent.code → macOS CGKeyCode table.
//!
//! Values taken from Apple's Events.h (Carbon HIToolbox). These are
//! physical-key codes, stable across layouts.

use std::{collections::HashMap, sync::OnceLock};

pub fn key_code(name: &str) -> Option<u16> {
    table().get(name).copied()
}

fn table() -> &'static HashMap<&'static str, u16> {
    static TABLE: OnceLock<HashMap<&'static str, u16>> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut m = HashMap::new();
        // Letters (ANSI)
        let letters: &[(&str, u16)] = &[
            ("KeyA", 0x00),
            ("KeyB", 0x0B),
            ("KeyC", 0x08),
            ("KeyD", 0x02),
            ("KeyE", 0x0E),
            ("KeyF", 0x03),
            ("KeyG", 0x05),
            ("KeyH", 0x04),
            ("KeyI", 0x22),
            ("KeyJ", 0x26),
            ("KeyK", 0x28),
            ("KeyL", 0x25),
            ("KeyM", 0x2E),
            ("KeyN", 0x2D),
            ("KeyO", 0x1F),
            ("KeyP", 0x23),
            ("KeyQ", 0x0C),
            ("KeyR", 0x0F),
            ("KeyS", 0x01),
            ("KeyT", 0x11),
            ("KeyU", 0x20),
            ("KeyV", 0x09),
            ("KeyW", 0x0D),
            ("KeyX", 0x07),
            ("KeyY", 0x10),
            ("KeyZ", 0x06),
        ];
        for &(n, c) in letters {
            m.insert(n, c);
        }

        // Digits (row above letters)
        let digits: &[(&str, u16)] = &[
            ("Digit0", 0x1D),
            ("Digit1", 0x12),
            ("Digit2", 0x13),
            ("Digit3", 0x14),
            ("Digit4", 0x15),
            ("Digit5", 0x17),
            ("Digit6", 0x16),
            ("Digit7", 0x1A),
            ("Digit8", 0x1C),
            ("Digit9", 0x19),
        ];
        for &(n, c) in digits {
            m.insert(n, c);
        }

        // Function keys
        let fkeys: &[(&str, u16)] = &[
            ("F1", 0x7A),
            ("F2", 0x78),
            ("F3", 0x63),
            ("F4", 0x76),
            ("F5", 0x60),
            ("F6", 0x61),
            ("F7", 0x62),
            ("F8", 0x64),
            ("F9", 0x65),
            ("F10", 0x6D),
            ("F11", 0x67),
            ("F12", 0x6F),
        ];
        for &(n, c) in fkeys {
            m.insert(n, c);
        }

        // Navigation / editing
        let nav: &[(&str, u16)] = &[
            ("Enter", 0x24),
            ("Escape", 0x35),
            ("Space", 0x31),
            ("Tab", 0x30),
            ("Backspace", 0x33),
            ("Delete", 0x75),
            ("ArrowLeft", 0x7B),
            ("ArrowRight", 0x7C),
            ("ArrowDown", 0x7D),
            ("ArrowUp", 0x7E),
            ("Home", 0x73),
            ("End", 0x77),
            ("PageUp", 0x74),
            ("PageDown", 0x79),
        ];
        for &(n, c) in nav {
            m.insert(n, c);
        }

        // Punctuation
        let punct: &[(&str, u16)] = &[
            ("Minus", 0x1B),
            ("Equal", 0x18),
            ("Comma", 0x2B),
            ("Period", 0x2F),
            ("Slash", 0x2C),
            ("Semicolon", 0x29),
            ("Quote", 0x27),
            ("Backquote", 0x32),
            ("BracketLeft", 0x21),
            ("BracketRight", 0x1E),
            ("Backslash", 0x2A),
        ];
        for &(n, c) in punct {
            m.insert(n, c);
        }

        m
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letter_codes_resolve() {
        assert_eq!(key_code("KeyA"), Some(0x00));
        assert_eq!(key_code("KeyZ"), Some(0x06));
    }

    #[test]
    fn enter_and_escape_resolve() {
        assert_eq!(key_code("Enter"), Some(0x24));
        assert_eq!(key_code("Escape"), Some(0x35));
    }

    #[test]
    fn unknown_name_returns_none() {
        assert_eq!(key_code("KeyAA"), None);
    }

    #[test]
    fn supported_set_all_resolve() {
        for name in porthole_core::key_names::supported() {
            assert!(key_code(name).is_some(), "no keycode for supported key {name}");
        }
    }
}
