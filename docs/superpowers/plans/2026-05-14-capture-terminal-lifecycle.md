# Capture Terminal Lifecycle Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make capture sessions transition predictably when startup is cancelled, a producer ends, or a producer fails.

**Architecture:** Keep explicit `DELETE /capture-sessions/{id}` as removal. Add a `closed` lifecycle for sessions whose producer/source ended while still inspectable. Publisher and owned-frame captures are both supervised by registry-owned tasks so native handles are dropped on close and producer errors update registry state.

**Tech Stack:** Rust, Tokio, Axum, porthole capture registry, existing `capture-transfer` shm leasing.

---

## Chunk 1: Registry Terminal States

### Task 1: Add closed lifecycle behavior

**Files:**
- Modify: `crates/portholed/src/capture_registry.rs`
- Test: `crates/portholed/src/capture_registry.rs`

- [x] Add a failing unit test that inserts a `Closed("capture stream ended")` session and asserts `latest_frame` rejects it before track lookup.
- [x] Run `cargo test -p portholed capture_registry::tests::latest_frame_rejects_closed_session_before_track_lookup --locked` and verify it fails to compile or fails on missing behavior.
- [x] Add `CaptureSessionLifecycle::Closed(String)` and `CaptureRegistryError::Closed`.
- [x] Map `closed` to status name/message and reject it in `latest_frame`.
- [x] Run the targeted test and verify it passes.

### Task 2: Mark owned-frame producer end as closed

**Files:**
- Modify: `crates/portholed/src/capture_registry.rs`
- Test: `crates/portholed/src/capture_registry.rs`

- [x] Add a failing async unit test around an owned-frame capture loop whose `next_frame()` returns `Ok(None)`.
- [x] Run the targeted test and verify red.
- [x] Extract the owned-frame background loop into a small helper and make `Ok(None)` call `mark_session_closed`.
- [x] Add the matching error-path test if the existing failed-session test does not cover producer failure.
- [x] Run `cargo test -p portholed capture_registry --locked`.

## Chunk 2: Startup Cancellation and Publisher Supervision

### Task 3: Wake startup waiters on close

**Files:**
- Modify: `crates/portholed/src/capture_registry.rs`
- Test: `crates/portholed/src/capture_registry.rs`

- [x] Add a failing unit test proving `close_session` sends a stored startup cancellation signal.
- [x] Add a `startup_cancel` sender to `CaptureSession`, sent from `Drop`.
- [x] In publisher startup, wait on first-frame, cancellation, or timeout.
- [x] After first-frame readiness, re-check that the session still exists before returning success.
- [x] Run targeted registry tests.

### Task 4: Supervise publisher session handles

**Files:**
- Modify: `crates/portholed/src/capture_registry.rs`
- Test: `crates/portholed/src/capture_registry.rs`

- [x] Add a failing async unit test for a shared capture-session monitor whose `next_frame()` returns an error and assert the registry marks the session failed.
- [x] Move publisher capture ownership into a `capture_task` instead of a passive handle field.
- [x] Make publisher monitor treat `Ok(None)` as `closed`, `Err(_)` as `failed`, and any unexpected owned frame as a normal publish.
- [x] Remove the no-longer-needed capture handle wrapper if unused.
- [x] Run `cargo test -p portholed capture_registry --locked`.

## Chunk 3: Routes, Docs, and Gates

### Task 5: Update route error mapping and docs

**Files:**
- Modify: `crates/portholed/src/routes/capture_sessions.rs`
- Modify: `docs/superpowers/specs/2026-05-14-capture-session-lifecycle-design.md`

- [x] Add/adjust route test coverage for closed-session error mapping if needed.
- [x] Map `CaptureRegistryError::Closed` to `invalid_argument` for now.
- [x] Document `closed`, startup cancellation, and producer supervision as this slice's concrete behavior.

### Task 6: Run full verification

**Files:**
- No edits.

- [x] Run `cargo +nightly-2026-03-12 fmt`.
- [x] Run `cargo build --workspace --locked`.
- [x] Run `cargo test --workspace --locked`.
- [x] Run `cargo clippy --workspace --all-targets --locked -- -D warnings`.
- [x] Run `cargo +nightly-2026-03-12 fmt --check`.
