# Capture Control Page Cursor Acquire Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [x]`) syntax for tracking.

**Goal:** Let daemon-backed consumers use the shared video control page to choose a producer cursor, while preserving socket leases and releases.

**Architecture:** Add an exact-cursor request to the capture transfer channel. Implement exact-cursor acquisition in `VideoSlotManager`, wire it through `portholed`'s fd socket, and teach `DaemonConsumer::latest_frame` to use the mapped control page after bootstrap.

**Tech Stack:** Rust, serde JSON wire messages, Unix-domain socket fd passing, mapped shared memory control page.

---

## Chunk 1: Wire Request And Exact Cursor Acquisition

### Task 1: Add Transfer Request Variant

**Files:**
- Modify: `crates/capture-transfer/src/transfer_channel.rs`

- [x] **Step 1: Write the failing request round-trip test**
- [x] **Step 2: Run `cargo test -p capture-transfer transfer_channel::tests::request_messages_roundtrip_with_snake_case_ops --locked` and verify failure**
- [x] **Step 3: Add `AcquireVideoFrameByCursor { session_id, track_id, producer_cursor }`**
- [x] **Step 4: Re-run the focused test and verify pass**

### Task 2: Add VideoSlotManager Exact-Cursor Acquire

**Files:**
- Modify: `crates/capture-transfer/src/video.rs`

- [x] **Step 1: Write failing tests for older live cursor and lapped cursor**
- [x] **Step 2: Run focused video tests and verify failure**
- [x] **Step 3: Add `TrackRingControl::entry_for_cursor` and `VideoSlotManager::acquire_cursor`**
- [x] **Step 4: Refactor `acquire_latest` to share the pin/desc path**
- [x] **Step 5: Re-run focused video tests and verify pass**

## Chunk 2: Socket And Daemon Integration

### Task 3: Wire Portholed Exact-Cursor Request

**Files:**
- Modify: `crates/portholed/src/capture_registry.rs`

- [x] **Step 1: Write a failing fd-socket test for exact cursor acquisition**
- [x] **Step 2: Run focused portholed test and verify failure**
- [x] **Step 3: Add registry helper for exact-cursor acquisition**
- [x] **Step 4: Handle `acquire_video_frame_by_cursor` in `handle_fd_connection`**
- [x] **Step 5: Re-run focused portholed test and verify pass**

### Task 4: Teach DaemonConsumer To Request Latest Cursor

**Files:**
- Modify: `crates/capture-transfer/src/daemon.rs`

- [x] **Step 1: Write a failing fake-server test expecting `acquire_video_frame_by_cursor` on the second request**
- [x] **Step 2: Run the focused daemon test and verify failure**
- [x] **Step 3: Read registered control page latest cursor before writing the request**
- [x] **Step 4: Fall back to `latest_video_frame` when no page or no cursor exists**
- [x] **Step 5: Re-run focused daemon tests and verify pass**

## Chunk 3: Verification And Commit

- [x] **Step 1: Run `cargo build --workspace --locked`**
- [x] **Step 2: Run `cargo test --workspace --locked`**
- [x] **Step 3: Run `cargo clippy --workspace --all-targets --locked -- -D warnings`**
- [x] **Step 4: Run `cargo +nightly-2026-03-12 fmt --check`**
- [x] **Step 5: Run `git diff --check`**
- [x] **Step 6: Commit the slice**
