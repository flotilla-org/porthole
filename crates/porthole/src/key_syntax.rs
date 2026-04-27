//! Tmux-style key syntax parser.
//!
//! Mirrors cleat's `keys.rs` shape (tokens with `C-x`/`M-x`/`S-x`/`^x`
//! modifier prefixes plus tmux-style named keys: Enter, Esc, Up, BSpace,
//! F1, etc.) but emits a structured token sequence instead of PTY bytes.
//! The CLI dispatches each token: `Text(s)` to `POST /text`, `Key{...}` to
//! `POST /key` with modifiers translated to porthole's wire enum.
//!
//! Two modifier conventions are accepted:
//! - cleat's: `C-` (Ctrl), `M-` (Meta/Alt), `S-` (Shift), `^x` (Ctrl alt syntax)
//! - macOS-friendly: `Cmd-` for ⌘
//!
//! Modifiers can be combined: `C-S-Up`, `Cmd-M-x`. Order doesn't matter.

use porthole_core::input::Modifier;

#[derive(Clone, Debug, PartialEq)]
pub enum KeyToken {
    /// Literal text — typed via the OS Unicode-string API. Adjacent text
    /// fragments are coalesced by the parser before this point.
    Text(String),
    /// A named key with optional modifiers. Name uses porthole's wire form
    /// (`Enter`, `ArrowUp`, `KeyC`, `F1`, etc.).
    Key { name: String, modifiers: Vec<Modifier> },
}

#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("unknown modifier in token {token:?}: {part:?} (expected C, M, S, Cmd, or ^)")]
    UnknownModifier { token: String, part: String },
    #[error("unknown key name in token {token:?}: {name:?}")]
    UnknownKey { token: String, name: String },
    #[error("empty token")]
    EmptyToken,
}

/// Parse a sequence of CLI argv tokens into a flat KeyToken stream.
///
/// A token that matches a named key (with optional modifiers) becomes a Key.
/// Anything else is literal text — and adjacent literal-text tokens are
/// coalesced with single-space separators ("hello" "world" → "hello world").
///
/// Tokens that LOOK like keys but contain unknown names — e.g. `Cmd-Frobble` —
/// produce `ParseError::UnknownKey`. Plain text that happens to start with
/// `C-`/`M-`/`S-`/`Cmd-` triggers the key parser; if you need such text
/// literal, use the `--literal` flag at the CLI (which skips parsing entirely).
pub fn parse_tokens(tokens: &[String]) -> Result<Vec<KeyToken>, ParseError> {
    let mut out: Vec<KeyToken> = Vec::with_capacity(tokens.len());
    for token in tokens {
        if token.is_empty() {
            return Err(ParseError::EmptyToken);
        }
        match parse_one(token) {
            ParseResult::Key(k) => out.push(k),
            ParseResult::Text => {
                if let Some(KeyToken::Text(prev)) = out.last_mut() {
                    prev.push(' ');
                    prev.push_str(token);
                } else {
                    out.push(KeyToken::Text(token.clone()));
                }
            }
            ParseResult::Error(e) => return Err(e),
        }
    }
    Ok(out)
}

/// Parse all tokens as literal text, coalesced with single spaces. Equivalent
/// to cleat's `-l` / `--literal` flag.
pub fn parse_literal(tokens: &[String]) -> KeyToken {
    KeyToken::Text(tokens.join(" "))
}

enum ParseResult {
    Key(KeyToken),
    Text,
    Error(ParseError),
}

fn parse_one(token: &str) -> ParseResult {
    // ^x is a Ctrl+x shorthand from cleat; equivalent to C-x.
    let (control_prefix, rest) = if let Some(rest) = token.strip_prefix('^') {
        (true, rest)
    } else {
        (false, token)
    };

    let (mut modifiers, base) = match parse_modifiers(rest) {
        Ok(m) => m,
        Err(_) if control_prefix => {
            return ParseResult::Error(ParseError::UnknownKey {
                token: token.to_string(),
                name: rest.to_string(),
            });
        }
        Err(_) => return ParseResult::Text,
    };
    if control_prefix && !modifiers.contains(&Modifier::Ctrl) {
        modifiers.push(Modifier::Ctrl);
    }

    // If base resolves to a known key name, emit a Key token. Otherwise:
    // - if we did pick up modifiers, that's a malformed token (modifiers without
    //   a recognised base) → ParseError.
    // - if no modifiers and base is just the original token, it's literal text.
    match resolve_named_key(base) {
        Some(name) => ParseResult::Key(KeyToken::Key { name, modifiers }),
        None if !modifiers.is_empty() || control_prefix => ParseResult::Error(ParseError::UnknownKey {
            token: token.to_string(),
            name: base.to_string(),
        }),
        None => ParseResult::Text,
    }
}

fn parse_modifiers(token: &str) -> Result<(Vec<Modifier>, &str), ParseError> {
    let mut parts: Vec<&str> = token.split('-').collect();
    if parts.len() < 2 {
        return Ok((Vec::new(), token));
    }
    // `Cmd-` is a multi-char modifier; recombine with the dash if it splits weirdly.
    // Specifically: ["Cmd", "x"] is fine via the lookup below. ["a", "b-c"] doesn't
    // happen because split('-') is greedy — we always end up with the base as the
    // last element.
    let base = parts.pop().expect("non-empty parts");
    if base.is_empty() {
        // Trailing dash means the base is empty — interpret as literal.
        return Ok((Vec::new(), token));
    }
    let mut modifiers = Vec::with_capacity(parts.len());
    for part in &parts {
        let m = match *part {
            "C" => Modifier::Ctrl,
            "M" => Modifier::Alt,
            "S" => Modifier::Shift,
            "Cmd" => Modifier::Cmd,
            _ => {
                return Err(ParseError::UnknownModifier {
                    token: token.to_string(),
                    part: part.to_string(),
                });
            }
        };
        if !modifiers.contains(&m) {
            modifiers.push(m);
        }
    }
    Ok((modifiers, base))
}

/// Map cleat-tmux key names AND DOM-style names onto porthole's wire vocabulary
/// (DOM KeyboardEvent.code: `Enter`, `Escape`, `ArrowUp`, `Backspace`, `KeyA`,
/// `Digit1`, `F1`...).
fn resolve_named_key(s: &str) -> Option<String> {
    let mapped = match s {
        // Identity / DOM-native — already in porthole's vocab.
        "Enter" | "Escape" | "Tab" | "Backspace" | "Space" | "Delete" => s,
        "ArrowUp" | "ArrowDown" | "ArrowLeft" | "ArrowRight" => s,
        "Home" | "End" | "PageUp" | "PageDown" => s,
        // Tmux aliases.
        "Esc" => "Escape",
        "BSpace" => "Backspace",
        "Up" => "ArrowUp",
        "Down" => "ArrowDown",
        "Left" => "ArrowLeft",
        "Right" => "ArrowRight",
        "PgUp" | "PPage" => "PageUp",
        "PgDn" | "NPage" => "PageDown",
        "DC" => "Delete",
        // Function keys and KeyX/DigitN passthrough.
        _ => {
            // F1..F12
            if let Some(rest) = s.strip_prefix('F')
                && let Ok(n) = rest.parse::<u8>()
                && (1..=12).contains(&n)
            {
                return Some(s.to_string());
            }
            // DOM-style KeyA..KeyZ and Digit0..Digit9 (used by porthole today).
            if (s.starts_with("Key") && s.len() == 4 && s.chars().nth(3).is_some_and(|c| c.is_ascii_uppercase()))
                || (s.starts_with("Digit") && s.len() == 6 && s.chars().nth(5).is_some_and(|c| c.is_ascii_digit()))
            {
                return Some(s.to_string());
            }
            // Single ASCII letter → KeyA..KeyZ (lowercase or uppercase both fine).
            if s.len() == 1
                && let Some(c) = s.chars().next()
                && c.is_ascii_alphabetic()
            {
                return Some(format!("Key{}", c.to_ascii_uppercase()));
            }
            // Single ASCII digit → Digit0..Digit9.
            if s.len() == 1
                && let Some(c) = s.chars().next()
                && c.is_ascii_digit()
            {
                return Some(format!("Digit{c}"));
            }
            return None;
        }
    };
    Some(mapped.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(strs: &[&str]) -> Vec<String> {
        strs.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn bare_text_token() {
        let out = parse_tokens(&s(&["hello"])).unwrap();
        assert_eq!(out, vec![KeyToken::Text("hello".into())]);
    }

    #[test]
    fn adjacent_text_tokens_coalesce_with_space() {
        let out = parse_tokens(&s(&["echo", "hi"])).unwrap();
        assert_eq!(out, vec![KeyToken::Text("echo hi".into())]);
    }

    #[test]
    fn named_key_enter() {
        let out = parse_tokens(&s(&["Enter"])).unwrap();
        assert_eq!(
            out,
            vec![KeyToken::Key {
                name: "Enter".into(),
                modifiers: vec![]
            }]
        );
    }

    #[test]
    fn ctrl_c_via_dash() {
        let out = parse_tokens(&s(&["C-c"])).unwrap();
        assert_eq!(
            out,
            vec![KeyToken::Key {
                name: "KeyC".into(),
                modifiers: vec![Modifier::Ctrl],
            }]
        );
    }

    #[test]
    fn ctrl_c_via_caret() {
        let out = parse_tokens(&s(&["^c"])).unwrap();
        assert_eq!(
            out,
            vec![KeyToken::Key {
                name: "KeyC".into(),
                modifiers: vec![Modifier::Ctrl],
            }]
        );
    }

    #[test]
    fn cmd_tab() {
        let out = parse_tokens(&s(&["Cmd-Tab"])).unwrap();
        assert_eq!(
            out,
            vec![KeyToken::Key {
                name: "Tab".into(),
                modifiers: vec![Modifier::Cmd],
            }]
        );
    }

    #[test]
    fn combined_modifiers() {
        let out = parse_tokens(&s(&["C-S-Up"])).unwrap();
        assert_eq!(
            out,
            vec![KeyToken::Key {
                name: "ArrowUp".into(),
                modifiers: vec![Modifier::Ctrl, Modifier::Shift],
            }]
        );
    }

    #[test]
    fn modifiers_dedupe() {
        let out = parse_tokens(&s(&["C-C-c"])).unwrap();
        assert_eq!(
            out,
            vec![KeyToken::Key {
                name: "KeyC".into(),
                modifiers: vec![Modifier::Ctrl],
            }]
        );
    }

    #[test]
    fn tmux_aliases_map_to_dom() {
        let out = parse_tokens(&s(&["Esc", "BSpace", "Up", "PgUp"])).unwrap();
        assert_eq!(
            out,
            vec![
                KeyToken::Key {
                    name: "Escape".into(),
                    modifiers: vec![]
                },
                KeyToken::Key {
                    name: "Backspace".into(),
                    modifiers: vec![]
                },
                KeyToken::Key {
                    name: "ArrowUp".into(),
                    modifiers: vec![]
                },
                KeyToken::Key {
                    name: "PageUp".into(),
                    modifiers: vec![]
                },
            ]
        );
    }

    #[test]
    fn dom_names_passthrough() {
        let out = parse_tokens(&s(&["ArrowUp", "Backspace", "KeyA", "Digit5", "F11"])).unwrap();
        assert_eq!(
            out,
            vec![
                KeyToken::Key {
                    name: "ArrowUp".into(),
                    modifiers: vec![]
                },
                KeyToken::Key {
                    name: "Backspace".into(),
                    modifiers: vec![]
                },
                KeyToken::Key {
                    name: "KeyA".into(),
                    modifiers: vec![]
                },
                KeyToken::Key {
                    name: "Digit5".into(),
                    modifiers: vec![]
                },
                KeyToken::Key {
                    name: "F11".into(),
                    modifiers: vec![]
                },
            ]
        );
    }

    #[test]
    fn text_then_key_then_text_stays_separated() {
        let out = parse_tokens(&s(&["hello", "Enter", "world"])).unwrap();
        assert_eq!(
            out,
            vec![
                KeyToken::Text("hello".into()),
                KeyToken::Key {
                    name: "Enter".into(),
                    modifiers: vec![]
                },
                KeyToken::Text("world".into()),
            ]
        );
    }

    #[test]
    fn unknown_key_with_modifier_errors() {
        let err = parse_tokens(&s(&["C-Frobble"])).unwrap_err();
        assert!(matches!(err, ParseError::UnknownKey { .. }), "got: {err:?}");
    }

    #[test]
    fn bare_dashed_text_is_literal() {
        // Multi-char text with embedded dashes that doesn't parse as modifiers
        // falls through as literal text.
        let out = parse_tokens(&s(&["hello-world"])).unwrap();
        assert_eq!(out, vec![KeyToken::Text("hello-world".into())]);
    }

    #[test]
    fn unknown_modifier_falls_through_to_text() {
        // "X-y" — X isn't a modifier → treated as literal text.
        let out = parse_tokens(&s(&["X-y"])).unwrap();
        assert_eq!(out, vec![KeyToken::Text("X-y".into())]);
    }

    #[test]
    fn empty_token_is_error() {
        let err = parse_tokens(&s(&[""])).unwrap_err();
        assert!(matches!(err, ParseError::EmptyToken));
    }

    #[test]
    fn parse_literal_concats_with_spaces() {
        let out = parse_literal(&s(&["echo", "hi", "there"]));
        assert_eq!(out, KeyToken::Text("echo hi there".into()));
    }

    #[test]
    fn single_letter_uppercase_or_lowercase_maps_to_key() {
        let out = parse_tokens(&s(&["C-A"])).unwrap();
        assert_eq!(
            out,
            vec![KeyToken::Key {
                name: "KeyA".into(),
                modifiers: vec![Modifier::Ctrl],
            }]
        );
        let out = parse_tokens(&s(&["C-z"])).unwrap();
        assert_eq!(
            out,
            vec![KeyToken::Key {
                name: "KeyZ".into(),
                modifiers: vec![Modifier::Ctrl],
            }]
        );
    }
}
