# Capture Control Page Atomic Accessors Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add acquire/release atomic accessor helpers for the mapped video control page while keeping it private and in-process.

**Architecture:** Keep the existing mapped layout and plain descriptor structs. Add localized `AtomicU64` pointer helpers in `control_page`, use them only for hot publication/cursor fields, and leave transfer-channel behavior unchanged.

**Tech Stack:** Rust, `std::sync::atomic`, existing `capture-transfer` tests, workspace CI gates.

---

## Chunk 1: Atomic Accessors

### Task 1: Pin atomic layout and helper behavior

**Files:**
- Modify: `crates/capture-transfer/src/control_page.rs`

- [x] **Step 1: Write failing tests**

Add tests for:

- hot header and entry fields are aligned for `AtomicU64`
- header producer cursor atomic helper roundtrips a value
- slot publication sequence atomic helper roundtrips a value

- [x] **Step 2: Run tests to verify failure**

Run: `cargo test -p capture-transfer control_page --locked`

Expected: fail because the atomic helper methods do not exist yet.

### Task 2: Implement atomic mapped accessors

**Files:**
- Modify: `crates/capture-transfer/src/control_page.rs`

- [x] **Step 1: Add atomic pointer helpers**

Add private `load_u64_atomic(offset, ordering)` and
`store_u64_atomic(offset, value, ordering)` helpers. Assert the offset is
`AtomicU64` aligned.

- [x] **Step 2: Add field-specific helpers**

Add field-specific helpers for:

- header `producer_cursor`
- header `latest_sequence`
- header `latest_index`
- header `len`
- entry `publication_sequence`

- [x] **Step 3: Use release/acquire in producer and reader paths**

Update `push`, `latest_cursor`, `oldest_live_cursor`, `header`, and
`read_entry_for_cursor` to use the atomic helpers for hot fields.

- [x] **Step 4: Run focused tests**

Run: `cargo test -p capture-transfer control_page --locked`

Expected: pass.

## Chunk 2: Integration Verification

### Task 3: Verify existing capture behavior

**Files:**
- Modify: `crates/capture-transfer/src/control_page.rs`

- [x] **Step 1: Run focused integration tests**

Run:

```bash
cargo test -p capture-transfer video::tests::control_page_snapshot_tracks_ring_header_after_wraparound --locked
cargo test -p capture-transfer video::tests::slow_consumer_skip_count_uses_ring_cursor_after_wraparound --locked
cargo test -p portholed capture_fd_socket --locked
```

Expected: pass.

### Task 4: Verify and publish

**Files:**
- Modify: `docs/superpowers/specs/2026-05-17-capture-control-page-atomic-accessors-design.md`
- Modify: `docs/superpowers/plans/2026-05-17-capture-control-page-atomic-accessors.md`

- [x] **Step 1: Mark docs implemented**

Set spec status to `implemented prototype` and mark completed plan boxes.

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
git commit -m "feat(capture): add control page atomic accessors"
```
