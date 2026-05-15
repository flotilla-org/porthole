# Capture Ring Cursors Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `VideoSlotManager` use an explicit in-process ring control/cursor model while preserving the current daemon wire protocol.

**Architecture:** Add `TrackRingControl` and `ConsumerRingCursor` to `crates/capture-transfer/src/video.rs`. Publishing advances a producer cursor and stores it in the ring entry. Latest-frame acquisition and release update per-consumer cursor state, replacing the current scattered last-acquired/skipped maps.

**Tech Stack:** Rust, existing mmap-backed CPU shared memory slots, existing daemon side-channel JSON and lease protocol.

---

## Chunk 1: Cursor Model In `VideoSlotManager`

### Task 1: Add cursor-state tests

**Files:**
- Modify: `crates/capture-transfer/src/video.rs`

- [x] Add a failing test that publishes more frames than ring capacity and asserts a debug control snapshot reports producer cursor, latest sequence, and capacity.
- [x] Add a failing test that a slow consumer records skipped frames from ring cursor movement after wraparound.
- [x] Add a failing test that releasing an acquired frame advances the consumer release cursor.
- [x] Add a multi-consumer regression test proving cursor state is independent per `(consumer_id, track_id)`.
- [x] Run `cargo test -p capture-transfer video --locked` and confirm the new tests fail for missing APIs/state.

### Task 2: Implement ring control state

**Files:**
- Modify: `crates/capture-transfer/src/video.rs`

- [x] Add `producer_cursor` to `FrameRingEntry`.
- [x] Replace `rings_by_track` with `controls_by_track: BTreeMap<TrackId, TrackRingControl>`.
- [x] Add `TrackRingControl` with fixed ring, producer cursor, latest entry, and snapshot helpers.
- [x] Add `ConsumerRingCursor` and replace `last_acquired_by_consumer` / `skipped_by_consumer` with `consumer_cursors`.
- [x] Use `Option<u64>` for uninitialized consumer acquire cursors instead of a zero sentinel.
- [x] Update publish/acquire/release/disconnect logic to read and write the new cursor state.
- [x] Run `cargo test -p capture-transfer video --locked`.

## Chunk 2: Docs And Gates

### Task 3: Keep protocol docs honest

**Files:**
- Modify: `docs/superpowers/specs/2026-05-12-capture-transfer-protocol-design.md`
- Modify: `docs/superpowers/specs/2026-05-15-capture-ring-cursors-design.md`

- [x] Update the existing protocol design to say the ring cursor model is now in-process and still not externally mapped.
- [x] Run `cargo +nightly-2026-03-12 fmt`.
- [x] Run `cargo build --workspace --locked`.
- [x] Run `cargo test --workspace --locked`.
- [x] Run `cargo clippy --workspace --all-targets --locked -- -D warnings`.
- [x] Run `cargo +nightly-2026-03-12 fmt --check`.
- [x] Run `git diff --check`.
