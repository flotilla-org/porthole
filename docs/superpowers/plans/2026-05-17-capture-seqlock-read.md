# Capture Seqlock Read Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add an internal seqlock-style validated read path for control-page video ring entries before introducing atomics or mmap.

**Architecture:** Extend `capture-transfer::control_page` with read errors and cursor-based read helpers. Keep storage in-process and non-atomic, but model the future double-read publication-sequence contract. Then route `VideoSlotManager` latest acquisition through the latest-lossy read helper.

**Tech Stack:** Rust, existing `capture-transfer` unit tests, existing video ring integration tests.

---

## Chunk 1: Validated Control-Page Reads

### Task 1: Add read error tests

**Files:**
- Modify: `crates/capture-transfer/src/control_page.rs`

- [x] **Step 1: Write failing tests**

Add tests for:

- empty `read_entry_for_cursor(1)` returns `VideoRingReadError::Empty`
- cursor newer than latest returns `NotPublished`
- cursor lapped after wraparound returns `Lapped`
- latest-lossy helper returns newest entry after wraparound

- [x] **Step 2: Run tests to verify failure**

Run: `cargo test -p capture-transfer control_page --locked`

Expected: fail because read errors/helpers do not exist.

### Task 2: Implement read errors and helpers

**Files:**
- Modify: `crates/capture-transfer/src/control_page.rs`

- [x] **Step 1: Add `VideoRingReadError`**

Add an internal/public module enum with variants:

```rust
Empty
NotPublished { requested_cursor: u64, latest_cursor: u64 }
Lapped { requested_cursor: u64, oldest_live_cursor: u64, latest_cursor: u64 }
SlotSequenceMismatch { requested_cursor: u64, first_sequence: u64, second_sequence: u64 }
```

- [x] **Step 2: Implement read helpers**

Implement:

```rust
pub fn read_entry_for_cursor(&self, cursor: u64) -> Result<VideoRingEntry, VideoRingReadError>
pub fn read_latest_lossy_entry(&self) -> Result<Option<VideoRingEntry>, VideoRingReadError>
```

Use mask indexing and double-read `publication_sequence` before/after cloning
the entry.

- [x] **Step 3: Run focused tests**

Run: `cargo test -p capture-transfer control_page --locked`

Expected: pass.

## Chunk 2: Video Integration

### Task 3: Route latest acquire through validated latest helper

**Files:**
- Modify: `crates/capture-transfer/src/video.rs`

- [x] **Step 1: Update `TrackRingControl::latest`**

Change it to return an owned `VideoRingEntry` from
`VideoTrackControlPage::read_latest_lossy_entry`.

- [x] **Step 2: Update `VideoSlotManager::acquire_latest`**

Remove the old `.cloned()` call because the control page now returns an owned
validated entry.

- [x] **Step 3: Run focused integration tests**

Run:

```bash
cargo test -p capture-transfer video::tests::control_page_snapshot_tracks_ring_header_after_wraparound --locked
cargo test -p capture-transfer video::tests::slow_consumer_skip_count_uses_ring_cursor_after_wraparound --locked
cargo test -p portholed capture_fd_socket --locked
```

Expected: pass.

## Chunk 3: Verification

### Task 4: Verify and publish

**Files:**
- Modify: `docs/superpowers/specs/2026-05-17-capture-seqlock-read-design.md`
- Modify: `docs/superpowers/plans/2026-05-17-capture-seqlock-read.md`

- [x] **Step 1: Mark docs implemented**

Update status/check boxes once implementation and gates pass.

- [x] **Step 2: Run full gates**

Run:

```bash
cargo build --workspace --locked
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo +nightly-2026-03-12 fmt --check
git diff --check
```

Expected: all pass.

- [x] **Step 3: Commit and open PR**

Commit message:

```bash
git commit -m "feat(capture): add validated control ring reads"
```
