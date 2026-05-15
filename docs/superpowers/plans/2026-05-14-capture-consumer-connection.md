# Capture Consumer Connection Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace one fd-side-channel connection per frame with one long-lived daemon consumer connection and explicit frame lease ids.

**Architecture:** The daemon fd side channel becomes a small line-delimited JSON protocol with `latest_video_frame` and `release_video_frame` operations. Each connection owns one stable `ConsumerId`, and the daemon tracks outstanding `lease_id -> AcquiredVideoFrame` entries until explicit release or disconnect cleanup.

**Tech Stack:** Rust, Unix-domain sockets, `SCM_RIGHTS`, serde JSON, existing `capture-transfer` C ABI.

---

## Chunk 1: Registry Lease Primitives

### Task 1: Stable consumer acquisition

**Files:**
- Modify: `crates/portholed/src/capture_registry.rs`
- Test: `crates/portholed/src/capture_registry.rs`

- [x] Add a failing unit test proving two acquisitions with the same `ConsumerId` update `consumer_skipped_count`.
- [x] Add `allocate_consumer_id`, `latest_frame_for_consumer`, and `disconnect_consumer`.
- [x] Keep `latest_frame` as a private compatibility wrapper only if needed by tests.
- [x] Run `cargo test -p portholed capture_registry --locked`.

## Chunk 2: Long-Lived Side Channel

### Task 2: Multiple frame requests on one socket

**Files:**
- Modify: `crates/portholed/src/capture_registry.rs`
- Modify: `crates/porthole-protocol/src/capture_sessions.rs`
- Test: `crates/portholed/src/server.rs`

- [x] Add `lease_id` to `LatestVideoFrameResponse`.
- [x] Add a failing server test that sends two `latest_video_frame` requests on one UDS connection and releases each lease.
- [x] Replace `handle_fd_connection`'s one-shot request handling with a loop.
- [x] Track `BTreeMap<u64, (String, AcquiredVideoFrame)>` leases per connection.
- [x] On release, remove the lease and call `registry.release_frame`.
- [x] On disconnect, release all outstanding leases and call `registry.disconnect_consumer`.
- [x] Run targeted server tests.

## Chunk 3: Daemon Client and C ABI

### Task 3: Hold the daemon connection in the consumer

**Files:**
- Modify: `crates/capture-transfer/src/daemon.rs`
- Modify: `crates/capture-transfer/src/ffi.rs`
- Test: `crates/capture-transfer/src/daemon.rs`
- Test: `crates/capture-transfer/src/ffi.rs`

- [x] Add a `DaemonConsumer` type that connects once to `SessionInfo.fd_socket_path`.
- [x] Move frame acquisition to `DaemonConsumer::latest_frame`.
- [x] Add `DaemonConsumer::release_frame`.
- [x] Update `FtConsumerKind::Daemon` to store `DaemonConsumer`.
- [x] Update `ft_consumer_acquire_latest_video_frame` and `ft_consumer_release_video_frame`.
- [x] Preserve release-on-destroy via connection close.
- [x] Run `cargo test -p capture-transfer --locked`.

## Chunk 4: Docs and Gates

### Task 4: Verification

**Files:**
- Modify: `docs/superpowers/specs/2026-05-12-capture-transfer-protocol-design.md`
- Modify: `docs/superpowers/specs/2026-05-13-capture-transfer-architecture-refinement-design.md`

- [x] Update docs to say the side channel is now long-lived per consumer, with explicit leases.
- [x] Run `cargo +nightly-2026-03-12 fmt`.
- [x] Run `cargo build --workspace --locked`.
- [x] Run `cargo test --workspace --locked`.
- [x] Run `cargo clippy --workspace --all-targets --locked -- -D warnings`.
- [x] Run `cargo +nightly-2026-03-12 fmt --check`.
