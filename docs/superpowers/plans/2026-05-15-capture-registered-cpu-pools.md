# Capture Registered CPU Pools Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Register reusable CPU shm pools once per daemon consumer connection and reuse cached mappings for later frames.

**Architecture:** Extend the fd side channel with server-to-client `register_cpu_pool` and `video_frame` messages. The daemon sends a pool fd only when a connection first sees a reusable pool generation; frame responses then carry metadata only. Immutable fallback frames can keep the existing per-frame fd path.

**Tech Stack:** Rust, Unix-domain sockets, `SCM_RIGHTS`, mmap, serde JSON, existing `capture-transfer` C ABI.

---

## Chunk 1: Pool Metadata From Slot Manager

### Task 1: Expose acquired frame pool registration metadata

**Files:**
- Modify: `crates/capture-transfer/src/video.rs`
- Test: `crates/capture-transfer/src/video.rs`

- [x] Add a failing test that publishes two reusable-pool frames and asserts the acquired frames expose the same pool fd metadata with distinct payload offsets.
- [x] Add a small public metadata type for reusable CPU pool registration.
- [x] Add an `AcquiredVideoFrame` method that returns pool registration metadata only for reusable-pool frames.
- [x] Keep immutable-per-frame acquisitions returning `None`.
- [x] Run `cargo test -p capture-transfer video --locked`.

## Chunk 2: Daemon Side-Channel Pool Registration

### Task 2: Send pool registration once per connection

**Files:**
- Modify: `crates/portholed/src/capture_registry.rs`
- Modify: `crates/portholed/src/server.rs`
- Test: `crates/portholed/src/server.rs`

- [x] Add a failing server test that acquires two reusable-pool frames and observes one `register_cpu_pool` fd followed by two `video_frame` messages without per-frame fds.
- [x] Add server-side response message types for `register_cpu_pool` and `video_frame`.
- [x] Track registered pool keys per fd connection.
- [x] Before sending a reusable-pool frame, send `register_cpu_pool` with the pool fd if the key is not already registered.
- [x] Send frame metadata as a tagged `video_frame` message.
- [x] Preserve the immutable fallback path by sending a tagged frame response accompanied by a per-frame fd.
- [x] Run targeted portholed server tests.

## Chunk 3: Daemon Client Pool Cache

### Task 3: Cache registered pool mappings in `DaemonConsumer`

**Files:**
- Modify: `crates/capture-transfer/src/daemon.rs`
- Modify: `crates/capture-transfer/src/ffi.rs` only if release/drop ownership requires it
- Test: `crates/capture-transfer/src/daemon.rs`

- [x] Add a failing daemon-client test where a fake server registers one CPU pool and sends two frame messages naming different offsets in that pool.
- [x] Add `DaemonConsumer` pool cache keyed by `(track_id, pool_id, slot_generation)`.
- [x] Parse tagged side-channel messages: `register_cpu_pool` and `video_frame`.
- [x] On pool registration, receive and mmap the fd read-only for the registered length.
- [x] On frame response, resolve bytes from the cached pool mapping and validate the range.
- [x] Keep immutable fallback support for a frame response accompanied by a per-frame fd.
- [x] Run `cargo test -p capture-transfer daemon --locked`.

## Chunk 4: Docs and Gates

### Task 4: Update protocol docs and verify

**Files:**
- Modify: `docs/superpowers/specs/2026-05-12-capture-transfer-protocol-design.md`
- Modify: `docs/superpowers/specs/2026-05-13-capture-transfer-architecture-refinement-design.md`
- Modify: `docs/superpowers/specs/2026-05-15-capture-registered-cpu-pools-design.md`

- [ ] Update existing protocol/refinement docs to say reusable CPU pools are registered once on the side channel.
- [ ] Run `cargo +nightly-2026-03-12 fmt`.
- [ ] Run `cargo build --workspace --locked`.
- [ ] Run `cargo test --workspace --locked`.
- [ ] Run `cargo clippy --workspace --all-targets --locked -- -D warnings`.
- [ ] Run `cargo +nightly-2026-03-12 fmt --check`.
