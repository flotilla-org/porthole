# Porthole System-Permissions Slice — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make macOS system-permission dependency a first-class part of porthole: a single `porthole onboard` setup command, remediation on every permission error, preflight helpers that auto-trigger OS prompts, and a dev bundle for grant stability.

**Architecture:** Extend `PortholeError` with a `details` JSON field that preserves through the wire layer. Rename "permissions" to "system_permissions" across wire and Rust types to reserve "permissions" for a future agent-policy layer. Add one new adapter trait method (`request_system_permission_prompt`) plus per-permission prompt bookkeeping on macOS. Preflight helpers live in the macOS adapter, trigger the OS prompt on miss, and return either `system_permission_needed` or `system_permission_request_failed`. The daemon gains one endpoint (`POST /system-permissions/request`) that `porthole onboard` drives. A `scripts/dev-bundle.sh` wraps the daemon in a `.app` bundle for stable TCC identity across rebuilds.

**Tech Stack:** Rust workspace (tokio async, axum routes, clap CLI, serde, async-trait), macOS Accessibility + Screen Capture TCC APIs via FFI, bash for dev tooling.

**Spec:** `docs/superpowers/specs/2026-04-23-porthole-permissions-slice-design.md`

**Execution location:** Run in a fresh worktree (see brainstorming skill's `using-git-worktrees`). The slice touches many files across all five crates.

---

## Overview of touched files

**Created:**
- `crates/porthole-protocol/src/system_permission.rs` — typed wire bodies
- `crates/portholed/src/routes/system_permissions.rs` — new endpoint
- `crates/porthole/src/commands/onboard.rs` — new CLI subcommand
- `scripts/dev-bundle.sh` — dev bundle builder
- `docs/development.md` — dev playbook

**Modified:**
- `crates/porthole-core/src/error.rs` — add `details` field, rename `PermissionNeeded`, add `SystemPermissionRequestFailed`
- `crates/porthole-core/src/permission.rs` — rename `PermissionStatus` → `SystemPermissionStatus`
- `crates/porthole-core/src/adapter.rs` — rename `permissions()` → `system_permissions()`, add `request_system_permission_prompt`
- `crates/porthole-core/src/in_memory.rs` — mechanical rename, new method impl
- `crates/porthole-protocol/src/error.rs` — `From<PortholeError>` carries `details`
- `crates/porthole-protocol/src/info.rs` — rename `permissions` field and `PermissionStatus` type
- `crates/porthole-protocol/src/lib.rs` — export new module
- `crates/porthole-adapter-macos/src/permissions.rs` — preflight helpers, bookkeeping, new FFI
- `crates/porthole-adapter-macos/src/lib.rs` — implement trait method, declare capability
- Per-method preflight calls in `crates/porthole-adapter-macos/src/{capture,input,close_focus,wait,search,window_alive,placement,snapshot,launch,artifact,enumerate}.rs`
- `crates/portholed/src/routes/errors.rs` — HTTP mapping for new code, merge rule for `ReplacePipelineError::Porthole`
- `crates/portholed/src/routes/info.rs` — renamed type imports
- `crates/portholed/src/routes/mod.rs` — register new route module
- `crates/portholed/src/server.rs` — mount route
- `crates/portholed/src/main.rs` — startup permission-missing warnings
- `crates/porthole/src/main.rs` — register `Onboard` subcommand
- `crates/porthole/src/commands/mod.rs` — export `onboard`
- `crates/porthole/src/commands/info.rs` — renamed field + remediation hint printing
- `AGENTS.md` — update remediation command from `porthole request-permission <name>` to `porthole onboard`

Tests live alongside implementation using `#[cfg(test)]` modules per the existing pattern, plus `#[ignore]`-gated macOS integration tests in `crates/porthole-adapter-macos/src/` per the existing convention (see `AGENTS.md`).

---

## Task 1: Extend `PortholeError` with `details` field

**Files:**
- Modify: `crates/porthole-core/src/error.rs`

- [ ] **Step 1: Write failing test for details roundtrip**

Add to the `tests` module in `crates/porthole-core/src/error.rs`:

```rust
#[test]
fn with_details_attaches_json_object() {
    let err = PortholeError::new(ErrorCode::PermissionNeeded, "accessibility needed")
        .with_details(serde_json::json!({ "permission": "accessibility" }));
    assert_eq!(err.code, ErrorCode::PermissionNeeded);
    let details = err.details.expect("details set");
    assert_eq!(details["permission"], "accessibility");
}

#[test]
fn default_constructor_leaves_details_none() {
    let err = PortholeError::new(ErrorCode::SurfaceDead, "gone");
    assert!(err.details.is_none());
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p porthole-core error::tests::with_details_attaches_json_object`

Expected: FAIL (compilation error — `with_details` and `details` don't exist).

- [ ] **Step 3: Add the field and builder**

Replace the `PortholeError` struct and `impl` in `crates/porthole-core/src/error.rs`:

```rust
#[derive(Debug, thiserror::Error)]
#[error("{code}: {message}")]
pub struct PortholeError {
    pub code: ErrorCode,
    pub message: String,
    pub details: Option<serde_json::Value>,
}

impl PortholeError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self { code, message: message.into(), details: None }
    }

    pub fn with_details(mut self, details: serde_json::Value) -> Self {
        self.details = Some(details);
        self
    }

    pub fn surface_not_found(id: &str) -> Self {
        Self::new(ErrorCode::SurfaceNotFound, format!("no tracked surface with id {id}"))
    }
}
```

- [ ] **Step 4: Run tests to confirm they pass**

Run: `cargo test -p porthole-core`

Expected: all tests pass (existing plus the two new ones).

- [ ] **Step 5: Commit**

```bash
git add crates/porthole-core/src/error.rs
git commit -m "feat(core): add details field to PortholeError"
```

---

## Task 2: Propagate `details` through `From<PortholeError> for WireError`

**Files:**
- Modify: `crates/porthole-protocol/src/error.rs`

- [ ] **Step 1: Update the failing assertion in the existing test**

Replace the existing test `porthole_error_api_error_has_no_details` in `crates/portholed/src/routes/errors.rs` (this test asserts `details.is_none()` for a PortholeError with no details — keep that assertion) and add a new test next to it:

```rust
#[test]
fn porthole_error_with_details_propagates_to_wire() {
    let err = PortholeError::new(ErrorCode::PermissionNeeded, "ax needed")
        .with_details(serde_json::json!({ "permission": "accessibility" }));
    let wire: porthole_protocol::error::WireError = err.into();
    assert_eq!(wire.code, ErrorCode::PermissionNeeded);
    let details = wire.details.expect("details propagated");
    assert_eq!(details["permission"], "accessibility");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p portholed porthole_error_with_details_propagates_to_wire`

Expected: FAIL (details is None — the From impl currently drops them).

- [ ] **Step 3: Update the From impl**

In `crates/porthole-protocol/src/error.rs`, replace the `From<PortholeError>` impl:

```rust
impl From<porthole_core::PortholeError> for WireError {
    fn from(err: porthole_core::PortholeError) -> Self {
        Self { code: err.code, message: err.message, details: err.details }
    }
}
```

- [ ] **Step 4: Run tests to confirm**

Run: `cargo test -p porthole-protocol -p portholed`

Expected: all pass. The original `porthole_error_api_error_has_no_details` test still passes (a PortholeError constructed without `with_details` has `details: None`, which propagates as `None`).

- [ ] **Step 5: Commit**

```bash
git add crates/porthole-protocol/src/error.rs crates/portholed/src/routes/errors.rs
git commit -m "feat(protocol): propagate PortholeError.details through WireError"
```

---

## Task 3: Merge rule for `ReplacePipelineError::Porthole`

**Files:**
- Modify: `crates/portholed/src/routes/errors.rs`

The current conversion overwrites `details` with a `{"old_handle_alive": ...}` object, clobbering any remediation from the wrapped `PortholeError`. Fix it to merge.

- [ ] **Step 1: Write failing test for merge behaviour**

Add to the `tests` module in `crates/portholed/src/routes/errors.rs`:

```rust
#[test]
fn replace_porthole_merges_old_handle_alive_into_existing_details() {
    use porthole_core::replace_pipeline::ReplacePipelineError;
    let wrapped = PortholeError::new(ErrorCode::PermissionNeeded, "ax")
        .with_details(serde_json::json!({
            "permission": "accessibility",
            "remediation": { "cli_command": "porthole onboard" }
        }));
    let api_err = ApiError::from(ReplacePipelineError::Porthole {
        error: wrapped,
        old_handle_alive: true,
    });
    let details = api_err.0.details.expect("details merged");
    assert_eq!(details["old_handle_alive"], true);
    assert_eq!(details["permission"], "accessibility");
    assert_eq!(details["remediation"]["cli_command"], "porthole onboard");
}

#[test]
fn replace_porthole_populates_details_when_wrapped_has_none() {
    use porthole_core::replace_pipeline::ReplacePipelineError;
    let wrapped = PortholeError::new(ErrorCode::SurfaceDead, "gone");
    let api_err = ApiError::from(ReplacePipelineError::Porthole {
        error: wrapped,
        old_handle_alive: false,
    });
    let details = api_err.0.details.expect("details populated");
    assert_eq!(details["old_handle_alive"], false);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p portholed replace_porthole_merges_old_handle_alive_into_existing_details`

Expected: FAIL (existing impl overwrites details; the remediation key is gone).

- [ ] **Step 3: Rewrite the `Porthole` arm**

In `crates/portholed/src/routes/errors.rs`, replace the `ReplacePipelineError::Porthole` arm:

```rust
ReplacePipelineError::Porthole { error, old_handle_alive } => {
    let mut wire: WireError = error.into();
    let merged = match wire.details.take() {
        Some(serde_json::Value::Object(mut map)) => {
            map.insert(
                "old_handle_alive".into(),
                serde_json::Value::Bool(old_handle_alive),
            );
            serde_json::Value::Object(map)
        }
        _ => serde_json::json!({ "old_handle_alive": old_handle_alive }),
    };
    wire.details = Some(merged);
    Self(wire)
}
```

- [ ] **Step 4: Run tests to confirm**

Run: `cargo test -p portholed`

Expected: all pass, including the existing `post_replace_preserves_permission_needed_from_close` test (still works because a `PortholeError` with `details: None` falls through to the default branch, setting just `old_handle_alive`).

- [ ] **Step 5: Commit**

```bash
git add crates/portholed/src/routes/errors.rs
git commit -m "fix(portholed): merge rather than overwrite details in ReplacePipelineError::Porthole"
```

---

## Task 4: Protocol-side typed bodies for system-permission errors and outcomes

**Files:**
- Create: `crates/porthole-protocol/src/system_permission.rs`
- Modify: `crates/porthole-protocol/src/lib.rs`

- [ ] **Step 1: Write the new module**

Create `crates/porthole-protocol/src/system_permission.rs`:

```rust
use serde::{Deserialize, Serialize};

/// Response body for `POST /system-permissions/request`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SystemPermissionPromptOutcome {
    pub permission: String,
    pub granted_before: bool,
    pub granted_after: bool,
    pub prompt_triggered: bool,
    pub requires_daemon_restart: bool,
    pub notes: String,
}

/// Body for `system_permission_needed`. Serialises into `WireError::details`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SystemPermissionNeededBody {
    pub permission: String,
    pub remediation: Remediation,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Remediation {
    pub cli_command: String,
    pub requires_daemon_restart: bool,
    pub settings_path: String,
    pub binary_path: String,
}

/// Body for `system_permission_request_failed`. The daemon cannot open the
/// prompt; the user must grant manually in Settings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SystemPermissionRequestFailedBody {
    pub permission: String,
    pub reason: String,
    pub settings_path: String,
    pub binary_path: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_outcome_roundtrip() {
        let o = SystemPermissionPromptOutcome {
            permission: "accessibility".into(),
            granted_before: false,
            granted_after: false,
            prompt_triggered: true,
            requires_daemon_restart: true,
            notes: "Open System Settings...".into(),
        };
        let json = serde_json::to_string(&o).unwrap();
        let back: SystemPermissionPromptOutcome = serde_json::from_str(&json).unwrap();
        assert_eq!(back, o);
    }

    #[test]
    fn needed_body_roundtrip() {
        let b = SystemPermissionNeededBody {
            permission: "accessibility".into(),
            remediation: Remediation {
                cli_command: "porthole onboard".into(),
                requires_daemon_restart: true,
                settings_path: "System Settings → Privacy & Security → Accessibility".into(),
                binary_path: "/path/to/portholed".into(),
            },
        };
        let json = serde_json::to_string(&b).unwrap();
        let back: SystemPermissionNeededBody = serde_json::from_str(&json).unwrap();
        assert_eq!(back, b);
    }

    #[test]
    fn request_failed_body_roundtrip() {
        let b = SystemPermissionRequestFailedBody {
            permission: "screen_recording".into(),
            reason: "process is not in a bundle".into(),
            settings_path: "System Settings → Privacy & Security → Screen Recording".into(),
            binary_path: "/path/to/portholed".into(),
        };
        let json = serde_json::to_string(&b).unwrap();
        let back: SystemPermissionRequestFailedBody = serde_json::from_str(&json).unwrap();
        assert_eq!(back, b);
    }
}
```

- [ ] **Step 2: Export from lib.rs**

Append to `crates/porthole-protocol/src/lib.rs`:

```rust
pub mod system_permission;
```

(Match the format of existing `pub mod` lines.)

- [ ] **Step 3: Run tests**

Run: `cargo test -p porthole-protocol system_permission`

Expected: three tests pass.

- [ ] **Step 4: Commit**

```bash
git add crates/porthole-protocol/src/system_permission.rs crates/porthole-protocol/src/lib.rs
git commit -m "feat(protocol): add system_permission typed bodies"
```

---

## Task 5: Rename `PermissionStatus` → `SystemPermissionStatus` and adapter method

This is a mechanical rename that touches core, protocol, daemon, CLI, and the macOS adapter. Do it as one commit because the type name is load-bearing in every consumer.

**Files:**
- Modify: `crates/porthole-core/src/permission.rs`
- Modify: `crates/porthole-core/src/adapter.rs`
- Modify: `crates/porthole-core/src/in_memory.rs`
- Modify: `crates/porthole-protocol/src/info.rs`
- Modify: `crates/portholed/src/routes/info.rs`
- Modify: `crates/porthole/src/commands/info.rs`
- Modify: `crates/porthole-adapter-macos/src/permissions.rs`
- Modify: `crates/porthole-adapter-macos/src/lib.rs`

- [ ] **Step 1: Rename the core type**

In `crates/porthole-core/src/permission.rs`, rename struct and the test:

```rust
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SystemPermissionStatus {
    pub name: String,
    pub granted: bool,
    pub purpose: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_permission_status_roundtrip() {
        let p = SystemPermissionStatus {
            name: "accessibility".into(),
            granted: false,
            purpose: "input injection and some wait conditions".into(),
        };
        let json = serde_json::to_string(&p).unwrap();
        let back: SystemPermissionStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(back, p);
    }
}
```

- [ ] **Step 2: Rename adapter trait method**

In `crates/porthole-core/src/adapter.rs`:
- Change the import at the top from `use crate::permission::PermissionStatus;` to `use crate::permission::SystemPermissionStatus;`.
- Change the trait method signature:

```rust
async fn system_permissions(&self) -> Result<Vec<SystemPermissionStatus>, PortholeError>;
```

- [ ] **Step 3: Update InMemoryAdapter**

In `crates/porthole-core/src/in_memory.rs`, rename references:
- `next_permissions` → `next_system_permissions`
- `set_next_permissions` → `set_next_system_permissions`
- `permissions_calls` → `system_permissions_calls`
- The `async fn permissions` impl → `async fn system_permissions`
- `Vec<PermissionStatus>` → `Vec<SystemPermissionStatus>`
- Import: `use porthole_core::permission::SystemPermissionStatus;` (if already imported under old name, update)

- [ ] **Step 4: Update protocol info**

In `crates/porthole-protocol/src/info.rs`, rename the `PermissionStatus` struct and the field:

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
    pub system_permissions: Vec<SystemPermissionStatus>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SystemPermissionStatus {
    pub name: String,
    pub granted: bool,
    pub purpose: String,
}
```

- [ ] **Step 5: Update daemon info route**

In `crates/portholed/src/routes/info.rs`, update imports and field names:

```rust
use axum::extract::State;
use axum::Json;
use porthole_core::permission::SystemPermissionStatus as CoreSystemPermission;
use porthole_protocol::info::{AdapterInfo, InfoResponse, SystemPermissionStatus};

use crate::routes::errors::ApiError;
use crate::state::AppState;

pub async fn get_info(State(state): State<AppState>) -> Result<Json<InfoResponse>, ApiError> {
    let perms = state.adapter.system_permissions().await.unwrap_or_default();
    Ok(Json(InfoResponse {
        daemon_version: state.daemon_version.to_string(),
        uptime_seconds: state.uptime_seconds(),
        adapters: vec![AdapterInfo {
            name: state.adapter.name().to_string(),
            loaded: true,
            capabilities: state.adapter.capabilities().into_iter().map(String::from).collect(),
            system_permissions: perms.into_iter().map(to_wire_permission).collect(),
        }],
    }))
}

fn to_wire_permission(p: CoreSystemPermission) -> SystemPermissionStatus {
    SystemPermissionStatus { name: p.name, granted: p.granted, purpose: p.purpose }
}
```

- [ ] **Step 6: Update CLI info command**

In `crates/porthole/src/commands/info.rs`, change field reference from `adapter.permissions` to `adapter.system_permissions` and the printed label from `permission` to `system permission`:

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
            adapter.capabilities.join(","),
        );
        for perm in &adapter.system_permissions {
            println!(
                "  system permission {}: {} ({})",
                perm.name,
                if perm.granted { "granted" } else { "MISSING" },
                perm.purpose,
            );
        }
    }
    Ok(())
}
```

- [ ] **Step 7: Update macOS adapter**

In `crates/porthole-adapter-macos/src/permissions.rs`:
- Change `use porthole_core::permission::PermissionStatus;` → `use porthole_core::permission::SystemPermissionStatus;`
- Rename function `pub async fn permissions` → `pub async fn system_permissions`
- Rename struct refs inside: `PermissionStatus` → `SystemPermissionStatus`
- Return type: `Result<Vec<SystemPermissionStatus>, PortholeError>`

In `crates/porthole-adapter-macos/src/lib.rs`:
- Change `use porthole_core::permission::PermissionStatus;` → `use porthole_core::permission::SystemPermissionStatus;`
- Rename the trait impl method `async fn permissions` → `async fn system_permissions`, calling `permissions::system_permissions().await`
- Update return type.

- [ ] **Step 8: Update any other call sites**

Run: `cargo build --workspace 2>&1 | head -50`

Check compile errors and fix any stragglers (likely in test modules that called `.permissions().await` or imported `PermissionStatus`). Common fix locations:
- `crates/portholed/src/server.rs` test code (uses `PermissionStatus` in `adapter.set_next_permissions`).

- [ ] **Step 9: Run full test suite**

Run: `cargo test --workspace`

Expected: all tests pass.

- [ ] **Step 10: Commit**

```bash
git add -u
git commit -m "refactor: rename PermissionStatus to SystemPermissionStatus and adapter method"
```

---

## Task 6: Rename `ErrorCode::PermissionNeeded` → `SystemPermissionNeeded`

**Files:**
- Modify: `crates/porthole-core/src/error.rs`
- Modify: `crates/portholed/src/routes/errors.rs`
- Any call sites referencing `ErrorCode::PermissionNeeded`

- [ ] **Step 1: Update the enum and Display impl**

In `crates/porthole-core/src/error.rs`:

```rust
pub enum ErrorCode {
    // ... existing variants ...
    SystemPermissionNeeded,  // was PermissionNeeded
    // ... rest ...
}
```

In the `Display` match arm:
```rust
Self::SystemPermissionNeeded => "system_permission_needed",
```

- [ ] **Step 2: Update HTTP status mapping**

In `crates/portholed/src/routes/errors.rs`:
```rust
ErrorCode::SystemPermissionNeeded => StatusCode::FORBIDDEN,
```

- [ ] **Step 3: Update test assertion**

In the `error_code_display_matches_wire_string` test in `crates/porthole-core/src/error.rs`, ensure or add:
```rust
assert_eq!(ErrorCode::SystemPermissionNeeded.to_string(), "system_permission_needed");
```

- [ ] **Step 4: Update all call sites**

Run: `cargo build --workspace 2>&1 | grep -i "PermissionNeeded"`

Update every reference. Expected locations:
- `crates/portholed/src/server.rs` test `post_replace_preserves_permission_needed_from_close` — change both the error-construction code and the assertion.
- Any other `ErrorCode::PermissionNeeded` usage.

- [ ] **Step 5: Run tests**

Run: `cargo test --workspace`

Expected: all pass.

- [ ] **Step 6: Commit**

```bash
git add -u
git commit -m "refactor: rename ErrorCode::PermissionNeeded to SystemPermissionNeeded"
```

---

## Task 7: Add `ErrorCode::SystemPermissionRequestFailed`

**Files:**
- Modify: `crates/porthole-core/src/error.rs`
- Modify: `crates/portholed/src/routes/errors.rs`

- [ ] **Step 1: Add the variant**

In `crates/porthole-core/src/error.rs`, extend `ErrorCode`:

```rust
SystemPermissionRequestFailed,
```

And its Display arm:
```rust
Self::SystemPermissionRequestFailed => "system_permission_request_failed",
```

- [ ] **Step 2: Add HTTP mapping**

In `crates/portholed/src/routes/errors.rs`, add to the match in `into_response`:

```rust
ErrorCode::SystemPermissionRequestFailed => StatusCode::INTERNAL_SERVER_ERROR,
```

- [ ] **Step 3: Test display string**

In the `error_code_display_matches_wire_string` test add:

```rust
assert_eq!(
    ErrorCode::SystemPermissionRequestFailed.to_string(),
    "system_permission_request_failed"
);
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p porthole-core -p portholed`

Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add -u
git commit -m "feat(core): add ErrorCode::SystemPermissionRequestFailed"
```

---

## Task 8: Add `request_system_permission_prompt` to the `Adapter` trait

The trait is implemented by `InMemoryAdapter` (returns `AdapterUnsupported`) and `MacOsAdapter` (real impl in Task 11).

**Files:**
- Modify: `crates/porthole-core/src/adapter.rs`
- Modify: `crates/porthole-core/src/in_memory.rs`

- [ ] **Step 1: Add the trait method**

In `crates/porthole-core/src/adapter.rs`, add a new method to the `Adapter` trait (after `system_permissions`):

```rust
/// Trigger the OS prompt for the named system permission. Returns a structured
/// result with the grant state before/after and any restart requirement.
/// Calling this for a permission that's already granted is a no-op that
/// still returns the current state.
///
/// `name` is a string matching one of the names the adapter advertises via
/// `system_permissions()`. Unknown names return an `InvalidArgument` error
/// with the supported names in details.
async fn request_system_permission_prompt(
    &self,
    name: &str,
) -> Result<SystemPermissionPromptOutcome, PortholeError>;
```

Also add the import at the top of the file:

```rust
use crate::permission::{SystemPermissionPromptOutcome, SystemPermissionStatus};
```

- [ ] **Step 2: Add `SystemPermissionPromptOutcome` to core**

Append to `crates/porthole-core/src/permission.rs`:

```rust
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SystemPermissionPromptOutcome {
    pub permission: String,
    pub granted_before: bool,
    pub granted_after: bool,
    pub prompt_triggered: bool,
    pub requires_daemon_restart: bool,
    pub notes: String,
}
```

(This mirrors the protocol type from Task 4; the core-side type lets the adapter trait return it without a protocol dependency. The daemon route converts between them — trivially, since the fields match.)

Add a roundtrip test to the same file:

```rust
#[test]
fn prompt_outcome_roundtrip() {
    let o = SystemPermissionPromptOutcome {
        permission: "accessibility".into(),
        granted_before: false,
        granted_after: false,
        prompt_triggered: true,
        requires_daemon_restart: true,
        notes: "restart the daemon".into(),
    };
    let json = serde_json::to_string(&o).unwrap();
    let back: SystemPermissionPromptOutcome = serde_json::from_str(&json).unwrap();
    assert_eq!(back, o);
}
```

- [ ] **Step 3: Implement on InMemoryAdapter**

In `crates/porthole-core/src/in_memory.rs`, in the `Adapter` impl block, add:

```rust
async fn request_system_permission_prompt(
    &self,
    _name: &str,
) -> Result<SystemPermissionPromptOutcome, PortholeError> {
    Err(PortholeError::new(
        ErrorCode::AdapterUnsupported,
        "in-memory adapter does not support system permission prompts",
    ))
}
```

Also add the import at the top:

```rust
use crate::permission::{SystemPermissionPromptOutcome, SystemPermissionStatus};
```

- [ ] **Step 4: Write a test for InMemory's behaviour**

In `crates/porthole-core/src/in_memory.rs` (inside its existing tests module, or create a new one at the bottom):

```rust
#[cfg(test)]
mod system_permission_prompt_tests {
    use super::*;
    use crate::adapter::Adapter;

    #[tokio::test]
    async fn in_memory_returns_adapter_unsupported() {
        let adapter = InMemoryAdapter::new();
        let err = adapter
            .request_system_permission_prompt("accessibility")
            .await
            .expect_err("should error");
        assert_eq!(err.code, ErrorCode::AdapterUnsupported);
    }

    #[test]
    fn in_memory_does_not_advertise_system_permission_prompt_capability() {
        let adapter = InMemoryAdapter::new();
        assert!(!adapter.capabilities().contains(&"system_permission_prompt"));
    }
}
```

- [ ] **Step 5: Run tests**

Run: `cargo test -p porthole-core`

Expected: all pass.

- [ ] **Step 6: Build the workspace**

Run: `cargo build --workspace`

Expected: clean build. The macOS adapter will fail until Task 11 implements the trait method; if so, add a placeholder in `crates/porthole-adapter-macos/src/lib.rs`:

```rust
async fn request_system_permission_prompt(
    &self,
    _name: &str,
) -> Result<porthole_core::permission::SystemPermissionPromptOutcome, PortholeError> {
    // Implemented in Task 11.
    Err(PortholeError::new(
        ErrorCode::SystemPermissionRequestFailed,
        "not yet implemented",
    ))
}
```

- [ ] **Step 7: Commit**

```bash
git add -u
git commit -m "feat(core): add request_system_permission_prompt to Adapter trait"
```

---

## Task 9: Macos adapter — prompt bookkeeping state

Track per-permission "have we called the prompt API this process?" bits so `prompt_triggered` is honest.

**Files:**
- Modify: `crates/porthole-adapter-macos/src/lib.rs`
- Modify: `crates/porthole-adapter-macos/src/permissions.rs`

- [ ] **Step 1: Change `MacOsAdapter` to hold state**

In `crates/porthole-adapter-macos/src/lib.rs`, replace the unit struct with one that carries bookkeeping:

```rust
use std::sync::atomic::{AtomicBool, Ordering};

pub struct MacOsAdapter {
    ax_prompted: AtomicBool,
    sr_prompted: AtomicBool,
}

impl MacOsAdapter {
    pub fn new() -> Self {
        Self {
            ax_prompted: AtomicBool::new(false),
            sr_prompted: AtomicBool::new(false),
        }
    }

    /// For preflight / request paths: mark a permission as having had its
    /// prompt API called. Returns the *previous* value (true if already
    /// prompted, false on first call).
    pub fn set_prompted(&self, name: &str) -> bool {
        match name {
            "accessibility" => self.ax_prompted.swap(true, Ordering::SeqCst),
            "screen_recording" => self.sr_prompted.swap(true, Ordering::SeqCst),
            _ => true, // unknown name: don't track, caller's problem
        }
    }

    /// Check without modifying.
    pub fn was_prompted(&self, name: &str) -> bool {
        match name {
            "accessibility" => self.ax_prompted.load(Ordering::SeqCst),
            "screen_recording" => self.sr_prompted.load(Ordering::SeqCst),
            _ => true,
        }
    }
}
```

- [ ] **Step 2: Verify build**

Run: `cargo build -p porthole-adapter-macos`

Expected: clean build.

- [ ] **Step 3: Commit**

```bash
git add crates/porthole-adapter-macos/src/lib.rs
git commit -m "feat(adapter-macos): add prompt bookkeeping state to MacOsAdapter"
```

---

## Task 10: Macos adapter — FFI additions and `request_system_permission_prompt` implementation

**Files:**
- Modify: `crates/porthole-adapter-macos/src/permissions.rs`
- Modify: `crates/porthole-adapter-macos/src/lib.rs`

- [ ] **Step 1: Add FFI declarations and prompt function**

In `crates/porthole-adapter-macos/src/permissions.rs`, extend the `extern "C"` block and add `request_prompt` functions. `core-foundation` is already a crate dep (macOS target), so use its wrappers for the `AXIsProcessTrustedWithOptions` options dictionary. Rewrite `permissions.rs`:

```rust
#![cfg(target_os = "macos")]

use core_foundation::base::TCFType;
use core_foundation::boolean::CFBoolean;
use core_foundation::dictionary::CFDictionary;
use core_foundation::string::CFString;
use porthole_core::permission::{SystemPermissionPromptOutcome, SystemPermissionStatus};
use porthole_core::{ErrorCode, PortholeError};

unsafe extern "C" {
    fn AXIsProcessTrusted() -> u8;
    fn AXIsProcessTrustedWithOptions(options: *const std::ffi::c_void) -> u8;
    fn CGPreflightScreenCaptureAccess() -> u8;
    fn CGRequestScreenCaptureAccess() -> u8;
}

/// Named constant from AppKit: `kAXTrustedCheckOptionPrompt`.
fn ax_trusted_check_option_prompt_key() -> CFString {
    CFString::from_static_string("AXTrustedCheckOptionPrompt")
}

fn ax_is_trusted_live() -> bool {
    unsafe { AXIsProcessTrusted() != 0 }
}

fn sr_is_granted_live() -> bool {
    unsafe { CGPreflightScreenCaptureAccess() != 0 }
}

/// Calls `AXIsProcessTrustedWithOptions` with `kAXTrustedCheckOptionPrompt: true`.
/// The OS may show a dialog on the first call per process for a given bundle
/// identity; subsequent calls are silent. Returns whether the process is
/// currently trusted, per AX's own return value.
fn ax_request_prompt() -> bool {
    let key = ax_trusted_check_option_prompt_key();
    let value = CFBoolean::true_value();
    let pairs = [(key.as_CFType(), value.as_CFType())];
    let dict = CFDictionary::from_CFType_pairs(&pairs);
    unsafe { AXIsProcessTrustedWithOptions(dict.as_concrete_TypeRef() as *const _) != 0 }
}

fn sr_request_prompt() -> bool {
    unsafe { CGRequestScreenCaptureAccess() != 0 }
}

pub async fn system_permissions() -> Result<Vec<SystemPermissionStatus>, PortholeError> {
    let ax = ax_is_trusted_live();
    let scr = sr_is_granted_live();
    Ok(vec![
        SystemPermissionStatus {
            name: "accessibility".into(),
            granted: ax,
            purpose: "input injection and some wait conditions".into(),
        },
        SystemPermissionStatus {
            name: "screen_recording".into(),
            granted: scr,
            purpose: "window screenshot capture and frame-diff waits".into(),
        },
    ])
}

/// Resolves the daemon's binary path for display in remediation blocks.
/// In dev builds this is the path inside `Portholed.app`.
pub fn daemon_binary_path() -> String {
    std::env::current_exe()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| "<unknown>".to_string())
}

/// Human-readable Settings path for a named permission.
pub fn settings_path_for(name: &str) -> &'static str {
    match name {
        "accessibility" => "System Settings → Privacy & Security → Accessibility",
        "screen_recording" => "System Settings → Privacy & Security → Screen Recording",
        _ => "System Settings → Privacy & Security",
    }
}

pub fn notes_for(name: &str, requires_restart: bool) -> String {
    let base = match name {
        "accessibility" => "Open System Settings → Privacy & Security → Accessibility and enable porthole.",
        "screen_recording" => "Open System Settings → Privacy & Security → Screen Recording and enable porthole.",
        _ => "Open System Settings → Privacy & Security and enable porthole.",
    };
    if requires_restart {
        format!("{base} After granting, restart the daemon so the AX runtime initialises with the new trust state.")
    } else {
        base.to_string()
    }
}

pub fn requires_daemon_restart(name: &str) -> bool {
    matches!(name, "accessibility")
}

pub(crate) fn is_granted(name: &str) -> Result<bool, PortholeError> {
    match name {
        "accessibility" => Ok(ax_is_trusted_live()),
        "screen_recording" => Ok(sr_is_granted_live()),
        _ => Err(PortholeError::new(
            ErrorCode::InvalidArgument,
            format!("unknown system permission: {name}"),
        )
        .with_details(serde_json::json!({
            "supported_names": ["accessibility", "screen_recording"]
        }))),
    }
}

/// Try to open the OS prompt. Returns `Ok(())` on success (or no-op), or
/// `Err(SystemPermissionRequestFailedBody)` if the OS rejected the call.
/// On macOS the underlying APIs don't currently signal rejection — they
/// return the current trust state regardless — so this is a structural
/// placeholder that validates the process is in a bundle context.
pub(crate) fn try_trigger_prompt(name: &str) -> Result<(), String> {
    let is_bundle = std::env::current_exe()
        .ok()
        .and_then(|p| p.ancestors().find(|p| p.extension().map(|e| e == "app").unwrap_or(false)).map(|_| ()))
        .is_some();
    if !is_bundle {
        return Err(
            "process is not running inside a .app bundle; TCC will not open a prompt. \
             Build via scripts/dev-bundle.sh and launch from the bundle."
                .to_string(),
        );
    }
    match name {
        "accessibility" => {
            ax_request_prompt();
            Ok(())
        }
        "screen_recording" => {
            sr_request_prompt();
            Ok(())
        }
        _ => Err(format!("unknown system permission: {name}")),
    }
}
```

- [ ] **Step 2: Implement `request_system_permission_prompt` on `MacOsAdapter`**

In `crates/porthole-adapter-macos/src/lib.rs`, inside the `Adapter` impl, replace the placeholder:

```rust
async fn request_system_permission_prompt(
    &self,
    name: &str,
) -> Result<porthole_core::permission::SystemPermissionPromptOutcome, PortholeError> {
    use porthole_core::permission::SystemPermissionPromptOutcome;

    // Name validation against our supported set. InvalidArgument carries
    // the supported list in details.
    let granted_before = permissions::is_granted(name)?;

    let was_prompted_before = self.was_prompted(name);

    if !granted_before {
        // Attempt to open the OS prompt.
        if let Err(reason) = permissions::try_trigger_prompt(name) {
            let body = porthole_protocol::system_permission::SystemPermissionRequestFailedBody {
                permission: name.to_string(),
                reason,
                settings_path: permissions::settings_path_for(name).to_string(),
                binary_path: permissions::daemon_binary_path(),
            };
            return Err(
                PortholeError::new(ErrorCode::SystemPermissionRequestFailed, "prompt rejected by OS")
                    .with_details(serde_json::to_value(body).unwrap_or_default()),
            );
        }
        self.set_prompted(name);
    }

    let granted_after = permissions::is_granted(name)?;
    let prompt_triggered = !granted_before && !was_prompted_before;
    let requires_daemon_restart = permissions::requires_daemon_restart(name);

    Ok(SystemPermissionPromptOutcome {
        permission: name.to_string(),
        granted_before,
        granted_after,
        prompt_triggered,
        requires_daemon_restart,
        notes: permissions::notes_for(name, requires_daemon_restart),
    })
}
```

Note: the macOS crate now needs `porthole-protocol` as a dependency for the `SystemPermissionRequestFailedBody` type. Add to `crates/porthole-adapter-macos/Cargo.toml`:

```toml
porthole-protocol = { path = "../porthole-protocol" }
```

- [ ] **Step 3: Declare the capability**

In `crates/porthole-adapter-macos/src/lib.rs`, extend the `capabilities()` return to include `"system_permission_prompt"`:

```rust
fn capabilities(&self) -> Vec<&'static str> {
    vec![
        // ... existing capabilities ...
        "system_permission_prompt",
    ]
}
```

- [ ] **Step 4: Build**

Run: `cargo build -p porthole-adapter-macos`

Expected: clean build on macOS.

- [ ] **Step 5: Add `#[ignore]`'d integration tests**

At the bottom of `crates/porthole-adapter-macos/src/permissions.rs`, add a tests module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::MacOsAdapter;
    use porthole_core::adapter::Adapter;

    #[tokio::test]
    #[ignore]
    async fn request_system_permission_prompt_accessibility_returns_outcome() {
        let adapter = MacOsAdapter::new();
        let outcome = adapter
            .request_system_permission_prompt("accessibility")
            .await
            .expect("no-panic");
        assert_eq!(outcome.permission, "accessibility");
        assert_eq!(outcome.granted_before, ax_is_trusted_live());
        assert!(outcome.requires_daemon_restart);
    }

    #[tokio::test]
    #[ignore]
    async fn prompt_bookkeeping_flips_on_first_call_only() {
        let adapter = MacOsAdapter::new();
        // Skip test if accessibility is already granted (bookkeeping never flips
        // in that case).
        if ax_is_trusted_live() {
            eprintln!("accessibility already granted; test skipped");
            return;
        }
        let first = adapter
            .request_system_permission_prompt("accessibility")
            .await
            .expect("no-panic");
        let second = adapter
            .request_system_permission_prompt("accessibility")
            .await
            .expect("no-panic");
        assert!(first.prompt_triggered, "first call should trigger prompt");
        assert!(!second.prompt_triggered, "second call should not re-trigger");
    }

    #[tokio::test]
    async fn unknown_permission_name_returns_invalid_argument() {
        let adapter = MacOsAdapter::new();
        let err = adapter
            .request_system_permission_prompt("coffee_grinder")
            .await
            .expect_err("should reject unknown name");
        assert_eq!(err.code, ErrorCode::InvalidArgument);
        let details = err.details.expect("details populated");
        let supported = details.get("supported_names").and_then(|v| v.as_array()).unwrap();
        assert!(supported.iter().any(|v| v == "accessibility"));
    }
}
```

- [ ] **Step 6: Run non-ignored tests**

Run: `cargo test -p porthole-adapter-macos`

Expected: the `unknown_permission_name_returns_invalid_argument` test passes; the two `#[ignore]`'d ones are skipped.

- [ ] **Step 7: Commit**

```bash
git add -u
git commit -m "feat(adapter-macos): implement request_system_permission_prompt"
```

---

## Task 11: Preflight helpers + `ensure_system_permission` trait method

Add `ensure_accessibility_granted` / `ensure_screen_recording_granted` helpers in `permissions.rs`. Also add an `ensure_system_permission` method to the `Adapter` trait so pipelines (notably `WaitPipeline`, whose `wait` method signature can't return a `PortholeError`) can preflight before dispatching. Each helper triggers the OS prompt as a side effect on miss, and returns either `system_permission_needed` (with `SystemPermissionNeededBody` in details) or `system_permission_request_failed` (with `SystemPermissionRequestFailedBody`).

**Files:**
- Modify: `crates/porthole-adapter-macos/src/permissions.rs`
- Modify: `crates/porthole-core/src/adapter.rs`
- Modify: `crates/porthole-core/src/in_memory.rs`
- Modify: `crates/porthole-adapter-macos/src/lib.rs`

Preflight helpers are free functions taking `&MacOsAdapter` (for access to bookkeeping state). The trait method wraps them.

- [ ] **Step 1: Add preflight helpers**

Append to `crates/porthole-adapter-macos/src/permissions.rs`:

```rust
use crate::MacOsAdapter;
use porthole_protocol::system_permission::{
    Remediation, SystemPermissionNeededBody, SystemPermissionRequestFailedBody,
};

fn build_needed_body(name: &str) -> SystemPermissionNeededBody {
    let requires_restart = requires_daemon_restart(name);
    SystemPermissionNeededBody {
        permission: name.to_string(),
        remediation: Remediation {
            cli_command: "porthole onboard".to_string(),
            requires_daemon_restart: requires_restart,
            settings_path: settings_path_for(name).to_string(),
            binary_path: daemon_binary_path(),
        },
    }
}

fn build_request_failed_body(name: &str, reason: String) -> SystemPermissionRequestFailedBody {
    SystemPermissionRequestFailedBody {
        permission: name.to_string(),
        reason,
        settings_path: settings_path_for(name).to_string(),
        binary_path: daemon_binary_path(),
    }
}

/// Preflight for operations that require Accessibility. Triggers the OS
/// prompt on first miss per daemon process.
pub fn ensure_accessibility_granted(adapter: &MacOsAdapter) -> Result<(), PortholeError> {
    ensure_granted(adapter, "accessibility")
}

/// Preflight for operations that require Screen Recording. Triggers the OS
/// prompt on first miss per daemon process.
pub fn ensure_screen_recording_granted(adapter: &MacOsAdapter) -> Result<(), PortholeError> {
    ensure_granted(adapter, "screen_recording")
}

fn ensure_granted(adapter: &MacOsAdapter, name: &str) -> Result<(), PortholeError> {
    if is_granted(name)? {
        return Ok(());
    }

    // Try to trigger prompt only on first miss per process.
    if !adapter.was_prompted(name) {
        match try_trigger_prompt(name) {
            Ok(()) => {
                adapter.set_prompted(name);
            }
            Err(reason) => {
                let body = build_request_failed_body(name, reason);
                return Err(PortholeError::new(
                    ErrorCode::SystemPermissionRequestFailed,
                    format!("cannot open prompt for {name}"),
                )
                .with_details(serde_json::to_value(body).unwrap_or_default()));
            }
        }
    }

    let body = build_needed_body(name);
    Err(PortholeError::new(
        ErrorCode::SystemPermissionNeeded,
        format!("{name} permission required"),
    )
    .with_details(serde_json::to_value(body).unwrap_or_default()))
}
```

- [ ] **Step 2: Add `ensure_system_permission` to the Adapter trait**

In `crates/porthole-core/src/adapter.rs`, add another trait method (next to `request_system_permission_prompt`):

```rust
/// Preflight: verify the named system permission is granted. If not, the
/// adapter may attempt to trigger an OS prompt as a side effect, then
/// returns `Err(PortholeError)` with code `system_permission_needed` or
/// `system_permission_request_failed`. Adapters that don't gate on
/// OS permissions return `Ok(())`.
async fn ensure_system_permission(&self, name: &str) -> Result<(), PortholeError>;
```

- [ ] **Step 3: InMemoryAdapter always succeeds**

In `crates/porthole-core/src/in_memory.rs`, add to the `Adapter` impl:

```rust
async fn ensure_system_permission(&self, _name: &str) -> Result<(), PortholeError> {
    Ok(())
}
```

- [ ] **Step 4: MacOsAdapter dispatches to helpers**

In `crates/porthole-adapter-macos/src/lib.rs`, add to the `Adapter` impl:

```rust
async fn ensure_system_permission(&self, name: &str) -> Result<(), PortholeError> {
    match name {
        "accessibility" => permissions::ensure_accessibility_granted(self),
        "screen_recording" => permissions::ensure_screen_recording_granted(self),
        _ => Err(PortholeError::new(
            ErrorCode::InvalidArgument,
            format!("unknown system permission: {name}"),
        )
        .with_details(serde_json::json!({
            "supported_names": ["accessibility", "screen_recording"]
        }))),
    }
}
```

- [ ] **Step 5: Build**

Run: `cargo build -p porthole-adapter-macos -p porthole-core`

Expected: clean.

- [ ] **Step 6: Add `#[ignore]`'d integration tests**

Append to the tests module in `crates/porthole-adapter-macos/src/permissions.rs`:

```rust
#[tokio::test]
#[ignore]
async fn ensure_accessibility_returns_needed_when_missing() {
    let adapter = MacOsAdapter::new();
    if ax_is_trusted_live() {
        eprintln!("granted; test skipped");
        return;
    }
    let err = ensure_accessibility_granted(&adapter).expect_err("should error");
    assert_eq!(err.code, ErrorCode::SystemPermissionNeeded);
    let details = err.details.expect("details populated");
    assert_eq!(details["permission"], "accessibility");
    assert_eq!(details["remediation"]["cli_command"], "porthole onboard");
}

#[tokio::test]
#[ignore]
async fn ensure_accessibility_returns_ok_when_granted() {
    let adapter = MacOsAdapter::new();
    if !ax_is_trusted_live() {
        eprintln!("not granted; test skipped");
        return;
    }
    ensure_accessibility_granted(&adapter).expect("should be Ok when granted");
}
```

- [ ] **Step 7: Run tests**

Run: `cargo test -p porthole-adapter-macos -p porthole-core`

Expected: pass (ignored tests skipped in default run).

- [ ] **Step 8: Commit**

```bash
git add -u
git commit -m "feat: preflight helpers + Adapter::ensure_system_permission"
```

---

## Task 12: Apply preflight to all guarded adapter methods

Per §7.2 of the spec, wire preflight calls at the top of every method that touches a guarded API. Mapping:

| Method | Permission(s) | Preflight location |
|---|---|---|
| `launch_process`, `launch_artifact` | accessibility | adapter method |
| `screenshot` | screen_recording | adapter method |
| `key`, `text`, `click`, `scroll` | accessibility | adapter method |
| `close`, `focus` | accessibility | adapter method |
| `wait` (stable/dirty conditions) | screen_recording + accessibility | `WaitPipeline` |
| `wait` (other conditions) | accessibility | `WaitPipeline` |
| `search`, `window_alive` | screen_recording | adapter method |
| `place_surface`, `snapshot_geometry` | accessibility | adapter method |

For everything except `wait`, call the preflight helper inside the adapter method — this returns a `PortholeError` cleanly. For `wait`, the trait method's signature (`Result<WaitOutcome, WaitTimeout>`) doesn't permit returning a `PortholeError` through the error channel, so preflight lives in the `WaitPipeline` before it dispatches to the adapter. The pipeline uses `adapter.ensure_system_permission(name)` (the trait method added in Task 11), which keeps it adapter-agnostic.

For the adapter-method path: change each function signature in the macOS adapter modules to accept `&MacOsAdapter`, and have the trait impl pass `self` down.

**Files:**
- Modify: `crates/porthole-adapter-macos/src/lib.rs` (pass self to helpers)
- Modify: `crates/porthole-adapter-macos/src/capture.rs`
- Modify: `crates/porthole-adapter-macos/src/input.rs`
- Modify: `crates/porthole-adapter-macos/src/close_focus.rs`
- Modify: `crates/porthole-adapter-macos/src/wait.rs`
- Modify: `crates/porthole-adapter-macos/src/search.rs`
- Modify: `crates/porthole-adapter-macos/src/window_alive.rs`
- Modify: `crates/porthole-adapter-macos/src/placement.rs`
- Modify: `crates/porthole-adapter-macos/src/snapshot.rs`
- Modify: `crates/porthole-adapter-macos/src/launch.rs`
- Modify: `crates/porthole-adapter-macos/src/artifact.rs`
- Modify: `crates/porthole-adapter-macos/src/enumerate.rs` (if `list_windows` is there)

- [ ] **Step 1: Update one function as a template — `screenshot`**

In `crates/porthole-adapter-macos/src/capture.rs`, change the signature and add the preflight call. Starting state (likely):

```rust
pub async fn screenshot(surface: &SurfaceInfo) -> Result<Screenshot, PortholeError> {
    // ...
}
```

Change to:

```rust
use crate::MacOsAdapter;
use crate::permissions::ensure_screen_recording_granted;

pub async fn screenshot(
    adapter: &MacOsAdapter,
    surface: &SurfaceInfo,
) -> Result<Screenshot, PortholeError> {
    ensure_screen_recording_granted(adapter)?;
    // ... existing body ...
}
```

Update the call site in `crates/porthole-adapter-macos/src/lib.rs`:

```rust
async fn screenshot(&self, surface: &SurfaceInfo) -> Result<Screenshot, PortholeError> {
    capture::screenshot(self, surface).await
}
```

- [ ] **Step 2: Repeat for each guarded method**

For each file listed in the mapping, make the same shape of change:
1. Add `use crate::MacOsAdapter;` and `use crate::permissions::{ensure_accessibility_granted, ensure_screen_recording_granted};` as needed.
2. Add `adapter: &MacOsAdapter` as the first argument to the exported function.
3. Call the appropriate preflight at the top, propagating `?`.
4. Update the trait impl call site in `lib.rs` to pass `self`.

For `wait`, **do not modify the adapter's `wait` function**. Instead, preflight in `WaitPipeline` before it dispatches to the adapter. In `crates/porthole-core/src/wait_pipeline.rs`, find the point where `self.adapter.wait(...)` is called (inside the `wait` method of `WaitPipeline`) and add preflight before it:

```rust
// before calling adapter.wait, preflight based on condition kind:
use crate::wait::WaitCondition as Wc;
let required: &[&str] = match condition {
    Wc::Stable { .. } | Wc::Dirty { .. } => &["screen_recording", "accessibility"],
    _ => &["accessibility"],
};
for name in required {
    self.adapter
        .ensure_system_permission(name)
        .await
        .map_err(WaitPipelineError::Porthole)?;
}
```

(Check the actual `WaitCondition` variants in `crates/porthole-core/src/wait.rs` and map accordingly — the spec's "stable/dirty" maps to whichever variants correspond to frame-diff waiting.)

For `displays` and `attention`, per §7.2 they're not strictly guarded (CG basics work unprivileged). Leave them alone.

For `system_permissions` (the adapter method), no preflight — reading grant state must work ungranted.

- [ ] **Step 3: Build**

Run: `cargo build -p porthole-adapter-macos`

Expected: clean. Fix each compile error as it surfaces.

- [ ] **Step 4: Run non-ignored tests**

Run: `cargo test -p porthole-adapter-macos`

Expected: all non-ignored tests pass.

- [ ] **Step 5: Commit**

```bash
git add -u
git commit -m "feat(adapter-macos): preflight every guarded method"
```

---

## Task 13: New route — `POST /system-permissions/request`

**Files:**
- Create: `crates/portholed/src/routes/system_permissions.rs`
- Modify: `crates/portholed/src/routes/mod.rs`
- Modify: `crates/portholed/src/server.rs`

- [ ] **Step 1: Create the route module**

Create `crates/portholed/src/routes/system_permissions.rs`:

```rust
use axum::extract::State;
use axum::Json;
use porthole_core::{ErrorCode, PortholeError};
use porthole_protocol::system_permission::SystemPermissionPromptOutcome;
use serde::Deserialize;

use crate::routes::errors::ApiError;
use crate::state::AppState;

#[derive(Deserialize)]
pub struct RequestBody {
    pub name: String,
}

pub async fn post_request(
    State(state): State<AppState>,
    Json(body): Json<RequestBody>,
) -> Result<Json<SystemPermissionPromptOutcome>, ApiError> {
    // Capability check first: if the adapter doesn't advertise
    // system_permission_prompt, return CapabilityMissing (501) without
    // dispatching.
    let caps = state.adapter.capabilities();
    if !caps.contains(&"system_permission_prompt") {
        return Err(ApiError::from(PortholeError::new(
            ErrorCode::CapabilityMissing,
            "adapter does not support system permission prompts",
        )));
    }

    let core_outcome = state
        .adapter
        .request_system_permission_prompt(&body.name)
        .await?;

    // Convert core type → wire type (identical fields; we re-shape to keep
    // the protocol crate as the single source of the wire types).
    Ok(Json(SystemPermissionPromptOutcome {
        permission: core_outcome.permission,
        granted_before: core_outcome.granted_before,
        granted_after: core_outcome.granted_after,
        prompt_triggered: core_outcome.prompt_triggered,
        requires_daemon_restart: core_outcome.requires_daemon_restart,
        notes: core_outcome.notes,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{to_bytes, Body};
    use axum::http::{Method, Request, StatusCode};
    use porthole_core::in_memory::InMemoryAdapter;
    use porthole_protocol::error::WireError;
    use std::sync::Arc;
    use tower::ServiceExt;

    use crate::server::build_router;

    async fn post_json(uri: &str, body: serde_json::Value) -> (StatusCode, serde_json::Value) {
        let adapter = Arc::new(InMemoryAdapter::new());
        let router = build_router(AppState::new(adapter));
        let req = Request::builder()
            .method(Method::POST)
            .uri(uri)
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        let res = router.oneshot(req).await.unwrap();
        let status = res.status();
        let bytes = to_bytes(res.into_body(), 1024 * 1024).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or(serde_json::json!({}));
        (status, json)
    }

    #[tokio::test]
    async fn in_memory_adapter_returns_capability_missing() {
        let (status, body) = post_json(
            "/system-permissions/request",
            serde_json::json!({ "name": "accessibility" }),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
        let err: WireError = serde_json::from_value(body).unwrap();
        assert_eq!(err.code, ErrorCode::CapabilityMissing);
    }
}
```

- [ ] **Step 2: Register the module**

In `crates/portholed/src/routes/mod.rs`, add:

```rust
pub mod system_permissions;
```

- [ ] **Step 3: Mount the route**

In `crates/portholed/src/server.rs`, import the new module in the `use crate::routes::{...}` block and add a route:

```rust
use crate::routes::system_permissions as system_permissions_route;
// ...
pub fn build_router(state: AppState) -> Router {
    Router::new()
        // ... existing routes ...
        .route(
            "/system-permissions/request",
            post(system_permissions_route::post_request),
        )
        .with_state(state)
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p portholed`

Expected: the new test passes. Existing tests still pass.

- [ ] **Step 5: Commit**

```bash
git add crates/portholed/src/routes/system_permissions.rs crates/portholed/src/routes/mod.rs crates/portholed/src/server.rs
git commit -m "feat(portholed): add POST /system-permissions/request"
```

---

## Task 14: Daemon startup — permission warnings

When the daemon starts, emit a `WARN` log for each missing system permission.

**Files:**
- Modify: `crates/portholed/src/main.rs`

- [ ] **Step 1: Find the startup path**

Read `crates/portholed/src/main.rs`. Locate where the adapter is constructed and `serve` is called.

- [ ] **Step 2: Add a permission check on startup**

Add, after the adapter is constructed but before `serve`:

```rust
use tracing::warn;
// ...
let perms = adapter.system_permissions().await.unwrap_or_default();
for p in &perms {
    if !p.granted {
        warn!(
            permission = %p.name,
            "{} system permission missing; calls that need it will return system_permission_needed. Run `porthole onboard` or see docs/development.md.",
            p.name
        );
    }
}
```

- [ ] **Step 3: Build**

Run: `cargo build -p portholed`

Expected: clean.

- [ ] **Step 4: Commit**

```bash
git add crates/portholed/src/main.rs
git commit -m "feat(portholed): warn on startup for missing system permissions"
```

---

## Task 15: CLI — add `Onboard` subcommand with blocking poll

The `porthole onboard` command reads `/info`, requests prompts for any ungranted permissions, polls until they're granted or a timeout elapses, and exits with a code per §5.4 of the spec.

**Files:**
- Create: `crates/porthole/src/commands/onboard.rs`
- Modify: `crates/porthole/src/commands/mod.rs`
- Modify: `crates/porthole/src/main.rs`

- [ ] **Step 1: Create the command module**

Create `crates/porthole/src/commands/onboard.rs`:

```rust
use std::time::{Duration, Instant};

use porthole_protocol::error::WireError;
use porthole_protocol::info::InfoResponse;
use porthole_protocol::system_permission::{
    SystemPermissionPromptOutcome, SystemPermissionRequestFailedBody,
};

use crate::client::{ClientError, DaemonClient};

pub struct OnboardOptions {
    pub wait_seconds: u64,
    pub no_wait: bool,
}

impl Default for OnboardOptions {
    fn default() -> Self {
        Self { wait_seconds: 60, no_wait: false }
    }
}

/// Return value carries the exit code the main binary should use.
pub struct OnboardResult {
    pub exit_code: i32,
}

pub async fn run(client: &DaemonClient, opts: OnboardOptions) -> Result<OnboardResult, ClientError> {
    // 1. Read initial /info.
    let info: InfoResponse = client.get_json("/info").await?;
    let Some(adapter) = info.adapters.into_iter().next() else {
        println!("no adapters loaded");
        return Ok(OnboardResult { exit_code: 0 });
    };
    let perms = adapter.system_permissions;
    if perms.is_empty() {
        println!("adapter {} advertises no system permissions; nothing to onboard", adapter.name);
        return Ok(OnboardResult { exit_code: 0 });
    }

    let granted_before: Vec<(String, bool)> = perms.iter().map(|p| (p.name.clone(), p.granted)).collect();
    let ungranted: Vec<String> = perms
        .iter()
        .filter(|p| !p.granted)
        .map(|p| p.name.clone())
        .collect();

    if ungranted.is_empty() {
        for p in &perms {
            println!("  system permission {}: granted", p.name);
        }
        return Ok(OnboardResult { exit_code: 0 });
    }

    // 2. Request prompts for each ungranted permission.
    let mut had_request_error = false;
    let mut restart_required_seen = false;
    for name in &ungranted {
        match client
            .post_json::<serde_json::Value, SystemPermissionPromptOutcome>(
                "/system-permissions/request",
                &serde_json::json!({ "name": name }),
            )
            .await
        {
            Ok(out) => {
                if out.requires_daemon_restart {
                    restart_required_seen = true;
                }
                if out.prompt_triggered {
                    println!("  dialog opened for {name}");
                } else {
                    println!(
                        "  prompt already fired earlier this daemon session for {name} — grant via {} (re-arm dialog by restarting the daemon)",
                        settings_path_fallback(name)
                    );
                }
            }
            Err(ClientError::Api(wire)) => {
                had_request_error = true;
                print_request_error(name, &wire);
            }
            Err(e) => {
                return Err(e);
            }
        }
    }

    // 3. Optionally skip polling.
    if opts.no_wait {
        return Ok(OnboardResult { exit_code: 3 });
    }

    // 4. Poll /info until all granted or timeout.
    let deadline = Instant::now() + Duration::from_secs(opts.wait_seconds);
    let final_info = poll_until_granted(client, deadline).await?;
    let final_perms = final_info
        .adapters
        .into_iter()
        .next()
        .map(|a| a.system_permissions)
        .unwrap_or_default();

    // 5. Summarise.
    let mut any_transition_requires_restart = false;
    for p in &final_perms {
        let before = granted_before
            .iter()
            .find(|(n, _)| n == &p.name)
            .map(|(_, b)| *b)
            .unwrap_or(false);
        let transitioned = !before && p.granted;
        let status = if p.granted { "granted" } else { "MISSING" };
        println!(
            "  system permission {}: {}{}",
            p.name,
            status,
            if transitioned { " (granted this session)" } else { "" }
        );
        if transitioned && restart_required_flag_for(&p.name) {
            any_transition_requires_restart = true;
        }
    }

    let any_still_missing = final_perms.iter().any(|p| !p.granted);

    // 6. Exit code.
    let exit_code = if any_still_missing || had_request_error {
        if any_still_missing {
            println!("\nAt least one permission is still ungranted. Grant in Settings and re-run `porthole onboard`.");
        }
        1
    } else if any_transition_requires_restart || restart_required_seen && granted_before.iter().any(|(n, b)| !b && restart_required_flag_for(n)) {
        println!("\nAll permissions granted. Restart the daemon before using Accessibility-dependent features.");
        2
    } else {
        0
    };

    Ok(OnboardResult { exit_code })
}

async fn poll_until_granted(
    client: &DaemonClient,
    deadline: Instant,
) -> Result<InfoResponse, ClientError> {
    let mut last_seen: Option<InfoResponse> = None;
    loop {
        let info: InfoResponse = client.get_json("/info").await?;
        let all_granted = info
            .adapters
            .first()
            .map(|a| a.system_permissions.iter().all(|p| p.granted))
            .unwrap_or(true);
        last_seen = Some(info);
        if all_granted || Instant::now() >= deadline {
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    Ok(last_seen.expect("at least one /info read"))
}

fn restart_required_flag_for(name: &str) -> bool {
    matches!(name, "accessibility")
}

fn settings_path_fallback(name: &str) -> &'static str {
    match name {
        "accessibility" => "System Settings → Privacy & Security → Accessibility",
        "screen_recording" => "System Settings → Privacy & Security → Screen Recording",
        _ => "System Settings → Privacy & Security",
    }
}

fn print_request_error(name: &str, err: &WireError) {
    eprintln!("  request failed for {name}: {} ({})", err.message, err.code);
    if let Some(details) = &err.details {
        if let Some(settings) = details.get("settings_path").and_then(|v| v.as_str()) {
            eprintln!("    grant manually: {settings}");
        }
        if let Some(reason) = details.get("reason").and_then(|v| v.as_str()) {
            eprintln!("    os reason: {reason}");
        }
    }
}
```

- [ ] **Step 2: Check `DaemonClient` API**

Read `crates/porthole/src/client.rs` to confirm the method names (`get_json`, `post_json`, `ClientError::Api(WireError)`). If the names differ, adapt the code above to match.

- [ ] **Step 3: Add `post_json` if missing**

If `DaemonClient::post_json<Req, Res>(path, body)` doesn't exist, add it mirroring `get_json`'s pattern. Most likely it does exist (other commands POST).

- [ ] **Step 4: Export the module**

In `crates/porthole/src/commands/mod.rs`, add:

```rust
pub mod onboard;
```

- [ ] **Step 5: Wire the subcommand in `main.rs`**

In `crates/porthole/src/main.rs`, add a variant to the `Command` enum:

```rust
/// Guided setup: trigger system-permission prompts and report status.
Onboard {
    /// Poll timeout in seconds (default 60).
    #[arg(long, default_value_t = 60)]
    wait: u64,
    /// Skip polling; exit immediately after firing prompts with code 3.
    #[arg(long)]
    no_wait: bool,
},
```

In the dispatch `match` block, add an arm:

```rust
Command::Onboard { wait, no_wait } => {
    let client = DaemonClient::new(socket_path()?);
    let result = porthole::commands::onboard::run(
        &client,
        porthole::commands::onboard::OnboardOptions {
            wait_seconds: wait,
            no_wait,
        },
    )
    .await?;
    std::process::exit(result.exit_code);
}
```

- [ ] **Step 6: Build**

Run: `cargo build -p porthole`

Expected: clean.

- [ ] **Step 7: Commit**

```bash
git add crates/porthole/src/commands/onboard.rs crates/porthole/src/commands/mod.rs crates/porthole/src/main.rs
git commit -m "feat(cli): add porthole onboard subcommand"
```

---

## Task 16: Tests for `porthole onboard` against a fake daemon

Rather than spin up a real daemon, factor the onboard logic so it takes a trait-object `&dyn OnboardClient` with `get_info` and `request_prompt` methods. The CLI binary wires it to `DaemonClient`; tests wire it to a scripted fake.

**Files:**
- Modify: `crates/porthole/src/commands/onboard.rs`

- [ ] **Step 1: Extract the daemon-shaped surface into a trait**

At the top of `onboard.rs`, add:

```rust
use async_trait::async_trait;
// ... existing imports ...

#[async_trait]
pub trait OnboardClient: Send + Sync {
    async fn get_info(&self) -> Result<InfoResponse, ClientError>;
    async fn request_prompt(
        &self,
        name: &str,
    ) -> Result<SystemPermissionPromptOutcome, ClientError>;
}

#[async_trait]
impl OnboardClient for DaemonClient {
    async fn get_info(&self) -> Result<InfoResponse, ClientError> {
        self.get_json("/info").await
    }
    async fn request_prompt(
        &self,
        name: &str,
    ) -> Result<SystemPermissionPromptOutcome, ClientError> {
        self.post_json(
            "/system-permissions/request",
            &serde_json::json!({ "name": name }),
        )
        .await
    }
}
```

Replace `client: &DaemonClient` in `run` and `poll_until_granted` with `client: &dyn OnboardClient`, and update the call sites to use the two trait methods. Keep a `Clock` abstraction for `Instant::now()` out of scope; the 500 ms sleep is fine as-is because tests use small wait windows (`wait_seconds: 1`).

- [ ] **Step 2: Make sure `async-trait` is a dep**

Check `crates/porthole/Cargo.toml`. If not, add:

```toml
async-trait = { workspace = true }
```

- [ ] **Step 3: Write fake-daemon tests**

Append to `onboard.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use porthole_core::ErrorCode;
    use porthole_protocol::info::{AdapterInfo, SystemPermissionStatus};
    use std::sync::{Arc, Mutex};

    // ClientError isn't Clone, so the fake stores WireError (which is Clone)
    // and reconstructs ClientError::Api on demand.
    struct FakeClient {
        info_sequence: Mutex<Vec<InfoResponse>>,
        prompt_results: Mutex<Vec<Result<SystemPermissionPromptOutcome, WireError>>>,
    }

    #[async_trait]
    impl OnboardClient for FakeClient {
        async fn get_info(&self) -> Result<InfoResponse, ClientError> {
            let mut q = self.info_sequence.lock().unwrap();
            Ok(if q.len() > 1 { q.remove(0) } else { q[0].clone() })
        }
        async fn request_prompt(
            &self,
            _name: &str,
        ) -> Result<SystemPermissionPromptOutcome, ClientError> {
            let mut q = self.prompt_results.lock().unwrap();
            let item = if q.len() > 1 { q.remove(0) } else { q[0].clone() };
            item.map_err(ClientError::Api)
        }
    }

    fn info_with(perms: Vec<(&str, bool)>) -> InfoResponse {
        InfoResponse {
            daemon_version: "test".into(),
            uptime_seconds: 0,
            adapters: vec![AdapterInfo {
                name: "fake".into(),
                loaded: true,
                capabilities: vec!["system_permission_prompt".into()],
                system_permissions: perms
                    .into_iter()
                    .map(|(n, g)| SystemPermissionStatus {
                        name: n.into(),
                        granted: g,
                        purpose: String::new(),
                    })
                    .collect(),
            }],
        }
    }

    fn outcome(name: &str, granted_after: bool, prompt_triggered: bool, requires_restart: bool) -> SystemPermissionPromptOutcome {
        SystemPermissionPromptOutcome {
            permission: name.into(),
            granted_before: false,
            granted_after,
            prompt_triggered,
            requires_daemon_restart: requires_restart,
            notes: String::new(),
        }
    }

    #[tokio::test]
    async fn all_granted_at_start_exits_zero() {
        let client = FakeClient {
            info_sequence: Mutex::new(vec![info_with(vec![("accessibility", true), ("screen_recording", true)])]),
            prompt_results: Mutex::new(vec![]),
        };
        let res = run(&client, OnboardOptions { wait_seconds: 1, no_wait: false }).await.unwrap();
        assert_eq!(res.exit_code, 0);
    }

    #[tokio::test]
    async fn ax_transition_to_granted_exits_two() {
        let client = FakeClient {
            info_sequence: Mutex::new(vec![
                info_with(vec![("accessibility", false), ("screen_recording", true)]),
                info_with(vec![("accessibility", true), ("screen_recording", true)]),
            ]),
            prompt_results: Mutex::new(vec![Ok(outcome("accessibility", false, true, true))]),
        };
        let res = run(&client, OnboardOptions { wait_seconds: 1, no_wait: false }).await.unwrap();
        assert_eq!(res.exit_code, 2);
    }

    #[tokio::test]
    async fn screen_recording_transition_exits_zero() {
        let client = FakeClient {
            info_sequence: Mutex::new(vec![
                info_with(vec![("screen_recording", false)]),
                info_with(vec![("screen_recording", true)]),
            ]),
            prompt_results: Mutex::new(vec![Ok(outcome("screen_recording", false, true, false))]),
        };
        let res = run(&client, OnboardOptions { wait_seconds: 1, no_wait: false }).await.unwrap();
        assert_eq!(res.exit_code, 0);
    }

    #[tokio::test]
    async fn still_ungranted_after_poll_exits_one() {
        let client = FakeClient {
            info_sequence: Mutex::new(vec![info_with(vec![("accessibility", false)])]),
            prompt_results: Mutex::new(vec![Ok(outcome("accessibility", false, true, true))]),
        };
        let res = run(&client, OnboardOptions { wait_seconds: 1, no_wait: false }).await.unwrap();
        assert_eq!(res.exit_code, 1);
    }

    #[tokio::test]
    async fn no_wait_exits_three_without_polling() {
        let client = FakeClient {
            info_sequence: Mutex::new(vec![info_with(vec![("accessibility", false)])]),
            prompt_results: Mutex::new(vec![Ok(outcome("accessibility", false, true, true))]),
        };
        let res = run(&client, OnboardOptions { wait_seconds: 1, no_wait: true }).await.unwrap();
        assert_eq!(res.exit_code, 3);
    }

    #[tokio::test]
    async fn request_error_forces_exit_one_even_if_info_shows_granted() {
        let wire = WireError {
            code: ErrorCode::SystemPermissionRequestFailed,
            message: "bundle missing".into(),
            details: Some(serde_json::json!({
                "permission": "accessibility",
                "reason": "not in bundle",
                "settings_path": "Settings → ...",
                "binary_path": "/x"
            })),
        };
        let client = FakeClient {
            info_sequence: Mutex::new(vec![
                info_with(vec![("accessibility", false)]),
                info_with(vec![("accessibility", true)]),
            ]),
            prompt_results: Mutex::new(vec![Err(wire)]),
        };
        let res = run(&client, OnboardOptions { wait_seconds: 1, no_wait: false }).await.unwrap();
        assert_eq!(res.exit_code, 1);
    }
}
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -p porthole onboard`

Expected: all six pass.

- [ ] **Step 5: Commit**

```bash
git add crates/porthole/src/commands/onboard.rs crates/porthole/Cargo.toml
git commit -m "test(cli): cover onboard exit-code paths with a fake client"
```

---

## Task 17: `porthole info` — show remediation hint when permission missing

**Files:**
- Modify: `crates/porthole/src/commands/info.rs`

- [ ] **Step 1: Update the print loop**

Replace the info-printing loop in `crates/porthole/src/commands/info.rs`:

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
            adapter.capabilities.join(","),
        );
        for perm in &adapter.system_permissions {
            if perm.granted {
                println!("  system permission {}: granted ({})", perm.name, perm.purpose);
            } else {
                let restart_hint = if perm.name == "accessibility" {
                    "  (will trigger the OS prompt; daemon restart required after grant)"
                } else {
                    "  (will trigger the OS prompt)"
                };
                println!(
                    "  system permission {}: MISSING ({})",
                    perm.name, perm.purpose
                );
                println!("    fix: porthole onboard{restart_hint}");
            }
        }
    }
    Ok(())
}
```

- [ ] **Step 2: Build**

Run: `cargo build -p porthole`

Expected: clean.

- [ ] **Step 3: Commit**

```bash
git add crates/porthole/src/commands/info.rs
git commit -m "feat(cli): add remediation hint to info output for missing permissions"
```

---

## Task 18: Update `AGENTS.md` remediation command

**Files:**
- Modify: `AGENTS.md`

- [ ] **Step 1: Replace the outdated command reference**

In `AGENTS.md`, change the line mentioning `porthole request-permission <name>` to `porthole onboard`. The file currently has something like:

```
- **Do** stop with status `BLOCKED`, state which permission is missing, and wait for the user to grant it (via `porthole request-permission <name>` once that ships, or manually in System Settings → Privacy & Security).
```

Change to:

```
- **Do** stop with status `BLOCKED`, state which permission is missing, and wait for the user to grant it (via `porthole onboard`, or manually in System Settings → Privacy & Security).
```

- [ ] **Step 2: Commit**

```bash
git add AGENTS.md
git commit -m "docs(agents): point permissions-blocker rule at porthole onboard"
```

---

## Task 19: Cross-route integration test — preflight produces remediation

**Files:**
- Modify: `crates/portholed/src/server.rs` (add test)

This test exercises the full path: scripted in-memory grant state → preflight stub returning `system_permission_needed` → route-level `WireError` with populated remediation. Since the in-memory adapter doesn't actually preflight, we need a different scripting mechanism. Options:

- a) Create a test-only adapter that wraps InMemory and applies preflight on specified methods.
- b) Just test the route-wrapper merge rule (Task 3 covered this already) and the wire shape (Task 4).
- c) Skip direct integration testing here — rely on macOS-adapter-level `#[ignore]` tests and unit tests.

Pick **(c)** — the existing unit tests at each layer cover the contract; an end-to-end integration test against in-memory would need a non-trivial test adapter for a single wire-level check.

- [ ] **Step 1: No-op**

Mark this task complete. Coverage comes from Tasks 3, 4, 10, 11, 13, 16.

---

## Task 20: Dev bundle script

**Files:**
- Create: `scripts/dev-bundle.sh`

- [ ] **Step 1: Write the script**

Create `scripts/dev-bundle.sh`:

```bash
#!/usr/bin/env bash
# Build porthole and wrap `portholed` in a .app bundle with ad-hoc codesigning.
# The bundle gives TCC a stable identity across rebuilds, so grants stick.

set -euo pipefail

PROFILE="debug"
REFRESH_ONLY=0
BUNDLE_ID="org.flotilla.porthole.dev"

usage() {
    cat <<EOF
Usage: $0 [--release] [--refresh]

  --release   Build release profile (default: debug).
  --refresh   Don't rebuild; just re-copy the binary into the existing bundle
              and re-sign. Use after cargo build to keep TCC grants.
EOF
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --release) PROFILE="release"; shift ;;
        --refresh) REFRESH_ONLY=1; shift ;;
        -h|--help) usage; exit 0 ;;
        *) echo "unknown arg: $1" >&2; usage; exit 1 ;;
    esac
done

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"

if [[ "$REFRESH_ONLY" -eq 0 ]]; then
    if [[ "$PROFILE" == "release" ]]; then
        cargo build --workspace --release
    else
        cargo build --workspace
    fi
fi

BIN="target/$PROFILE/portholed"
if [[ ! -f "$BIN" ]]; then
    echo "missing binary: $BIN" >&2
    exit 1
fi

APP="target/$PROFILE/Portholed.app"
mkdir -p "$APP/Contents/MacOS"

cat > "$APP/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleIdentifier</key>
    <string>$BUNDLE_ID</string>
    <key>CFBundleName</key>
    <string>Portholed</string>
    <key>CFBundleExecutable</key>
    <string>portholed</string>
    <key>CFBundleVersion</key>
    <string>0.0.0-dev</string>
    <key>CFBundleShortVersionString</key>
    <string>0.0.0-dev</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>LSBackgroundOnly</key>
    <true/>
</dict>
</plist>
EOF

cp "$BIN" "$APP/Contents/MacOS/portholed"
chmod +x "$APP/Contents/MacOS/portholed"

codesign -s - --force --deep "$APP"

echo "bundle built: $APP"
echo "launch it: \"$APP/Contents/MacOS/portholed\""
echo "run onboarding: ./target/$PROFILE/porthole onboard"
```

- [ ] **Step 2: Make executable and test**

```bash
chmod +x scripts/dev-bundle.sh
./scripts/dev-bundle.sh
```

Expected: the script builds the workspace, writes `target/debug/Portholed.app`, signs it, and prints next-step hints.

- [ ] **Step 3: Verify signing**

Run: `codesign -v target/debug/Portholed.app && echo ok`

Expected: "ok" (codesign verification succeeds).

- [ ] **Step 4: Verify bundle runs**

Run: `./target/debug/Portholed.app/Contents/MacOS/portholed --help`

Expected: help output or at least no error about missing binary (the daemon may not have a `--help`; any non-panic is acceptable).

- [ ] **Step 5: Refresh mode test**

Run: `touch crates/portholed/src/main.rs && cargo build -p portholed && ./scripts/dev-bundle.sh --refresh`

Expected: the binary inside the bundle is replaced; codesign still valid.

- [ ] **Step 6: Commit**

```bash
git add scripts/dev-bundle.sh
git commit -m "feat(tooling): add scripts/dev-bundle.sh for TCC-stable dev bundle"
```

---

## Task 21: Development playbook doc

**Files:**
- Create: `docs/development.md`

- [ ] **Step 1: Write the playbook**

Create `docs/development.md`:

```markdown
# Porthole development playbook

This covers first-time setup, day-to-day workflow, and what to do when grants go sideways.

## First-time setup

Porthole's macOS adapter needs **Accessibility** and **Screen Recording** system permissions. Grants are tied to a binary's code signature + path; the dev bundle gives a stable identity so grants persist across rebuilds.

```sh
git clone <repo>
cd porthole
cargo build --workspace --release
./scripts/dev-bundle.sh --release
open -R target/release/Portholed.app    # reveal in Finder
./target/release/Portholed.app/Contents/MacOS/portholed &
./target/release/porthole onboard
```

`porthole onboard` does three things:
1. Reads `/info` to see which permissions are ungranted.
2. Asks the daemon to trigger the OS prompt for each one (Settings opens automatically).
3. Polls until granted or 60 seconds pass, then prints a summary.

Exit codes:
- **0** — all granted already; no action taken.
- **1** — at least one permission still ungranted after waiting. Grant in Settings and re-run.
- **2** — all granted now, but **restart the daemon**. Accessibility grants need a process restart for the AX runtime to pick them up.
- **3** — `--no-wait` mode; prompts fired, grant state unknown.

## Rebuild workflow

Cargo replaces `target/<profile>/portholed` but the bundle's copy is stale. Two options:

```sh
./scripts/dev-bundle.sh --refresh    # re-copy and re-sign; keeps TCC grants
```

or just `cargo build` and run the binary from `target/<profile>/portholed` directly — but that's a *different* path from TCC's perspective, so you'll be prompted to grant again. Prefer the bundle.

## If grants get stuck

macOS's TCC database can report stale state after crashes, force-quits, or bundle-identity changes. Reset:

```sh
tccutil reset Accessibility org.flotilla.porthole.dev
tccutil reset ScreenCapture org.flotilla.porthole.dev
./scripts/dev-bundle.sh --refresh
./target/debug/Portholed.app/Contents/MacOS/portholed &
./target/debug/porthole onboard
```

## Debug vs release bundle

They're separate TCC identities. If you switch frequently, grant both. Or stick to one profile and refresh it on rebuild.

## Integration tests

Tests marked `#[ignore]` in `porthole-adapter-macos` run against a real desktop session. Execute with:

```sh
cargo test -p porthole-adapter-macos -- --ignored
```

These tests use whatever daemon is currently running (or spawn their own from `CARGO_BIN_EXE_portholed` — a different path and thus a different TCC identity). Run the bundled daemon for the realistic path.

## What *not* to do when permissions are missing

Per `AGENTS.md`: stop, state the missing permission, tell the user to run `porthole onboard`, wait. Do not build mock layers, feature flags, or "degrade to empty" paths. Preflight returns `system_permission_needed` with remediation — surface that, don't route around it.
```

- [ ] **Step 2: Commit**

```bash
git add docs/development.md
git commit -m "docs: add development playbook for permission setup"
```

---

## Task 22: Test script — verify `dev-bundle.sh`

**Files:**
- Create: `scripts/tests/test-dev-bundle.sh`

Optional but nice: a script that's easy to run by hand to verify the bundle still builds. Don't wire into CI (building a .app isn't useful in CI on Linux).

- [ ] **Step 1: Write the test script**

Create `scripts/tests/test-dev-bundle.sh`:

```bash
#!/usr/bin/env bash
set -euo pipefail
REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$REPO_ROOT"
./scripts/dev-bundle.sh
codesign -v target/debug/Portholed.app
./target/debug/Portholed.app/Contents/MacOS/portholed --help > /dev/null 2>&1 || true
echo "dev-bundle: ok"
```

- [ ] **Step 2: Make executable and run**

```bash
chmod +x scripts/tests/test-dev-bundle.sh
./scripts/tests/test-dev-bundle.sh
```

Expected: "dev-bundle: ok".

- [ ] **Step 3: Commit**

```bash
git add scripts/tests/test-dev-bundle.sh
git commit -m "test(tooling): add dev-bundle smoke test"
```

---

## Task 23: Final validation

- [ ] **Step 1: Clean build**

Run: `cargo clean && cargo build --workspace`

Expected: clean build across all crates.

- [ ] **Step 2: Full test run**

Run: `cargo test --workspace`

Expected: all tests pass. Ignored tests remain ignored.

- [ ] **Step 3: Manual smoke**

```bash
./scripts/dev-bundle.sh
./target/debug/Portholed.app/Contents/MacOS/portholed &
./target/debug/porthole info
./target/debug/porthole onboard
```

Expected:
- `info` prints adapter info, system_permissions state, and remediation hint (`fix: porthole onboard`) when a permission is missing.
- `onboard` walks through the prompts, or exits 0 if already granted.
- If Accessibility transitions from missing → granted, exit code is 2 with restart advice.

- [ ] **Step 4: Cross-check spec coverage**

Open `docs/superpowers/specs/2026-04-23-porthole-permissions-slice-design.md` alongside this plan. Confirm each section's requirements are covered by at least one task.

- [ ] **Step 5: Mark slice complete**

```bash
git log --oneline main..HEAD   # see all slice commits
```

Expected: ~22 commits spanning the tasks above, each atomic and self-contained.

---

## Cross-references

- Spec: `docs/superpowers/specs/2026-04-23-porthole-permissions-slice-design.md`
- Agent blocker rule: `AGENTS.md`
- Dev playbook (created in Task 21): `docs/development.md`
