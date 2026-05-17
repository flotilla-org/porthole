# Capture Control Page Full Descriptor Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Expand the mapped video control page ring entry so it carries the fixed frame descriptor fields needed by the future hot metadata path.

**Architecture:** Keep the existing control page publication protocol and fd-socket lease authority. Grow the fixed `repr(C)` ring entry, publish fields from `VideoFrameDesc`, and make the consumer's shadow comparison validate every fixed descriptor field that the socket also returns.

**Tech Stack:** Rust, shared-memory `repr(C)` structs, JSON fd-socket test fixtures, Cargo workspace gates.

---

## File Structure

- Modify `crates/capture-transfer/src/model.rs`: give `PixelFormat` stable numeric values and test them.
- Modify `crates/capture-transfer/src/control_page.rs`: expand `VideoRingEntry` and `PendingVideoRingEntry`; update push/read tests.
- Modify `crates/capture-transfer/src/video.rs`: publish full fixed descriptors into the control page; add a manager-level descriptor snapshot test.
- Modify `crates/capture-transfer/src/daemon.rs`: strengthen `compare_control_page_shadow`; update fake-server control-page entries and add a mismatch test for a newly covered field.
- Keep `crates/portholed/src/capture_registry.rs` untouched unless compile errors show helper constructors need alignment.

## Chunk 1: Shared Descriptor Shape

### Task 1: Stable Pixel Format Values

**Files:**
- Modify: `crates/capture-transfer/src/model.rs`

- [ ] **Step 1: Write the failing test**

Extend `frame_metadata_defaults_are_explicit` to assert:

```rust
assert_eq!(PixelFormat::Bgra8Unorm as u32, 1);
assert_eq!(PixelFormat::Rgba8Unorm as u32, 2);
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p capture-transfer model::tests::frame_metadata_defaults_are_explicit --locked`

Expected: FAIL because `PixelFormat` cannot yet be cast to `u32`.

- [ ] **Step 3: Write minimal implementation**

Add `#[repr(u32)]` to `PixelFormat` and assign:

```rust
Bgra8Unorm = 1,
Rgba8Unorm = 2,
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p capture-transfer model::tests::frame_metadata_defaults_are_explicit --locked`

Expected: PASS.

### Task 2: Control Page Entry Carries Full Fixed Descriptor

**Files:**
- Modify: `crates/capture-transfer/src/control_page.rs`

- [ ] **Step 1: Write the failing test**

Add a helper that constructs `PendingVideoRingEntry` with non-default values for every fixed descriptor field, then add a test that pushes it, maps the page read-only, shadow-reads cursor `1`, and asserts the returned `VideoRingEntry` equals the expected descriptor plus publication fields.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p capture-transfer control_page::tests::read_only_control_page_shadow_reads_full_descriptor_from_fd --locked`

Expected: FAIL because the fields do not exist on `PendingVideoRingEntry` / `VideoRingEntry`.

- [ ] **Step 3: Write minimal implementation**

Add the approved fixed descriptor fields to both structs. In `push`, copy the new pending fields into `VideoRingEntry` while preserving the existing publication sequence protocol.

- [ ] **Step 4: Run focused control-page tests**

Run: `cargo test -p capture-transfer control_page::tests --locked`

Expected: PASS.

## Chunk 2: Publishing And Shadow Validation

### Task 3: VideoSlotManager Publishes Full Descriptor To Control Page

**Files:**
- Modify: `crates/capture-transfer/src/video.rs`

- [ ] **Step 1: Write the failing test**

Add a test that mutates a `VideoFrameDesc` with distinctive timestamp, dimensions, pixel format, map length, color/clock/sync/damage values, drop counters, publishes it, and checks `debug_control_page_snapshot(track).entries.last()` contains the matching numeric descriptor values.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p capture-transfer video::tests::publishing_frame_writes_full_descriptor_to_control_page --locked`

Expected: FAIL because `store_published_payload` does not fill the expanded pending fields yet.

- [ ] **Step 3: Write minimal implementation**

Populate every new `PendingVideoRingEntry` field from `payload.desc`, converting enum values with `as u32`.

- [ ] **Step 4: Run focused video tests**

Run: `cargo test -p capture-transfer video::tests --locked`

Expected: PASS.

### Task 4: Daemon Shadow Comparison Covers Descriptor Fields

**Files:**
- Modify: `crates/capture-transfer/src/daemon.rs`

- [ ] **Step 1: Write the failing test**

Change `daemon_consumer_rejects_control_page_shadow_mismatch` so the control-page entry matches payload length but differs in a newly covered field such as `timestamp_ns` or `payload_map_len`.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p capture-transfer daemon::tests::daemon_consumer_rejects_control_page_shadow_mismatch --locked`

Expected: FAIL because the current comparison ignores the new field.

- [ ] **Step 3: Write minimal implementation**

Extend `compare_control_page_shadow` to compare all fixed descriptor fields now present in `VideoRingEntry`.

- [ ] **Step 4: Update fake-server fixtures**

Update every `PendingVideoRingEntry` literal in daemon tests to use a helper with full descriptor defaults matching `latest_frame_json_with_offset`.

- [ ] **Step 5: Run focused daemon tests**

Run: `cargo test -p capture-transfer daemon::tests --locked`

Expected: PASS.

## Chunk 3: Verification And Finish

### Task 5: Workspace Gates

**Files:**
- Modify only files already touched if verification exposes compile, lint, or format issues.

- [ ] **Step 1: Run full build**

Run: `cargo build --workspace --locked`

Expected: PASS.

- [ ] **Step 2: Run full tests**

Run: `cargo test --workspace --locked`

Expected: PASS.

- [ ] **Step 3: Run clippy**

Run: `cargo clippy --workspace --all-targets --locked -- -D warnings`

Expected: PASS.

- [ ] **Step 4: Run pinned fmt check**

Run: `cargo +nightly-2026-03-12 fmt --check`

Expected: PASS.

- [ ] **Step 5: Run diff whitespace check**

Run: `git diff --check`

Expected: PASS.

- [ ] **Step 6: Commit implementation**

Commit docs, tests, and implementation with a focused message after the gates are clean.
