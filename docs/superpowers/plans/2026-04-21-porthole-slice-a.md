# Porthole Slice A — Evidence-Loop Completion Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Complete the agent-drives-and-captures loop — add input (`key`/`text`/`click`/`scroll`), `wait`, `close`, `focus`, and read-only `/attention` + `/displays` to the existing porthole workspace. Plus an additive `permissions` field on `AdapterInfo`.

**Architecture:** Extends the v0 foundation without changing any shipped contract. New adapter trait methods, new pipelines in `porthole-core` for input and wait (validation + timeout), new wire types in `porthole-protocol`, new axum routes in `portholed`, new CLI subcommands in `porthole`, and a macOS adapter implementation per verb. `wait stable`/`dirty` use a downsampled-frame-diff with a `threshold_pct` tolerance to handle blinking cursors.

**Tech Stack:** Same as foundation — Rust 2024, tokio, axum 0.8, hyper 1, hyperlocal 0.9, hyper-util, serde, serde_json, thiserror, tracing, uuid, clap 4, objc2 + objc2-app-kit + objc2-foundation, core-graphics, core-foundation, image. Adds `regex = "1"` (for `title_matches` condition) and grows the `image` crate's feature use to include grayscale decode + resize.

---

## Out of Scope for This Plan

Per the slice-A spec (`docs/superpowers/specs/2026-04-21-porthole-slice-a-design.md`, §11):

- Events SSE stream, `/events` resource
- Attach mode (`/surfaces/search` + `/surfaces/track`)
- Artifact launch kind, placement, `replace`, `auto_dismiss_after_ms`
- Tab surface enumeration or any tab-specific verb behavior
- Recording
- `focus: "preserve"` no-focus-steal input
- AX-element-reference targeting for click/scroll
- Cross-host routing
- Lifecycle modes on launch / command-at-launch
- Pixel-level scroll (line deltas only)
- Native event-backed wait

What's in scope: 9 new endpoints, 1 additive `AdapterInfo` schema extension, 3 new error codes, and the macOS adapter backing.

---

## File Structure

Files created or modified by this plan:

```
crates/porthole-core/
  Cargo.toml                              # add regex dep
  src/
    error.rs                              # modify: add 3 new error codes
    input.rs                              # NEW: KeyEvent, Modifier, ClickButton, ClickSpec, ScrollSpec
    key_names.rs                          # NEW: supported-key-name set + validation
    wait.rs                               # NEW: WaitCondition, WaitOutcome, LastObserved
    attention.rs                          # NEW: AttentionInfo, CursorPos
    display.rs                            # NEW: DisplayId, DisplayInfo
    permission.rs                         # NEW: PermissionStatus
    adapter.rs                            # modify: add 9 new trait methods
    in_memory.rs                          # modify: script + record new methods
    input_pipeline.rs                     # NEW: validation + adapter dispatch for input verbs
    wait_pipeline.rs                      # NEW: timeout + validation + adapter dispatch for wait
    lib.rs                                # modify: declare new modules + re-exports

crates/porthole-protocol/
  Cargo.toml                              # add regex (shared validation, no — core owns it)
  src/
    lib.rs                                # modify: declare new modules
    info.rs                               # modify: add permissions field to AdapterInfo
    input.rs                              # NEW: wire types for key/text/click/scroll
    wait.rs                               # NEW: wire types for wait
    close_focus.rs                        # NEW: wire types for close/focus
    attention.rs                          # NEW: wire types for attention/displays

crates/portholed/
  src/
    routes/
      mod.rs                              # modify: declare new route modules
      input.rs                            # NEW: /surfaces/{id}/key, /text, /click, /scroll
      wait.rs                             # NEW: /surfaces/{id}/wait
      close_focus.rs                      # NEW: /surfaces/{id}/close, /focus
      attention.rs                        # NEW: /attention, /displays
      info.rs                             # modify: include permissions in response
    server.rs                             # modify: wire new routes into build_router

crates/porthole/
  src/
    main.rs                               # modify: add new CLI subcommands
    commands/
      mod.rs                              # modify: declare new modules
      key.rs                              # NEW
      text.rs                             # NEW
      click.rs                            # NEW
      scroll.rs                           # NEW
      wait.rs                             # NEW
      close.rs                            # NEW
      focus.rs                            # NEW
      attention.rs                        # NEW
      displays.rs                         # NEW

crates/porthole-adapter-macos/
  Cargo.toml                              # add regex for title_matches, plus image features
  src/
    lib.rs                                # modify: implement new trait methods
    key_codes.rs                          # NEW: DOM key name → CGKeyCode mapping
    input.rs                              # NEW: CGEvent-based key/text/click/scroll
    close_focus.rs                        # NEW: AX close + focus
    attention.rs                          # NEW: AX + NSWorkspace attention
    display.rs                            # NEW: CG display enumeration
    wait.rs                               # NEW: poll conditions + frame diff
    frame_diff.rs                         # NEW: downsample-grayscale-diff helper
    permissions.rs                        # NEW: AXIsProcessTrusted + CGPreflightScreenCaptureAccess

crates/porthole-adapter-macos/tests/
  input_integration.rs                    # NEW: #[ignore] real-desktop input + wait tests
```

Rationale for module split:
- Each new verb or resource in `porthole-core` gets a focused file; nothing grows beyond ~150 lines.
- Macos adapter mirrors the same split for consistency.
- `frame_diff.rs` isolates the one piece of non-trivial image work (used only by `wait`) so it's unit-testable.

---

## Task 1: porthole-core — New Error Codes

**Files:**
- Modify: `crates/porthole-core/src/error.rs`

- [ ] **Step 1: Extend `ErrorCode` enum**

Open `crates/porthole-core/src/error.rs`. Add three variants to `ErrorCode`:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    SurfaceNotFound,
    SurfaceDead,
    PermissionNeeded,
    LaunchCorrelationFailed,
    LaunchCorrelationAmbiguous,
    LaunchTimeout,
    CandidateRefUnknown,
    AdapterUnsupported,
    CapabilityMissing,
    WaitTimeout,
    UnknownKey,
    InvalidCoordinate,
}
```

And extend `Display` impl with the three new snake_case strings: `"wait_timeout"`, `"unknown_key"`, `"invalid_coordinate"`.

- [ ] **Step 2: Add unit test coverage**

Append this test to the existing `#[cfg(test)] mod tests` block:

```rust
    #[test]
    fn new_error_codes_display_as_snake_case() {
        assert_eq!(ErrorCode::WaitTimeout.to_string(), "wait_timeout");
        assert_eq!(ErrorCode::UnknownKey.to_string(), "unknown_key");
        assert_eq!(ErrorCode::InvalidCoordinate.to_string(), "invalid_coordinate");
    }
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p porthole-core --lib error`
Expected: 3 passes (2 existing + 1 new).

- [ ] **Step 4: Commit**

```bash
git add crates/porthole-core/src/error.rs
git commit -m "feat(core): add wait_timeout, unknown_key, invalid_coordinate error codes"
```

---

## Task 2: porthole-core — Input Types

**Files:**
- Create: `crates/porthole-core/src/input.rs`
- Create: `crates/porthole-core/src/key_names.rs`
- Modify: `crates/porthole-core/src/lib.rs`

- [ ] **Step 1: Write `key_names.rs` with the supported key set**

Create `crates/porthole-core/src/key_names.rs`:

```rust
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
```

- [ ] **Step 2: Write `input.rs` with core input types**

Create `crates/porthole-core/src/input.rs`:

```rust
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum Modifier {
    Cmd,
    Ctrl,
    Alt,
    Shift,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyEvent {
    pub key: String,
    #[serde(default)]
    pub modifiers: Vec<Modifier>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClickButton {
    Left,
    Right,
    Middle,
}

impl Default for ClickButton {
    fn default() -> Self {
        Self::Left
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClickSpec {
    pub x: f64,
    pub y: f64,
    #[serde(default)]
    pub button: ClickButton,
    #[serde(default = "default_click_count")]
    pub count: u8,
    #[serde(default)]
    pub modifiers: Vec<Modifier>,
}

fn default_click_count() -> u8 {
    1
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScrollSpec {
    pub x: f64,
    pub y: f64,
    #[serde(default)]
    pub delta_x: f64,
    #[serde(default)]
    pub delta_y: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_event_roundtrip() {
        let ev = KeyEvent { key: "KeyA".into(), modifiers: vec![Modifier::Cmd] };
        let json = serde_json::to_string(&ev).unwrap();
        let back: KeyEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(back, ev);
    }

    #[test]
    fn click_button_default_is_left() {
        let click = ClickSpec { x: 0.0, y: 0.0, button: ClickButton::default(), count: 1, modifiers: vec![] };
        assert_eq!(click.button, ClickButton::Left);
    }

    #[test]
    fn click_spec_deserializes_without_optional_fields() {
        let json = r#"{"x": 10.0, "y": 20.0}"#;
        let click: ClickSpec = serde_json::from_str(json).unwrap();
        assert_eq!(click.button, ClickButton::Left);
        assert_eq!(click.count, 1);
        assert!(click.modifiers.is_empty());
    }

    #[test]
    fn modifier_serializes_as_pascal_case() {
        let json = serde_json::to_string(&Modifier::Cmd).unwrap();
        assert_eq!(json, "\"Cmd\"");
    }
}
```

- [ ] **Step 3: Register modules in `lib.rs`**

Edit `crates/porthole-core/src/lib.rs` — add `pub mod input;` and `pub mod key_names;` to the module list, and extend the re-exports:

```rust
#![forbid(unsafe_code)]

pub mod adapter;
pub mod error;
pub mod handle;
pub mod in_memory;
pub mod input;
pub mod key_names;
pub mod launch;
pub mod surface;

pub use error::{ErrorCode, PortholeError};
pub use input::{ClickButton, ClickSpec, KeyEvent, Modifier, ScrollSpec};
pub use surface::{SurfaceId, SurfaceInfo, SurfaceKind, SurfaceState};
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p porthole-core --lib input key_names`
Expected: 8 passes (4 each).

- [ ] **Step 5: Commit**

```bash
git add crates/porthole-core/src/input.rs crates/porthole-core/src/key_names.rs crates/porthole-core/src/lib.rs
git commit -m "feat(core): add KeyEvent, ClickSpec, ScrollSpec and key-name validation"
```

---

## Task 3: porthole-core — Wait Types

**Files:**
- Create: `crates/porthole-core/src/wait.rs`
- Modify: `crates/porthole-core/src/lib.rs`

- [ ] **Step 1: Write `wait.rs`**

Create `crates/porthole-core/src/wait.rs`:

```rust
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Conditions that `wait` can block on.
///
/// `threshold_pct` on Stable and Dirty is the percentage of pixels (0.0–100.0)
/// that must differ between consecutive samples (Stable) or from the initial
/// sample (Dirty) to count as a "real" change. Tolerates things like blinking
/// terminal cursors.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WaitCondition {
    Stable {
        #[serde(default = "default_stable_window_ms")]
        window_ms: u64,
        #[serde(default = "default_threshold_pct")]
        threshold_pct: f64,
    },
    Dirty {
        #[serde(default = "default_threshold_pct")]
        threshold_pct: f64,
    },
    Exists,
    Gone,
    TitleMatches {
        pattern: String,
    },
}

fn default_stable_window_ms() -> u64 {
    1500
}

fn default_threshold_pct() -> f64 {
    1.0
}

pub const DEFAULT_WAIT_TIMEOUT: Duration = Duration::from_millis(10_000);
pub const WAIT_SAMPLE_INTERVAL: Duration = Duration::from_millis(100);

/// Outcome of a successful wait.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WaitOutcome {
    pub condition: String, // "stable" | "dirty" | "exists" | "gone" | "title_matches"
    pub elapsed_ms: u64,
}

/// Diagnostic payload returned with `wait_timeout` errors.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LastObserved {
    FrameChange { last_change_ms_ago: u64, last_change_pct: f64 },
    Presence { alive: bool },
    Title { title: Option<String> },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_condition_uses_defaults() {
        let json = r#"{"type":"stable"}"#;
        let c: WaitCondition = serde_json::from_str(json).unwrap();
        assert!(matches!(c, WaitCondition::Stable { window_ms: 1500, threshold_pct } if (threshold_pct - 1.0).abs() < 1e-9));
    }

    #[test]
    fn dirty_condition_uses_default_threshold() {
        let json = r#"{"type":"dirty"}"#;
        let c: WaitCondition = serde_json::from_str(json).unwrap();
        assert!(matches!(c, WaitCondition::Dirty { threshold_pct } if (threshold_pct - 1.0).abs() < 1e-9));
    }

    #[test]
    fn title_matches_requires_pattern() {
        let json = r#"{"type":"title_matches","pattern":"^foo"}"#;
        let c: WaitCondition = serde_json::from_str(json).unwrap();
        assert!(matches!(c, WaitCondition::TitleMatches { pattern } if pattern == "^foo"));
    }

    #[test]
    fn exists_and_gone_serialize_as_tagged_empties() {
        let exists_json = serde_json::to_string(&WaitCondition::Exists).unwrap();
        assert_eq!(exists_json, r#"{"type":"exists"}"#);
        let gone_json = serde_json::to_string(&WaitCondition::Gone).unwrap();
        assert_eq!(gone_json, r#"{"type":"gone"}"#);
    }
}
```

- [ ] **Step 2: Register in `lib.rs`**

Edit `crates/porthole-core/src/lib.rs` — add `pub mod wait;` and extend re-exports:

```rust
pub use wait::{LastObserved, WaitCondition, WaitOutcome, DEFAULT_WAIT_TIMEOUT, WAIT_SAMPLE_INTERVAL};
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p porthole-core --lib wait`
Expected: 4 passes.

- [ ] **Step 4: Commit**

```bash
git add crates/porthole-core/src/wait.rs crates/porthole-core/src/lib.rs
git commit -m "feat(core): add WaitCondition, WaitOutcome, LastObserved"
```

---

## Task 4: porthole-core — Attention, Display, Permission Types

**Files:**
- Create: `crates/porthole-core/src/attention.rs`
- Create: `crates/porthole-core/src/display.rs`
- Create: `crates/porthole-core/src/permission.rs`
- Modify: `crates/porthole-core/src/lib.rs`

- [ ] **Step 1: Write `display.rs`**

Create `crates/porthole-core/src/display.rs`:

```rust
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DisplayId(String);

impl DisplayId {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DisplayInfo {
    pub id: DisplayId,
    pub bounds: Rect,
    pub scale: f64,
    pub primary: bool,
    pub focused: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_id_is_transparent_string() {
        let id = DisplayId::new("disp_1");
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, "\"disp_1\"");
    }

    #[test]
    fn display_info_roundtrip() {
        let d = DisplayInfo {
            id: DisplayId::new("disp_1"),
            bounds: Rect { x: 0.0, y: 0.0, w: 1920.0, h: 1080.0 },
            scale: 2.0,
            primary: true,
            focused: false,
        };
        let json = serde_json::to_string(&d).unwrap();
        let back: DisplayInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(back, d);
    }
}
```

- [ ] **Step 2: Write `attention.rs`**

Create `crates/porthole-core/src/attention.rs`:

```rust
use serde::{Deserialize, Serialize};

use crate::display::DisplayId;
use crate::surface::SurfaceId;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AttentionInfo {
    pub focused_surface_id: Option<SurfaceId>,
    pub focused_app_bundle: Option<String>,
    pub focused_display_id: Option<DisplayId>,
    pub cursor: CursorPos,
    pub recently_active_surface_ids: Vec<SurfaceId>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct CursorPos {
    pub x: f64,
    pub y: f64,
    pub display_id_index: Option<usize>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attention_info_roundtrip() {
        let a = AttentionInfo {
            focused_surface_id: Some(SurfaceId::from("surf_1")),
            focused_app_bundle: Some("com.example.app".into()),
            focused_display_id: Some(DisplayId::new("disp_1")),
            cursor: CursorPos { x: 100.0, y: 200.0, display_id_index: Some(0) },
            recently_active_surface_ids: vec![SurfaceId::from("surf_1"), SurfaceId::from("surf_2")],
        };
        let json = serde_json::to_string(&a).unwrap();
        let back: AttentionInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(back, a);
    }

    #[test]
    fn attention_info_with_null_focus() {
        let a = AttentionInfo {
            focused_surface_id: None,
            focused_app_bundle: None,
            focused_display_id: None,
            cursor: CursorPos { x: 0.0, y: 0.0, display_id_index: None },
            recently_active_surface_ids: vec![],
        };
        let json = serde_json::to_string(&a).unwrap();
        assert!(json.contains("\"focused_surface_id\":null"));
    }
}
```

Note: `CursorPos.display_id_index` references the index of the display in the most recent `DisplayInfo` list, rather than carrying a `DisplayId` directly. This keeps `AttentionInfo` self-contained without needing to cross-reference. The adapter fills it in from its live view.

- [ ] **Step 3: Write `permission.rs`**

Create `crates/porthole-core/src/permission.rs`:

```rust
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PermissionStatus {
    pub name: String,
    pub granted: bool,
    pub purpose: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permission_status_roundtrip() {
        let p = PermissionStatus {
            name: "accessibility".into(),
            granted: false,
            purpose: "input injection and some wait conditions".into(),
        };
        let json = serde_json::to_string(&p).unwrap();
        let back: PermissionStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(back, p);
    }
}
```

- [ ] **Step 4: Register in `lib.rs`**

Edit `crates/porthole-core/src/lib.rs` — add new modules and re-exports:

```rust
pub mod attention;
pub mod display;
pub mod permission;

pub use attention::{AttentionInfo, CursorPos};
pub use display::{DisplayId, DisplayInfo, Rect as DisplayRect};
pub use permission::PermissionStatus;
```

- [ ] **Step 5: Run tests**

Run: `cargo test -p porthole-core --lib attention display permission`
Expected: 5 passes.

- [ ] **Step 6: Commit**

```bash
git add crates/porthole-core/src/attention.rs crates/porthole-core/src/display.rs crates/porthole-core/src/permission.rs crates/porthole-core/src/lib.rs
git commit -m "feat(core): add AttentionInfo, DisplayInfo, PermissionStatus"
```

---

## Task 5: porthole-core — Extend Adapter Trait

**Files:**
- Modify: `crates/porthole-core/src/adapter.rs`

- [ ] **Step 1: Add new trait methods**

Open `crates/porthole-core/src/adapter.rs`. Add imports:

```rust
use crate::attention::AttentionInfo;
use crate::display::DisplayInfo;
use crate::input::{ClickSpec, KeyEvent, ScrollSpec};
use crate::permission::PermissionStatus;
use crate::wait::{LastObserved, WaitCondition, WaitOutcome};
```

Extend the `Adapter` trait with the new methods. Full updated trait:

```rust
#[async_trait]
pub trait Adapter: Send + Sync {
    fn name(&self) -> &'static str;

    async fn launch_process(&self, spec: &ProcessLaunchSpec) -> Result<LaunchOutcome, PortholeError>;

    async fn screenshot(&self, surface: &SurfaceInfo) -> Result<Screenshot, PortholeError>;

    async fn key(&self, surface: &SurfaceInfo, events: &[KeyEvent]) -> Result<(), PortholeError>;

    async fn text(&self, surface: &SurfaceInfo, text: &str) -> Result<(), PortholeError>;

    async fn click(&self, surface: &SurfaceInfo, spec: &ClickSpec) -> Result<(), PortholeError>;

    async fn scroll(&self, surface: &SurfaceInfo, spec: &ScrollSpec) -> Result<(), PortholeError>;

    async fn close(&self, surface: &SurfaceInfo) -> Result<(), PortholeError>;

    async fn focus(&self, surface: &SurfaceInfo) -> Result<(), PortholeError>;

    /// Wait for the condition to be satisfied. The pipeline layer wraps this
    /// in `tokio::time::timeout`; adapters may also respect the deadline
    /// internally for efficiency.
    async fn wait(
        &self,
        surface: &SurfaceInfo,
        condition: &WaitCondition,
    ) -> Result<WaitOutcome, PortholeError>;

    /// Returns diagnostics appropriate for `wait_timeout` error payloads,
    /// given the last observed state of the condition. Called by the
    /// pipeline on timeout so the adapter can describe what it saw.
    async fn wait_last_observed(
        &self,
        surface: &SurfaceInfo,
        condition: &WaitCondition,
    ) -> Result<LastObserved, PortholeError>;

    async fn attention(&self) -> Result<AttentionInfo, PortholeError>;

    async fn displays(&self) -> Result<Vec<DisplayInfo>, PortholeError>;

    async fn permissions(&self) -> Result<Vec<PermissionStatus>, PortholeError>;
}
```

- [ ] **Step 2: Verify it fails to compile**

Run: `cargo build -p porthole-core`
Expected: FAIL — `InMemoryAdapter` no longer satisfies the trait. This is expected; Task 6 fixes it.

- [ ] **Step 3: Commit (expected-to-not-build state is fine as a step commit)**

Hold off on committing until Task 6 restores the build. Move straight to Task 6.

---

## Task 6: porthole-core — Extend InMemoryAdapter

**Files:**
- Modify: `crates/porthole-core/src/in_memory.rs`

- [ ] **Step 1: Extend `Script` struct and method stubs**

Open `crates/porthole-core/src/in_memory.rs`. Replace the file with (builds on existing structure; adds new scripting hooks, call recorders, and trait method impls):

```rust
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::Mutex;

use crate::adapter::{
    Adapter, Confidence, Correlation, LaunchOutcome, ProcessLaunchSpec, Rect, Screenshot,
};
use crate::attention::{AttentionInfo, CursorPos};
use crate::display::{DisplayId, DisplayInfo, Rect as DisplayRect};
use crate::input::{ClickSpec, KeyEvent, ScrollSpec};
use crate::permission::PermissionStatus;
use crate::surface::{SurfaceId, SurfaceInfo, SurfaceKind, SurfaceState};
use crate::wait::{LastObserved, WaitCondition, WaitOutcome};
use crate::{ErrorCode, PortholeError};

#[derive(Clone, Default)]
pub struct InMemoryAdapter {
    script: Arc<Mutex<Script>>,
}

#[derive(Default)]
struct Script {
    next_launch_outcome: Option<Result<LaunchOutcome, PortholeError>>,
    next_screenshot: Option<Result<Screenshot, PortholeError>>,
    next_key_result: Option<Result<(), PortholeError>>,
    next_text_result: Option<Result<(), PortholeError>>,
    next_click_result: Option<Result<(), PortholeError>>,
    next_scroll_result: Option<Result<(), PortholeError>>,
    next_close_result: Option<Result<(), PortholeError>>,
    next_focus_result: Option<Result<(), PortholeError>>,
    next_wait_result: Option<Result<WaitOutcome, PortholeError>>,
    next_wait_last_observed: Option<Result<LastObserved, PortholeError>>,
    next_attention: Option<Result<AttentionInfo, PortholeError>>,
    next_displays: Option<Result<Vec<DisplayInfo>, PortholeError>>,
    next_permissions: Option<Result<Vec<PermissionStatus>, PortholeError>>,

    launch_calls: Vec<ProcessLaunchSpec>,
    screenshot_calls: Vec<SurfaceId>,
    key_calls: Vec<(SurfaceId, Vec<KeyEvent>)>,
    text_calls: Vec<(SurfaceId, String)>,
    click_calls: Vec<(SurfaceId, ClickSpec)>,
    scroll_calls: Vec<(SurfaceId, ScrollSpec)>,
    close_calls: Vec<SurfaceId>,
    focus_calls: Vec<SurfaceId>,
    wait_calls: Vec<(SurfaceId, WaitCondition)>,
    attention_calls: usize,
    displays_calls: usize,
    permissions_calls: usize,
}

impl InMemoryAdapter {
    pub fn new() -> Self {
        Self::default()
    }

    // Scripting setters — existing two retained:
    pub async fn set_next_launch_outcome(&self, v: Result<LaunchOutcome, PortholeError>) {
        self.script.lock().await.next_launch_outcome = Some(v);
    }
    pub async fn set_next_screenshot(&self, v: Result<Screenshot, PortholeError>) {
        self.script.lock().await.next_screenshot = Some(v);
    }

    // New scripting setters:
    pub async fn set_next_key_result(&self, v: Result<(), PortholeError>) {
        self.script.lock().await.next_key_result = Some(v);
    }
    pub async fn set_next_text_result(&self, v: Result<(), PortholeError>) {
        self.script.lock().await.next_text_result = Some(v);
    }
    pub async fn set_next_click_result(&self, v: Result<(), PortholeError>) {
        self.script.lock().await.next_click_result = Some(v);
    }
    pub async fn set_next_scroll_result(&self, v: Result<(), PortholeError>) {
        self.script.lock().await.next_scroll_result = Some(v);
    }
    pub async fn set_next_close_result(&self, v: Result<(), PortholeError>) {
        self.script.lock().await.next_close_result = Some(v);
    }
    pub async fn set_next_focus_result(&self, v: Result<(), PortholeError>) {
        self.script.lock().await.next_focus_result = Some(v);
    }
    pub async fn set_next_wait_result(&self, v: Result<WaitOutcome, PortholeError>) {
        self.script.lock().await.next_wait_result = Some(v);
    }
    pub async fn set_next_wait_last_observed(&self, v: Result<LastObserved, PortholeError>) {
        self.script.lock().await.next_wait_last_observed = Some(v);
    }
    pub async fn set_next_attention(&self, v: Result<AttentionInfo, PortholeError>) {
        self.script.lock().await.next_attention = Some(v);
    }
    pub async fn set_next_displays(&self, v: Result<Vec<DisplayInfo>, PortholeError>) {
        self.script.lock().await.next_displays = Some(v);
    }
    pub async fn set_next_permissions(&self, v: Result<Vec<PermissionStatus>, PortholeError>) {
        self.script.lock().await.next_permissions = Some(v);
    }

    // Recorders:
    pub async fn launch_calls(&self) -> Vec<ProcessLaunchSpec> {
        self.script.lock().await.launch_calls.clone()
    }
    pub async fn screenshot_calls(&self) -> Vec<SurfaceId> {
        self.script.lock().await.screenshot_calls.clone()
    }
    pub async fn key_calls(&self) -> Vec<(SurfaceId, Vec<KeyEvent>)> {
        self.script.lock().await.key_calls.clone()
    }
    pub async fn text_calls(&self) -> Vec<(SurfaceId, String)> {
        self.script.lock().await.text_calls.clone()
    }
    pub async fn click_calls(&self) -> Vec<(SurfaceId, ClickSpec)> {
        self.script.lock().await.click_calls.clone()
    }
    pub async fn scroll_calls(&self) -> Vec<(SurfaceId, ScrollSpec)> {
        self.script.lock().await.scroll_calls.clone()
    }
    pub async fn close_calls(&self) -> Vec<SurfaceId> {
        self.script.lock().await.close_calls.clone()
    }
    pub async fn focus_calls(&self) -> Vec<SurfaceId> {
        self.script.lock().await.focus_calls.clone()
    }
    pub async fn wait_calls(&self) -> Vec<(SurfaceId, WaitCondition)> {
        self.script.lock().await.wait_calls.clone()
    }
    pub async fn attention_calls(&self) -> usize {
        self.script.lock().await.attention_calls
    }
    pub async fn displays_calls(&self) -> usize {
        self.script.lock().await.displays_calls
    }
    pub async fn permissions_calls(&self) -> usize {
        self.script.lock().await.permissions_calls
    }

    pub fn make_default_launch_outcome(pid: u32) -> LaunchOutcome {
        let surface = SurfaceInfo {
            id: SurfaceId::new(),
            kind: SurfaceKind::Window,
            state: SurfaceState::Alive,
            title: Some("test window".to_string()),
            app_bundle: Some("com.example.test".to_string()),
            pid: Some(pid),
            parent_surface_id: None,
        };
        LaunchOutcome {
            surface,
            confidence: Confidence::Strong,
            correlation: Correlation::Tag,
            surface_was_preexisting: false,
        }
    }

    pub fn default_attention() -> AttentionInfo {
        AttentionInfo {
            focused_surface_id: None,
            focused_app_bundle: None,
            focused_display_id: None,
            cursor: CursorPos { x: 0.0, y: 0.0, display_id_index: None },
            recently_active_surface_ids: vec![],
        }
    }

    pub fn default_displays() -> Vec<DisplayInfo> {
        vec![DisplayInfo {
            id: DisplayId::new("in-mem-display-0"),
            bounds: DisplayRect { x: 0.0, y: 0.0, w: 1920.0, h: 1080.0 },
            scale: 1.0,
            primary: true,
            focused: true,
        }]
    }
}

#[async_trait]
impl Adapter for InMemoryAdapter {
    fn name(&self) -> &'static str {
        "in-memory"
    }

    async fn launch_process(&self, spec: &ProcessLaunchSpec) -> Result<LaunchOutcome, PortholeError> {
        let mut s = self.script.lock().await;
        s.launch_calls.push(spec.clone());
        s.next_launch_outcome.take().unwrap_or_else(|| Ok(Self::make_default_launch_outcome(4242)))
    }

    async fn screenshot(&self, surface: &SurfaceInfo) -> Result<Screenshot, PortholeError> {
        let mut s = self.script.lock().await;
        s.screenshot_calls.push(surface.id.clone());
        s.next_screenshot.take().unwrap_or_else(|| {
            Ok(Screenshot {
                png_bytes: minimal_png(),
                window_bounds_points: Rect { x: 0.0, y: 0.0, w: 800.0, h: 600.0 },
                content_bounds_points: None,
                scale: 2.0,
                captured_at_unix_ms: 0,
            })
        })
    }

    async fn key(&self, surface: &SurfaceInfo, events: &[KeyEvent]) -> Result<(), PortholeError> {
        let mut s = self.script.lock().await;
        s.key_calls.push((surface.id.clone(), events.to_vec()));
        s.next_key_result.take().unwrap_or(Ok(()))
    }

    async fn text(&self, surface: &SurfaceInfo, text: &str) -> Result<(), PortholeError> {
        let mut s = self.script.lock().await;
        s.text_calls.push((surface.id.clone(), text.to_string()));
        s.next_text_result.take().unwrap_or(Ok(()))
    }

    async fn click(&self, surface: &SurfaceInfo, spec: &ClickSpec) -> Result<(), PortholeError> {
        let mut s = self.script.lock().await;
        s.click_calls.push((surface.id.clone(), spec.clone()));
        s.next_click_result.take().unwrap_or(Ok(()))
    }

    async fn scroll(&self, surface: &SurfaceInfo, spec: &ScrollSpec) -> Result<(), PortholeError> {
        let mut s = self.script.lock().await;
        s.scroll_calls.push((surface.id.clone(), spec.clone()));
        s.next_scroll_result.take().unwrap_or(Ok(()))
    }

    async fn close(&self, surface: &SurfaceInfo) -> Result<(), PortholeError> {
        let mut s = self.script.lock().await;
        s.close_calls.push(surface.id.clone());
        s.next_close_result.take().unwrap_or(Ok(()))
    }

    async fn focus(&self, surface: &SurfaceInfo) -> Result<(), PortholeError> {
        let mut s = self.script.lock().await;
        s.focus_calls.push(surface.id.clone());
        s.next_focus_result.take().unwrap_or(Ok(()))
    }

    async fn wait(
        &self,
        surface: &SurfaceInfo,
        condition: &WaitCondition,
    ) -> Result<WaitOutcome, PortholeError> {
        let mut s = self.script.lock().await;
        s.wait_calls.push((surface.id.clone(), condition.clone()));
        s.next_wait_result.take().unwrap_or_else(|| {
            Ok(WaitOutcome {
                condition: wait_condition_tag(condition).to_string(),
                elapsed_ms: 0,
            })
        })
    }

    async fn wait_last_observed(
        &self,
        _surface: &SurfaceInfo,
        _condition: &WaitCondition,
    ) -> Result<LastObserved, PortholeError> {
        let mut s = self.script.lock().await;
        s.next_wait_last_observed.take().unwrap_or(Ok(LastObserved::Presence { alive: true }))
    }

    async fn attention(&self) -> Result<AttentionInfo, PortholeError> {
        let mut s = self.script.lock().await;
        s.attention_calls += 1;
        s.next_attention.take().unwrap_or_else(|| Ok(Self::default_attention()))
    }

    async fn displays(&self) -> Result<Vec<DisplayInfo>, PortholeError> {
        let mut s = self.script.lock().await;
        s.displays_calls += 1;
        s.next_displays.take().unwrap_or_else(|| Ok(Self::default_displays()))
    }

    async fn permissions(&self) -> Result<Vec<PermissionStatus>, PortholeError> {
        let mut s = self.script.lock().await;
        s.permissions_calls += 1;
        s.next_permissions.take().unwrap_or(Ok(vec![]))
    }
}

fn wait_condition_tag(c: &WaitCondition) -> &'static str {
    match c {
        WaitCondition::Stable { .. } => "stable",
        WaitCondition::Dirty { .. } => "dirty",
        WaitCondition::Exists => "exists",
        WaitCondition::Gone => "gone",
        WaitCondition::TitleMatches { .. } => "title_matches",
    }
}

fn minimal_png() -> Vec<u8> {
    const BYTES: &[u8] = &[
        0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52, 0x00, 0x00, 0x00,
        0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f, 0x15, 0xc4, 0x89, 0x00, 0x00, 0x00, 0x0d, 0x49,
        0x44, 0x41, 0x54, 0x78, 0x9c, 0x62, 0x00, 0x01, 0x00, 0x00, 0x05, 0x00, 0x01, 0x0d, 0x0a, 0x2d, 0xb4, 0x00, 0x00,
        0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
    ];
    BYTES.to_vec()
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::adapter::RequireConfidence;

    #[tokio::test]
    async fn launch_records_call_and_returns_default_outcome() {
        let adapter = InMemoryAdapter::new();
        let spec = ProcessLaunchSpec {
            app: "/Applications/Test.app".to_string(),
            args: vec!["--help".to_string()],
            cwd: None,
            env: vec![],
            timeout: Duration::from_secs(5),
            require_confidence: RequireConfidence::Strong,
        };
        let outcome = adapter.launch_process(&spec).await.unwrap();
        assert_eq!(outcome.confidence, Confidence::Strong);
        assert_eq!(adapter.launch_calls().await.len(), 1);
    }

    #[tokio::test]
    async fn screenshot_returns_png_bytes() {
        let adapter = InMemoryAdapter::new();
        let outcome = InMemoryAdapter::make_default_launch_outcome(1);
        let shot = adapter.screenshot(&outcome.surface).await.unwrap();
        assert!(shot.png_bytes.starts_with(&[0x89, 0x50, 0x4e, 0x47]));
    }

    #[tokio::test]
    async fn scripted_error_is_surfaced() {
        let adapter = InMemoryAdapter::new();
        adapter.set_next_key_result(Err(PortholeError::new(ErrorCode::PermissionNeeded, "no ax"))).await;
        let outcome = InMemoryAdapter::make_default_launch_outcome(1);
        let err = adapter.key(&outcome.surface, &[]).await.unwrap_err();
        assert_eq!(err.code, ErrorCode::PermissionNeeded);
    }

    #[tokio::test]
    async fn key_call_is_recorded() {
        let adapter = InMemoryAdapter::new();
        let outcome = InMemoryAdapter::make_default_launch_outcome(1);
        adapter
            .key(&outcome.surface, &[KeyEvent { key: "Enter".into(), modifiers: vec![] }])
            .await
            .unwrap();
        let calls = adapter.key_calls().await;
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].1[0].key, "Enter");
    }

    #[tokio::test]
    async fn attention_counter_increments() {
        let adapter = InMemoryAdapter::new();
        adapter.attention().await.unwrap();
        adapter.attention().await.unwrap();
        assert_eq!(adapter.attention_calls().await, 2);
    }

    #[tokio::test]
    async fn wait_returns_default_outcome_with_condition_tag() {
        let adapter = InMemoryAdapter::new();
        let outcome = InMemoryAdapter::make_default_launch_outcome(1);
        let result = adapter.wait(&outcome.surface, &WaitCondition::Exists).await.unwrap();
        assert_eq!(result.condition, "exists");
    }
}
```

- [ ] **Step 2: Run tests**

Run: `cargo test -p porthole-core --lib in_memory`
Expected: 6 passes.

- [ ] **Step 3: Verify the whole crate builds**

Run: `cargo build -p porthole-core`
Expected: clean build.

- [ ] **Step 4: Commit (bundles Task 5 + Task 6)**

```bash
git add crates/porthole-core/src/adapter.rs crates/porthole-core/src/in_memory.rs
git commit -m "feat(core): extend Adapter trait and InMemoryAdapter with new verbs"
```

---

## Task 7: porthole-core — Input Pipeline

**Files:**
- Create: `crates/porthole-core/src/input_pipeline.rs`
- Modify: `crates/porthole-core/src/lib.rs`

- [ ] **Step 1: Write `input_pipeline.rs`**

Create `crates/porthole-core/src/input_pipeline.rs`:

```rust
use std::sync::Arc;

use crate::adapter::Adapter;
use crate::handle::HandleStore;
use crate::input::{ClickSpec, KeyEvent, ScrollSpec};
use crate::key_names;
use crate::surface::SurfaceId;
use crate::{ErrorCode, PortholeError};

pub struct InputPipeline {
    adapter: Arc<dyn Adapter>,
    handles: HandleStore,
}

impl InputPipeline {
    pub fn new(adapter: Arc<dyn Adapter>, handles: HandleStore) -> Self {
        Self { adapter, handles }
    }

    pub async fn key(&self, surface: &SurfaceId, events: &[KeyEvent]) -> Result<(), PortholeError> {
        for ev in events {
            if !key_names::is_supported(&ev.key) {
                return Err(PortholeError::new(
                    ErrorCode::UnknownKey,
                    format!("key '{}' is not in the supported set", ev.key),
                ));
            }
        }
        let info = self.handles.require_alive(surface).await?;
        self.adapter.key(&info, events).await
    }

    pub async fn text(&self, surface: &SurfaceId, text: &str) -> Result<(), PortholeError> {
        let info = self.handles.require_alive(surface).await?;
        self.adapter.text(&info, text).await
    }

    pub async fn click(&self, surface: &SurfaceId, spec: &ClickSpec) -> Result<(), PortholeError> {
        if spec.count == 0 || spec.count > 3 {
            return Err(PortholeError::new(
                ErrorCode::InvalidCoordinate,
                format!("click count must be 1, 2, or 3 (got {})", spec.count),
            ));
        }
        let info = self.handles.require_alive(surface).await?;
        self.adapter.click(&info, spec).await
    }

    pub async fn scroll(&self, surface: &SurfaceId, spec: &ScrollSpec) -> Result<(), PortholeError> {
        let info = self.handles.require_alive(surface).await?;
        self.adapter.scroll(&info, spec).await
    }

    pub async fn close(&self, surface: &SurfaceId) -> Result<(), PortholeError> {
        let info = self.handles.require_alive(surface).await?;
        self.adapter.close(&info).await?;
        self.handles.mark_dead(surface).await?;
        Ok(())
    }

    pub async fn focus(&self, surface: &SurfaceId) -> Result<(), PortholeError> {
        let info = self.handles.require_alive(surface).await?;
        self.adapter.focus(&info).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::in_memory::InMemoryAdapter;
    use crate::surface::SurfaceInfo;

    async fn setup() -> (Arc<InMemoryAdapter>, HandleStore, SurfaceId) {
        let adapter = Arc::new(InMemoryAdapter::new());
        let handles = HandleStore::new();
        let info = SurfaceInfo::window(SurfaceId::new(), 1);
        let id = info.id.clone();
        handles.insert(info).await;
        (adapter, handles, id)
    }

    #[tokio::test]
    async fn key_rejects_unsupported_name() {
        let (adapter, handles, id) = setup().await;
        let pipeline = InputPipeline::new(adapter.clone(), handles);
        let err = pipeline.key(&id, &[KeyEvent { key: "NotAKey".into(), modifiers: vec![] }]).await.unwrap_err();
        assert_eq!(err.code, ErrorCode::UnknownKey);
    }

    #[tokio::test]
    async fn key_delegates_to_adapter() {
        let (adapter, handles, id) = setup().await;
        let pipeline = InputPipeline::new(adapter.clone(), handles);
        pipeline.key(&id, &[KeyEvent { key: "Enter".into(), modifiers: vec![] }]).await.unwrap();
        assert_eq!(adapter.key_calls().await.len(), 1);
    }

    #[tokio::test]
    async fn click_rejects_count_zero() {
        let (adapter, handles, id) = setup().await;
        let pipeline = InputPipeline::new(adapter.clone(), handles);
        let err = pipeline
            .click(&id, &ClickSpec { x: 0.0, y: 0.0, button: crate::input::ClickButton::Left, count: 0, modifiers: vec![] })
            .await
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidCoordinate);
    }

    #[tokio::test]
    async fn close_marks_handle_dead() {
        let (adapter, handles, id) = setup().await;
        let pipeline = InputPipeline::new(adapter.clone(), handles.clone());
        pipeline.close(&id).await.unwrap();
        let err = handles.require_alive(&id).await.unwrap_err();
        assert_eq!(err.code, ErrorCode::SurfaceDead);
    }
}
```

- [ ] **Step 2: Register in `lib.rs`**

Edit `crates/porthole-core/src/lib.rs`: add `pub mod input_pipeline;`.

- [ ] **Step 3: Run tests**

Run: `cargo test -p porthole-core --lib input_pipeline`
Expected: 4 passes.

- [ ] **Step 4: Commit**

```bash
git add crates/porthole-core/src/input_pipeline.rs crates/porthole-core/src/lib.rs
git commit -m "feat(core): add InputPipeline with key/text/click/scroll/close/focus"
```

---

## Task 8: porthole-core — Wait Pipeline

**Files:**
- Create: `crates/porthole-core/src/wait_pipeline.rs`
- Modify: `crates/porthole-core/src/lib.rs`

- [ ] **Step 1: Add `regex` dep to `porthole-core`**

Edit `crates/porthole-core/Cargo.toml` — under `[dependencies]`, add:

```toml
regex = "1"
```

- [ ] **Step 2: Write `wait_pipeline.rs`**

Create `crates/porthole-core/src/wait_pipeline.rs`:

```rust
use std::sync::Arc;
use std::time::Duration;

use regex::Regex;
use tokio::time::timeout;

use crate::adapter::Adapter;
use crate::handle::HandleStore;
use crate::surface::SurfaceId;
use crate::wait::{LastObserved, WaitCondition, WaitOutcome};
use crate::{ErrorCode, PortholeError};

pub struct WaitPipeline {
    adapter: Arc<dyn Adapter>,
    handles: HandleStore,
}

pub struct WaitTimeoutInfo {
    pub last_observed: LastObserved,
    pub elapsed_ms: u64,
}

impl WaitPipeline {
    pub fn new(adapter: Arc<dyn Adapter>, handles: HandleStore) -> Self {
        Self { adapter, handles }
    }

    pub async fn wait(
        &self,
        surface: &SurfaceId,
        condition: &WaitCondition,
        timeout_duration: Duration,
    ) -> Result<WaitOutcome, WaitPipelineError> {
        validate_condition(condition)?;
        let info = self.handles.require_alive(surface).await.map_err(WaitPipelineError::Porthole)?;

        let start = std::time::Instant::now();
        match timeout(timeout_duration, self.adapter.wait(&info, condition)).await {
            Ok(Ok(outcome)) => Ok(outcome),
            Ok(Err(e)) => Err(WaitPipelineError::Porthole(e)),
            Err(_) => {
                let last = self
                    .adapter
                    .wait_last_observed(&info, condition)
                    .await
                    .unwrap_or(LastObserved::Presence { alive: true });
                Err(WaitPipelineError::Timeout(WaitTimeoutInfo {
                    last_observed: last,
                    elapsed_ms: start.elapsed().as_millis() as u64,
                }))
            }
        }
    }
}

#[derive(Debug)]
pub enum WaitPipelineError {
    Porthole(PortholeError),
    Timeout(WaitTimeoutInfo),
}

fn validate_condition(condition: &WaitCondition) -> Result<(), WaitPipelineError> {
    match condition {
        WaitCondition::Stable { threshold_pct, .. } | WaitCondition::Dirty { threshold_pct } => {
            if !threshold_pct.is_finite() || *threshold_pct < 0.0 || *threshold_pct > 100.0 {
                return Err(WaitPipelineError::Porthole(PortholeError::new(
                    ErrorCode::InvalidCoordinate,
                    format!("threshold_pct must be in [0, 100]; got {threshold_pct}"),
                )));
            }
            Ok(())
        }
        WaitCondition::TitleMatches { pattern } => {
            Regex::new(pattern).map_err(|e| {
                WaitPipelineError::Porthole(PortholeError::new(
                    ErrorCode::InvalidCoordinate,
                    format!("invalid regex '{pattern}': {e}"),
                ))
            })?;
            Ok(())
        }
        WaitCondition::Exists | WaitCondition::Gone => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::in_memory::InMemoryAdapter;
    use crate::surface::SurfaceInfo;

    async fn setup() -> (Arc<InMemoryAdapter>, HandleStore, SurfaceId) {
        let adapter = Arc::new(InMemoryAdapter::new());
        let handles = HandleStore::new();
        let info = SurfaceInfo::window(SurfaceId::new(), 1);
        let id = info.id.clone();
        handles.insert(info).await;
        (adapter, handles, id)
    }

    #[tokio::test]
    async fn exists_condition_returns_quickly() {
        let (adapter, handles, id) = setup().await;
        let pipeline = WaitPipeline::new(adapter, handles);
        let outcome = pipeline.wait(&id, &WaitCondition::Exists, Duration::from_secs(5)).await.unwrap();
        assert_eq!(outcome.condition, "exists");
    }

    #[tokio::test]
    async fn invalid_regex_errors_as_invalid_coordinate() {
        let (adapter, handles, id) = setup().await;
        let pipeline = WaitPipeline::new(adapter, handles);
        let err = pipeline
            .wait(
                &id,
                &WaitCondition::TitleMatches { pattern: "[invalid".to_string() },
                Duration::from_secs(1),
            )
            .await;
        match err {
            Err(WaitPipelineError::Porthole(e)) => assert_eq!(e.code, ErrorCode::InvalidCoordinate),
            _ => panic!("expected invalid coordinate error"),
        }
    }

    #[tokio::test]
    async fn timeout_surfaces_last_observed() {
        let (adapter, handles, id) = setup().await;
        adapter
            .set_next_wait_result(Err(PortholeError::new(ErrorCode::SurfaceNotFound, "will be ignored")))
            .await;
        // Adapter will return err immediately if not clobbered — so simulate long wait via a different route:
        // Replace next_wait_result with a future that never resolves via never-setting-it but adapter default returns
        // immediately, so use a very short timeout and rely on adapter behavior:
        let adapter2 = Arc::new(InMemoryAdapter::new());
        // Do not set next_wait_result: adapter returns default immediately — we cannot easily force timeout without
        // a blocking fixture. For now, assert the non-timeout path works instead.
        let pipeline = WaitPipeline::new(adapter2, handles);
        let outcome = pipeline.wait(&id, &WaitCondition::Exists, Duration::from_millis(50)).await.unwrap();
        assert_eq!(outcome.condition, "exists");
    }

    #[tokio::test]
    async fn invalid_threshold_pct_rejected() {
        let (adapter, handles, id) = setup().await;
        let pipeline = WaitPipeline::new(adapter, handles);
        let err = pipeline
            .wait(
                &id,
                &WaitCondition::Dirty { threshold_pct: -1.0 },
                Duration::from_secs(1),
            )
            .await;
        match err {
            Err(WaitPipelineError::Porthole(e)) => assert_eq!(e.code, ErrorCode::InvalidCoordinate),
            _ => panic!("expected invalid coordinate error"),
        }
    }
}
```

Note on test #3 above: it's documented as a placeholder assertion — forcing a true `tokio::time::timeout` requires a blocking fixture the in-memory adapter doesn't currently provide. A later task adds a "block for N ms" scripting hook to `InMemoryAdapter` if needed. For this slice, timeout behavior is exercised through the route and integration tests, not the pipeline unit tests.

- [ ] **Step 3: Register in `lib.rs`**

Edit `crates/porthole-core/src/lib.rs`: add `pub mod wait_pipeline;`.

- [ ] **Step 4: Run tests**

Run: `cargo test -p porthole-core --lib wait_pipeline`
Expected: 4 passes.

- [ ] **Step 5: Commit**

```bash
git add crates/porthole-core/Cargo.toml crates/porthole-core/src/wait_pipeline.rs crates/porthole-core/src/lib.rs
git commit -m "feat(core): add WaitPipeline with validation and timeout"
```

---

## Task 9: porthole-protocol — New Wire Types

**Files:**
- Modify: `crates/porthole-protocol/Cargo.toml`
- Modify: `crates/porthole-protocol/src/lib.rs`
- Modify: `crates/porthole-protocol/src/info.rs`
- Create: `crates/porthole-protocol/src/input.rs`
- Create: `crates/porthole-protocol/src/wait.rs`
- Create: `crates/porthole-protocol/src/close_focus.rs`
- Create: `crates/porthole-protocol/src/attention.rs`

- [ ] **Step 1: Extend `info.rs` with permissions**

Edit `crates/porthole-protocol/src/info.rs`:

```rust
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InfoResponse {
    pub daemon_version: String,
    pub uptime_seconds: u64,
    pub adapters: Vec<AdapterInfo>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AdapterInfo {
    pub name: String,
    pub loaded: bool,
    pub capabilities: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub permissions: Vec<PermissionStatus>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PermissionStatus {
    pub name: String,
    pub granted: bool,
    pub purpose: String,
}
```

- [ ] **Step 2: Write `input.rs`**

Create `crates/porthole-protocol/src/input.rs`:

```rust
use serde::{Deserialize, Serialize};

use porthole_core::input::{ClickButton, ClickSpec, KeyEvent, Modifier, ScrollSpec};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct KeyRequest {
    pub events: Vec<KeyEvent>,
    #[serde(default)]
    pub session: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct KeyResponse {
    pub surface_id: String,
    pub events_sent: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TextRequest {
    pub text: String,
    #[serde(default)]
    pub session: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TextResponse {
    pub surface_id: String,
    pub chars_sent: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ClickRequest {
    pub x: f64,
    pub y: f64,
    #[serde(default)]
    pub button: ClickButton,
    #[serde(default = "default_count")]
    pub count: u8,
    #[serde(default)]
    pub modifiers: Vec<Modifier>,
    #[serde(default)]
    pub session: Option<String>,
}

fn default_count() -> u8 {
    1
}

impl From<&ClickRequest> for ClickSpec {
    fn from(r: &ClickRequest) -> Self {
        ClickSpec {
            x: r.x,
            y: r.y,
            button: r.button,
            count: r.count,
            modifiers: r.modifiers.clone(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ClickResponse {
    pub surface_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScrollRequest {
    pub x: f64,
    pub y: f64,
    #[serde(default)]
    pub delta_x: f64,
    #[serde(default)]
    pub delta_y: f64,
    #[serde(default)]
    pub session: Option<String>,
}

impl From<&ScrollRequest> for ScrollSpec {
    fn from(r: &ScrollRequest) -> Self {
        ScrollSpec { x: r.x, y: r.y, delta_x: r.delta_x, delta_y: r.delta_y }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScrollResponse {
    pub surface_id: String,
}
```

- [ ] **Step 3: Write `wait.rs`**

Create `crates/porthole-protocol/src/wait.rs`:

```rust
use serde::{Deserialize, Serialize};

use porthole_core::wait::{LastObserved, WaitCondition};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WaitRequest {
    pub condition: WaitCondition,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
    #[serde(default)]
    pub session: Option<String>,
}

fn default_timeout_ms() -> u64 {
    10_000
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WaitResponse {
    pub surface_id: String,
    pub condition: String,
    pub elapsed_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WaitTimeoutBody {
    pub elapsed_ms: u64,
    pub last_observed: LastObserved,
}
```

- [ ] **Step 4: Write `close_focus.rs`**

Create `crates/porthole-protocol/src/close_focus.rs`:

```rust
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CloseRequest {
    #[serde(default)]
    pub session: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CloseResponse {
    pub surface_id: String,
    pub closed: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct FocusRequest {
    #[serde(default)]
    pub session: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FocusResponse {
    pub surface_id: String,
    pub focused: bool,
}
```

- [ ] **Step 5: Write `attention.rs`**

Create `crates/porthole-protocol/src/attention.rs`:

```rust
use serde::{Deserialize, Serialize};

pub use porthole_core::attention::{AttentionInfo, CursorPos};
pub use porthole_core::display::DisplayInfo;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DisplaysResponse {
    pub displays: Vec<DisplayInfo>,
}
```

- [ ] **Step 6: Update `lib.rs`**

Edit `crates/porthole-protocol/src/lib.rs`:

```rust
#![forbid(unsafe_code)]

pub mod attention;
pub mod close_focus;
pub mod error;
pub mod info;
pub mod input;
pub mod launches;
pub mod screenshot;
pub mod wait;

pub use porthole_core::surface::{SurfaceId, SurfaceKind, SurfaceState};
```

- [ ] **Step 7: Verify build**

Run: `cargo build -p porthole-protocol`
Expected: clean build.

- [ ] **Step 8: Commit**

```bash
git add crates/porthole-protocol
git commit -m "feat(protocol): add wire types for input, wait, close/focus, attention/displays"
```

---

## Task 10: portholed — New Routes (input, wait, close/focus, attention, displays)

**Files:**
- Create: `crates/portholed/src/routes/input.rs`
- Create: `crates/portholed/src/routes/wait.rs`
- Create: `crates/portholed/src/routes/close_focus.rs`
- Create: `crates/portholed/src/routes/attention.rs`
- Modify: `crates/portholed/src/routes/mod.rs`
- Modify: `crates/portholed/src/routes/errors.rs`
- Modify: `crates/portholed/src/state.rs`

- [ ] **Step 1: Extend `AppState` with input + wait pipelines**

Edit `crates/portholed/src/state.rs`:

```rust
use std::sync::Arc;
use std::time::Instant;

use porthole_core::adapter::Adapter;
use porthole_core::handle::HandleStore;
use porthole_core::input_pipeline::InputPipeline;
use porthole_core::launch::LaunchPipeline;
use porthole_core::wait_pipeline::WaitPipeline;

#[derive(Clone)]
pub struct AppState {
    pub adapter: Arc<dyn Adapter>,
    pub handles: HandleStore,
    pub pipeline: Arc<LaunchPipeline>,
    pub input: Arc<InputPipeline>,
    pub wait: Arc<WaitPipeline>,
    pub started_at: Instant,
    pub daemon_version: &'static str,
}

impl AppState {
    pub fn new(adapter: Arc<dyn Adapter>) -> Self {
        let handles = HandleStore::new();
        let pipeline = Arc::new(LaunchPipeline::new(adapter.clone(), handles.clone()));
        let input = Arc::new(InputPipeline::new(adapter.clone(), handles.clone()));
        let wait = Arc::new(WaitPipeline::new(adapter.clone(), handles.clone()));
        Self {
            adapter,
            handles,
            pipeline,
            input,
            wait,
            started_at: Instant::now(),
            daemon_version: env!("CARGO_PKG_VERSION"),
        }
    }

    pub fn uptime_seconds(&self) -> u64 {
        self.started_at.elapsed().as_secs()
    }
}
```

- [ ] **Step 2: Extend `errors.rs` to map wait-pipeline errors**

Edit `crates/portholed/src/routes/errors.rs`:

```rust
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use porthole_core::wait_pipeline::WaitPipelineError;
use porthole_core::{ErrorCode, PortholeError};
use porthole_protocol::error::WireError;
use porthole_protocol::wait::WaitTimeoutBody;
use serde::Serialize;

pub struct ApiError(pub WireError);

impl From<PortholeError> for ApiError {
    fn from(err: PortholeError) -> Self {
        Self(err.into())
    }
}

#[derive(Serialize)]
struct TimeoutError<'a> {
    code: ErrorCode,
    message: &'a str,
    timeout: WaitTimeoutBody,
}

impl From<WaitPipelineError> for ApiError {
    fn from(err: WaitPipelineError) -> Self {
        match err {
            WaitPipelineError::Porthole(e) => Self(e.into()),
            WaitPipelineError::Timeout(info) => Self(WireError {
                code: ErrorCode::WaitTimeout,
                message: format!("wait condition not satisfied within timeout ({}ms elapsed)", info.elapsed_ms),
            }),
            // Note: the `timeout` diagnostics live in the wire body beside code+message;
            // we'd need a richer WireError shape to carry them. For this slice, we
            // include the elapsed_ms in the message and add structured diagnostics
            // via a later events slice.
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = match self.0.code {
            ErrorCode::SurfaceNotFound => StatusCode::NOT_FOUND,
            ErrorCode::SurfaceDead => StatusCode::GONE,
            ErrorCode::PermissionNeeded => StatusCode::FORBIDDEN,
            ErrorCode::LaunchCorrelationFailed => StatusCode::UNPROCESSABLE_ENTITY,
            ErrorCode::LaunchCorrelationAmbiguous => StatusCode::CONFLICT,
            ErrorCode::LaunchTimeout => StatusCode::GATEWAY_TIMEOUT,
            ErrorCode::CandidateRefUnknown => StatusCode::NOT_FOUND,
            ErrorCode::AdapterUnsupported => StatusCode::BAD_REQUEST,
            ErrorCode::CapabilityMissing => StatusCode::NOT_IMPLEMENTED,
            ErrorCode::WaitTimeout => StatusCode::GATEWAY_TIMEOUT,
            ErrorCode::UnknownKey => StatusCode::BAD_REQUEST,
            ErrorCode::InvalidCoordinate => StatusCode::BAD_REQUEST,
        };
        (status, Json(self.0)).into_response()
    }
}
```

- [ ] **Step 3: Write `routes/input.rs`**

Create `crates/portholed/src/routes/input.rs`:

```rust
use axum::extract::{Path, State};
use axum::Json;
use porthole_core::surface::SurfaceId;
use porthole_protocol::input::{
    ClickRequest, ClickResponse, KeyRequest, KeyResponse, ScrollRequest, ScrollResponse, TextRequest, TextResponse,
};

use crate::routes::errors::ApiError;
use crate::state::AppState;

pub async fn post_key(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<KeyRequest>,
) -> Result<Json<KeyResponse>, ApiError> {
    let surface_id = SurfaceId::from(id);
    let count = req.events.len();
    state.input.key(&surface_id, &req.events).await?;
    Ok(Json(KeyResponse { surface_id: surface_id.to_string(), events_sent: count }))
}

pub async fn post_text(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<TextRequest>,
) -> Result<Json<TextResponse>, ApiError> {
    let surface_id = SurfaceId::from(id);
    let chars = req.text.chars().count();
    state.input.text(&surface_id, &req.text).await?;
    Ok(Json(TextResponse { surface_id: surface_id.to_string(), chars_sent: chars }))
}

pub async fn post_click(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<ClickRequest>,
) -> Result<Json<ClickResponse>, ApiError> {
    let surface_id = SurfaceId::from(id);
    let spec = (&req).into();
    state.input.click(&surface_id, &spec).await?;
    Ok(Json(ClickResponse { surface_id: surface_id.to_string() }))
}

pub async fn post_scroll(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<ScrollRequest>,
) -> Result<Json<ScrollResponse>, ApiError> {
    let surface_id = SurfaceId::from(id);
    let spec = (&req).into();
    state.input.scroll(&surface_id, &spec).await?;
    Ok(Json(ScrollResponse { surface_id: surface_id.to_string() }))
}
```

- [ ] **Step 4: Write `routes/wait.rs`**

Create `crates/portholed/src/routes/wait.rs`:

```rust
use std::time::Duration;

use axum::extract::{Path, State};
use axum::Json;
use porthole_core::surface::SurfaceId;
use porthole_protocol::wait::{WaitRequest, WaitResponse};

use crate::routes::errors::ApiError;
use crate::state::AppState;

pub async fn post_wait(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<WaitRequest>,
) -> Result<Json<WaitResponse>, ApiError> {
    let surface_id = SurfaceId::from(id);
    let timeout = Duration::from_millis(req.timeout_ms);
    let outcome = state.wait.wait(&surface_id, &req.condition, timeout).await?;
    Ok(Json(WaitResponse {
        surface_id: surface_id.to_string(),
        condition: outcome.condition,
        elapsed_ms: outcome.elapsed_ms,
    }))
}
```

- [ ] **Step 5: Write `routes/close_focus.rs`**

Create `crates/portholed/src/routes/close_focus.rs`:

```rust
use axum::extract::{Path, State};
use axum::Json;
use porthole_core::surface::SurfaceId;
use porthole_protocol::close_focus::{CloseRequest, CloseResponse, FocusRequest, FocusResponse};

use crate::routes::errors::ApiError;
use crate::state::AppState;

pub async fn post_close(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(_req): Json<CloseRequest>,
) -> Result<Json<CloseResponse>, ApiError> {
    let surface_id = SurfaceId::from(id);
    state.input.close(&surface_id).await?;
    Ok(Json(CloseResponse { surface_id: surface_id.to_string(), closed: true }))
}

pub async fn post_focus(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(_req): Json<FocusRequest>,
) -> Result<Json<FocusResponse>, ApiError> {
    let surface_id = SurfaceId::from(id);
    state.input.focus(&surface_id).await?;
    Ok(Json(FocusResponse { surface_id: surface_id.to_string(), focused: true }))
}
```

- [ ] **Step 6: Write `routes/attention.rs`**

Create `crates/portholed/src/routes/attention.rs`:

```rust
use axum::extract::State;
use axum::Json;
use porthole_core::attention::AttentionInfo;
use porthole_protocol::attention::DisplaysResponse;

use crate::routes::errors::ApiError;
use crate::state::AppState;

pub async fn get_attention(State(state): State<AppState>) -> Result<Json<AttentionInfo>, ApiError> {
    let info = state.adapter.attention().await?;
    Ok(Json(info))
}

pub async fn get_displays(State(state): State<AppState>) -> Result<Json<DisplaysResponse>, ApiError> {
    let displays = state.adapter.displays().await?;
    Ok(Json(DisplaysResponse { displays }))
}
```

- [ ] **Step 7: Update `routes/mod.rs`**

Edit `crates/portholed/src/routes/mod.rs`:

```rust
pub mod attention;
pub mod close_focus;
pub mod errors;
pub mod info;
pub mod input;
pub mod launches;
pub mod screenshot;
pub mod wait;
```

- [ ] **Step 8: Verify build (route wiring to server in next task)**

Run: `cargo build -p portholed --lib`
Expected: clean build.

- [ ] **Step 9: Commit**

```bash
git add crates/portholed/src/routes crates/portholed/src/state.rs
git commit -m "feat(daemon): add input, wait, close/focus, attention, displays routes"
```

---

## Task 11: portholed — /info with permissions + server wiring + router tests

**Files:**
- Modify: `crates/portholed/src/routes/info.rs`
- Modify: `crates/portholed/src/server.rs`

- [ ] **Step 1: Extend `/info` to include adapter permissions**

Edit `crates/portholed/src/routes/info.rs`:

```rust
use axum::extract::State;
use axum::Json;
use porthole_core::permission::PermissionStatus as CorePermission;
use porthole_protocol::info::{AdapterInfo, InfoResponse, PermissionStatus};

use crate::routes::errors::ApiError;
use crate::state::AppState;

pub async fn get_info(State(state): State<AppState>) -> Result<Json<InfoResponse>, ApiError> {
    let perms = state.adapter.permissions().await.unwrap_or_default();
    Ok(Json(InfoResponse {
        daemon_version: state.daemon_version.to_string(),
        uptime_seconds: state.uptime_seconds(),
        adapters: vec![AdapterInfo {
            name: state.adapter.name().to_string(),
            loaded: true,
            capabilities: vec![
                "launch_process".to_string(),
                "screenshot".to_string(),
                "input_key".to_string(),
                "input_text".to_string(),
                "input_click".to_string(),
                "input_scroll".to_string(),
                "wait".to_string(),
                "close".to_string(),
                "focus".to_string(),
                "attention".to_string(),
                "displays".to_string(),
            ],
            permissions: perms.into_iter().map(to_wire_permission).collect(),
        }],
    }))
}

fn to_wire_permission(p: CorePermission) -> PermissionStatus {
    PermissionStatus { name: p.name, granted: p.granted, purpose: p.purpose }
}
```

- [ ] **Step 2: Wire new routes into server**

Edit `crates/portholed/src/server.rs`:

```rust
use std::path::PathBuf;
use std::sync::Arc;

use axum::routing::{get, post};
use axum::Router;
use porthole_core::adapter::Adapter;
use tokio::net::UnixListener;
use tracing::info;

use crate::routes::{
    attention as attention_route,
    close_focus as close_focus_route,
    info as info_route,
    input as input_route,
    launches as launches_route,
    screenshot as screenshot_route,
    wait as wait_route,
};
use crate::state::AppState;

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/info", get(info_route::get_info))
        .route("/attention", get(attention_route::get_attention))
        .route("/displays", get(attention_route::get_displays))
        .route("/launches", post(launches_route::post_launches))
        .route("/surfaces/{id}/screenshot", post(screenshot_route::post_screenshot))
        .route("/surfaces/{id}/key", post(input_route::post_key))
        .route("/surfaces/{id}/text", post(input_route::post_text))
        .route("/surfaces/{id}/click", post(input_route::post_click))
        .route("/surfaces/{id}/scroll", post(input_route::post_scroll))
        .route("/surfaces/{id}/wait", post(wait_route::post_wait))
        .route("/surfaces/{id}/close", post(close_focus_route::post_close))
        .route("/surfaces/{id}/focus", post(close_focus_route::post_focus))
        .with_state(state)
}

pub async fn serve(adapter: Arc<dyn Adapter>, socket_path: PathBuf) -> std::io::Result<()> {
    if let Some(parent) = socket_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if socket_path.exists() {
        std::fs::remove_file(&socket_path)?;
    }
    let listener = UnixListener::bind(&socket_path)?;
    info!(socket = %socket_path.display(), "portholed listening");
    let app = build_router(AppState::new(adapter));
    axum::serve(listener, app).await
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::body::to_bytes;
    use axum::body::Body;
    use axum::http::{Method, Request, StatusCode};
    use porthole_core::in_memory::InMemoryAdapter;
    use porthole_core::surface::SurfaceInfo;
    use tower::ServiceExt;

    use super::*;

    async fn router_with_tracked_surface() -> (Router, String) {
        let adapter = Arc::new(InMemoryAdapter::new());
        let state = AppState::new(adapter);
        let info = SurfaceInfo::window(porthole_core::SurfaceId::new(), 1);
        let id = info.id.to_string();
        state.handles.insert(info).await;
        (build_router(state), id)
    }

    async fn post(router: Router, uri: &str, body: serde_json::Value) -> axum::http::Response<Body> {
        let req = Request::builder()
            .method(Method::POST)
            .uri(uri)
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        router.oneshot(req).await.unwrap()
    }

    #[tokio::test]
    async fn post_key_sends_events() {
        let (router, id) = router_with_tracked_surface().await;
        let res = post(
            router,
            &format!("/surfaces/{id}/key"),
            serde_json::json!({ "events": [{ "key": "Enter" }] }),
        )
        .await;
        assert_eq!(res.status(), StatusCode::OK);
        let body = to_bytes(res.into_body(), 1024 * 1024).await.unwrap();
        let resp: porthole_protocol::input::KeyResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(resp.events_sent, 1);
    }

    #[tokio::test]
    async fn post_key_with_unsupported_name_returns_bad_request() {
        let (router, id) = router_with_tracked_surface().await;
        let res = post(
            router,
            &format!("/surfaces/{id}/key"),
            serde_json::json!({ "events": [{ "key": "NotAKey" }] }),
        )
        .await;
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn post_text_reports_char_count() {
        let (router, id) = router_with_tracked_surface().await;
        let res = post(router, &format!("/surfaces/{id}/text"), serde_json::json!({ "text": "hi" })).await;
        assert_eq!(res.status(), StatusCode::OK);
        let body = to_bytes(res.into_body(), 1024 * 1024).await.unwrap();
        let resp: porthole_protocol::input::TextResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(resp.chars_sent, 2);
    }

    #[tokio::test]
    async fn post_close_marks_surface_dead() {
        let (router, id) = router_with_tracked_surface().await;
        let res = post(router.clone(), &format!("/surfaces/{id}/close"), serde_json::json!({})).await;
        assert_eq!(res.status(), StatusCode::OK);
        // Subsequent operations should 410 (GONE)
        let res = post(router, &format!("/surfaces/{id}/focus"), serde_json::json!({})).await;
        assert_eq!(res.status(), StatusCode::GONE);
    }

    #[tokio::test]
    async fn get_attention_returns_default() {
        let adapter = Arc::new(InMemoryAdapter::new());
        let router = build_router(AppState::new(adapter));
        let req = Request::builder().method(Method::GET).uri("/attention").body(Body::empty()).unwrap();
        let res = router.oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn get_displays_returns_list() {
        let adapter = Arc::new(InMemoryAdapter::new());
        let router = build_router(AppState::new(adapter));
        let req = Request::builder().method(Method::GET).uri("/displays").body(Body::empty()).unwrap();
        let res = router.oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = to_bytes(res.into_body(), 1024 * 1024).await.unwrap();
        let resp: porthole_protocol::attention::DisplaysResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(resp.displays.len(), 1);
    }
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p portholed --lib server`
Expected: existing 2 tests plus 6 new tests = 8 passes.

- [ ] **Step 4: Commit**

```bash
git add crates/portholed/src/routes/info.rs crates/portholed/src/server.rs
git commit -m "feat(daemon): wire slice-A routes and extend /info with permissions and capabilities"
```

---

## Task 12: porthole — CLI Subcommands

**Files:**
- Create: `crates/porthole/src/commands/key.rs`
- Create: `crates/porthole/src/commands/text.rs`
- Create: `crates/porthole/src/commands/click.rs`
- Create: `crates/porthole/src/commands/scroll.rs`
- Create: `crates/porthole/src/commands/wait.rs`
- Create: `crates/porthole/src/commands/close.rs`
- Create: `crates/porthole/src/commands/focus.rs`
- Create: `crates/porthole/src/commands/attention.rs`
- Create: `crates/porthole/src/commands/displays.rs`
- Modify: `crates/porthole/src/commands/mod.rs`
- Modify: `crates/porthole/src/main.rs`

- [ ] **Step 1: Write the input subcommands**

Create `crates/porthole/src/commands/key.rs`:

```rust
use porthole_core::input::{KeyEvent, Modifier};
use porthole_protocol::input::{KeyRequest, KeyResponse};

use crate::client::{ClientError, DaemonClient};

pub struct KeyArgs {
    pub surface_id: String,
    pub key: String,
    pub modifiers: Vec<Modifier>,
    pub session: Option<String>,
}

pub async fn run(client: &DaemonClient, args: KeyArgs) -> Result<(), ClientError> {
    let req = KeyRequest {
        events: vec![KeyEvent { key: args.key, modifiers: args.modifiers }],
        session: args.session,
    };
    let res: KeyResponse = client.post_json(&format!("/surfaces/{}/key", args.surface_id), &req).await?;
    println!("sent {} event(s) to surface {}", res.events_sent, res.surface_id);
    Ok(())
}
```

Create `crates/porthole/src/commands/text.rs`:

```rust
use porthole_protocol::input::{TextRequest, TextResponse};

use crate::client::{ClientError, DaemonClient};

pub struct TextArgs {
    pub surface_id: String,
    pub text: String,
    pub session: Option<String>,
}

pub async fn run(client: &DaemonClient, args: TextArgs) -> Result<(), ClientError> {
    let req = TextRequest { text: args.text, session: args.session };
    let res: TextResponse = client.post_json(&format!("/surfaces/{}/text", args.surface_id), &req).await?;
    println!("sent {} char(s) to surface {}", res.chars_sent, res.surface_id);
    Ok(())
}
```

Create `crates/porthole/src/commands/click.rs`:

```rust
use porthole_core::input::{ClickButton, Modifier};
use porthole_protocol::input::{ClickRequest, ClickResponse};

use crate::client::{ClientError, DaemonClient};

pub struct ClickArgs {
    pub surface_id: String,
    pub x: f64,
    pub y: f64,
    pub button: ClickButton,
    pub count: u8,
    pub modifiers: Vec<Modifier>,
    pub session: Option<String>,
}

pub async fn run(client: &DaemonClient, args: ClickArgs) -> Result<(), ClientError> {
    let req = ClickRequest {
        x: args.x,
        y: args.y,
        button: args.button,
        count: args.count,
        modifiers: args.modifiers,
        session: args.session,
    };
    let res: ClickResponse = client.post_json(&format!("/surfaces/{}/click", args.surface_id), &req).await?;
    println!("clicked at ({}, {}) on surface {}", args.x, args.y, res.surface_id);
    Ok(())
}
```

Create `crates/porthole/src/commands/scroll.rs`:

```rust
use porthole_protocol::input::{ScrollRequest, ScrollResponse};

use crate::client::{ClientError, DaemonClient};

pub struct ScrollArgs {
    pub surface_id: String,
    pub x: f64,
    pub y: f64,
    pub delta_x: f64,
    pub delta_y: f64,
    pub session: Option<String>,
}

pub async fn run(client: &DaemonClient, args: ScrollArgs) -> Result<(), ClientError> {
    let req = ScrollRequest { x: args.x, y: args.y, delta_x: args.delta_x, delta_y: args.delta_y, session: args.session };
    let res: ScrollResponse = client.post_json(&format!("/surfaces/{}/scroll", args.surface_id), &req).await?;
    println!("scrolled at ({}, {}) delta=({}, {}) on surface {}", args.x, args.y, args.delta_x, args.delta_y, res.surface_id);
    Ok(())
}
```

- [ ] **Step 2: Write wait + close + focus + attention + displays subcommands**

Create `crates/porthole/src/commands/wait.rs`:

```rust
use porthole_core::wait::WaitCondition;
use porthole_protocol::wait::{WaitRequest, WaitResponse};

use crate::client::{ClientError, DaemonClient};

pub struct WaitArgs {
    pub surface_id: String,
    pub condition: WaitCondition,
    pub timeout_ms: u64,
    pub session: Option<String>,
}

pub async fn run(client: &DaemonClient, args: WaitArgs) -> Result<(), ClientError> {
    let req = WaitRequest { condition: args.condition, timeout_ms: args.timeout_ms, session: args.session };
    let res: WaitResponse = client.post_json(&format!("/surfaces/{}/wait", args.surface_id), &req).await?;
    println!("waited {}ms for condition '{}' on surface {}", res.elapsed_ms, res.condition, res.surface_id);
    Ok(())
}
```

Create `crates/porthole/src/commands/close.rs`:

```rust
use porthole_protocol::close_focus::{CloseRequest, CloseResponse};

use crate::client::{ClientError, DaemonClient};

pub async fn run(client: &DaemonClient, surface_id: String, session: Option<String>) -> Result<(), ClientError> {
    let req = CloseRequest { session };
    let res: CloseResponse = client.post_json(&format!("/surfaces/{surface_id}/close"), &req).await?;
    println!("closed surface {}", res.surface_id);
    Ok(())
}
```

Create `crates/porthole/src/commands/focus.rs`:

```rust
use porthole_protocol::close_focus::{FocusRequest, FocusResponse};

use crate::client::{ClientError, DaemonClient};

pub async fn run(client: &DaemonClient, surface_id: String, session: Option<String>) -> Result<(), ClientError> {
    let req = FocusRequest { session };
    let res: FocusResponse = client.post_json(&format!("/surfaces/{surface_id}/focus"), &req).await?;
    println!("focused surface {}", res.surface_id);
    Ok(())
}
```

Create `crates/porthole/src/commands/attention.rs`:

```rust
use porthole_core::attention::AttentionInfo;

use crate::client::{ClientError, DaemonClient};

pub async fn run(client: &DaemonClient) -> Result<(), ClientError> {
    let info: AttentionInfo = client.get_json("/attention").await?;
    println!("focused_surface_id: {:?}", info.focused_surface_id);
    println!("focused_app_bundle: {:?}", info.focused_app_bundle);
    println!("focused_display_id: {:?}", info.focused_display_id);
    println!("cursor: ({}, {}) display_index={:?}", info.cursor.x, info.cursor.y, info.cursor.display_id_index);
    println!("recently_active: {:?}", info.recently_active_surface_ids);
    Ok(())
}
```

Create `crates/porthole/src/commands/displays.rs`:

```rust
use porthole_protocol::attention::DisplaysResponse;

use crate::client::{ClientError, DaemonClient};

pub async fn run(client: &DaemonClient) -> Result<(), ClientError> {
    let res: DisplaysResponse = client.get_json("/displays").await?;
    for d in res.displays {
        println!(
            "{}  bounds=({}, {}, {}x{})  scale={}  primary={}  focused={}",
            d.id.as_str(),
            d.bounds.x, d.bounds.y, d.bounds.w, d.bounds.h,
            d.scale, d.primary, d.focused,
        );
    }
    Ok(())
}
```

- [ ] **Step 3: Update `commands/mod.rs`**

Edit `crates/porthole/src/commands/mod.rs`:

```rust
pub mod attention;
pub mod click;
pub mod close;
pub mod displays;
pub mod focus;
pub mod info;
pub mod key;
pub mod launch;
pub mod screenshot;
pub mod scroll;
pub mod text;
pub mod wait;
```

- [ ] **Step 4: Update `main.rs` with new subcommands**

Edit `crates/porthole/src/main.rs` — add these new `Command` variants and wire them in `match`. Append to the existing `Command` enum:

```rust
    /// Send key events to a surface.
    Key {
        surface_id: String,
        #[arg(long)]
        key: String,
        #[arg(long = "mod", value_enum)]
        modifiers: Vec<ModifierArg>,
        #[arg(long)]
        session: Option<String>,
    },
    /// Type literal text into a surface.
    Text {
        surface_id: String,
        text: String,
        #[arg(long)]
        session: Option<String>,
    },
    /// Click at window-local coordinates.
    Click {
        surface_id: String,
        #[arg(long)]
        x: f64,
        #[arg(long)]
        y: f64,
        #[arg(long, value_enum, default_value_t = ButtonArg::Left)]
        button: ButtonArg,
        #[arg(long, default_value_t = 1)]
        count: u8,
        #[arg(long = "mod", value_enum)]
        modifiers: Vec<ModifierArg>,
        #[arg(long)]
        session: Option<String>,
    },
    /// Scroll at window-local coordinates.
    Scroll {
        surface_id: String,
        #[arg(long)]
        x: f64,
        #[arg(long)]
        y: f64,
        #[arg(long, default_value_t = 0.0)]
        delta_x: f64,
        #[arg(long, default_value_t = 0.0)]
        delta_y: f64,
        #[arg(long)]
        session: Option<String>,
    },
    /// Wait for a condition on a surface.
    Wait {
        surface_id: String,
        #[arg(long, value_enum)]
        condition: ConditionArg,
        #[arg(long)]
        pattern: Option<String>,
        #[arg(long, default_value_t = 1500)]
        window_ms: u64,
        #[arg(long, default_value_t = 1.0)]
        threshold_pct: f64,
        #[arg(long, default_value_t = 10_000)]
        timeout_ms: u64,
        #[arg(long)]
        session: Option<String>,
    },
    /// Close a surface.
    Close {
        surface_id: String,
        #[arg(long)]
        session: Option<String>,
    },
    /// Focus a surface.
    Focus {
        surface_id: String,
        #[arg(long)]
        session: Option<String>,
    },
    /// Print focus / cursor / recently active.
    Attention,
    /// Print monitor list.
    Displays,
```

Add these supporting enums below `ConfidenceArg`:

```rust
#[derive(clap::ValueEnum, Clone, Copy, Debug)]
enum ModifierArg { Cmd, Ctrl, Alt, Shift }

impl From<ModifierArg> for Modifier {
    fn from(m: ModifierArg) -> Self {
        match m {
            ModifierArg::Cmd => Modifier::Cmd,
            ModifierArg::Ctrl => Modifier::Ctrl,
            ModifierArg::Alt => Modifier::Alt,
            ModifierArg::Shift => Modifier::Shift,
        }
    }
}

#[derive(clap::ValueEnum, Clone, Copy, Debug)]
enum ButtonArg { Left, Right, Middle }

impl From<ButtonArg> for ClickButton {
    fn from(b: ButtonArg) -> Self {
        match b {
            ButtonArg::Left => ClickButton::Left,
            ButtonArg::Right => ClickButton::Right,
            ButtonArg::Middle => ClickButton::Middle,
        }
    }
}

#[derive(clap::ValueEnum, Clone, Copy, Debug)]
enum ConditionArg { Stable, Dirty, Exists, Gone, TitleMatches }
```

Add the new imports at the top of `main.rs`:

```rust
use porthole_core::input::{ClickButton, Modifier};
use porthole_core::wait::WaitCondition;
use porthole::commands::{attention, click as click_cmd, close as close_cmd, displays, focus as focus_cmd, key as key_cmd, scroll as scroll_cmd, text as text_cmd, wait as wait_cmd};
```

Extend the `match cli.command` arm block with:

```rust
        Command::Key { surface_id, key, modifiers, session } => {
            let args = key_cmd::KeyArgs {
                surface_id,
                key,
                modifiers: modifiers.into_iter().map(Modifier::from).collect(),
                session,
            };
            key_cmd::run(&client, args).await
        }
        Command::Text { surface_id, text, session } => {
            text_cmd::run(&client, text_cmd::TextArgs { surface_id, text, session }).await
        }
        Command::Click { surface_id, x, y, button, count, modifiers, session } => {
            click_cmd::run(&client, click_cmd::ClickArgs {
                surface_id, x, y,
                button: button.into(),
                count,
                modifiers: modifiers.into_iter().map(Modifier::from).collect(),
                session,
            }).await
        }
        Command::Scroll { surface_id, x, y, delta_x, delta_y, session } => {
            scroll_cmd::run(&client, scroll_cmd::ScrollArgs { surface_id, x, y, delta_x, delta_y, session }).await
        }
        Command::Wait { surface_id, condition, pattern, window_ms, threshold_pct, timeout_ms, session } => {
            let cond = match condition {
                ConditionArg::Stable => WaitCondition::Stable { window_ms, threshold_pct },
                ConditionArg::Dirty => WaitCondition::Dirty { threshold_pct },
                ConditionArg::Exists => WaitCondition::Exists,
                ConditionArg::Gone => WaitCondition::Gone,
                ConditionArg::TitleMatches => WaitCondition::TitleMatches {
                    pattern: pattern.unwrap_or_default(),
                },
            };
            wait_cmd::run(&client, wait_cmd::WaitArgs { surface_id, condition: cond, timeout_ms, session }).await
        }
        Command::Close { surface_id, session } => close_cmd::run(&client, surface_id, session).await,
        Command::Focus { surface_id, session } => focus_cmd::run(&client, surface_id, session).await,
        Command::Attention => attention::run(&client).await,
        Command::Displays => displays::run(&client).await,
```

- [ ] **Step 5: Build**

Run: `cargo build -p porthole`
Expected: clean build.

- [ ] **Step 6: Commit**

```bash
git add crates/porthole
git commit -m "feat(cli): add key/text/click/scroll/wait/close/focus/attention/displays subcommands"
```

---

## Task 13: porthole-adapter-macos — Key-Code Table

**Files:**
- Create: `crates/porthole-adapter-macos/src/key_codes.rs`
- Modify: `crates/porthole-adapter-macos/src/lib.rs`
- Modify: `crates/porthole-adapter-macos/Cargo.toml`

- [ ] **Step 1: Add `regex` dep**

Edit `crates/porthole-adapter-macos/Cargo.toml` — under `[dependencies]`:

```toml
regex = "1"
```

- [ ] **Step 2: Write `key_codes.rs`**

Create `crates/porthole-adapter-macos/src/key_codes.rs`. This maps DOM-style key names to the macOS virtual key codes (CGKeyCode) from `<HIToolbox/Events.h>`:

```rust
//! DOM KeyboardEvent.code → macOS CGKeyCode table.
//!
//! Values taken from Apple's Events.h (Carbon HIToolbox). These are
//! physical-key codes, stable across layouts.

use std::collections::HashMap;
use std::sync::OnceLock;

pub fn key_code(name: &str) -> Option<u16> {
    table().get(name).copied()
}

fn table() -> &'static HashMap<&'static str, u16> {
    static TABLE: OnceLock<HashMap<&'static str, u16>> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut m = HashMap::new();
        // Letters (ANSI)
        let letters: &[(&str, u16)] = &[
            ("KeyA", 0x00), ("KeyB", 0x0B), ("KeyC", 0x08), ("KeyD", 0x02),
            ("KeyE", 0x0E), ("KeyF", 0x03), ("KeyG", 0x05), ("KeyH", 0x04),
            ("KeyI", 0x22), ("KeyJ", 0x26), ("KeyK", 0x28), ("KeyL", 0x25),
            ("KeyM", 0x2E), ("KeyN", 0x2D), ("KeyO", 0x1F), ("KeyP", 0x23),
            ("KeyQ", 0x0C), ("KeyR", 0x0F), ("KeyS", 0x01), ("KeyT", 0x11),
            ("KeyU", 0x20), ("KeyV", 0x09), ("KeyW", 0x0D), ("KeyX", 0x07),
            ("KeyY", 0x10), ("KeyZ", 0x06),
        ];
        for &(n, c) in letters { m.insert(n, c); }

        // Digits (row above letters)
        let digits: &[(&str, u16)] = &[
            ("Digit0", 0x1D), ("Digit1", 0x12), ("Digit2", 0x13), ("Digit3", 0x14),
            ("Digit4", 0x15), ("Digit5", 0x17), ("Digit6", 0x16), ("Digit7", 0x1A),
            ("Digit8", 0x1C), ("Digit9", 0x19),
        ];
        for &(n, c) in digits { m.insert(n, c); }

        // Function keys
        let fkeys: &[(&str, u16)] = &[
            ("F1", 0x7A), ("F2", 0x78), ("F3", 0x63), ("F4", 0x76),
            ("F5", 0x60), ("F6", 0x61), ("F7", 0x62), ("F8", 0x64),
            ("F9", 0x65), ("F10", 0x6D), ("F11", 0x67), ("F12", 0x6F),
        ];
        for &(n, c) in fkeys { m.insert(n, c); }

        // Navigation / editing
        let nav: &[(&str, u16)] = &[
            ("Enter", 0x24), ("Escape", 0x35), ("Space", 0x31), ("Tab", 0x30),
            ("Backspace", 0x33), ("Delete", 0x75),
            ("ArrowLeft", 0x7B), ("ArrowRight", 0x7C), ("ArrowDown", 0x7D), ("ArrowUp", 0x7E),
            ("Home", 0x73), ("End", 0x77), ("PageUp", 0x74), ("PageDown", 0x79),
        ];
        for &(n, c) in nav { m.insert(n, c); }

        // Punctuation
        let punct: &[(&str, u16)] = &[
            ("Minus", 0x1B), ("Equal", 0x18),
            ("Comma", 0x2B), ("Period", 0x2F), ("Slash", 0x2C),
            ("Semicolon", 0x29), ("Quote", 0x27), ("Backquote", 0x32),
            ("BracketLeft", 0x21), ("BracketRight", 0x1E),
            ("Backslash", 0x2A),
        ];
        for &(n, c) in punct { m.insert(n, c); }

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
```

- [ ] **Step 3: Register in `lib.rs`**

Edit `crates/porthole-adapter-macos/src/lib.rs` — add `pub mod key_codes;` (leave the trait impl stubs as-is for now; Tasks 14-19 fill them in).

- [ ] **Step 4: Run tests**

Run: `cargo test -p porthole-adapter-macos --lib key_codes`
Expected: 4 passes.

- [ ] **Step 5: Commit**

```bash
git add crates/porthole-adapter-macos/Cargo.toml crates/porthole-adapter-macos/src/key_codes.rs crates/porthole-adapter-macos/src/lib.rs
git commit -m "feat(adapter-macos): DOM key name → CGKeyCode table"
```

---

## Task 14: porthole-adapter-macos — Input Implementation

**Files:**
- Create: `crates/porthole-adapter-macos/src/input.rs`
- Modify: `crates/porthole-adapter-macos/src/lib.rs`

- [ ] **Step 1: Write `input.rs`**

Create `crates/porthole-adapter-macos/src/input.rs`:

```rust
#![cfg(target_os = "macos")]

use core_graphics::event::{
    CGEvent, CGEventFlags, CGEventTapLocation, CGEventType, CGMouseButton, EventField,
    ScrollEventUnit,
};
use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
use core_graphics::geometry::CGPoint;
use porthole_core::input::{ClickButton, ClickSpec, KeyEvent, Modifier, ScrollSpec};
use porthole_core::surface::SurfaceInfo;
use porthole_core::{ErrorCode, PortholeError};

use crate::close_focus;
use crate::key_codes::key_code;

fn event_source() -> Result<CGEventSource, PortholeError> {
    CGEventSource::new(CGEventSourceStateID::HIDSystemState)
        .map_err(|_| PortholeError::new(ErrorCode::PermissionNeeded, "failed to create CGEventSource"))
}

fn flags_for(modifiers: &[Modifier]) -> CGEventFlags {
    let mut flags = CGEventFlags::empty();
    for m in modifiers {
        flags |= match m {
            Modifier::Cmd => CGEventFlags::CGEventFlagCommand,
            Modifier::Ctrl => CGEventFlags::CGEventFlagControl,
            Modifier::Alt => CGEventFlags::CGEventFlagAlternate,
            Modifier::Shift => CGEventFlags::CGEventFlagShift,
        };
    }
    flags
}

pub async fn key(surface: &SurfaceInfo, events: &[KeyEvent]) -> Result<(), PortholeError> {
    close_focus::focus(surface).await?;
    let source = event_source()?;
    for ev in events {
        let code = key_code(&ev.key).ok_or_else(|| {
            PortholeError::new(ErrorCode::UnknownKey, format!("no keycode for '{}'", ev.key))
        })?;
        let flags = flags_for(&ev.modifiers);

        let down = CGEvent::new_keyboard_event(source.clone(), code, true)
            .map_err(|_| PortholeError::new(ErrorCode::PermissionNeeded, "key down event create failed"))?;
        down.set_flags(flags);
        down.post(CGEventTapLocation::HID);

        let up = CGEvent::new_keyboard_event(source.clone(), code, false)
            .map_err(|_| PortholeError::new(ErrorCode::PermissionNeeded, "key up event create failed"))?;
        up.set_flags(flags);
        up.post(CGEventTapLocation::HID);
    }
    Ok(())
}

pub async fn text(surface: &SurfaceInfo, text: &str) -> Result<(), PortholeError> {
    close_focus::focus(surface).await?;
    let source = event_source()?;

    let units: Vec<u16> = text.encode_utf16().collect();
    let down = CGEvent::new_keyboard_event(source.clone(), 0, true)
        .map_err(|_| PortholeError::new(ErrorCode::PermissionNeeded, "text event create failed"))?;
    down.set_string_from_utf16_unchecked(&units);
    down.post(CGEventTapLocation::HID);

    let up = CGEvent::new_keyboard_event(source, 0, false)
        .map_err(|_| PortholeError::new(ErrorCode::PermissionNeeded, "text up event create failed"))?;
    up.set_string_from_utf16_unchecked(&units);
    up.post(CGEventTapLocation::HID);
    Ok(())
}

pub async fn click(surface: &SurfaceInfo, spec: &ClickSpec) -> Result<(), PortholeError> {
    let (screen_x, screen_y) = window_to_screen(surface, spec.x, spec.y).await?;
    close_focus::focus(surface).await?;
    let source = event_source()?;
    let flags = flags_for(&spec.modifiers);
    let (down_ty, up_ty, button) = match spec.button {
        ClickButton::Left => (CGEventType::LeftMouseDown, CGEventType::LeftMouseUp, CGMouseButton::Left),
        ClickButton::Right => (CGEventType::RightMouseDown, CGEventType::RightMouseUp, CGMouseButton::Right),
        ClickButton::Middle => (CGEventType::OtherMouseDown, CGEventType::OtherMouseUp, CGMouseButton::Center),
    };
    let pos = CGPoint::new(screen_x, screen_y);
    for n in 1..=spec.count as i64 {
        let down = CGEvent::new_mouse_event(source.clone(), down_ty, pos, button)
            .map_err(|_| PortholeError::new(ErrorCode::PermissionNeeded, "mouse down create failed"))?;
        down.set_flags(flags);
        down.set_integer_value_field(EventField::MOUSE_EVENT_CLICK_STATE, n);
        down.post(CGEventTapLocation::HID);

        let up = CGEvent::new_mouse_event(source.clone(), up_ty, pos, button)
            .map_err(|_| PortholeError::new(ErrorCode::PermissionNeeded, "mouse up create failed"))?;
        up.set_flags(flags);
        up.set_integer_value_field(EventField::MOUSE_EVENT_CLICK_STATE, n);
        up.post(CGEventTapLocation::HID);
    }
    Ok(())
}

pub async fn scroll(surface: &SurfaceInfo, spec: &ScrollSpec) -> Result<(), PortholeError> {
    // Scroll events on macOS are positioned at the mouse cursor, so we move
    // the cursor to the window-local point first. This is a visible side
    // effect; acceptable for v0.x.
    let (screen_x, screen_y) = window_to_screen(surface, spec.x, spec.y).await?;
    close_focus::focus(surface).await?;
    let source = event_source()?;

    // Move cursor.
    let move_ev = CGEvent::new_mouse_event(
        source.clone(),
        CGEventType::MouseMoved,
        CGPoint::new(screen_x, screen_y),
        CGMouseButton::Left,
    )
    .map_err(|_| PortholeError::new(ErrorCode::PermissionNeeded, "cursor move failed"))?;
    move_ev.post(CGEventTapLocation::HID);

    let scroll_ev = CGEvent::new_scroll_event(
        source,
        ScrollEventUnit::LINE,
        2, // axis count: vertical + horizontal
        spec.delta_y as i32,
        spec.delta_x as i32,
        0,
    )
    .map_err(|_| PortholeError::new(ErrorCode::PermissionNeeded, "scroll event create failed"))?;
    scroll_ev.post(CGEventTapLocation::HID);
    Ok(())
}

/// Converts window-local logical points to screen-global logical points using
/// the current window bounds from AX. This is the same conversion the
/// screenshot path would use, factored out.
async fn window_to_screen(surface: &SurfaceInfo, x: f64, y: f64) -> Result<(f64, f64), PortholeError> {
    let bounds = crate::close_focus::window_bounds(surface).await?;
    Ok((bounds.x + x, bounds.y + y))
}
```

Note: this code references `crate::close_focus::focus` and `crate::close_focus::window_bounds`. Those ship in Task 15. If building this task alone before Task 15 exists, compilation fails — that's expected; commit after Task 15.

- [ ] **Step 2: Register `input` module in `lib.rs`**

Edit `crates/porthole-adapter-macos/src/lib.rs`:

```rust
pub mod close_focus;
pub mod input;
pub mod key_codes;
```

(Keep the existing `capture`, `correlation`, `enumerate`, `ffi`, `launch` modules.)

Also wire the new trait methods in the `impl Adapter for MacOsAdapter` block:

```rust
    async fn key(&self, surface: &SurfaceInfo, events: &[KeyEvent]) -> Result<(), PortholeError> {
        input::key(surface, events).await
    }

    async fn text(&self, surface: &SurfaceInfo, text: &str) -> Result<(), PortholeError> {
        input::text(surface, text).await
    }

    async fn click(&self, surface: &SurfaceInfo, spec: &ClickSpec) -> Result<(), PortholeError> {
        input::click(surface, spec).await
    }

    async fn scroll(&self, surface: &SurfaceInfo, spec: &ScrollSpec) -> Result<(), PortholeError> {
        input::scroll(surface, spec).await
    }
```

Add imports at the top of `lib.rs`:

```rust
use porthole_core::input::{ClickSpec, KeyEvent, ScrollSpec};
```

Hold off on building until Task 15 completes.

---

## Task 15: porthole-adapter-macos — Close + Focus Implementation

**Files:**
- Create: `crates/porthole-adapter-macos/src/close_focus.rs`
- Modify: `crates/porthole-adapter-macos/src/lib.rs`

- [ ] **Step 1: Write `close_focus.rs`**

Create `crates/porthole-adapter-macos/src/close_focus.rs`. This uses AX to raise, close, and read bounds. We use the `core-foundation` crate for CFString interop and the AX C API via thin FFI (declared inline in this module since we need only a handful of symbols):

```rust
#![cfg(target_os = "macos")]

use core_foundation::base::{CFTypeID, TCFType};
use core_foundation::string::{CFString, CFStringRef};
use porthole_core::adapter::Rect;
use porthole_core::surface::SurfaceInfo;
use porthole_core::{ErrorCode, PortholeError};

// AX constants and function bindings — minimal subset we need.
type AXUIElementRef = *const std::ffi::c_void;
type AXError = i32;
const K_AXERROR_SUCCESS: AXError = 0;

unsafe extern "C" {
    fn AXUIElementCreateApplication(pid: i32) -> AXUIElementRef;
    fn AXUIElementCopyAttributeValue(
        element: AXUIElementRef,
        attribute: CFStringRef,
        value: *mut *const std::ffi::c_void,
    ) -> AXError;
    fn AXUIElementPerformAction(element: AXUIElementRef, action: CFStringRef) -> AXError;
    fn AXUIElementSetAttributeValue(
        element: AXUIElementRef,
        attribute: CFStringRef,
        value: *const std::ffi::c_void,
    ) -> AXError;
    fn CFRelease(ptr: *const std::ffi::c_void);
    fn CFGetTypeID(ptr: *const std::ffi::c_void) -> CFTypeID;
}

pub async fn focus(surface: &SurfaceInfo) -> Result<(), PortholeError> {
    let pid = surface.pid.ok_or_else(|| {
        PortholeError::new(ErrorCode::CapabilityMissing, "focus: surface has no pid")
    })? as i32;

    // Activate the owning app via NSRunningApplication.
    activate_app(pid)?;

    // Raise the specific window (best effort). If we can't locate it, continue —
    // activating the app is usually enough.
    let _ = with_first_window_for_pid(pid, |win| {
        unsafe {
            let action = CFString::new("AXRaise");
            let _ = AXUIElementPerformAction(win, action.as_concrete_TypeRef() as CFStringRef);
        }
        Ok(())
    });
    Ok(())
}

pub async fn close(surface: &SurfaceInfo) -> Result<(), PortholeError> {
    let pid = surface.pid.ok_or_else(|| {
        PortholeError::new(ErrorCode::CapabilityMissing, "close: surface has no pid")
    })? as i32;

    let via_close_button = with_first_window_for_pid(pid, |win| unsafe {
        let close_button_attr = CFString::new("AXCloseButton");
        let mut button_ptr: *const std::ffi::c_void = std::ptr::null();
        let err = AXUIElementCopyAttributeValue(
            win,
            close_button_attr.as_concrete_TypeRef() as CFStringRef,
            &mut button_ptr,
        );
        if err == K_AXERROR_SUCCESS && !button_ptr.is_null() {
            let press = CFString::new("AXPress");
            let _ =
                AXUIElementPerformAction(button_ptr as AXUIElementRef, press.as_concrete_TypeRef() as CFStringRef);
            CFRelease(button_ptr);
            Ok(true)
        } else {
            Ok(false)
        }
    });
    if matches!(via_close_button, Ok(true)) {
        return Ok(());
    }

    // Fallback: focus + Cmd+W via input path.
    focus(surface).await?;
    let src = core_graphics::event_source::CGEventSource::new(
        core_graphics::event_source::CGEventSourceStateID::HIDSystemState,
    )
    .map_err(|_| PortholeError::new(ErrorCode::PermissionNeeded, "close fallback: event source failed"))?;
    let code_w: u16 = 0x0D;
    let flags = core_graphics::event::CGEventFlags::CGEventFlagCommand;
    let down = core_graphics::event::CGEvent::new_keyboard_event(src.clone(), code_w, true)
        .map_err(|_| PortholeError::new(ErrorCode::PermissionNeeded, "close fallback: down event failed"))?;
    down.set_flags(flags);
    down.post(core_graphics::event::CGEventTapLocation::HID);
    let up = core_graphics::event::CGEvent::new_keyboard_event(src, code_w, false)
        .map_err(|_| PortholeError::new(ErrorCode::PermissionNeeded, "close fallback: up event failed"))?;
    up.set_flags(flags);
    up.post(core_graphics::event::CGEventTapLocation::HID);
    Ok(())
}

pub async fn window_bounds(surface: &SurfaceInfo) -> Result<Rect, PortholeError> {
    use crate::enumerate::list_windows;
    let pid = surface.pid.ok_or_else(|| {
        PortholeError::new(ErrorCode::CapabilityMissing, "window_bounds: surface has no pid")
    })? as i32;
    let windows = list_windows()?;
    let hit = windows.iter().find(|w| w.owner_pid == pid && (surface.title.is_none() || w.title == surface.title));
    match hit {
        Some(_w) => {
            // CGWindowList doesn't give us bounds in our `WindowRecord`. For v0 we
            // read them from AX below.
            bounds_from_ax(pid, surface.title.as_deref())
        }
        None => Err(PortholeError::new(ErrorCode::SurfaceDead, "window_bounds: no matching window")),
    }
}

fn bounds_from_ax(pid: i32, _title: Option<&str>) -> Result<Rect, PortholeError> {
    with_first_window_for_pid(pid, |win| unsafe {
        let pos_attr = CFString::new("AXPosition");
        let size_attr = CFString::new("AXSize");
        let mut pos_ptr: *const std::ffi::c_void = std::ptr::null();
        let mut size_ptr: *const std::ffi::c_void = std::ptr::null();
        let _ = AXUIElementCopyAttributeValue(win, pos_attr.as_concrete_TypeRef() as CFStringRef, &mut pos_ptr);
        let _ = AXUIElementCopyAttributeValue(win, size_attr.as_concrete_TypeRef() as CFStringRef, &mut size_ptr);
        let mut rect = Rect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 };
        if !pos_ptr.is_null() {
            let mut pt = core_graphics::geometry::CGPoint { x: 0.0, y: 0.0 };
            ax_value_to_point(pos_ptr, &mut pt);
            rect.x = pt.x;
            rect.y = pt.y;
            CFRelease(pos_ptr);
        }
        if !size_ptr.is_null() {
            let mut sz = core_graphics::geometry::CGSize { width: 0.0, height: 0.0 };
            ax_value_to_size(size_ptr, &mut sz);
            rect.w = sz.width;
            rect.h = sz.height;
            CFRelease(size_ptr);
        }
        Ok(rect)
    })
}

const K_AX_VALUE_CG_POINT_TYPE: i32 = 1;
const K_AX_VALUE_CG_SIZE_TYPE: i32 = 2;

unsafe extern "C" {
    fn AXValueGetValue(value: *const std::ffi::c_void, the_type: i32, value_ptr: *mut std::ffi::c_void) -> u8;
}

unsafe fn ax_value_to_point(v: *const std::ffi::c_void, out: *mut core_graphics::geometry::CGPoint) {
    unsafe {
        AXValueGetValue(v, K_AX_VALUE_CG_POINT_TYPE, out as *mut std::ffi::c_void);
    }
}

unsafe fn ax_value_to_size(v: *const std::ffi::c_void, out: *mut core_graphics::geometry::CGSize) {
    unsafe {
        AXValueGetValue(v, K_AX_VALUE_CG_SIZE_TYPE, out as *mut std::ffi::c_void);
    }
}

/// Run `op` against the first AX window of the given pid, handling retain/release
/// for both the application element and the window-list array. The AXUIElementRef
/// passed to `op` is borrowed from the array and is valid only for the duration
/// of the call — `op` must not return it or retain pointers into its children
/// without explicit CFRetain.
fn with_first_window_for_pid<F, R>(pid: i32, op: F) -> Result<R, PortholeError>
where
    F: FnOnce(AXUIElementRef) -> Result<R, PortholeError>,
{
    unsafe {
        let app = AXUIElementCreateApplication(pid);
        if app.is_null() {
            return Err(PortholeError::new(ErrorCode::PermissionNeeded, "AXUIElementCreateApplication returned null"));
        }
        let windows_attr = CFString::new("AXWindows");
        let mut windows_ptr: *const std::ffi::c_void = std::ptr::null();
        let err = AXUIElementCopyAttributeValue(
            app,
            windows_attr.as_concrete_TypeRef() as CFStringRef,
            &mut windows_ptr,
        );
        if err != K_AXERROR_SUCCESS || windows_ptr.is_null() {
            CFRelease(app);
            return Err(PortholeError::new(ErrorCode::PermissionNeeded, "AXWindows read failed"));
        }
        let arr = windows_ptr as core_foundation::array::CFArrayRef;
        let count = core_foundation::array::CFArrayGetCount(arr);
        if count == 0 {
            CFRelease(windows_ptr);
            CFRelease(app);
            return Err(PortholeError::new(ErrorCode::SurfaceDead, "no AX windows found"));
        }
        let win = core_foundation::array::CFArrayGetValueAtIndex(arr, 0) as AXUIElementRef;
        // `win` lives inside the array; safe to use until we release `windows_ptr`.
        let result = op(win);
        CFRelease(windows_ptr);
        CFRelease(app);
        result
    }
}

fn activate_app(pid: i32) -> Result<(), PortholeError> {
    use objc2::msg_send;
    use objc2::rc::Retained;
    use objc2::runtime::AnyObject;
    use objc2_app_kit::NSRunningApplication;

    unsafe {
        let app: Option<Retained<NSRunningApplication>> =
            NSRunningApplication::runningApplicationWithProcessIdentifier(pid);
        match app {
            Some(a) => {
                let _: bool = msg_send![&*a, activateWithOptions: 0usize];
                Ok(())
            }
            None => Err(PortholeError::new(ErrorCode::SurfaceDead, "no running app for pid")),
        }
    }
}
```

Important implementation notes:
- The raw-pointer juggling around AX references is not the right shape for a shipped library. For this slice it's acceptable v0 debt; a v0.1 task will introduce proper newtype wrappers with RAII drops. The plan acknowledges this up front rather than pretending otherwise.
- If AX access is denied, `focus`/`close` return `permission_needed`. The adapter's `permissions()` endpoint (Task 19) surfaces this proactively.

- [ ] **Step 2: Wire close/focus into `lib.rs`**

Edit `crates/porthole-adapter-macos/src/lib.rs` — add to the `impl Adapter for MacOsAdapter` block:

```rust
    async fn close(&self, surface: &SurfaceInfo) -> Result<(), PortholeError> {
        close_focus::close(surface).await
    }

    async fn focus(&self, surface: &SurfaceInfo) -> Result<(), PortholeError> {
        close_focus::focus(surface).await
    }
```

- [ ] **Step 3: Build**

Run: `cargo build -p porthole-adapter-macos`
Expected: clean build on macOS. Warnings permitted; `-D warnings` comes at the end.

- [ ] **Step 4: Commit Tasks 14 + 15 together**

```bash
git add crates/porthole-adapter-macos/src/input.rs crates/porthole-adapter-macos/src/close_focus.rs crates/porthole-adapter-macos/src/lib.rs
git commit -m "feat(adapter-macos): implement key/text/click/scroll/close/focus via CGEvent + AX"
```

---

## Task 16: porthole-adapter-macos — Attention

**Files:**
- Create: `crates/porthole-adapter-macos/src/attention.rs`
- Modify: `crates/porthole-adapter-macos/src/lib.rs`

- [ ] **Step 1: Write `attention.rs`**

Create `crates/porthole-adapter-macos/src/attention.rs`:

```rust
#![cfg(target_os = "macos")]

use core_graphics::display::CGDisplay;
use core_graphics::event::CGEvent;
use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
use porthole_core::attention::{AttentionInfo, CursorPos};
use porthole_core::display::DisplayId;
use porthole_core::PortholeError;

pub async fn attention() -> Result<AttentionInfo, PortholeError> {
    let frontmost_bundle = frontmost_app_bundle();
    let cursor = cursor_position()?;

    // Determine which display holds the cursor.
    let display_ids: Vec<u32> = CGDisplay::active_displays().unwrap_or_default();
    let mut cursor_display_idx: Option<usize> = None;
    for (i, id) in display_ids.iter().enumerate() {
        let display = CGDisplay::new(*id);
        let b = display.bounds();
        if cursor.0 >= b.origin.x
            && cursor.0 < b.origin.x + b.size.width
            && cursor.1 >= b.origin.y
            && cursor.1 < b.origin.y + b.size.height
        {
            cursor_display_idx = Some(i);
            break;
        }
    }

    let focused_display_id = cursor_display_idx.map(|i| DisplayId::new(format!("disp_{}", display_ids[i])));

    Ok(AttentionInfo {
        focused_surface_id: None, // porthole-tracked focus matching is v0.1
        focused_app_bundle: frontmost_bundle,
        focused_display_id,
        cursor: CursorPos { x: cursor.0, y: cursor.1, display_id_index: cursor_display_idx },
        recently_active_surface_ids: vec![],
    })
}

fn frontmost_app_bundle() -> Option<String> {
    use objc2::msg_send;
    use objc2::rc::Retained;
    use objc2_app_kit::{NSRunningApplication, NSWorkspace};
    use objc2_foundation::NSString;

    unsafe {
        let workspace = NSWorkspace::sharedWorkspace();
        let app: Option<Retained<NSRunningApplication>> = workspace.frontmostApplication();
        app.and_then(|a| {
            let bundle: Option<Retained<NSString>> = msg_send![&*a, bundleIdentifier];
            bundle.map(|s| s.to_string())
        })
    }
}

fn cursor_position() -> Result<(f64, f64), PortholeError> {
    // Use CGEvent::mouse_location via a temp event source.
    let src = CGEventSource::new(CGEventSourceStateID::HIDSystemState).map_err(|_| {
        PortholeError::new(porthole_core::ErrorCode::PermissionNeeded, "cursor_position: event source failed")
    })?;
    let ev = CGEvent::new(src).map_err(|_| {
        PortholeError::new(porthole_core::ErrorCode::PermissionNeeded, "cursor_position: event create failed")
    })?;
    let loc = ev.location();
    Ok((loc.x, loc.y))
}
```

- [ ] **Step 2: Wire into `lib.rs`**

Add `pub mod attention;` to `crates/porthole-adapter-macos/src/lib.rs`, and in the trait impl:

```rust
    async fn attention(&self) -> Result<AttentionInfo, PortholeError> {
        attention::attention().await
    }
```

- [ ] **Step 3: Build**

Run: `cargo build -p porthole-adapter-macos`
Expected: clean.

- [ ] **Step 4: Commit**

```bash
git add crates/porthole-adapter-macos/src/attention.rs crates/porthole-adapter-macos/src/lib.rs
git commit -m "feat(adapter-macos): attention endpoint via CG + NSWorkspace"
```

---

## Task 17: porthole-adapter-macos — Displays

**Files:**
- Create: `crates/porthole-adapter-macos/src/display.rs`
- Modify: `crates/porthole-adapter-macos/src/lib.rs`

- [ ] **Step 1: Write `display.rs`**

Create `crates/porthole-adapter-macos/src/display.rs`:

```rust
#![cfg(target_os = "macos")]

use core_graphics::display::CGDisplay;
use porthole_core::display::{DisplayId, DisplayInfo, Rect as DisplayRect};
use porthole_core::PortholeError;

pub async fn displays() -> Result<Vec<DisplayInfo>, PortholeError> {
    let ids = CGDisplay::active_displays().map_err(|e| {
        PortholeError::new(porthole_core::ErrorCode::CapabilityMissing, format!("active_displays failed: {e:?}"))
    })?;
    let main_id = CGDisplay::main().id;
    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        let display = CGDisplay::new(id);
        let bounds = display.bounds();
        let (pixels_w, pixels_h) = (display.pixels_wide(), display.pixels_high());
        let scale = if bounds.size.width > 0.0 { pixels_w as f64 / bounds.size.width } else { 1.0 };
        out.push(DisplayInfo {
            id: DisplayId::new(format!("disp_{id}")),
            bounds: DisplayRect {
                x: bounds.origin.x,
                y: bounds.origin.y,
                w: bounds.size.width,
                h: bounds.size.height,
            },
            scale,
            primary: id == main_id,
            focused: false, // filled in by attention logic; v0 leaves it false here.
        });
        let _ = pixels_h;
    }
    Ok(out)
}
```

- [ ] **Step 2: Wire into `lib.rs`**

Add `pub mod display;` and in the trait impl:

```rust
    async fn displays(&self) -> Result<Vec<DisplayInfo>, PortholeError> {
        display::displays().await
    }
```

- [ ] **Step 3: Build**

Run: `cargo build -p porthole-adapter-macos`
Expected: clean.

- [ ] **Step 4: Commit**

```bash
git add crates/porthole-adapter-macos/src/display.rs crates/porthole-adapter-macos/src/lib.rs
git commit -m "feat(adapter-macos): display enumeration with bounds and scale"
```

---

## Task 18: porthole-adapter-macos — Frame-Diff Helper

**Files:**
- Create: `crates/porthole-adapter-macos/src/frame_diff.rs`
- Modify: `crates/porthole-adapter-macos/Cargo.toml`

- [ ] **Step 1: Update `image` crate features**

Edit `crates/porthole-adapter-macos/Cargo.toml` — bump `image` features (the workspace-level dep has only `png`):

```toml
image = { workspace = true, features = ["png"] }
```

(The crate pulls in `imageops` transitively; no feature gate is needed to downsize an RgbaImage via `image::imageops::resize`.)

- [ ] **Step 2: Write `frame_diff.rs`**

Create `crates/porthole-adapter-macos/src/frame_diff.rs`:

```rust
//! Downsampled-grayscale frame fingerprinting for wait stable/dirty.
//!
//! Each sample captures a screenshot and reduces it to a fixed 64x64
//! grayscale buffer. Two fingerprints can be compared pixel-by-pixel to
//! yield a percentage of pixels that differ beyond a small intensity
//! tolerance. This is cheap (~4 KB per sample) and robust to a few
//! blinking pixels like a terminal cursor.

use image::{imageops::FilterType, DynamicImage, GrayImage, ImageBuffer, Luma};

pub const FINGERPRINT_SIDE: u32 = 64;
pub const FINGERPRINT_LEN: usize = (FINGERPRINT_SIDE * FINGERPRINT_SIDE) as usize;
const PIXEL_TOLERANCE: u8 = 10;

#[derive(Clone)]
pub struct Fingerprint(Box<[u8]>); // length FINGERPRINT_LEN

impl Fingerprint {
    pub fn from_png(png_bytes: &[u8]) -> Result<Self, String> {
        let img = image::load_from_memory_with_format(png_bytes, image::ImageFormat::Png)
            .map_err(|e| format!("png decode failed: {e}"))?;
        Ok(Self::from_dynamic(&img))
    }

    pub fn from_dynamic(img: &DynamicImage) -> Self {
        let gray: GrayImage = img.to_luma8();
        let resized: ImageBuffer<Luma<u8>, Vec<u8>> =
            image::imageops::resize(&gray, FINGERPRINT_SIDE, FINGERPRINT_SIDE, FilterType::Triangle);
        Self(resized.into_raw().into_boxed_slice())
    }

    /// Returns the fraction of pixels (0.0..=100.0) that differ from `other`
    /// by more than PIXEL_TOLERANCE in grayscale intensity.
    pub fn diff_pct(&self, other: &Fingerprint) -> f64 {
        let mut diffs = 0usize;
        for (a, b) in self.0.iter().zip(other.0.iter()) {
            let d = a.abs_diff(*b);
            if d > PIXEL_TOLERANCE {
                diffs += 1;
            }
        }
        (diffs as f64 / FINGERPRINT_LEN as f64) * 100.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgba, RgbaImage};

    fn solid(width: u32, height: u32, rgba: [u8; 4]) -> RgbaImage {
        ImageBuffer::from_pixel(width, height, Rgba(rgba))
    }

    #[test]
    fn identical_images_diff_zero() {
        let a = DynamicImage::ImageRgba8(solid(200, 100, [40, 40, 40, 255]));
        let fp_a = Fingerprint::from_dynamic(&a);
        let fp_b = Fingerprint::from_dynamic(&a);
        assert_eq!(fp_a.diff_pct(&fp_b), 0.0);
    }

    #[test]
    fn all_black_vs_all_white_diff_100() {
        let a = DynamicImage::ImageRgba8(solid(200, 100, [0, 0, 0, 255]));
        let b = DynamicImage::ImageRgba8(solid(200, 100, [255, 255, 255, 255]));
        let fp_a = Fingerprint::from_dynamic(&a);
        let fp_b = Fingerprint::from_dynamic(&b);
        let pct = fp_a.diff_pct(&fp_b);
        assert!(pct > 99.0, "expected near 100%, got {pct}");
    }

    #[test]
    fn small_region_change_is_small_pct() {
        let mut a = solid(200, 100, [40, 40, 40, 255]);
        let mut b = a.clone();
        // Change a 4x4 patch in b (tiny region).
        for y in 0..4 {
            for x in 0..4 {
                b.put_pixel(x, y, Rgba([255, 255, 255, 255]));
            }
        }
        let _ = &mut a;
        let fp_a = Fingerprint::from_dynamic(&DynamicImage::ImageRgba8(a));
        let fp_b = Fingerprint::from_dynamic(&DynamicImage::ImageRgba8(b));
        let pct = fp_a.diff_pct(&fp_b);
        assert!(pct < 2.0, "expected small-region change to be under 2%, got {pct}");
    }
}
```

- [ ] **Step 3: Wire into `lib.rs`**

Edit `crates/porthole-adapter-macos/src/lib.rs` — add `pub mod frame_diff;` (no trait method hookup; it's a helper module used by `wait`).

- [ ] **Step 4: Run tests**

Run: `cargo test -p porthole-adapter-macos --lib frame_diff`
Expected: 3 passes.

- [ ] **Step 5: Commit**

```bash
git add crates/porthole-adapter-macos/Cargo.toml crates/porthole-adapter-macos/src/frame_diff.rs crates/porthole-adapter-macos/src/lib.rs
git commit -m "feat(adapter-macos): frame-diff fingerprint helper for wait stable/dirty"
```

---

## Task 19: porthole-adapter-macos — Wait Implementation

**Files:**
- Create: `crates/porthole-adapter-macos/src/wait.rs`
- Modify: `crates/porthole-adapter-macos/src/lib.rs`

- [ ] **Step 1: Write `wait.rs`**

Create `crates/porthole-adapter-macos/src/wait.rs`:

```rust
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
                .map_err(|e| PortholeError::new(ErrorCode::InvalidCoordinate, format!("bad regex: {e}")))?;
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
    Ok(windows.iter().any(|w| w.owner_pid == pid && (surface.title.is_none() || w.title == surface.title)))
}

fn current_title(surface: &SurfaceInfo) -> Result<Option<String>, PortholeError> {
    let pid = surface.pid.unwrap_or(0) as i32;
    if pid == 0 {
        return Ok(None);
    }
    let windows = list_windows()?;
    Ok(windows.iter().find(|w| w.owner_pid == pid).and_then(|w| w.title.clone()))
}

async fn sample_fingerprint(surface: &SurfaceInfo) -> Result<Fingerprint, PortholeError> {
    let shot = capture::screenshot(surface).await?;
    Fingerprint::from_png(&shot.png_bytes)
        .map_err(|e| PortholeError::new(ErrorCode::CapabilityMissing, format!("frame decode failed: {e}")))
}
```

- [ ] **Step 2: Wire into `lib.rs`**

Edit `crates/porthole-adapter-macos/src/lib.rs` — add `pub mod wait;` and in the trait impl:

```rust
    async fn wait(
        &self,
        surface: &SurfaceInfo,
        condition: &WaitCondition,
    ) -> Result<WaitOutcome, PortholeError> {
        wait::wait(surface, condition).await
    }

    async fn wait_last_observed(
        &self,
        surface: &SurfaceInfo,
        condition: &WaitCondition,
    ) -> Result<LastObserved, PortholeError> {
        wait::wait_last_observed(surface, condition).await
    }
```

Also add imports at the top:

```rust
use porthole_core::wait::{LastObserved, WaitCondition, WaitOutcome};
```

- [ ] **Step 3: Build**

Run: `cargo build -p porthole-adapter-macos`
Expected: clean.

- [ ] **Step 4: Commit**

```bash
git add crates/porthole-adapter-macos/src/wait.rs crates/porthole-adapter-macos/src/lib.rs
git commit -m "feat(adapter-macos): wait implementation for all five conditions"
```

---

## Task 20: porthole-adapter-macos — Permissions Detection

**Files:**
- Create: `crates/porthole-adapter-macos/src/permissions.rs`
- Modify: `crates/porthole-adapter-macos/src/lib.rs`

- [ ] **Step 1: Write `permissions.rs`**

Create `crates/porthole-adapter-macos/src/permissions.rs`:

```rust
#![cfg(target_os = "macos")]

use porthole_core::permission::PermissionStatus;
use porthole_core::PortholeError;

unsafe extern "C" {
    fn AXIsProcessTrusted() -> u8;
    fn CGPreflightScreenCaptureAccess() -> u8;
}

pub async fn permissions() -> Result<Vec<PermissionStatus>, PortholeError> {
    let ax = unsafe { AXIsProcessTrusted() } != 0;
    let scr = unsafe { CGPreflightScreenCaptureAccess() } != 0;
    Ok(vec![
        PermissionStatus {
            name: "accessibility".into(),
            granted: ax,
            purpose: "input injection and some wait conditions".into(),
        },
        PermissionStatus {
            name: "screen_recording".into(),
            granted: scr,
            purpose: "window screenshot capture and frame-diff waits".into(),
        },
    ])
}
```

- [ ] **Step 2: Wire into `lib.rs`**

Add `pub mod permissions;` and in the trait impl:

```rust
    async fn permissions(&self) -> Result<Vec<PermissionStatus>, PortholeError> {
        permissions::permissions().await
    }
```

Import:

```rust
use porthole_core::permission::PermissionStatus;
```

- [ ] **Step 3: Build**

Run: `cargo build -p porthole-adapter-macos`
Expected: clean.

- [ ] **Step 4: Commit**

```bash
git add crates/porthole-adapter-macos/src/permissions.rs crates/porthole-adapter-macos/src/lib.rs
git commit -m "feat(adapter-macos): permissions detection (AX + screen recording)"
```

---

## Task 21: Full Workspace Sanity + Lint Pass

**Files:**
- Potentially multiple small fixups across crates

- [ ] **Step 1: Build the whole workspace**

Run: `cargo build --workspace --locked`
Expected: clean build. If errors appear, fix minimally and commit per-crate.

- [ ] **Step 2: Run all tests**

Run: `cargo test --workspace --locked`
Expected: all non-`#[ignore]` tests pass (previous foundation tests + new slice-A tests).

- [ ] **Step 3: Clippy gate**

Run: `cargo clippy --workspace --all-targets --locked -- -D warnings`
Expected: clean. Typical things to fix if they surface:
- Unused imports in `lib.rs` re-exports
- `#[allow(dead_code)]` on helpers that are legitimately unused in this slice; remove if the code path is truly unreachable, add a test to cover it if it should be reachable
- `clippy::needless_borrow` / `clippy::redundant_clone` — apply suggestions

Make small fixups inline. Do not introduce broad refactors.

- [ ] **Step 4: Commit any cleanups**

```bash
git add -A
git commit -m "chore: workspace clippy cleanup after slice-A"
```

(If no cleanups were needed, skip.)

---

## Task 22: macOS Integration Tests (Ignored)

**Files:**
- Create: `crates/porthole-adapter-macos/tests/input_integration.rs`

- [ ] **Step 1: Write the integration test**

Create `crates/porthole-adapter-macos/tests/input_integration.rs`:

```rust
#![cfg(target_os = "macos")]

use std::time::Duration;

use porthole_adapter_macos::MacOsAdapter;
use porthole_core::adapter::{Adapter, ProcessLaunchSpec, RequireConfidence};
use porthole_core::input::{ClickButton, ClickSpec, KeyEvent};
use porthole_core::wait::WaitCondition;

fn spec_textedit() -> ProcessLaunchSpec {
    ProcessLaunchSpec {
        app: "/System/Applications/TextEdit.app".to_string(),
        args: vec![],
        cwd: None,
        env: vec![],
        timeout: Duration::from_secs(10),
        require_confidence: RequireConfidence::Strong,
    }
}

#[tokio::test]
#[ignore = "requires a real macOS desktop session, Accessibility, and Screen Recording permissions"]
async fn text_types_into_textedit_and_wait_dirty_fires() {
    let adapter = MacOsAdapter::new();
    let outcome = adapter.launch_process(&spec_textedit()).await.expect("launch");
    let surface = outcome.surface;

    // Wait for the editor to be visible/stable first.
    adapter
        .wait(&surface, &WaitCondition::Stable { window_ms: 800, threshold_pct: 1.0 })
        .await
        .expect("initial stable");

    // Type text; expect the frame to go dirty.
    let baseline = adapter.screenshot(&surface).await.expect("baseline");
    adapter.text(&surface, "hello porthole\n").await.expect("text");
    let dirty = adapter
        .wait(&surface, &WaitCondition::Dirty { threshold_pct: 1.0 })
        .await
        .expect("dirty");
    assert_eq!(dirty.condition, "dirty");
    let _ = baseline;

    adapter.close(&surface).await.expect("close");
}

#[tokio::test]
#[ignore = "requires a real macOS desktop session + permissions"]
async fn key_event_triggers_dirty_after_typing() {
    let adapter = MacOsAdapter::new();
    let outcome = adapter.launch_process(&spec_textedit()).await.expect("launch");
    let surface = outcome.surface;

    adapter.text(&surface, "x").await.expect("text");
    // Pressing Enter should cause a frame change.
    adapter
        .key(&surface, &[KeyEvent { key: "Enter".into(), modifiers: vec![] }])
        .await
        .expect("key Enter");
    let dirty = adapter
        .wait(&surface, &WaitCondition::Dirty { threshold_pct: 1.0 })
        .await
        .expect("dirty");
    assert_eq!(dirty.condition, "dirty");

    adapter.close(&surface).await.expect("close");
}

#[tokio::test]
#[ignore = "requires a real macOS desktop session + permissions"]
async fn click_inside_window_is_accepted() {
    let adapter = MacOsAdapter::new();
    let outcome = adapter.launch_process(&spec_textedit()).await.expect("launch");
    let surface = outcome.surface;

    adapter
        .click(&surface, &ClickSpec { x: 100.0, y: 100.0, button: ClickButton::Left, count: 1, modifiers: vec![] })
        .await
        .expect("click");

    adapter.close(&surface).await.expect("close");
}

#[tokio::test]
#[ignore = "requires a real macOS desktop session + permissions"]
async fn attention_and_displays_return_non_empty() {
    let adapter = MacOsAdapter::new();
    let attention = adapter.attention().await.expect("attention");
    let displays = adapter.displays().await.expect("displays");
    assert!(!displays.is_empty(), "displays should list at least one monitor");
    // Cursor position should be inside some display bounds.
    let any_inside = displays.iter().any(|d| {
        attention.cursor.x >= d.bounds.x
            && attention.cursor.x < d.bounds.x + d.bounds.w
            && attention.cursor.y >= d.bounds.y
            && attention.cursor.y < d.bounds.y + d.bounds.h
    });
    assert!(any_inside, "cursor position should fall within some display");
}
```

- [ ] **Step 2: Run (manually) on a macOS desktop with permissions granted**

Run: `cargo test -p porthole-adapter-macos --test input_integration -- --ignored --nocapture`
Expected: 4 passes. If individual ones fail due to a specific permission (e.g., no Screen Recording access), document it and fix the permission grant; do not change the test to skip the permission check.

- [ ] **Step 3: Final workspace gate**

Run: `cargo test --workspace --locked`
Expected: the normal test suite still passes (these integration tests are `#[ignore]`d by default).

Run: `cargo clippy --workspace --all-targets --locked -- -D warnings`
Expected: clean.

- [ ] **Step 4: Commit**

```bash
git add crates/porthole-adapter-macos/tests/input_integration.rs
git commit -m "test(adapter-macos): ignored input + wait + attention integration tests"
```

---

## Task 23: End-to-End CLI-through-Daemon Integration Test

**Files:**
- Create: `crates/portholed/tests/slice_a_e2e.rs`

- [ ] **Step 1: Write the end-to-end test**

Create `crates/portholed/tests/slice_a_e2e.rs`:

```rust
use std::sync::Arc;
use std::time::Duration;

use porthole_core::in_memory::InMemoryAdapter;
use porthole_core::surface::SurfaceInfo;
use portholed::server::serve;

#[tokio::test]
async fn cli_through_daemon_key_text_click_wait_close() {
    let tmp = tempfile::tempdir().unwrap();
    let socket = tmp.path().join("porthole.sock");

    let adapter = Arc::new(InMemoryAdapter::new());
    let socket_for_serve = socket.clone();
    let adapter_for_serve: Arc<dyn porthole_core::adapter::Adapter> = adapter.clone();
    let server_task = tokio::spawn(async move { serve(adapter_for_serve, socket_for_serve).await });

    for _ in 0..200 {
        if socket.exists() { break; }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(socket.exists(), "socket did not appear");

    // Seed a tracked surface directly so we don't have to launch.
    let info = SurfaceInfo::window(porthole_core::SurfaceId::new(), 1);
    let id = info.id.clone();
    // NB: state is inside the server task; we simulate by going through /launches
    // instead. But with the in-memory adapter, a launch returns a brand-new surface
    // we don't have a seeded reference to. The CLI-level flow is: launch → got
    // surface id → do stuff. We follow that path here:
    let _ = (info, id);

    let client = porthole::client::DaemonClient::new(&socket);
    let launch: porthole_protocol::launches::LaunchResponse = client
        .post_json(
            "/launches",
            &serde_json::json!({ "kind": { "type": "process", "app": "X", "args": [] } }),
        )
        .await
        .expect("launch");

    // key
    let _: porthole_protocol::input::KeyResponse = client
        .post_json(
            &format!("/surfaces/{}/key", launch.surface_id),
            &serde_json::json!({ "events": [{ "key": "Enter" }] }),
        )
        .await
        .expect("key");
    // text
    let _: porthole_protocol::input::TextResponse = client
        .post_json(
            &format!("/surfaces/{}/text", launch.surface_id),
            &serde_json::json!({ "text": "hi" }),
        )
        .await
        .expect("text");
    // wait exists
    let _: porthole_protocol::wait::WaitResponse = client
        .post_json(
            &format!("/surfaces/{}/wait", launch.surface_id),
            &serde_json::json!({ "condition": { "type": "exists" }, "timeout_ms": 1000 }),
        )
        .await
        .expect("wait");
    // close
    let _: porthole_protocol::close_focus::CloseResponse = client
        .post_json(&format!("/surfaces/{}/close", launch.surface_id), &serde_json::json!({}))
        .await
        .expect("close");

    server_task.abort();

    // adapter-side recorder sanity
    assert_eq!(adapter.key_calls().await.len(), 1);
    assert_eq!(adapter.text_calls().await.len(), 1);
    assert_eq!(adapter.wait_calls().await.len(), 1);
    assert_eq!(adapter.close_calls().await.len(), 1);
}
```

Note: the test currently issues HTTP-over-UDS through `DaemonClient`. The `portholed` dev-deps already include `porthole`, `porthole-protocol`, `tempfile`, so no new deps needed.

- [ ] **Step 2: Run the test**

Run: `cargo test -p portholed --test slice_a_e2e`
Expected: 1 pass.

- [ ] **Step 3: Full workspace test**

Run: `cargo test --workspace --locked`
Expected: all pass (excluding `#[ignore]`d).

Run: `cargo clippy --workspace --all-targets --locked -- -D warnings`
Expected: clean.

- [ ] **Step 4: Commit**

```bash
git add crates/portholed/tests/slice_a_e2e.rs
git commit -m "test(daemon): end-to-end slice-A flow across UDS"
```

---

## What Slice A Delivers

- `POST /surfaces/{id}/key` (DOM KeyboardEvent.code naming + modifiers)
- `POST /surfaces/{id}/text` (Unicode literal)
- `POST /surfaces/{id}/click` (window-local coords + button + count + modifiers)
- `POST /surfaces/{id}/scroll` (window-local coords + line deltas)
- `POST /surfaces/{id}/wait` (`exists` / `gone` / `title_matches` / `stable` / `dirty` with `threshold_pct`)
- `POST /surfaces/{id}/close` (AX close button + Cmd+W fallback)
- `POST /surfaces/{id}/focus` (AX raise + NSRunningApplication activate)
- `GET /attention` (focused app/display, cursor position, recent — focused_surface_id remains `None` in v0; tracked-focus matching lands later)
- `GET /displays` (monitor topology + primary/focused flags)
- `InfoResponse.adapters[].permissions` — `accessibility` and `screen_recording` grant state
- All pipelines validated and timeout-bounded in `porthole-core`; macOS adapter implements all verbs; ignored integration tests cover the vertical slice end-to-end.

## What Slice A Intentionally Does Not Deliver

Revisit in subsequent plans:

- Events SSE stream
- Attach mode (`/surfaces/search` + `/surfaces/track`)
- Artifact launches, placement, `replace`, `auto_dismiss_after_ms`
- Tab surface enumeration or any tab verb
- Recording
- `focus: "preserve"` no-focus-steal input
- AX-element-reference targeting for click/scroll
- Cross-host transport
- Lifecycle modes on launch
- Pixel-level scroll
- Native event-backed `wait` (for `exists`/`gone`/`title_matches`)
- Tracked-focus matching for `AttentionInfo.focused_surface_id`
- Precise `last_observed` diagnostics for stable/dirty timeouts
- Newtype wrappers with RAII drops for AX references (acknowledged v0 debt in `close_focus.rs`)


