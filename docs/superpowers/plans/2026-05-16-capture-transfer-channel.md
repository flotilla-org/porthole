# Capture Transfer Channel Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace ad hoc capture transfer channel JSON handling with typed messages and carry producer cursors over the existing frame metadata channel.

**Architecture:** Add a `capture-transfer` channel-message module shared by the daemon client and `portholed`. Keep newline JSON plus `SCM_RIGHTS` unchanged, but use typed request/message enums for serde. Thread `producer_cursor` from `VideoSlotManager` through `LatestVideoFrameResponse` to daemon client `DaemonFrame`.

**Tech Stack:** Rust, serde JSON, Unix-domain sockets, `SCM_RIGHTS`, existing capture-transfer shared-memory frame pools.

---

## Chunk 1: Typed Channel Messages

### Task 1: Add channel message tests

**Files:**
- Create: `crates/capture-transfer/src/transfer_channel.rs`
- Modify: `crates/capture-transfer/src/lib.rs`

- [x] Add tests that serialize/deserialize `latest_video_frame` and `release_video_frame` requests.
- [x] Add tests that serialize/deserialize `register_cpu_pool` and `video_frame` server messages.
- [x] Include `producer_cursor` in the `video_frame` fixture.
- [x] Run `cargo test -p capture-transfer transfer_channel --locked` and confirm the tests fail before implementation.

### Task 2: Implement channel message types

**Files:**
- Modify: `crates/capture-transfer/src/transfer_channel.rs`
- Modify: `crates/capture-transfer/src/lib.rs`

- [x] Add `CaptureTransferRequest` with internally tagged `latest_video_frame` and `release_video_frame` variants.
- [x] Add `CaptureTransferMessage` with internally tagged `register_cpu_pool` and `video_frame` variants.
- [x] Add helper methods where they remove repeated matching boilerplate.
- [x] Run `cargo test -p capture-transfer transfer_channel --locked`.

## Chunk 2: Producer Cursor Propagation

### Task 3: Thread producer cursor through frame metadata

**Files:**
- Modify: `crates/capture-transfer/src/video.rs`
- Modify: `crates/porthole-protocol/src/capture_sessions.rs`
- Modify: `crates/portholed/src/capture_registry.rs`
- Modify: `crates/capture-transfer/src/daemon.rs`
- Test: `crates/capture-transfer/src/daemon.rs`
- Test: `crates/portholed/src/server.rs`

- [x] Add a failing daemon-client test assertion for `DaemonFrame::producer_cursor`.
- [x] Add a failing portholed channel test assertion for `producer_cursor` in emitted `video_frame` JSON.
- [x] Add an `AcquiredVideoFrame::producer_cursor()` accessor.
- [x] Add `producer_cursor` to `LatestVideoFrameResponse` and set it from the acquired frame.
- [x] Add `producer_cursor` to the typed `video_frame` channel message and daemon parsing.
- [x] Run `cargo test -p capture-transfer daemon --locked`.
- [x] Run targeted `portholed` capture fd socket tests.

## Chunk 3: Replace Ad Hoc Channel JSON

### Task 4: Use typed messages on both channel endpoints

**Files:**
- Modify: `crates/capture-transfer/src/daemon.rs`
- Modify: `crates/portholed/src/capture_registry.rs`
- Modify: `docs/superpowers/specs/2026-05-12-capture-transfer-protocol-design.md`
- Modify: `docs/superpowers/specs/2026-05-13-capture-transfer-architecture-refinement-design.md`
- Modify: `docs/superpowers/specs/2026-05-14-capture-consumer-connection-design.md`
- Modify: `docs/superpowers/specs/2026-05-15-capture-registered-cpu-pools-design.md`

- [x] Replace client request writes with `CaptureTransferRequest` serialization.
- [x] Replace daemon request parsing with `CaptureTransferRequest` deserialization.
- [x] Replace daemon `register_cpu_pool` and `video_frame` writes with `CaptureTransferMessage` serialization.
- [x] Replace daemon client `Value` parsing with `CaptureTransferMessage` deserialization.
- [x] Rename docs/comments from "side channel" to "capture transfer channel" where this socket is meant.
- [x] Run `cargo +nightly-2026-03-12 fmt`.
- [x] Run `cargo build --workspace --locked`.
- [x] Run `cargo test --workspace --locked`.
- [x] Run `cargo clippy --workspace --all-targets --locked -- -D warnings`.
- [x] Run `cargo +nightly-2026-03-12 fmt --check`.
- [x] Run `git diff --check`.
