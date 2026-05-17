# Capture Publication Layout Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Harden the internal capture control page toward the jackstay publication layout before exposing mmap or atomics.

**Architecture:** Extend `capture-transfer::control_page` with layout identity fields, power-of-two capacity/mask indexing, per-slot publication sequence, and latest-lossy cursor helpers. Keep `VideoSlotManager` and daemon transfer behavior unchanged; this is an internal layout evolution only.

**Tech Stack:** Rust, existing `capture-transfer` unit tests, existing CPU pool/ring integration tests.

---

## Chunk 1: Control Page Layout Metadata

### Task 1: Add fixed layout identity and power-of-two capacity

**Files:**
- Modify: `crates/capture-transfer/src/control_page.rs`

- [x] **Step 1: Write failing header layout test**

Add a test asserting:

- `VideoTrackControlPage::new(3)` rounds control capacity to `4`
- `index_mask == 3`
- magic/version/header_len/entry_len are non-zero and match the Rust struct sizes
- `VideoTrackControlPage::new(0)` remains capacity `1` and mask `0`

- [x] **Step 2: Run test to verify it fails**

Run: `cargo test -p capture-transfer control_page --locked`

Expected: fail because the fields do not exist yet.

- [x] **Step 3: Implement header fields and capacity rounding**

Add constants for magic/version. Extend `VideoTrackControlHeader`. Implement a
small `ring_capacity_for(requested: usize) -> usize` helper using
`next_power_of_two` with a minimum of one.

- [x] **Step 4: Run focused tests**

Run: `cargo test -p capture-transfer control_page --locked`

Expected: pass.

## Chunk 2: Publication Sequence And Lap Helpers

### Task 2: Add per-slot publication sequence

**Files:**
- Modify: `crates/capture-transfer/src/control_page.rs`

- [x] **Step 1: Write failing publication sequence test**

Extend the wraparound test to assert each entry's `publication_sequence` equals
the entry's producer cursor.

- [x] **Step 2: Implement `publication_sequence`**

Add `publication_sequence: u64` to `VideoRingEntry`. In `push`, write the entry
with `publication_sequence` equal to the newly assigned producer cursor.

- [x] **Step 3: Run focused tests**

Run: `cargo test -p capture-transfer control_page --locked`

Expected: pass.

### Task 3: Add latest-lossy cursor helpers

**Files:**
- Modify: `crates/capture-transfer/src/control_page.rs`

- [x] **Step 1: Write failing lap/resync test**

Add a test covering empty, partially filled, and wrapped pages:

- empty `latest_cursor()` and `oldest_live_cursor()` return `None`
- after two publishes into capacity four, oldest is `1`, latest is `2`, cursor
  `1` is not lapped
- after five publishes into capacity four, oldest is `2`, latest is `5`, cursor
  `1` is lapped, cursor `2` is not

- [x] **Step 2: Implement helpers**

Add methods:

```rust
pub fn latest_cursor(&self) -> Option<u64>
pub fn oldest_live_cursor(&self) -> Option<u64>
pub fn cursor_lapped(&self, expected_cursor: u64) -> bool
```

- [x] **Step 3: Run focused tests**

Run: `cargo test -p capture-transfer control_page --locked`

Expected: pass.

## Chunk 3: Integration And Verification

### Task 4: Preserve existing video behavior

**Files:**
- Modify if needed: `crates/capture-transfer/src/video.rs`
- Modify: `docs/superpowers/specs/2026-05-16-capture-control-page-design.md`
- Modify: `docs/superpowers/specs/2026-05-17-capture-publication-layout-design.md`

- [x] **Step 1: Update integration expectations if needed**

If power-of-two control capacity changes debug snapshots, update tests to assert
the new control-page capacity while preserving pool slot-count behavior.

- [x] **Step 2: Run focused integration tests**

Run:

```bash
cargo test -p capture-transfer video::tests::control_page_snapshot_tracks_ring_header_after_wraparound --locked
cargo test -p capture-transfer video::tests::slow_consumer_skip_count_uses_ring_cursor_after_wraparound --locked
cargo test -p portholed capture_fd_socket --locked
```

Expected: pass.

- [x] **Step 3: Run full gates**

Run:

```bash
cargo build --workspace --locked
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo +nightly-2026-03-12 fmt --check
git diff --check
```

Expected: all pass.

- [x] **Step 4: Commit and open PR**

Commit message:

```bash
git commit -m "feat(capture): harden control publication layout"
```
