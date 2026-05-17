# Capture Control Page Map Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Move `VideoTrackControlPage` from field/`Vec` storage to one contiguous in-process mmap-backed page while preserving current behavior.

**Architecture:** Reuse `SharedMemorySegment` as the private backing store. Add explicit layout helpers in `control_page`, keep unsafe typed loads/stores local to that module, and leave transfer-channel consumers unchanged.

**Tech Stack:** Rust, `capture-transfer` unit tests, existing workspace CI gates.

---

## Chunk 1: Mapped Layout

### Task 1: Pin layout invariants

**Files:**
- Modify: `crates/capture-transfer/src/control_page.rs`

- [x] **Step 1: Write failing layout tests**

Add tests for:

- `VideoTrackControlPage::layout()` returns aligned `entries_offset`, rounded capacity, and mapped byte length.
- `VideoTrackControlPage::mapped_len()` matches the layout byte length.
- a new page's raw entry slots are zeroed before publish.

- [x] **Step 2: Run tests to verify failure**

Run: `cargo test -p capture-transfer control_page --locked`

Expected: fail because the layout/mapped helpers do not exist.

### Task 2: Implement mapped backing storage

**Files:**
- Modify: `crates/capture-transfer/src/control_page.rs`

- [x] **Step 1: Add layout helpers**

Add:

```rust
pub const CONTROL_PAGE_ALIGNMENT: usize = 128;

pub struct VideoTrackControlLayout {
    pub header_offset: usize,
    pub entries_offset: usize,
    pub entry_len: usize,
    pub capacity: usize,
    pub byte_len: usize,
}
```

- [x] **Step 2: Replace direct fields with mapped storage**

Change `VideoTrackControlPage` to own a `SharedMemorySegment` and a
`VideoTrackControlLayout`. Implement private typed header/entry load/store
helpers.

- [x] **Step 3: Preserve existing API behavior**

Update `push`, `read_entry_for_cursor`, cursor helpers, `ring_snapshot`, and
`snapshot` to use the typed mapped helpers.

- [x] **Step 4: Run focused tests**

Run: `cargo test -p capture-transfer control_page --locked`

Expected: pass.

## Chunk 2: Integration Verification

### Task 3: Verify existing video behavior

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
- Modify: `docs/superpowers/specs/2026-05-17-capture-control-page-map-design.md`
- Modify: `docs/superpowers/plans/2026-05-17-capture-control-page-map.md`

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
git commit -m "feat(capture): back control page with mapped storage"
```
