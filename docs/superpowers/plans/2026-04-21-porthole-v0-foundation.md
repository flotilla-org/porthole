# Porthole v0 Foundation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship the structural foundation and first working end-to-end slice of porthole v0 — a Cargo workspace with a core library, a wire-protocol crate, an HTTP-over-UDS daemon (`portholed`), a CLI (`porthole`), an in-memory adapter for tests, and a macOS adapter that implements process launch with tag correlation and window screenshot.

**Architecture:** Library-first Rust workspace mirroring flotilla and cleat. Core logic (adapter trait, handle store, launch state machine, error types) lives in `porthole-core` with no HTTP or OS dependencies. Wire types live in `porthole-protocol`. `portholed` is a thin axum-on-UDS adapter over core. `porthole` is a thin hyper-on-UDS client with clap for CLI parsing. `porthole-adapter-macos` talks to the OS via `objc2`, `objc2-app-kit`, and the `core-graphics` crate. The end of this plan is a passing integration test that launches a subprocess, tag-correlates its window, and screenshots it.

**Tech Stack:** Rust 2024 edition, tokio, axum 0.8, hyper 1, hyperlocal 0.9, hyper-util, serde, serde_json, thiserror, tracing, tracing-subscriber, uuid, clap 4, objc2 + objc2-app-kit + objc2-foundation, core-graphics, tempfile (tests), async-trait, bytes, image (PNG encode).

---

## Out of Scope for This Plan

The following are explicitly deferred to subsequent plans:

- Input verbs (`key`, `text`, `click`, `scroll`) and focus-preserve semantics
- `wait` verb with its conditions
- Attach mode (`/surfaces/search` + `/surfaces/track`)
- `close`, `focus`, `replace` verbs
- Artifact launch kind, placement, `auto_dismiss_after_ms`
- Events SSE stream, `/attention`, `/displays` read model
- Recording
- Tab surface enumeration and tab verbs
- Lifecycle modes beyond `exit_on_command_end`
- OpenAPI generation

What *is* in scope: the skeleton these features slot into, plus one working end-to-end vertical slice (launch + screenshot).

---

## File Structure

Files created by this plan:

```
Cargo.toml                                          # workspace manifest
rustfmt.toml                                        # formatting config (matches flotilla)
crates/
  porthole-core/
    Cargo.toml
    src/
      lib.rs                                        # re-exports
      error.rs                                      # PortholeError, ErrorCode
      surface.rs                                    # SurfaceId, SurfaceKind, SurfaceState, SurfaceInfo
      handle.rs                                     # HandleStore
      adapter.rs                                    # Adapter trait + LaunchSpec + LaunchOutcome
      launch.rs                                     # launch pipeline: spec → adapter → handle
      in_memory.rs                                  # test adapter used by everything
  porthole-protocol/
    Cargo.toml
    src/
      lib.rs                                        # re-exports
      info.rs                                       # InfoResponse
      launches.rs                                   # LaunchRequest, LaunchResponse, Confidence, Correlation
      screenshot.rs                                 # ScreenshotRequest, ScreenshotResponse, Geometry
      error.rs                                      # WireError
  portholed/
    Cargo.toml
    src/
      main.rs                                       # arg parsing, logging init, run()
      runtime.rs                                    # socket path discovery, XDG runtime dir
      state.rs                                      # AppState (adapter + handle store)
      server.rs                                     # axum Router wiring, UDS listener
      routes/
        mod.rs
        info.rs
        launches.rs
        screenshot.rs
        errors.rs                                   # WireError → axum response
  porthole/
    Cargo.toml
    src/
      main.rs                                       # clap parser, subcommand dispatch
      client.rs                                     # hyper-over-UDS HTTP client
      runtime.rs                                    # shared socket path discovery + daemon autospawn
      commands/
        mod.rs
        info.rs
        launch.rs
        screenshot.rs
  porthole-adapter-macos/
    Cargo.toml
    src/
      lib.rs                                        # MacOsAdapter impl
      launch.rs                                     # NSWorkspace launch with tag env injection
      correlation.rs                                # tag-based correlation via ps + AX
      enumerate.rs                                  # window enumeration via CGWindowList
      capture.rs                                    # CGWindowListCreateImage → PNG
      ffi.rs                                        # tiny FFI shims we need that aren't in the crates
tests/
  integration/
    cli_info.rs                                     # CLI info end-to-end
    macos_launch_capture.rs                         # macOS launch + screenshot (cfg-gated)
```

Rationale:

- Core has zero HTTP or OS dependencies — testable in any environment.
- Protocol is serde types only, used by both daemon and CLI.
- Daemon and CLI are both thin; each has one file per subcommand/route to keep files focused.
- macOS adapter is split by responsibility (launch / correlation / enumerate / capture / ffi) so each file fits in a reviewer's head.

---

## Task 1: Initialize Cargo Workspace

**Files:**
- Create: `Cargo.toml`
- Create: `rustfmt.toml`
- Create: `.gitignore`

- [ ] **Step 1: Write `Cargo.toml` workspace manifest**

```toml
[workspace]
resolver = "3"
members = [
    "crates/porthole-core",
    "crates/porthole-protocol",
    "crates/portholed",
    "crates/porthole",
    "crates/porthole-adapter-macos",
]

[workspace.package]
edition = "2024"
rust-version = "1.85"
license = "MIT OR Apache-2.0"
repository = "https://github.com/flotilla-org/porthole"

[workspace.dependencies]
async-trait = "0.1"
axum = { version = "0.8", default-features = false, features = ["http1", "json", "tokio", "matched-path"] }
bytes = "1"
clap = { version = "4", features = ["derive"] }
hyper = { version = "1", features = ["http1", "client", "server"] }
hyper-util = { version = "0.1", features = ["client-legacy", "http1", "tokio"] }
hyperlocal = "0.9"
image = { version = "0.25", default-features = false, features = ["png"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
thiserror = "2"
tokio = { version = "1", features = ["full"] }
tower = "0.5"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
uuid = { version = "1", features = ["v4", "serde"] }

[workspace.lints.rust]
unsafe_op_in_unsafe_fn = "warn"

[workspace.lints.clippy]
all = { level = "warn", priority = -1 }
```

- [ ] **Step 2: Write `rustfmt.toml` matching flotilla conventions**

```
max_width = 140
imports_granularity = "Crate"
group_imports = "StdExternalCrate"
```

- [ ] **Step 3: Write `.gitignore`**

```
/target
**/*.rs.bk
Cargo.lock.bak
.DS_Store
```

- [ ] **Step 4: Verify it parses**

Run: `cargo metadata --no-deps --format-version 1 | jq -r '.workspace_members | length'`
Expected: `0` (no crates exist yet — workspace parses but is empty; we'll add crates next).

Note: if `cargo metadata` errors because a member directory doesn't exist, that's expected until the next tasks add the crates. Use `cargo tree --workspace 2>&1 | head -5` as a syntax sanity check; it should report "no such directory" errors rather than TOML parse errors.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml rustfmt.toml .gitignore
git commit -m "chore: initialize porthole cargo workspace"
```

---

## Task 2: porthole-core — Error Types

**Files:**
- Create: `crates/porthole-core/Cargo.toml`
- Create: `crates/porthole-core/src/lib.rs`
- Create: `crates/porthole-core/src/error.rs`

- [ ] **Step 1: Write `crates/porthole-core/Cargo.toml`**

```toml
[package]
name = "porthole-core"
version = "0.0.0"
edition.workspace = true
rust-version.workspace = true
license.workspace = true
repository.workspace = true

[dependencies]
async-trait = { workspace = true }
serde = { workspace = true }
thiserror = { workspace = true }
tracing = { workspace = true }
uuid = { workspace = true }

[dev-dependencies]
tokio = { workspace = true }

[lints]
workspace = true
```

- [ ] **Step 2: Write `crates/porthole-core/src/lib.rs`**

```rust
#![forbid(unsafe_code)]

pub mod adapter;
pub mod error;
pub mod handle;
pub mod in_memory;
pub mod launch;
pub mod surface;

pub use error::{ErrorCode, PortholeError};
pub use surface::{SurfaceId, SurfaceInfo, SurfaceKind, SurfaceState};
```

- [ ] **Step 3: Write the failing test**

Create `crates/porthole-core/src/error.rs` with:

```rust
use std::fmt;

use serde::{Deserialize, Serialize};

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
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::SurfaceNotFound => "surface_not_found",
            Self::SurfaceDead => "surface_dead",
            Self::PermissionNeeded => "permission_needed",
            Self::LaunchCorrelationFailed => "launch_correlation_failed",
            Self::LaunchCorrelationAmbiguous => "launch_correlation_ambiguous",
            Self::LaunchTimeout => "launch_timeout",
            Self::CandidateRefUnknown => "candidate_ref_unknown",
            Self::AdapterUnsupported => "adapter_unsupported",
            Self::CapabilityMissing => "capability_missing",
        };
        f.write_str(s)
    }
}

#[derive(Debug, thiserror::Error)]
#[error("{code}: {message}")]
pub struct PortholeError {
    pub code: ErrorCode,
    pub message: String,
}

impl PortholeError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self { code, message: message.into() }
    }

    pub fn surface_not_found(id: &str) -> Self {
        Self::new(ErrorCode::SurfaceNotFound, format!("no tracked surface with id {id}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_code_display_matches_wire_string() {
        assert_eq!(ErrorCode::SurfaceNotFound.to_string(), "surface_not_found");
        assert_eq!(ErrorCode::LaunchCorrelationAmbiguous.to_string(), "launch_correlation_ambiguous");
    }

    #[test]
    fn surface_not_found_helper_sets_code() {
        let err = PortholeError::surface_not_found("abc");
        assert_eq!(err.code, ErrorCode::SurfaceNotFound);
        assert!(err.message.contains("abc"));
    }
}
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -p porthole-core --lib`
Expected: two passes for `error::tests::*`.

- [ ] **Step 5: Commit**

```bash
git add crates/porthole-core/Cargo.toml crates/porthole-core/src/lib.rs crates/porthole-core/src/error.rs
git commit -m "feat(core): add ErrorCode and PortholeError"
```

---

## Task 3: porthole-core — Surface Types

**Files:**
- Create: `crates/porthole-core/src/surface.rs`

- [ ] **Step 1: Write the failing test**

Create `crates/porthole-core/src/surface.rs`:

```rust
use std::fmt;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SurfaceId(String);

impl SurfaceId {
    pub fn new() -> Self {
        Self(format!("surf_{}", Uuid::new_v4().simple()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for SurfaceId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for SurfaceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SurfaceKind {
    Window,
    Tab,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SurfaceState {
    Alive,
    Dead,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SurfaceInfo {
    pub id: SurfaceId,
    pub kind: SurfaceKind,
    pub state: SurfaceState,
    pub title: Option<String>,
    pub app_bundle: Option<String>,
    pub pid: Option<u32>,
    pub parent_surface_id: Option<SurfaceId>,
}

impl SurfaceInfo {
    pub fn window(id: SurfaceId, pid: u32) -> Self {
        Self {
            id,
            kind: SurfaceKind::Window,
            state: SurfaceState::Alive,
            title: None,
            app_bundle: None,
            pid: Some(pid),
            parent_surface_id: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_surface_id_is_unique() {
        let a = SurfaceId::new();
        let b = SurfaceId::new();
        assert_ne!(a, b);
        assert!(a.as_str().starts_with("surf_"));
    }

    #[test]
    fn window_helper_sets_defaults() {
        let info = SurfaceInfo::window(SurfaceId::new(), 1234);
        assert_eq!(info.kind, SurfaceKind::Window);
        assert_eq!(info.state, SurfaceState::Alive);
        assert_eq!(info.pid, Some(1234));
        assert!(info.parent_surface_id.is_none());
    }

    #[test]
    fn surface_kind_roundtrips_as_snake_case() {
        let s = serde_json::to_string(&SurfaceKind::Window).unwrap();
        assert_eq!(s, "\"window\"");
        let k: SurfaceKind = serde_json::from_str("\"tab\"").unwrap();
        assert_eq!(k, SurfaceKind::Tab);
    }
}
```

- [ ] **Step 2: Add `serde_json` as a dev-dependency**

Modify `crates/porthole-core/Cargo.toml` — append to `[dev-dependencies]`:

```toml
serde_json = { workspace = true }
```

- [ ] **Step 3: Run the tests**

Run: `cargo test -p porthole-core --lib surface`
Expected: three passes.

- [ ] **Step 4: Commit**

```bash
git add crates/porthole-core/Cargo.toml crates/porthole-core/src/surface.rs
git commit -m "feat(core): add SurfaceId, SurfaceKind, SurfaceInfo"
```

---

## Task 4: porthole-core — Handle Store

**Files:**
- Create: `crates/porthole-core/src/handle.rs`

The handle store owns the daemon's view of tracked surfaces. Concurrent access via `tokio::sync::RwLock` because many HTTP handlers will read, relatively few will mutate.

- [ ] **Step 1: Write the failing test**

Create `crates/porthole-core/src/handle.rs`:

```rust
use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::RwLock;

use crate::surface::{SurfaceId, SurfaceInfo, SurfaceState};
use crate::{ErrorCode, PortholeError};

#[derive(Default, Clone)]
pub struct HandleStore {
    inner: Arc<RwLock<HashMap<SurfaceId, SurfaceInfo>>>,
}

impl HandleStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn insert(&self, info: SurfaceInfo) {
        let mut guard = self.inner.write().await;
        guard.insert(info.id.clone(), info);
    }

    pub async fn get(&self, id: &SurfaceId) -> Result<SurfaceInfo, PortholeError> {
        let guard = self.inner.read().await;
        guard.get(id).cloned().ok_or_else(|| PortholeError::surface_not_found(id.as_str()))
    }

    pub async fn mark_dead(&self, id: &SurfaceId) -> Result<(), PortholeError> {
        let mut guard = self.inner.write().await;
        match guard.get_mut(id) {
            Some(info) => {
                info.state = SurfaceState::Dead;
                Ok(())
            }
            None => Err(PortholeError::surface_not_found(id.as_str())),
        }
    }

    pub async fn require_alive(&self, id: &SurfaceId) -> Result<SurfaceInfo, PortholeError> {
        let info = self.get(id).await?;
        if info.state == SurfaceState::Dead {
            return Err(PortholeError::new(ErrorCode::SurfaceDead, format!("surface {id} is dead")));
        }
        Ok(info)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn insert_then_get_roundtrips() {
        let store = HandleStore::new();
        let info = SurfaceInfo::window(SurfaceId::new(), 42);
        let id = info.id.clone();
        store.insert(info).await;
        let fetched = store.get(&id).await.unwrap();
        assert_eq!(fetched.pid, Some(42));
    }

    #[tokio::test]
    async fn get_missing_returns_surface_not_found() {
        let store = HandleStore::new();
        let err = store.get(&SurfaceId::new()).await.unwrap_err();
        assert_eq!(err.code, ErrorCode::SurfaceNotFound);
    }

    #[tokio::test]
    async fn require_alive_fails_on_dead_surface() {
        let store = HandleStore::new();
        let info = SurfaceInfo::window(SurfaceId::new(), 1);
        let id = info.id.clone();
        store.insert(info).await;
        store.mark_dead(&id).await.unwrap();
        let err = store.require_alive(&id).await.unwrap_err();
        assert_eq!(err.code, ErrorCode::SurfaceDead);
    }
}
```

- [ ] **Step 2: Run the tests**

Run: `cargo test -p porthole-core --lib handle`
Expected: three passes.

- [ ] **Step 3: Commit**

```bash
git add crates/porthole-core/src/handle.rs
git commit -m "feat(core): add HandleStore for tracked surface lifecycle"
```

---

## Task 5: porthole-core — Adapter Trait + LaunchSpec

**Files:**
- Create: `crates/porthole-core/src/adapter.rs`

The adapter trait is what platform backends implement. Deliberately small in this plan; more methods land in later plans (input, capture verbs beyond screenshot, attach helpers). Capture is parameterised on a byte vector so the in-memory adapter can return stubs and the macOS adapter can return a real PNG.

- [ ] **Step 1: Write the failing test**

Create `crates/porthole-core/src/adapter.rs`:

```rust
use std::time::Duration;

use async_trait::async_trait;

use crate::surface::SurfaceInfo;
use crate::PortholeError;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequireConfidence {
    Strong,
    Plausible,
    Weak,
}

impl Default for RequireConfidence {
    fn default() -> Self {
        Self::Strong
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Confidence {
    Strong,
    Plausible,
    Weak,
}

impl Confidence {
    pub fn meets(self, required: RequireConfidence) -> bool {
        match (self, required) {
            (Confidence::Strong, _) => true,
            (Confidence::Plausible, RequireConfidence::Plausible | RequireConfidence::Weak) => true,
            (Confidence::Weak, RequireConfidence::Weak) => true,
            _ => false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Correlation {
    Tag,
    PidTree,
    Temporal,
    DocumentMatch,
    FrontmostChanged,
}

#[derive(Clone, Debug)]
pub struct ProcessLaunchSpec {
    pub app: String,
    pub args: Vec<String>,
    pub cwd: Option<String>,
    pub env: Vec<(String, String)>,
    pub timeout: Duration,
    pub require_confidence: RequireConfidence,
}

#[derive(Clone, Debug)]
pub struct LaunchOutcome {
    pub surface: SurfaceInfo,
    pub confidence: Confidence,
    pub correlation: Correlation,
    pub surface_was_preexisting: bool,
}

#[derive(Clone, Debug)]
pub struct Screenshot {
    pub png_bytes: Vec<u8>,
    pub window_bounds_points: Rect,
    pub content_bounds_points: Option<Rect>,
    pub scale: f64,
    pub captured_at_unix_ms: u64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

#[async_trait]
pub trait Adapter: Send + Sync {
    fn name(&self) -> &'static str;

    async fn launch_process(&self, spec: &ProcessLaunchSpec) -> Result<LaunchOutcome, PortholeError>;

    async fn screenshot(&self, surface: &SurfaceInfo) -> Result<Screenshot, PortholeError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strong_meets_any_required() {
        assert!(Confidence::Strong.meets(RequireConfidence::Strong));
        assert!(Confidence::Strong.meets(RequireConfidence::Plausible));
        assert!(Confidence::Strong.meets(RequireConfidence::Weak));
    }

    #[test]
    fn plausible_fails_strong_requirement() {
        assert!(!Confidence::Plausible.meets(RequireConfidence::Strong));
        assert!(Confidence::Plausible.meets(RequireConfidence::Plausible));
    }

    #[test]
    fn weak_only_meets_weak() {
        assert!(!Confidence::Weak.meets(RequireConfidence::Plausible));
        assert!(Confidence::Weak.meets(RequireConfidence::Weak));
    }
}
```

- [ ] **Step 2: Run the tests**

Run: `cargo test -p porthole-core --lib adapter`
Expected: three passes.

- [ ] **Step 3: Commit**

```bash
git add crates/porthole-core/src/adapter.rs
git commit -m "feat(core): add Adapter trait, LaunchSpec, Confidence"
```

---

## Task 6: porthole-core — In-Memory Adapter

**Files:**
- Create: `crates/porthole-core/src/in_memory.rs`

The in-memory adapter is the most important testing tool in the system. Every daemon and route test uses it.

- [ ] **Step 1: Write the failing test**

Create `crates/porthole-core/src/in_memory.rs`:

```rust
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::Mutex;

use crate::adapter::{
    Adapter, Confidence, Correlation, LaunchOutcome, ProcessLaunchSpec, Rect, Screenshot,
};
use crate::surface::{SurfaceId, SurfaceInfo, SurfaceKind, SurfaceState};
use crate::{ErrorCode, PortholeError};

#[derive(Clone, Default)]
pub struct InMemoryAdapter {
    script: Arc<Mutex<Script>>,
}

#[derive(Default)]
struct Script {
    next_launch_outcome: Option<Result<LaunchOutcome, PortholeError>>,
    next_screenshot: Option<Result<Screenshot, PortholeError>>,
    launch_calls: Vec<ProcessLaunchSpec>,
    screenshot_calls: Vec<SurfaceId>,
}

impl InMemoryAdapter {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn set_next_launch_outcome(&self, outcome: Result<LaunchOutcome, PortholeError>) {
        self.script.lock().await.next_launch_outcome = Some(outcome);
    }

    pub async fn set_next_screenshot(&self, out: Result<Screenshot, PortholeError>) {
        self.script.lock().await.next_screenshot = Some(out);
    }

    pub async fn launch_calls(&self) -> Vec<ProcessLaunchSpec> {
        self.script.lock().await.launch_calls.clone()
    }

    pub async fn screenshot_calls(&self) -> Vec<SurfaceId> {
        self.script.lock().await.screenshot_calls.clone()
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
}

#[async_trait]
impl Adapter for InMemoryAdapter {
    fn name(&self) -> &'static str {
        "in-memory"
    }

    async fn launch_process(&self, spec: &ProcessLaunchSpec) -> Result<LaunchOutcome, PortholeError> {
        let mut script = self.script.lock().await;
        script.launch_calls.push(spec.clone());
        script
            .next_launch_outcome
            .take()
            .unwrap_or_else(|| Ok(Self::make_default_launch_outcome(4242)))
    }

    async fn screenshot(&self, surface: &SurfaceInfo) -> Result<Screenshot, PortholeError> {
        let mut script = self.script.lock().await;
        script.screenshot_calls.push(surface.id.clone());
        script.next_screenshot.take().unwrap_or_else(|| {
            Ok(Screenshot {
                png_bytes: minimal_png(),
                window_bounds_points: Rect { x: 0.0, y: 0.0, w: 800.0, h: 600.0 },
                content_bounds_points: None,
                scale: 2.0,
                captured_at_unix_ms: 0,
            })
        })
    }
}

fn minimal_png() -> Vec<u8> {
    // 1x1 transparent PNG — smallest valid image. Tests only check presence and shape.
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
    use super::*;

    #[tokio::test]
    async fn launch_records_call_and_returns_default_outcome() {
        let adapter = InMemoryAdapter::new();
        let spec = ProcessLaunchSpec {
            app: "/Applications/Test.app".to_string(),
            args: vec!["--help".to_string()],
            cwd: None,
            env: vec![],
            timeout: std::time::Duration::from_secs(5),
            require_confidence: crate::adapter::RequireConfidence::Strong,
        };
        let outcome = adapter.launch_process(&spec).await.unwrap();
        assert_eq!(outcome.confidence, Confidence::Strong);
        let calls = adapter.launch_calls().await;
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].app, "/Applications/Test.app");
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
        adapter
            .set_next_launch_outcome(Err(PortholeError::new(
                ErrorCode::LaunchCorrelationAmbiguous,
                "two candidates",
            )))
            .await;
        let spec = ProcessLaunchSpec {
            app: "x".to_string(),
            args: vec![],
            cwd: None,
            env: vec![],
            timeout: std::time::Duration::from_secs(1),
            require_confidence: crate::adapter::RequireConfidence::Strong,
        };
        let err = adapter.launch_process(&spec).await.unwrap_err();
        assert_eq!(err.code, ErrorCode::LaunchCorrelationAmbiguous);
    }
}
```

- [ ] **Step 2: Run the tests**

Run: `cargo test -p porthole-core --lib in_memory`
Expected: three passes.

- [ ] **Step 3: Commit**

```bash
git add crates/porthole-core/src/in_memory.rs
git commit -m "feat(core): add InMemoryAdapter for tests"
```

---

## Task 7: porthole-core — Launch Pipeline

**Files:**
- Create: `crates/porthole-core/src/launch.rs`

The launch pipeline is thin: call the adapter, verify confidence meets requirement, insert the resulting surface into the handle store, return the outcome. All the OS-adjacent work is the adapter's job.

- [ ] **Step 1: Write the failing test**

Create `crates/porthole-core/src/launch.rs`:

```rust
use std::sync::Arc;

use crate::adapter::{Adapter, LaunchOutcome, ProcessLaunchSpec};
use crate::handle::HandleStore;
use crate::{ErrorCode, PortholeError};

pub struct LaunchPipeline {
    adapter: Arc<dyn Adapter>,
    handles: HandleStore,
}

impl LaunchPipeline {
    pub fn new(adapter: Arc<dyn Adapter>, handles: HandleStore) -> Self {
        Self { adapter, handles }
    }

    pub async fn launch_process(&self, spec: &ProcessLaunchSpec) -> Result<LaunchOutcome, PortholeError> {
        let outcome = self.adapter.launch_process(spec).await?;
        if !outcome.confidence.meets(spec.require_confidence) {
            return Err(PortholeError::new(
                ErrorCode::LaunchCorrelationAmbiguous,
                format!(
                    "launch correlation returned confidence {:?}; required {:?}",
                    outcome.confidence, spec.require_confidence
                ),
            ));
        }
        self.handles.insert(outcome.surface.clone()).await;
        Ok(outcome)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::adapter::{Confidence, Correlation, RequireConfidence};
    use crate::in_memory::InMemoryAdapter;
    use crate::surface::SurfaceState;

    fn spec(required: RequireConfidence) -> ProcessLaunchSpec {
        ProcessLaunchSpec {
            app: "test".to_string(),
            args: vec![],
            cwd: None,
            env: vec![],
            timeout: Duration::from_secs(1),
            require_confidence: required,
        }
    }

    #[tokio::test]
    async fn strong_launch_succeeds_and_stores_handle() {
        let adapter = Arc::new(InMemoryAdapter::new());
        let handles = HandleStore::new();
        let pipeline = LaunchPipeline::new(adapter.clone(), handles.clone());
        let outcome = pipeline.launch_process(&spec(RequireConfidence::Strong)).await.unwrap();
        assert_eq!(outcome.confidence, Confidence::Strong);
        let stored = handles.get(&outcome.surface.id).await.unwrap();
        assert_eq!(stored.state, SurfaceState::Alive);
    }

    #[tokio::test]
    async fn plausible_adapter_outcome_fails_strong_requirement() {
        let adapter = Arc::new(InMemoryAdapter::new());
        let handles = HandleStore::new();
        let mut outcome = InMemoryAdapter::make_default_launch_outcome(1);
        outcome.confidence = Confidence::Plausible;
        outcome.correlation = Correlation::PidTree;
        adapter.set_next_launch_outcome(Ok(outcome)).await;
        let pipeline = LaunchPipeline::new(adapter.clone(), handles);
        let err = pipeline.launch_process(&spec(RequireConfidence::Strong)).await.unwrap_err();
        assert_eq!(err.code, ErrorCode::LaunchCorrelationAmbiguous);
    }

    #[tokio::test]
    async fn plausible_adapter_outcome_succeeds_with_plausible_requirement() {
        let adapter = Arc::new(InMemoryAdapter::new());
        let handles = HandleStore::new();
        let mut outcome = InMemoryAdapter::make_default_launch_outcome(1);
        outcome.confidence = Confidence::Plausible;
        adapter.set_next_launch_outcome(Ok(outcome)).await;
        let pipeline = LaunchPipeline::new(adapter.clone(), handles);
        let result = pipeline.launch_process(&spec(RequireConfidence::Plausible)).await;
        assert!(result.is_ok());
    }
}
```

- [ ] **Step 2: Run the tests**

Run: `cargo test -p porthole-core --lib launch`
Expected: three passes.

- [ ] **Step 3: Commit**

```bash
git add crates/porthole-core/src/launch.rs
git commit -m "feat(core): add LaunchPipeline with confidence enforcement"
```

---

## Task 8: porthole-protocol — Wire Types

**Files:**
- Create: `crates/porthole-protocol/Cargo.toml`
- Create: `crates/porthole-protocol/src/lib.rs`
- Create: `crates/porthole-protocol/src/info.rs`
- Create: `crates/porthole-protocol/src/launches.rs`
- Create: `crates/porthole-protocol/src/screenshot.rs`
- Create: `crates/porthole-protocol/src/error.rs`

Wire types are the shared language between `portholed` and `porthole`. Keep them minimal for v0; add more as verbs arrive.

- [ ] **Step 1: Write `crates/porthole-protocol/Cargo.toml`**

```toml
[package]
name = "porthole-protocol"
version = "0.0.0"
edition.workspace = true
rust-version.workspace = true
license.workspace = true
repository.workspace = true

[dependencies]
porthole-core = { path = "../porthole-core" }
serde = { workspace = true }
serde_json = { workspace = true }
uuid = { workspace = true }

[dev-dependencies]

[lints]
workspace = true
```

- [ ] **Step 2: Write `crates/porthole-protocol/src/lib.rs`**

```rust
#![forbid(unsafe_code)]

pub mod error;
pub mod info;
pub mod launches;
pub mod screenshot;

pub use porthole_core::surface::{SurfaceId, SurfaceKind, SurfaceState};
```

- [ ] **Step 3: Write `crates/porthole-protocol/src/info.rs`**

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
}
```

- [ ] **Step 4: Write `crates/porthole-protocol/src/launches.rs`**

```rust
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::SurfaceId;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LaunchRequest {
    pub kind: LaunchKind,
    #[serde(default)]
    pub session: Option<String>,
    #[serde(default = "default_require_confidence")]
    pub require_confidence: WireConfidence,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
}

fn default_require_confidence() -> WireConfidence {
    WireConfidence::Strong
}

fn default_timeout_ms() -> u64 {
    10_000
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LaunchKind {
    Process(ProcessLaunch),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProcessLaunch {
    pub app: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WireConfidence {
    Strong,
    Plausible,
    Weak,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WireCorrelation {
    Tag,
    PidTree,
    Temporal,
    DocumentMatch,
    FrontmostChanged,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LaunchResponse {
    pub launch_id: String,
    pub surface_id: SurfaceId,
    pub surface_was_preexisting: bool,
    pub confidence: WireConfidence,
    pub correlation: WireCorrelation,
}
```

- [ ] **Step 5: Write `crates/porthole-protocol/src/screenshot.rs`**

```rust
use serde::{Deserialize, Serialize};

use crate::SurfaceId;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ScreenshotRequest {
    #[serde(default)]
    pub session: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScreenshotResponse {
    pub surface_id: SurfaceId,
    pub png_base64: String,
    pub window_bounds: Rect,
    pub content_bounds: Option<Rect>,
    pub scale: f64,
    pub captured_at_unix_ms: u64,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}
```

- [ ] **Step 6: Write `crates/porthole-protocol/src/error.rs`**

```rust
use porthole_core::ErrorCode;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WireError {
    pub code: ErrorCode,
    pub message: String,
}

impl From<porthole_core::PortholeError> for WireError {
    fn from(err: porthole_core::PortholeError) -> Self {
        Self { code: err.code, message: err.message }
    }
}
```

- [ ] **Step 7: Verify it builds**

Run: `cargo build -p porthole-protocol`
Expected: clean build.

- [ ] **Step 8: Commit**

```bash
git add crates/porthole-protocol
git commit -m "feat(protocol): add wire types for info, launches, screenshot"
```

---

## Task 9: portholed — Runtime Directory and AppState

**Files:**
- Create: `crates/portholed/Cargo.toml`
- Create: `crates/portholed/src/runtime.rs`
- Create: `crates/portholed/src/state.rs`

`runtime.rs` discovers the UDS path; `state.rs` carries the adapter + handle store into axum route handlers.

- [ ] **Step 1: Write `crates/portholed/Cargo.toml`**

```toml
[package]
name = "portholed"
version = "0.0.0"
edition.workspace = true
rust-version.workspace = true
license.workspace = true
repository.workspace = true
default-run = "portholed"

[dependencies]
async-trait = { workspace = true }
axum = { workspace = true }
bytes = { workspace = true }
hyper = { workspace = true }
hyper-util = { workspace = true }
porthole-core = { path = "../porthole-core" }
porthole-protocol = { path = "../porthole-protocol" }
serde = { workspace = true }
serde_json = { workspace = true }
thiserror = { workspace = true }
tokio = { workspace = true }
tower = { workspace = true }
tracing = { workspace = true }
tracing-subscriber = { workspace = true }
uuid = { workspace = true }
base64 = "0.22"

[target.'cfg(target_os = "macos")'.dependencies]
porthole-adapter-macos = { path = "../porthole-adapter-macos" }

[dev-dependencies]
tempfile = "3"

[[bin]]
name = "portholed"
path = "src/main.rs"

[lints]
workspace = true
```

- [ ] **Step 2: Write the failing test**

Create `crates/portholed/src/runtime.rs`:

```rust
use std::path::PathBuf;

pub fn socket_path() -> PathBuf {
    if let Ok(dir) = std::env::var("PORTHOLE_RUNTIME_DIR") {
        return PathBuf::from(dir).join("porthole.sock");
    }
    if let Ok(dir) = std::env::var("XDG_RUNTIME_DIR") {
        return PathBuf::from(dir).join("porthole").join("porthole.sock");
    }
    if let Ok(tmp) = std::env::var("TMPDIR") {
        let uid = unsafe { libc_getuid() };
        return PathBuf::from(tmp).join(format!("porthole-{uid}")).join("porthole.sock");
    }
    let uid = unsafe { libc_getuid() };
    PathBuf::from("/tmp").join(format!("porthole-{uid}")).join("porthole.sock")
}

unsafe extern "C" {
    #[link_name = "getuid"]
    fn libc_getuid() -> u32;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn porthole_runtime_dir_wins() {
        // SAFETY: tests are serial-friendly via distinct env var names; this
        // test only touches PORTHOLE_RUNTIME_DIR.
        // Note: set_var is marked unsafe in edition 2024; we accept this in tests.
        unsafe {
            std::env::set_var("PORTHOLE_RUNTIME_DIR", "/tmp/test-porthole");
        }
        let p = socket_path();
        assert_eq!(p, PathBuf::from("/tmp/test-porthole/porthole.sock"));
        unsafe {
            std::env::remove_var("PORTHOLE_RUNTIME_DIR");
        }
    }
}
```

- [ ] **Step 3: Write `crates/portholed/src/state.rs`**

```rust
use std::sync::Arc;
use std::time::Instant;

use porthole_core::adapter::Adapter;
use porthole_core::handle::HandleStore;
use porthole_core::launch::LaunchPipeline;

#[derive(Clone)]
pub struct AppState {
    pub adapter: Arc<dyn Adapter>,
    pub handles: HandleStore,
    pub pipeline: Arc<LaunchPipeline>,
    pub started_at: Instant,
    pub daemon_version: &'static str,
}

impl AppState {
    pub fn new(adapter: Arc<dyn Adapter>) -> Self {
        let handles = HandleStore::new();
        let pipeline = Arc::new(LaunchPipeline::new(adapter.clone(), handles.clone()));
        Self {
            adapter,
            handles,
            pipeline,
            started_at: Instant::now(),
            daemon_version: env!("CARGO_PKG_VERSION"),
        }
    }

    pub fn uptime_seconds(&self) -> u64 {
        self.started_at.elapsed().as_secs()
    }
}
```

- [ ] **Step 4: Run the test**

Add `pub mod runtime;` and `pub mod state;` to a new `crates/portholed/src/lib.rs`:

```rust
#![forbid(unsafe_code)]

pub mod runtime;
pub mod state;
pub mod server;
pub mod routes;
```

(We'll create `server.rs` and `routes/` in later tasks — the missing modules will make this not compile yet, so *do not create* `lib.rs` in this task. Instead, for now use this temporary lib.rs:)

```rust
#![forbid(unsafe_code)]

pub mod runtime;
pub mod state;
```

Run: `cargo test -p portholed --lib runtime`
Expected: one pass.

- [ ] **Step 5: Commit**

```bash
git add crates/portholed/Cargo.toml crates/portholed/src/lib.rs crates/portholed/src/runtime.rs crates/portholed/src/state.rs
git commit -m "feat(daemon): add socket path discovery and AppState"
```

---

## Task 10: portholed — Routes for /info

**Files:**
- Create: `crates/portholed/src/routes/mod.rs`
- Create: `crates/portholed/src/routes/errors.rs`
- Create: `crates/portholed/src/routes/info.rs`

- [ ] **Step 1: Write `crates/portholed/src/routes/errors.rs`**

```rust
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use porthole_core::{ErrorCode, PortholeError};
use porthole_protocol::error::WireError;

pub struct ApiError(pub WireError);

impl From<PortholeError> for ApiError {
    fn from(err: PortholeError) -> Self {
        Self(err.into())
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
        };
        (status, Json(self.0)).into_response()
    }
}
```

- [ ] **Step 2: Write `crates/portholed/src/routes/info.rs`**

```rust
use axum::extract::State;
use axum::Json;
use porthole_protocol::info::{AdapterInfo, InfoResponse};

use crate::state::AppState;

pub async fn get_info(State(state): State<AppState>) -> Json<InfoResponse> {
    Json(InfoResponse {
        daemon_version: state.daemon_version.to_string(),
        uptime_seconds: state.uptime_seconds(),
        adapters: vec![AdapterInfo {
            name: state.adapter.name().to_string(),
            loaded: true,
            capabilities: vec!["launch_process".to_string(), "screenshot".to_string()],
        }],
    })
}
```

- [ ] **Step 3: Write `crates/portholed/src/routes/mod.rs`**

```rust
pub mod errors;
pub mod info;
```

Also update `crates/portholed/src/lib.rs` — add `pub mod routes;`:

```rust
#![forbid(unsafe_code)]

pub mod routes;
pub mod runtime;
pub mod state;
```

- [ ] **Step 4: Verify it builds**

Run: `cargo build -p portholed --lib`
Expected: clean build.

- [ ] **Step 5: Commit**

```bash
git add crates/portholed/src/routes
git commit -m "feat(daemon): add /info route and error mapping"
```

---

## Task 11: portholed — Routes for /launches and /surfaces/{id}/screenshot

**Files:**
- Create: `crates/portholed/src/routes/launches.rs`
- Create: `crates/portholed/src/routes/screenshot.rs`
- Modify: `crates/portholed/src/routes/mod.rs`

- [ ] **Step 1: Write `crates/portholed/src/routes/launches.rs`**

```rust
use std::collections::BTreeMap;
use std::time::Duration;

use axum::extract::State;
use axum::Json;
use porthole_core::adapter::{ProcessLaunchSpec, RequireConfidence};
use porthole_protocol::launches::{
    LaunchKind, LaunchRequest, LaunchResponse, WireConfidence, WireCorrelation,
};
use uuid::Uuid;

use crate::routes::errors::ApiError;
use crate::state::AppState;

pub async fn post_launches(
    State(state): State<AppState>,
    Json(req): Json<LaunchRequest>,
) -> Result<Json<LaunchResponse>, ApiError> {
    let spec = request_to_spec(&req)?;
    let outcome = state.pipeline.launch_process(&spec).await?;
    let launch_id = format!("launch_{}", Uuid::new_v4().simple());
    Ok(Json(LaunchResponse {
        launch_id,
        surface_id: outcome.surface.id.clone(),
        surface_was_preexisting: outcome.surface_was_preexisting,
        confidence: confidence_to_wire(outcome.confidence),
        correlation: correlation_to_wire(outcome.correlation),
    }))
}

fn request_to_spec(req: &LaunchRequest) -> Result<ProcessLaunchSpec, ApiError> {
    match &req.kind {
        LaunchKind::Process(p) => Ok(ProcessLaunchSpec {
            app: p.app.clone(),
            args: p.args.clone(),
            cwd: p.cwd.clone(),
            env: to_env_vec(&p.env),
            timeout: Duration::from_millis(req.timeout_ms),
            require_confidence: wire_to_require(req.require_confidence),
        }),
    }
}

fn to_env_vec(map: &BTreeMap<String, String>) -> Vec<(String, String)> {
    map.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
}

fn wire_to_require(c: WireConfidence) -> RequireConfidence {
    match c {
        WireConfidence::Strong => RequireConfidence::Strong,
        WireConfidence::Plausible => RequireConfidence::Plausible,
        WireConfidence::Weak => RequireConfidence::Weak,
    }
}

fn confidence_to_wire(c: porthole_core::adapter::Confidence) -> WireConfidence {
    use porthole_core::adapter::Confidence::*;
    match c {
        Strong => WireConfidence::Strong,
        Plausible => WireConfidence::Plausible,
        Weak => WireConfidence::Weak,
    }
}

fn correlation_to_wire(c: porthole_core::adapter::Correlation) -> WireCorrelation {
    use porthole_core::adapter::Correlation::*;
    match c {
        Tag => WireCorrelation::Tag,
        PidTree => WireCorrelation::PidTree,
        Temporal => WireCorrelation::Temporal,
        DocumentMatch => WireCorrelation::DocumentMatch,
        FrontmostChanged => WireCorrelation::FrontmostChanged,
    }
}
```

- [ ] **Step 2: Write `crates/portholed/src/routes/screenshot.rs`**

```rust
use axum::extract::{Path, State};
use axum::Json;
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use porthole_core::surface::SurfaceId;
use porthole_protocol::screenshot::{Rect, ScreenshotRequest, ScreenshotResponse};

use crate::routes::errors::ApiError;
use crate::state::AppState;

pub async fn post_screenshot(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(_req): Json<ScreenshotRequest>,
) -> Result<Json<ScreenshotResponse>, ApiError> {
    let surface_id = SurfaceId::from(id.clone());
    let info = state.handles.require_alive(&surface_id).await?;
    let shot = state.adapter.screenshot(&info).await?;
    let png_b64 = B64.encode(&shot.png_bytes);
    Ok(Json(ScreenshotResponse {
        surface_id: info.id,
        png_base64: png_b64,
        window_bounds: to_rect(shot.window_bounds_points),
        content_bounds: shot.content_bounds_points.map(to_rect),
        scale: shot.scale,
        captured_at_unix_ms: shot.captured_at_unix_ms,
    }))
}

fn to_rect(r: porthole_core::adapter::Rect) -> Rect {
    Rect { x: r.x, y: r.y, w: r.w, h: r.h }
}
```

- [ ] **Step 3: Add `From<String>` for `SurfaceId`**

Modify `crates/porthole-core/src/surface.rs` — add under the `impl SurfaceId` block:

```rust
impl From<String> for SurfaceId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<&str> for SurfaceId {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}
```

- [ ] **Step 4: Update routes mod**

Modify `crates/portholed/src/routes/mod.rs`:

```rust
pub mod errors;
pub mod info;
pub mod launches;
pub mod screenshot;
```

- [ ] **Step 5: Verify it builds**

Run: `cargo build -p portholed --lib`
Expected: clean build.

- [ ] **Step 6: Commit**

```bash
git add crates/porthole-core/src/surface.rs crates/portholed/src/routes
git commit -m "feat(daemon): add /launches and screenshot routes"
```

---

## Task 12: portholed — Server Bootstrap (UDS)

**Files:**
- Create: `crates/portholed/src/server.rs`

Serve axum over a `tokio::net::UnixListener`. Creating the parent directory and removing stale sockets is the daemon's responsibility.

- [ ] **Step 1: Write the failing test**

Create `crates/portholed/src/server.rs`:

```rust
use std::path::PathBuf;
use std::sync::Arc;

use axum::routing::{get, post};
use axum::Router;
use porthole_core::adapter::Adapter;
use tokio::net::UnixListener;
use tracing::info;

use crate::routes::{info as info_route, launches as launches_route, screenshot as screenshot_route};
use crate::state::AppState;

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/info", get(info_route::get_info))
        .route("/launches", post(launches_route::post_launches))
        .route("/surfaces/{id}/screenshot", post(screenshot_route::post_screenshot))
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
    use tower::ServiceExt;

    use super::*;

    #[tokio::test]
    async fn get_info_returns_adapter_info() {
        let adapter = Arc::new(InMemoryAdapter::new());
        let router = build_router(AppState::new(adapter));
        let req = Request::builder().method(Method::GET).uri("/info").body(Body::empty()).unwrap();
        let res = router.oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = to_bytes(res.into_body(), 1024 * 1024).await.unwrap();
        let info: porthole_protocol::info::InfoResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(info.adapters.len(), 1);
        assert_eq!(info.adapters[0].name, "in-memory");
    }

    #[tokio::test]
    async fn post_launch_then_screenshot_roundtrips() {
        let adapter = Arc::new(InMemoryAdapter::new());
        let router = build_router(AppState::new(adapter));
        let launch_body = serde_json::json!({
            "kind": { "type": "process", "app": "test", "args": [] },
            "require_confidence": "strong"
        });
        let req = Request::builder()
            .method(Method::POST)
            .uri("/launches")
            .header("content-type", "application/json")
            .body(Body::from(launch_body.to_string()))
            .unwrap();
        let res = router.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = to_bytes(res.into_body(), 1024 * 1024).await.unwrap();
        let launch: porthole_protocol::launches::LaunchResponse = serde_json::from_slice(&body).unwrap();

        let req = Request::builder()
            .method(Method::POST)
            .uri(format!("/surfaces/{}/screenshot", launch.surface_id))
            .header("content-type", "application/json")
            .body(Body::from("{}"))
            .unwrap();
        let res = router.oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = to_bytes(res.into_body(), 4 * 1024 * 1024).await.unwrap();
        let shot: porthole_protocol::screenshot::ScreenshotResponse = serde_json::from_slice(&body).unwrap();
        assert!(!shot.png_base64.is_empty());
    }
}
```

- [ ] **Step 2: Update `crates/portholed/src/lib.rs`**

```rust
#![forbid(unsafe_code)]

pub mod routes;
pub mod runtime;
pub mod server;
pub mod state;
```

- [ ] **Step 3: Run the tests**

Run: `cargo test -p portholed --lib server`
Expected: two passes.

- [ ] **Step 4: Commit**

```bash
git add crates/portholed/src/lib.rs crates/portholed/src/server.rs
git commit -m "feat(daemon): wire router over UnixListener and cover routes in tests"
```

---

## Task 13: portholed — main.rs

**Files:**
- Create: `crates/portholed/src/main.rs`

main selects the adapter based on target_os (macOS → macos adapter; anything else → in-memory placeholder), then calls `serve`.

- [ ] **Step 1: Write `crates/portholed/src/main.rs`**

```rust
use std::sync::Arc;

use portholed::runtime::socket_path;
use portholed::server::serve;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> std::io::Result<()> {
    tracing_subscriber::fmt().with_env_filter(EnvFilter::from_default_env().add_directive("info".parse().unwrap())).init();

    let adapter = build_adapter();
    let path = socket_path();
    serve(adapter, path).await
}

#[cfg(target_os = "macos")]
fn build_adapter() -> Arc<dyn porthole_core::adapter::Adapter> {
    Arc::new(porthole_adapter_macos::MacOsAdapter::new())
}

#[cfg(not(target_os = "macos"))]
fn build_adapter() -> Arc<dyn porthole_core::adapter::Adapter> {
    tracing::warn!("no native adapter for this platform; falling back to in-memory adapter");
    Arc::new(porthole_core::in_memory::InMemoryAdapter::new())
}
```

- [ ] **Step 2: Verify non-macOS build**

Run on current platform: `cargo build -p portholed`
Expected: on macOS this will fail because `porthole-adapter-macos` doesn't exist yet; skip this step until Task 16 creates the macOS adapter. For non-macOS platforms it should succeed now.

**Deferred step:** `cargo build -p portholed` will succeed after Task 16.

- [ ] **Step 3: Commit**

```bash
git add crates/portholed/src/main.rs
git commit -m "feat(daemon): add main.rs with platform-conditional adapter wiring"
```

---

## Task 14: porthole — CLI Client Foundations

**Files:**
- Create: `crates/porthole/Cargo.toml`
- Create: `crates/porthole/src/client.rs`
- Create: `crates/porthole/src/runtime.rs`
- Create: `crates/porthole/src/main.rs`
- Create: `crates/porthole/src/commands/mod.rs`
- Create: `crates/porthole/src/commands/info.rs`

- [ ] **Step 1: Write `crates/porthole/Cargo.toml`**

```toml
[package]
name = "porthole"
version = "0.0.0"
edition.workspace = true
rust-version.workspace = true
license.workspace = true
repository.workspace = true

[dependencies]
base64 = "0.22"
bytes = { workspace = true }
clap = { workspace = true }
hyper = { workspace = true }
hyper-util = { workspace = true }
hyperlocal = { workspace = true }
http-body-util = "0.1"
porthole-core = { path = "../porthole-core" }
porthole-protocol = { path = "../porthole-protocol" }
serde = { workspace = true }
serde_json = { workspace = true }
thiserror = { workspace = true }
tokio = { workspace = true }

[[bin]]
name = "porthole"
path = "src/main.rs"

[lints]
workspace = true
```

- [ ] **Step 2: Write `crates/porthole/src/runtime.rs`**

```rust
use std::path::PathBuf;

pub fn socket_path() -> PathBuf {
    if let Ok(dir) = std::env::var("PORTHOLE_RUNTIME_DIR") {
        return PathBuf::from(dir).join("porthole.sock");
    }
    if let Ok(dir) = std::env::var("XDG_RUNTIME_DIR") {
        return PathBuf::from(dir).join("porthole").join("porthole.sock");
    }
    if let Ok(tmp) = std::env::var("TMPDIR") {
        let uid = unsafe { libc_getuid() };
        return PathBuf::from(tmp).join(format!("porthole-{uid}")).join("porthole.sock");
    }
    let uid = unsafe { libc_getuid() };
    PathBuf::from("/tmp").join(format!("porthole-{uid}")).join("porthole.sock")
}

unsafe extern "C" {
    #[link_name = "getuid"]
    fn libc_getuid() -> u32;
}
```

Note: duplicate of the daemon's runtime discovery. Intentional — avoids a circular dep between CLI and daemon. If this pattern recurs, promote a shared crate; not yet warranted.

- [ ] **Step 3: Write `crates/porthole/src/client.rs`**

```rust
use std::path::Path;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::{Method, Request};
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;
use hyperlocal::{UnixClientExt, UnixConnector, Uri as UnixUri};
use porthole_protocol::error::WireError;
use serde::de::DeserializeOwned;
use serde::Serialize;

pub struct DaemonClient {
    socket: std::path::PathBuf,
    http: Client<UnixConnector, Full<Bytes>>,
}

impl DaemonClient {
    pub fn new(socket: impl AsRef<Path>) -> Self {
        Self {
            socket: socket.as_ref().to_path_buf(),
            http: Client::unix(),
        }
    }

    pub async fn get_json<T: DeserializeOwned>(&self, path: &str) -> Result<T, ClientError> {
        let uri: hyper::Uri = UnixUri::new(&self.socket, path).into();
        let req = Request::builder().method(Method::GET).uri(uri).body(Full::new(Bytes::new()))?;
        self.send_and_parse(req).await
    }

    pub async fn post_json<B: Serialize, T: DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T, ClientError> {
        let uri: hyper::Uri = UnixUri::new(&self.socket, path).into();
        let body_bytes = serde_json::to_vec(body)?;
        let req = Request::builder()
            .method(Method::POST)
            .uri(uri)
            .header("content-type", "application/json")
            .body(Full::new(Bytes::from(body_bytes)))?;
        self.send_and_parse(req).await
    }

    async fn send_and_parse<T: DeserializeOwned>(
        &self,
        req: Request<Full<Bytes>>,
    ) -> Result<T, ClientError> {
        let res = self.http.request(req).await?;
        let status = res.status();
        let body = res.into_body().collect().await?.to_bytes();
        if !status.is_success() {
            let wire: WireError = serde_json::from_slice(&body).map_err(ClientError::from)?;
            return Err(ClientError::Api(wire));
        }
        let value = serde_json::from_slice(&body)?;
        Ok(value)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("http: {0}")]
    Http(#[from] hyper::Error),
    #[error("http legacy: {0}")]
    HttpLegacy(#[from] hyper_util::client::legacy::Error),
    #[error("request build: {0}")]
    RequestBuild(#[from] hyper::http::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("api: {} ({})", .0.code, .0.message)]
    Api(WireError),
}
```

- [ ] **Step 4: Write `crates/porthole/src/commands/info.rs`**

```rust
use porthole_protocol::info::InfoResponse;

use crate::client::{ClientError, DaemonClient};

pub async fn run(client: &DaemonClient) -> Result<(), ClientError> {
    let info: InfoResponse = client.get_json("/info").await?;
    println!("daemon_version: {}", info.daemon_version);
    println!("uptime_seconds: {}", info.uptime_seconds);
    for adapter in info.adapters {
        println!(
            "adapter: {} (loaded={}) capabilities={}",
            adapter.name,
            adapter.loaded,
            adapter.capabilities.join(",")
        );
    }
    Ok(())
}
```

- [ ] **Step 5: Write `crates/porthole/src/commands/mod.rs`**

```rust
pub mod info;
```

- [ ] **Step 6: Write `crates/porthole/src/main.rs`**

```rust
use clap::{Parser, Subcommand};
use porthole::client::DaemonClient;
use porthole::runtime::socket_path;

#[derive(Parser)]
#[command(version, about = "porthole — OS-level presentation substrate")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Print daemon info and loaded adapters.
    Info,
}

#[tokio::main]
async fn main() -> std::process::ExitCode {
    let cli = Cli::parse();
    let client = DaemonClient::new(socket_path());
    let result = match cli.command {
        Command::Info => porthole::commands::info::run(&client).await,
    };
    match result {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}
```

- [ ] **Step 7: Add a lib.rs for the CLI so tests can import its modules**

Create `crates/porthole/src/lib.rs`:

```rust
#![forbid(unsafe_code)]

pub mod client;
pub mod commands;
pub mod runtime;
```

Update `Cargo.toml` — add above the `[[bin]]` block:

```toml
[lib]
name = "porthole"
path = "src/lib.rs"
```

- [ ] **Step 8: Verify it builds**

Run: `cargo build -p porthole`
Expected: clean build.

- [ ] **Step 9: Commit**

```bash
git add crates/porthole
git commit -m "feat(cli): add porthole CLI with info subcommand"
```

---

## Task 15: CLI ↔ Daemon Integration Test

**Files:**
- Create: `crates/portholed/tests/cli_info.rs`

Spawn the daemon on a tempdir UDS, run `porthole info` against it, assert output.

- [ ] **Step 1: Write the integration test**

Create `crates/portholed/tests/cli_info.rs`:

```rust
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use portholed::server::serve;
use porthole_core::in_memory::InMemoryAdapter;

#[tokio::test]
async fn daemon_serves_info_over_uds() {
    let tmp = tempfile::tempdir().unwrap();
    let socket = tmp.path().join("porthole.sock");

    let adapter = Arc::new(InMemoryAdapter::new());
    let socket_for_serve = socket.clone();
    let server_task = tokio::spawn(async move { serve(adapter, socket_for_serve).await });

    // Wait for the socket to appear (up to ~2s).
    for _ in 0..200 {
        if socket.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(socket.exists(), "socket did not appear");

    let client = porthole::client::DaemonClient::new(&socket);
    let info: porthole_protocol::info::InfoResponse = client.get_json("/info").await.expect("get_json");
    assert_eq!(info.adapters.len(), 1);
    assert_eq!(info.adapters[0].name, "in-memory");

    server_task.abort();
}
```

- [ ] **Step 2: Add `porthole` and `tempfile` as dev-deps of portholed**

Modify `crates/portholed/Cargo.toml` — append to `[dev-dependencies]`:

```toml
porthole = { path = "../porthole" }
porthole-protocol = { path = "../porthole-protocol" }
```

(tempfile is already present.)

- [ ] **Step 3: Run the test**

Run: `cargo test -p portholed --test cli_info`
Expected: one pass.

- [ ] **Step 4: Commit**

```bash
git add crates/portholed/Cargo.toml crates/portholed/tests/cli_info.rs
git commit -m "test(daemon): end-to-end integration across UDS"
```

---

## Task 16: porthole-adapter-macos — Skeleton

**Files:**
- Create: `crates/porthole-adapter-macos/Cargo.toml`
- Create: `crates/porthole-adapter-macos/src/lib.rs`
- Create: `crates/porthole-adapter-macos/src/enumerate.rs`
- Create: `crates/porthole-adapter-macos/src/launch.rs`
- Create: `crates/porthole-adapter-macos/src/correlation.rs`
- Create: `crates/porthole-adapter-macos/src/capture.rs`
- Create: `crates/porthole-adapter-macos/src/ffi.rs`

- [ ] **Step 1: Write `crates/porthole-adapter-macos/Cargo.toml`**

```toml
[package]
name = "porthole-adapter-macos"
version = "0.0.0"
edition.workspace = true
rust-version.workspace = true
license.workspace = true
repository.workspace = true

[dependencies]
async-trait = { workspace = true }
porthole-core = { path = "../porthole-core" }
thiserror = { workspace = true }
tokio = { workspace = true }
tracing = { workspace = true }
uuid = { workspace = true }
image = { workspace = true }
bytes = { workspace = true }

[target.'cfg(target_os = "macos")'.dependencies]
core-graphics = "0.24"
core-foundation = "0.10"
objc2 = "0.5"
objc2-foundation = "0.2"
objc2-app-kit = "0.2"

[lints]
workspace = true
```

- [ ] **Step 2: Write `crates/porthole-adapter-macos/src/lib.rs`**

```rust
#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

use async_trait::async_trait;
use porthole_core::adapter::{
    Adapter, LaunchOutcome, ProcessLaunchSpec, Screenshot,
};
use porthole_core::surface::SurfaceInfo;
use porthole_core::PortholeError;

pub mod capture;
pub mod correlation;
pub mod enumerate;
pub mod ffi;
pub mod launch;

pub struct MacOsAdapter;

impl MacOsAdapter {
    pub fn new() -> Self {
        Self
    }
}

impl Default for MacOsAdapter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Adapter for MacOsAdapter {
    fn name(&self) -> &'static str {
        "macos"
    }

    async fn launch_process(&self, spec: &ProcessLaunchSpec) -> Result<LaunchOutcome, PortholeError> {
        launch::launch_process(spec).await
    }

    async fn screenshot(&self, surface: &SurfaceInfo) -> Result<Screenshot, PortholeError> {
        capture::screenshot(surface).await
    }
}
```

- [ ] **Step 3: Write skeleton stubs for the submodules**

Create `crates/porthole-adapter-macos/src/enumerate.rs`:

```rust
use porthole_core::PortholeError;

#[derive(Clone, Debug)]
pub struct WindowRecord {
    pub cg_window_id: u32,
    pub owner_pid: i32,
    pub title: Option<String>,
    pub app_bundle: Option<String>,
}

#[cfg(target_os = "macos")]
pub fn list_windows() -> Result<Vec<WindowRecord>, PortholeError> {
    // Implemented in Task 17.
    Err(PortholeError::new(
        porthole_core::ErrorCode::CapabilityMissing,
        "list_windows not yet implemented",
    ))
}

#[cfg(not(target_os = "macos"))]
pub fn list_windows() -> Result<Vec<WindowRecord>, PortholeError> {
    Err(PortholeError::new(
        porthole_core::ErrorCode::AdapterUnsupported,
        "macOS adapter not supported on this platform",
    ))
}
```

Create `crates/porthole-adapter-macos/src/launch.rs`:

```rust
use porthole_core::adapter::{LaunchOutcome, ProcessLaunchSpec};
use porthole_core::PortholeError;

pub async fn launch_process(_spec: &ProcessLaunchSpec) -> Result<LaunchOutcome, PortholeError> {
    // Implemented in Task 18.
    Err(PortholeError::new(
        porthole_core::ErrorCode::CapabilityMissing,
        "macOS launch_process not yet implemented",
    ))
}
```

Create `crates/porthole-adapter-macos/src/correlation.rs`:

```rust
pub const PORTHOLE_LAUNCH_TAG_ENV: &str = "PORTHOLE_LAUNCH_TAG";

pub fn new_launch_tag() -> String {
    format!("plt_{}", uuid::Uuid::new_v4().simple())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_launch_tag_has_prefix() {
        assert!(new_launch_tag().starts_with("plt_"));
    }
}
```

Create `crates/porthole-adapter-macos/src/capture.rs`:

```rust
use porthole_core::adapter::Screenshot;
use porthole_core::surface::SurfaceInfo;
use porthole_core::PortholeError;

pub async fn screenshot(_surface: &SurfaceInfo) -> Result<Screenshot, PortholeError> {
    // Implemented in Task 19.
    Err(PortholeError::new(
        porthole_core::ErrorCode::CapabilityMissing,
        "macOS screenshot not yet implemented",
    ))
}
```

Create `crates/porthole-adapter-macos/src/ffi.rs`:

```rust
// Reserved for tiny FFI shims not covered by the objc2/core-graphics crates.
// Kept empty for now; populated as needed by later tasks.
```

- [ ] **Step 4: Verify it builds**

Run: `cargo build -p porthole-adapter-macos`
Expected: clean build on macOS (empty FFI, stub functions). On non-macOS this crate still compiles but all entry points return `AdapterUnsupported`.

- [ ] **Step 5: Run the one real test**

Run: `cargo test -p porthole-adapter-macos --lib`
Expected: one pass (the `new_launch_tag_has_prefix` test).

- [ ] **Step 6: Verify portholed now builds end-to-end on macOS**

Run: `cargo build --workspace`
Expected: clean build across all crates.

- [ ] **Step 7: Commit**

```bash
git add crates/porthole-adapter-macos
git commit -m "feat(adapter-macos): skeleton with stub entry points"
```

---

## Task 17: macOS — Window Enumeration

**Files:**
- Modify: `crates/porthole-adapter-macos/src/enumerate.rs`

Use `CGWindowListCopyWindowInfo` to enumerate on-screen windows. Returns records with CGWindowID, owning PID, title, and bundle.

- [ ] **Step 1: Write the implementation**

Replace `crates/porthole-adapter-macos/src/enumerate.rs` entirely:

```rust
use porthole_core::{ErrorCode, PortholeError};

#[derive(Clone, Debug)]
pub struct WindowRecord {
    pub cg_window_id: u32,
    pub owner_pid: i32,
    pub title: Option<String>,
    pub app_bundle: Option<String>,
}

#[cfg(target_os = "macos")]
pub fn list_windows() -> Result<Vec<WindowRecord>, PortholeError> {
    use core_foundation::array::CFArray;
    use core_foundation::base::{CFType, TCFType};
    use core_foundation::dictionary::CFDictionary;
    use core_foundation::number::CFNumber;
    use core_foundation::string::CFString;
    use core_graphics::window::{
        kCGNullWindowID, kCGWindowListExcludeDesktopElements, kCGWindowListOptionOnScreenOnly,
        CGWindowListCopyWindowInfo,
    };

    let opts = kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements;
    let arr: CFArray<CFDictionary<CFString, CFType>> =
        unsafe { CFArray::wrap_under_create_rule(CGWindowListCopyWindowInfo(opts, kCGNullWindowID)) };

    let mut out = Vec::with_capacity(arr.len() as usize);
    for item in arr.iter() {
        let dict: &CFDictionary<CFString, CFType> = &*item;

        let owner_pid = dict
            .find(CFString::from_static_string("kCGWindowOwnerPID"))
            .and_then(|v| v.downcast::<CFNumber>().and_then(|n| n.to_i32()))
            .unwrap_or(0);
        let cg_window_id = dict
            .find(CFString::from_static_string("kCGWindowNumber"))
            .and_then(|v| v.downcast::<CFNumber>().and_then(|n| n.to_i32()))
            .map(|n| n as u32)
            .unwrap_or(0);
        let title = dict
            .find(CFString::from_static_string("kCGWindowName"))
            .and_then(|v| v.downcast::<CFString>())
            .map(|s| s.to_string());
        let app_bundle = dict
            .find(CFString::from_static_string("kCGWindowOwnerName"))
            .and_then(|v| v.downcast::<CFString>())
            .map(|s| s.to_string());

        out.push(WindowRecord { cg_window_id, owner_pid, title, app_bundle });
    }

    if out.is_empty() {
        // On sandboxed CI this may be empty. Not an error per se, but worth surfacing as info.
        tracing::debug!("list_windows returned empty result");
    }
    Ok(out)
}

#[cfg(not(target_os = "macos"))]
pub fn list_windows() -> Result<Vec<WindowRecord>, PortholeError> {
    Err(PortholeError::new(
        ErrorCode::AdapterUnsupported,
        "macOS adapter not supported on this platform",
    ))
}

#[cfg(test)]
#[cfg(target_os = "macos")]
mod tests {
    use super::*;

    #[test]
    fn list_windows_runs_without_panicking() {
        // We do not assert a window count — CI sandboxes may have zero windows.
        let result = list_windows();
        assert!(result.is_ok(), "list_windows failed: {result:?}");
    }
}
```

- [ ] **Step 2: Run the test**

Run (on macOS): `cargo test -p porthole-adapter-macos --lib enumerate`
Expected: one pass.

- [ ] **Step 3: Commit**

```bash
git add crates/porthole-adapter-macos/src/enumerate.rs
git commit -m "feat(adapter-macos): enumerate on-screen windows via CGWindowListCopyWindowInfo"
```

---

## Task 18: macOS — Process Launch with Tag Correlation

**Files:**
- Modify: `crates/porthole-adapter-macos/src/launch.rs`

Launch via `open -na <bundle> --env PORTHOLE_LAUNCH_TAG=<tag> --args <args>`, then poll `list_windows()` + read target process env (via `ps eww`) to correlate. Strong confidence if we find a window whose owning pid has the env var; `LaunchCorrelationFailed` if we don't within the timeout.

Note: `ps -e` requires extra privileges on recent macOS for reading other users' envs, but reading one's own user's processes is fine and sufficient here.

- [ ] **Step 1: Write the failing unit test**

Modify `crates/porthole-adapter-macos/src/launch.rs`:

```rust
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use porthole_core::adapter::{Confidence, Correlation, LaunchOutcome, ProcessLaunchSpec};
use porthole_core::surface::{SurfaceId, SurfaceInfo, SurfaceKind, SurfaceState};
use porthole_core::{ErrorCode, PortholeError};
use tokio::process::Command;
use tokio::time::sleep;

use crate::correlation::{new_launch_tag, PORTHOLE_LAUNCH_TAG_ENV};
use crate::enumerate::list_windows;

pub async fn launch_process(spec: &ProcessLaunchSpec) -> Result<LaunchOutcome, PortholeError> {
    #[cfg(not(target_os = "macos"))]
    {
        let _ = spec;
        return Err(PortholeError::new(ErrorCode::AdapterUnsupported, "macOS adapter on non-macOS"));
    }

    #[cfg(target_os = "macos")]
    {
        let tag = new_launch_tag();
        let child = build_and_spawn(spec, &tag)?;
        let deadline = Instant::now() + spec.timeout;
        loop {
            if let Some(window) = find_window_with_tag(&tag).await? {
                let surface = SurfaceInfo {
                    id: SurfaceId::new(),
                    kind: SurfaceKind::Window,
                    state: SurfaceState::Alive,
                    title: window.title,
                    app_bundle: window.app_bundle,
                    pid: Some(window.owner_pid as u32),
                    parent_surface_id: None,
                };
                return Ok(LaunchOutcome {
                    surface,
                    confidence: Confidence::Strong,
                    correlation: Correlation::Tag,
                    surface_was_preexisting: false,
                });
            }
            if Instant::now() >= deadline {
                // Clean up our launcher child if it's still around.
                drop(child);
                return Err(PortholeError::new(
                    ErrorCode::LaunchCorrelationFailed,
                    "no window found carrying the launch tag within the timeout",
                ));
            }
            sleep(Duration::from_millis(100)).await;
        }
    }
}

#[cfg(target_os = "macos")]
fn build_and_spawn(spec: &ProcessLaunchSpec, tag: &str) -> Result<tokio::process::Child, PortholeError> {
    let mut cmd = Command::new("/usr/bin/open");
    cmd.arg("-n").arg("-a").arg(&spec.app);
    cmd.arg("--env").arg(format!("{PORTHOLE_LAUNCH_TAG_ENV}={tag}"));
    for (k, v) in &spec.env {
        cmd.arg("--env").arg(format!("{k}={v}"));
    }
    if !spec.args.is_empty() {
        cmd.arg("--args");
        for a in &spec.args {
            cmd.arg(a);
        }
    }
    if let Some(cwd) = &spec.cwd {
        cmd.current_dir(cwd);
    }
    cmd.spawn().map_err(|e| PortholeError::new(ErrorCode::CapabilityMissing, format!("failed to spawn open: {e}")))
}

#[cfg(target_os = "macos")]
async fn find_window_with_tag(tag: &str) -> Result<Option<crate::enumerate::WindowRecord>, PortholeError> {
    let windows = list_windows()?;
    for window in windows {
        if pid_has_env(window.owner_pid, PORTHOLE_LAUNCH_TAG_ENV, tag).await {
            return Ok(Some(window));
        }
    }
    Ok(None)
}

#[cfg(target_os = "macos")]
async fn pid_has_env(pid: i32, key: &str, expected: &str) -> bool {
    let out = Command::new("/bin/ps").args(["eww", "-o", "command=", "-p", &pid.to_string()]).output().await;
    let Ok(out) = out else { return false };
    let text = String::from_utf8_lossy(&out.stdout);
    let needle = format!("{key}={expected}");
    text.contains(&needle)
}

#[allow(dead_code)]
fn now_unix_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    #[tokio::test]
    async fn launch_missing_app_fails_with_correlation_failed() {
        let spec = ProcessLaunchSpec {
            app: "/Applications/__definitely_not_installed__.app".to_string(),
            args: vec![],
            cwd: None,
            env: vec![],
            timeout: Duration::from_millis(500),
            require_confidence: porthole_core::adapter::RequireConfidence::Strong,
        };
        let err = launch_process(&spec).await.unwrap_err();
        // `open` will exit nonzero but our poll loop still hits the deadline.
        assert!(matches!(err.code, ErrorCode::LaunchCorrelationFailed | ErrorCode::CapabilityMissing));
    }
}
```

- [ ] **Step 2: Run the test**

Run: `cargo test -p porthole-adapter-macos --lib launch`
Expected: one pass.

- [ ] **Step 3: Commit**

```bash
git add crates/porthole-adapter-macos/src/launch.rs
git commit -m "feat(adapter-macos): launch_process with PORTHOLE_LAUNCH_TAG correlation"
```

---

## Task 19: macOS — Window Capture via CGWindowListCreateImage

**Files:**
- Modify: `crates/porthole-adapter-macos/src/capture.rs`

Capture the window by its CGWindowID using `CGWindowListCreateImage`, then encode to PNG via the `image` crate.

Context: `SurfaceInfo` currently carries PID but not CGWindowID. To keep this plan tractable we look up the CGWindowID by scanning `list_windows()` for the matching PID and (if present) title. A later plan should put the CGWindowID directly in a platform-specific field of `SurfaceInfo` (or in an adapter-owned side-table keyed by `SurfaceId`) so this re-lookup is unnecessary.

- [ ] **Step 1: Write the implementation**

Replace `crates/porthole-adapter-macos/src/capture.rs` entirely:

```rust
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
        use core_foundation::base::TCFType;
        use core_graphics::geometry::{CGPoint, CGRect, CGSize};
        use core_graphics::image::CGImage;
        use core_graphics::window::{
            kCGWindowImageBoundsIgnoreFraming, kCGWindowImageDefault, kCGWindowListOptionIncludingWindow,
            CGWindowListCreateImage,
        };

        let pid = surface.pid.ok_or_else(|| {
            PortholeError::new(ErrorCode::CapabilityMissing, "surface has no pid; cannot locate CGWindowID")
        })? as i32;

        let cg_window_id = locate_cg_window_id(pid, surface.title.as_deref())?;

        // An empty rect tells CG to use the window's own bounds when combined with
        // kCGWindowListOptionIncludingWindow. If the current crate version exposes a
        // `CGRect::null()` helper, prefer that.
        let zero_rect = CGRect::new(&CGPoint::new(0.0, 0.0), &CGSize::new(0.0, 0.0));
        let image_opt = unsafe {
            CGWindowListCreateImage(
                zero_rect,
                kCGWindowListOptionIncludingWindow,
                cg_window_id,
                kCGWindowImageBoundsIgnoreFraming | kCGWindowImageDefault,
            )
        };
        let image: CGImage = match image_opt {
            Some(img) => img,
            None => {
                return Err(PortholeError::new(
                    ErrorCode::PermissionNeeded,
                    "CGWindowListCreateImage returned null — likely missing Screen Recording permission",
                ));
            }
        };
        // Note: if your `core-graphics` version returns `CGImageRef` (a raw pointer) rather than
        // `Option<CGImage>`, adjust this branch to null-check the ref and then
        // `CGImage::wrap_under_create_rule(raw)`.

        let width = image.width() as u32;
        let height = image.height() as u32;
        let bytes_per_row = image.bytes_per_row();
        let data = image.data();

        let bgra: Vec<u8> = data.bytes().to_vec();
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

        let mut png_bytes = Vec::new();
        {
            use image::codecs::png::PngEncoder;
            use image::{ColorType, ImageEncoder};
            let encoder = PngEncoder::new(&mut png_bytes);
            encoder
                .write_image(&rgba, width, height, ColorType::Rgba8.into())
                .map_err(|e| PortholeError::new(ErrorCode::CapabilityMissing, format!("png encode failed: {e}")))?;
        }

        Ok(Screenshot {
            png_bytes,
            window_bounds_points: Rect { x: 0.0, y: 0.0, w: width as f64, h: height as f64 },
            content_bounds_points: None,
            scale: 1.0,
            captured_at_unix_ms: SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0),
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
```

- [ ] **Step 2: Verify it builds**

Run: `cargo build -p porthole-adapter-macos`
Expected: clean build.

- [ ] **Step 3: Commit**

```bash
git add crates/porthole-adapter-macos/src/capture.rs
git commit -m "feat(adapter-macos): screenshot via CGWindowListCreateImage with PNG encode"
```

---

## Task 20: CLI — `launch` and `screenshot` Subcommands

**Files:**
- Create: `crates/porthole/src/commands/launch.rs`
- Create: `crates/porthole/src/commands/screenshot.rs`
- Modify: `crates/porthole/src/commands/mod.rs`
- Modify: `crates/porthole/src/main.rs`

- [ ] **Step 1: Write `crates/porthole/src/commands/launch.rs`**

```rust
use std::collections::BTreeMap;

use porthole_protocol::launches::{LaunchKind, LaunchRequest, LaunchResponse, ProcessLaunch, WireConfidence};

use crate::client::{ClientError, DaemonClient};

pub struct LaunchArgs {
    pub app: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub cwd: Option<String>,
    pub session: Option<String>,
    pub timeout_ms: u64,
    pub require_confidence: WireConfidence,
}

pub async fn run(client: &DaemonClient, args: LaunchArgs) -> Result<(), ClientError> {
    let mut env = BTreeMap::new();
    for (k, v) in args.env {
        env.insert(k, v);
    }
    let req = LaunchRequest {
        kind: LaunchKind::Process(ProcessLaunch { app: args.app, args: args.args, cwd: args.cwd, env }),
        session: args.session,
        require_confidence: args.require_confidence,
        timeout_ms: args.timeout_ms,
    };
    let res: LaunchResponse = client.post_json("/launches", &req).await?;
    println!("launch_id: {}", res.launch_id);
    println!("surface_id: {}", res.surface_id);
    println!("confidence: {:?}", res.confidence);
    println!("correlation: {:?}", res.correlation);
    println!("surface_was_preexisting: {}", res.surface_was_preexisting);
    Ok(())
}
```

- [ ] **Step 2: Extend `ClientError` with a local variant**

Modify `crates/porthole/src/client.rs` — add a new variant to `ClientError`:

```rust
    #[error("{0}")]
    Local(String),
```

- [ ] **Step 3: Write `crates/porthole/src/commands/screenshot.rs`**

```rust
use std::path::PathBuf;

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use porthole_protocol::screenshot::{ScreenshotRequest, ScreenshotResponse};

use crate::client::{ClientError, DaemonClient};

pub struct ScreenshotArgs {
    pub surface_id: String,
    pub output: PathBuf,
    pub session: Option<String>,
}

pub async fn run(client: &DaemonClient, args: ScreenshotArgs) -> Result<(), ClientError> {
    let req = ScreenshotRequest { session: args.session };
    let res: ScreenshotResponse = client
        .post_json(&format!("/surfaces/{}/screenshot", args.surface_id), &req)
        .await?;
    let bytes = B64.decode(&res.png_base64).map_err(|e| ClientError::Local(format!("base64 decode: {e}")))?;
    std::fs::write(&args.output, &bytes).map_err(|e| ClientError::Local(format!("write {}: {e}", args.output.display())))?;
    println!("wrote {} ({} bytes)", args.output.display(), bytes.len());
    println!("window_bounds: {}x{} at {},{}", res.window_bounds.w, res.window_bounds.h, res.window_bounds.x, res.window_bounds.y);
    println!("scale: {}", res.scale);
    Ok(())
}
```

- [ ] **Step 4: Update `crates/porthole/src/commands/mod.rs`**

```rust
pub mod info;
pub mod launch;
pub mod screenshot;
```

- [ ] **Step 5: Update `crates/porthole/src/main.rs`**

```rust
use std::path::PathBuf;

use clap::{Parser, Subcommand};
use porthole::client::DaemonClient;
use porthole::commands::launch::LaunchArgs;
use porthole::commands::screenshot::ScreenshotArgs;
use porthole::runtime::socket_path;
use porthole_protocol::launches::WireConfidence;

#[derive(Parser)]
#[command(version, about = "porthole — OS-level presentation substrate")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Print daemon info and loaded adapters.
    Info,
    /// Launch a process.
    Launch {
        /// App bundle path or executable.
        #[arg(long)]
        app: String,
        /// Extra arguments passed to the app.
        #[arg(long = "arg", value_name = "ARG")]
        args: Vec<String>,
        /// `KEY=VALUE` env vars.
        #[arg(long = "env", value_name = "KEY=VALUE")]
        env: Vec<String>,
        /// Working directory.
        #[arg(long)]
        cwd: Option<String>,
        /// Session tag.
        #[arg(long)]
        session: Option<String>,
        /// Launch timeout in milliseconds.
        #[arg(long, default_value_t = 10_000)]
        timeout_ms: u64,
        /// Minimum required correlation confidence.
        #[arg(long, value_enum, default_value_t = ConfidenceArg::Strong)]
        require_confidence: ConfidenceArg,
    },
    /// Screenshot a surface.
    Screenshot {
        /// Surface id returned by `launch`.
        surface_id: String,
        /// Output path (PNG).
        #[arg(long)]
        out: PathBuf,
        /// Session tag.
        #[arg(long)]
        session: Option<String>,
    },
}

#[derive(clap::ValueEnum, Clone, Copy, Debug)]
enum ConfidenceArg {
    Strong,
    Plausible,
    Weak,
}

impl From<ConfidenceArg> for WireConfidence {
    fn from(c: ConfidenceArg) -> Self {
        match c {
            ConfidenceArg::Strong => WireConfidence::Strong,
            ConfidenceArg::Plausible => WireConfidence::Plausible,
            ConfidenceArg::Weak => WireConfidence::Weak,
        }
    }
}

#[tokio::main]
async fn main() -> std::process::ExitCode {
    let cli = Cli::parse();
    let client = DaemonClient::new(socket_path());
    let result = match cli.command {
        Command::Info => porthole::commands::info::run(&client).await,
        Command::Launch { app, args, env, cwd, session, timeout_ms, require_confidence } => {
            let parsed_env: Vec<(String, String)> = env
                .into_iter()
                .filter_map(|s| s.split_once('=').map(|(k, v)| (k.to_string(), v.to_string())))
                .collect();
            porthole::commands::launch::run(
                &client,
                LaunchArgs {
                    app,
                    args,
                    env: parsed_env,
                    cwd,
                    session,
                    timeout_ms,
                    require_confidence: require_confidence.into(),
                },
            )
            .await
        }
        Command::Screenshot { surface_id, out, session } => {
            porthole::commands::screenshot::run(&client, ScreenshotArgs { surface_id, output: out, session }).await
        }
    };
    match result {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}
```

- [ ] **Step 6: Build it**

Run: `cargo build -p porthole`
Expected: clean build.

- [ ] **Step 7: Commit**

```bash
git add crates/porthole
git commit -m "feat(cli): add launch and screenshot subcommands"
```

---

## Task 21: End-to-End macOS Integration Test

**Files:**
- Create: `crates/porthole-adapter-macos/tests/launch_capture.rs`

A single `#[ignore]`d test that spawns a short-lived `open -a "TextEdit"` via the real adapter, tag-correlates its window, screenshots it, writes the PNG to a tempfile, and verifies the first bytes are the PNG magic. Ignored by default because it needs a real desktop session and accessibility permissions; run manually with `cargo test -p porthole-adapter-macos -- --ignored`.

- [ ] **Step 1: Write the test**

Create `crates/porthole-adapter-macos/tests/launch_capture.rs`:

```rust
#![cfg(target_os = "macos")]

use std::time::Duration;

use porthole_adapter_macos::MacOsAdapter;
use porthole_core::adapter::{Adapter, ProcessLaunchSpec, RequireConfidence};

#[tokio::test]
#[ignore = "requires a real macOS desktop session with Screen Recording permission"]
async fn launch_textedit_and_capture() {
    let adapter = MacOsAdapter::new();
    let spec = ProcessLaunchSpec {
        app: "/System/Applications/TextEdit.app".to_string(),
        args: vec![],
        cwd: None,
        env: vec![],
        timeout: Duration::from_secs(10),
        require_confidence: RequireConfidence::Strong,
    };
    let outcome = adapter.launch_process(&spec).await.expect("launch");
    assert!(outcome.surface.pid.is_some());

    let shot = adapter.screenshot(&outcome.surface).await.expect("screenshot");
    assert!(shot.png_bytes.starts_with(&[0x89, 0x50, 0x4e, 0x47]), "not a PNG");
    assert!(shot.window_bounds_points.w > 0.0);
}
```

- [ ] **Step 2: Run the test (manually on a macOS desktop)**

Run: `cargo test -p porthole-adapter-macos --test launch_capture -- --ignored --nocapture`
Expected: PASS. If PASS fails with `permission_needed`, grant the running terminal Screen Recording permission in System Settings → Privacy & Security → Screen Recording, then re-run.

- [ ] **Step 3: Verify the whole workspace still builds clean**

Run: `cargo clippy --workspace --all-targets --locked -- -D warnings`
Expected: no warnings or errors.

Run: `cargo test --workspace --locked`
Expected: all non-`#[ignore]` tests pass.

- [ ] **Step 4: Commit**

```bash
git add crates/porthole-adapter-macos/tests/launch_capture.rs
git commit -m "test(adapter-macos): end-to-end launch + screenshot integration"
```

---

## What v0-foundation Delivers

- `cargo build --workspace` clean on macOS and non-macOS (with degraded adapter on non-macOS).
- `portholed` serves HTTP over UDS with `/info`, `POST /launches`, and `POST /surfaces/{id}/screenshot`.
- `porthole info`, `porthole launch`, `porthole screenshot` work end-to-end against the daemon.
- Core logic fully unit-tested against the in-memory adapter.
- One real-macOS integration test that launches TextEdit and screenshots it (gated behind `--ignored`).
- Strong-by-default confidence enforcement from the spec lives in `LaunchPipeline`.
- Error codes map to typed HTTP statuses.

## What v0-foundation Intentionally Does *Not* Deliver

Revisit in subsequent plans:

- Input, wait, close, focus, replace verbs
- Artifact launch kind, placement, dismiss-after, auto-replace semantics
- Attach mode (`/surfaces/search` + `/surfaces/track`)
- Events SSE, attention, displays read model
- Tab surfaces and their restricted verb set
- Lifecycle modes beyond `exit_on_command_end`
- Recording
- Daemon auto-spawn from the CLI (the CLI assumes the daemon is running)
- OpenAPI spec generation
- Structured tracing beyond `info`-level logs
