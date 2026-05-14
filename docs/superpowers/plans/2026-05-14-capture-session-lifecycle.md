# Capture Session Lifecycle Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make capture sessions expose explicit readiness state and add a clean close path without changing the current fd-per-frame transfer.

**Architecture:** `portholed` remains the lifecycle owner. Protocol responses gain session status fields, the registry tracks `starting`/`ready`/`failed`, and `DELETE /capture-sessions/{id}` removes the session so native capture handles and background tasks are dropped. The current fd sidecar and shm pool behavior stay unchanged.

**Tech Stack:** Rust, axum, tokio, porthole protocol structs, capture-transfer CPU shm pools.

---

## Chunk 1: Protocol Status Shape

### Task 1: Add capture session status fields

**Files:**
- Modify: `crates/porthole-protocol/src/capture_sessions.rs`
- Test through existing daemon/server tests in later chunks.

- [ ] **Step 1: Add status constants or enum-equivalent strings**

Add fields to both response structs:

```rust
pub status: String,
pub status_message: Option<String>,
```

Keep `LatestVideoFrameRequest` and `LatestVideoFrameResponse` unchanged.

- [ ] **Step 2: Run the narrow compile check**

Run:

```sh
cargo test -p porthole-protocol --locked
```

Expected: compile failures in `portholed` call sites until Chunk 2 fills the new fields.

## Chunk 2: Registry Lifecycle State

### Task 2: Introduce registry status and ready checks

**Files:**
- Modify: `crates/portholed/src/capture_registry.rs`
- Test: `crates/portholed/src/capture_registry.rs` unit tests

- [ ] **Step 1: Add a failing unit test for not-ready latest-frame behavior**

Add a test that inserts a `CaptureSession` with status `Starting` and an empty
`VideoSlotManager`, then calls `latest_frame`. It should assert the error is
`CaptureRegistryError::NotReady`.

- [ ] **Step 2: Add lifecycle state**

Add:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
enum CaptureSessionLifecycle {
    Starting,
    Ready,
    Failed(String),
}
```

Add helpers:

```rust
fn status_name(&self) -> &'static str;
fn status_message(&self) -> Option<String>;
fn is_ready(&self) -> bool;
```

Add `lifecycle: CaptureSessionLifecycle` to `CaptureSession`.

- [ ] **Step 3: Add registry errors**

Add:

```rust
NotReady { session_id: String, status: &'static str },
Failed { session_id: String, message: String },
```

`latest_frame` should reject non-ready sessions before calling
`VideoSlotManager::acquire_latest`.

- [ ] **Step 4: Make publisher first-frame commit mark ready**

In `RegistryVideoFramePublisher::publish_frame`, after committing the frame and
updating dimensions, set `session.lifecycle = CaptureSessionLifecycle::Ready`.

- [ ] **Step 5: Make synthetic and owned-frame sessions start ready**

Set lifecycle to `Ready` when constructing synthetic and owned-frame sessions.
Publisher sessions start as `Starting`.

- [ ] **Step 6: Run registry tests**

Run:

```sh
cargo test -p portholed capture_registry --locked
```

Expected: new and existing tests pass.

## Chunk 3: Close Session API

### Task 3: Add explicit remove/close route

**Files:**
- Modify: `crates/portholed/src/capture_registry.rs`
- Modify: `crates/portholed/src/routes/capture_sessions.rs`
- Modify: `crates/portholed/src/server.rs`
- Test: `crates/portholed/src/server.rs`

- [ ] **Step 1: Write failing server test**

Add a test that creates a synthetic session, sends:

```http
DELETE /capture-sessions/{session_id}
```

and verifies:

- response status is 200
- follow-up `GET /capture-sessions/{session_id}` is 404

- [ ] **Step 2: Add registry close method**

Expose:

```rust
pub fn close_session(&self, session_id: &str) -> Result<(), CaptureRegistryError>
```

It removes the session or returns `UnknownSession`.

- [ ] **Step 3: Abort owned-frame tasks on drop**

Implement `Drop for CaptureSession`:

```rust
if let Some(task) = self._capture_task.take() {
    task.abort();
}
```

This makes `close_session` stop owned-frame capture instead of detaching the
task.

- [ ] **Step 4: Add axum route**

Add:

```rust
.route("/capture-sessions/{id}", get(capture_sessions_route::get_session).delete(capture_sessions_route::delete_session))
```

and implement `delete_session`.

- [ ] **Step 5: Run server tests**

Run:

```sh
cargo test -p portholed --locked
```

Expected: server and capture registry tests pass.

## Chunk 4: CLI Close Command

### Task 4: Expose session close in CLI

**Files:**
- Modify: `crates/porthole/src/main.rs` or command enum file if capture-session subcommands live there
- Modify: `crates/porthole/src/commands/capture_session.rs`
- Test: existing CLI compile plus any capture session CLI tests

- [ ] **Step 1: Inspect current capture-session command wiring**

Use `rg -n "CaptureSession|capture-session|Subcommand" crates/porthole/src`.

- [ ] **Step 2: Add command handler**

Add:

```sh
porthole capture-session close <session_id>
```

Handler calls `DaemonClient::delete` or adds a small delete helper if the client
does not have one yet.

- [ ] **Step 3: Keep output terse**

Print:

```text
closed capture session <session_id>
```

No JSON mode needed unless existing capture-session subcommands already expose
one uniformly.

- [ ] **Step 4: Run CLI tests**

Run:

```sh
cargo test -p porthole --locked
```

Expected: CLI tests pass.

## Chunk 5: Full Verification and Manual Smoke

### Task 5: Verify no regression in the live CPU path

**Files:**
- No intended source changes.

- [ ] **Step 1: Run required gates**

Run:

```sh
cargo build --workspace --locked
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo +nightly-2026-03-12 fmt --check
```

Expected: all pass.

- [ ] **Step 2: Run dev bundle signing regression**

Run:

```sh
./scripts/tests/test-dev-bundle.sh
```

Expected: signs with Apple Development identity and exits 0.

- [ ] **Step 3: Run Simulator smoke if a Simulator surface is present**

Run:

```sh
porthole search --app-name Simulator --json
./scripts/manual-capture-transfer-smoke.sh --surface-id <surface-id> --frames 120
```

Expected: viewer consumes the requested frames and exits 0.

- [ ] **Step 4: Commit**

Commit message:

```sh
git commit -m "feat(capture): add session lifecycle close path"
```
