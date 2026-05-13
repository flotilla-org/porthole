# Capture Transfer Daemon SHM Leases Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make daemon-backed CPU shared-memory frames safe to serve from reusable pool slots by keeping acquired frames pinned until the fd-side-channel connection closes.

**Architecture:** Reuse the current one-request-per-frame Unix socket connection as the first cross-process lease. The daemon acquires the latest frame, sends the fd plus metadata, then keeps the acquired frame pinned while the connection remains open. The consumer library keeps that socket alive inside `DaemonFrame` and drops it after unmapping the frame. This is still not a shared ring/control-block protocol, but it gives reusable daemon pools a real release boundary.

**Tech Stack:** Rust workspace, Unix-domain socket fd passing, existing `VideoSlotManager` pin/release model, serde JSON frame metadata.

---

## Scope

This plan implements:

- daemon frame lease lifetime tied to fd-side-channel connection lifetime
- daemon-backed sessions using `VideoSlotManager::new_reusable_pool`
- tests that prove frames remain pinned until release
- documentation of connection-close release semantics

This plan does not implement:

- explicit release request ids
- long-lived streaming subscriptions
- shared rings or wake primitives
- pool registration messages
- GPU/native handle release fences

## File Structure

- Modify `crates/portholed/src/capture_registry.rs`
  - Return acquired frame ownership from `latest_frame`.
  - Release that frame only after the fd connection closes or errors.
  - Use reusable pool storage for synthetic and surface capture sessions.
- Modify `crates/capture-transfer/src/daemon.rs`
  - Store the fd-side-channel socket in `DaemonFrame` so dropping the frame closes the lease.
- Modify `crates/portholed/src/server.rs`
  - Update tests for pooled daemon frames and offset-based reads.
- Modify `docs/superpowers/specs/2026-05-12-capture-transfer-protocol-design.md`
  - Replace the prior immutable-daemon caveat with connection-lifetime lease semantics.

## Chunk 1: Lease Lifetime

### Task 1: Add pin lifetime tests

- [ ] **Step 1: Write failing test**

Add a `capture_registry` unit test proving `latest_frame` leaves the acquired frame pinned until it is explicitly released.

- [ ] **Step 2: Run focused test**

Run:

```bash
cargo test -p portholed capture_registry::tests::latest_frame_reply_keeps_frame_pinned_until_release --locked
```

Expected: FAIL because `latest_frame` currently releases immediately.

- [ ] **Step 3: Implement lease ownership**

Return `AcquiredVideoFrame` in `LatestFrameReply` and add a registry helper to release it.

### Task 2: Hold the socket from consumer side

- [ ] **Step 1: Write/adjust tests**

Ensure daemon frame acquisition tests still pass while the server waits for connection close.

- [ ] **Step 2: Implement socket ownership**

Store the `UnixStream` in `DaemonFrame`; drop order should unmap first, then close the socket.

### Task 3: Enable reusable daemon pools

- [ ] **Step 1: Switch session storage**

Create synthetic and surface sessions with `VideoSlotManager::new_reusable_pool(3)`.

- [ ] **Step 2: Update daemon tests**

Daemon tests should read from `payload_offset..payload_offset + payload_len`, not assume offset zero.

## Final Verification

- [ ] Run `cargo build --workspace --locked`
- [ ] Run `cargo test --workspace --locked`
- [ ] Run `cargo clippy --workspace --all-targets --locked -- -D warnings`
- [ ] Run `cargo +nightly-2026-03-12 fmt --check`
