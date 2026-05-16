# Capture Control Page Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add an internal fixed-layout capture control page model and route the video ring cursor implementation through it without changing public transport behavior.

**Architecture:** Create a focused `capture-transfer::control_page` module that owns the fixed-size track header and video ring entries. `video.rs` keeps frame storage, leases, and consumer cursor ownership, but `TrackRingControl` delegates producer cursor/latest/ring-entry state to `VideoTrackControlPage`. The daemon wire protocol and C ABI remain unchanged.

**Tech Stack:** Rust, existing `capture-transfer` unit tests, existing mmap-backed CPU pool model.

---

## Chunk 1: Internal Control Page Module

### Task 1: Add fixed-layout control page types

**Files:**
- Create: `crates/capture-transfer/src/control_page.rs`
- Modify: `crates/capture-transfer/src/lib.rs`

- [x] **Step 1: Write failing tests for empty and wrapped control pages**

Add tests in `control_page.rs` for:

- `VideoTrackControlPage::new(0)` clamps to capacity `1`
- empty header has `producer_cursor = 0`, `len = 0`, and no latest entry
- pushing three entries into capacity two leaves entries for cursors `2` and `3`
  in oldest-to-newest order

- [x] **Step 2: Run tests to verify failure**

Run: `cargo test -p capture-transfer control_page --locked`

Expected: fail because the module/types do not exist yet.

- [x] **Step 3: Implement minimal control page model**

Define:

- `EMPTY_LATEST_INDEX: u64 = u64::MAX`
- `VideoTrackControlHeader`
- `VideoRingEntry`
- `PendingVideoRingEntry`
- `VideoTrackControlPage`
- `VideoTrackControlSnapshot`

Implement `new`, `push`, `latest`, `snapshot`, and `ring_snapshot`.

- [x] **Step 4: Export the module**

Add `pub mod control_page;` to `crates/capture-transfer/src/lib.rs`.

- [x] **Step 5: Run tests**

Run: `cargo test -p capture-transfer control_page --locked`

Expected: pass.

## Chunk 2: Video Ring Integration

### Task 2: Replace private metadata ring with control page

**Files:**
- Modify: `crates/capture-transfer/src/video.rs`
- Test: existing `video::tests`

- [x] **Step 1: Update imports and type aliases**

Import `PendingVideoRingEntry`, `VideoRingEntry`, and `VideoTrackControlPage`
from `control_page`. Remove the private `MetadataRing`, `FrameRingEntry`, and
`PendingFrameRingEntry` definitions once their call sites are migrated.

- [x] **Step 2: Route `TrackRingControl` through `VideoTrackControlPage`**

Change `TrackRingControl` to store only:

```rust
struct TrackRingControl {
    page: VideoTrackControlPage,
}
```

Delegate `push`, `latest`, `ring_snapshot`, and `snapshot` to the page. Preserve
the existing public `TrackRingControlSnapshot` shape.

- [x] **Step 3: Update publish path**

In `store_published_payload`, build a `PendingVideoRingEntry` with the same
fields previously used by `PendingFrameRingEntry`.

- [x] **Step 4: Update tests for control-page source**

Extend `ring_control_snapshot_tracks_producer_cursor_and_latest_sequence` to
assert the snapshot entries are `VideoRingEntry` values from the new module and
that wraparound still reports capacity `2`, producer cursor `3`, latest sequence
`Some(3)`, and cursors `[2, 3]`.

- [x] **Step 5: Run focused tests**

Run:

```bash
cargo test -p capture-transfer video::tests::ring_control_snapshot_tracks_producer_cursor_and_latest_sequence --locked
cargo test -p capture-transfer video::tests::slow_consumer_skip_count_uses_ring_cursor_after_wraparound --locked
```

Expected: pass.

## Chunk 3: Verification And Documentation

### Task 3: Verify unchanged transport behavior

**Files:**
- Modify if needed: `docs/superpowers/specs/2026-05-12-capture-transfer-protocol-design.md`
- Modify if needed: `docs/superpowers/specs/2026-05-15-capture-ring-cursors-design.md`

- [x] **Step 1: Update docs if implementation details differ**

If code names or semantics differ from the spec, update the design docs. Do not
expand scope into shared mmap, atomics, or wake primitives.

- [x] **Step 2: Run focused transport tests**

Run:

```bash
cargo test -p capture-transfer transfer_channel --locked
cargo test -p capture-transfer daemon --locked
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
git commit -m "feat(capture): add internal control page model"
```

Open a PR from `capture-control-page`.
