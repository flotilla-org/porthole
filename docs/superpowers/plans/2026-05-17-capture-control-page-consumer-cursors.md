# Capture Control Page Consumer Cursors Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add writable consumer release cursors to the mapped video control page while keeping socket leases authoritative.

**Architecture:** Extend the control page layout with a fixed consumer cursor table after the ring entries. The daemon allocates one slot per consumer/track and tells the client which slot it owns; the client writes `release_cursor` there before sending the existing lease release request.

**Tech Stack:** Rust, `repr(C)` shared-memory structs, atomic `u64` cursor stores, JSON fd-socket messages, Cargo workspace gates.

---

## File Structure

- Modify `crates/capture-transfer/src/shm.rs`: add read-write fd mapping support.
- Modify `crates/capture-transfer/src/control_page.rs`: add consumer cursor layout, header fields, slot allocation, atomic release cursor accessors, writable mapping, and tests.
- Modify `crates/capture-transfer/src/transfer_channel.rs`: include `consumer_id` and `consumer_slot` in `register_video_control_page`.
- Modify `crates/capture-transfer/src/video.rs`: allocate/update/unregister consumer cursor slots from `VideoSlotManager`.
- Modify `crates/capture-transfer/src/daemon.rs`: map control pages writable, remember slot assignments, write release cursor before socket release, and update fake-server tests.
- Modify `crates/portholed/src/capture_registry.rs`: request a consumer-specific control-page registration and send the slot fields over the fd socket.

## Chunk 1: Control Page Cursor Table

### Task 1: Layout And Header

**Files:**
- Modify: `crates/capture-transfer/src/control_page.rs`

- [ ] **Step 1: Write the failing test**

Add `control_page_layout_includes_consumer_cursor_region`, asserting the layout has `consumer_entries_offset`, `consumer_entry_len`, `consumer_capacity`, and a `byte_len` covering both ring and cursor regions.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p capture-transfer control_page::tests::control_page_layout_includes_consumer_cursor_region --locked`

Expected: FAIL because the fields do not exist.

- [ ] **Step 3: Implement minimal layout support**

Add `VideoConsumerCursorEntry`, header fields, layout fields, version bump, and validation checks.

- [ ] **Step 4: Run focused tests**

Run: `cargo test -p capture-transfer control_page::tests --locked`

Expected: PASS after updating existing layout/header assertions.

### Task 2: Cursor Slot Atomic Access

**Files:**
- Modify: `crates/capture-transfer/src/control_page.rs`

- [ ] **Step 1: Write failing tests**

Add tests for registering a consumer cursor slot, storing/reloading `release_cursor`, and unregistering the slot.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p capture-transfer control_page::tests::consumer_cursor --locked`

Expected: FAIL because slot APIs do not exist.

- [ ] **Step 3: Implement slot APIs**

Add methods to allocate/reuse a slot for a non-zero consumer id, store server acquire snapshots, store release cursors atomically, read cursor entries, and clear a slot on disconnect.

- [ ] **Step 4: Run focused tests**

Run: `cargo test -p capture-transfer control_page::tests --locked`

Expected: PASS.

## Chunk 2: Server And Client Integration

### Task 3: Transfer Message Carries Slot Assignment

**Files:**
- Modify: `crates/capture-transfer/src/transfer_channel.rs`

- [ ] **Step 1: Write failing test**

Update `server_messages_roundtrip_with_producer_cursor` to expect `consumer_id` and `consumer_slot` on `RegisterVideoControlPage`.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p capture-transfer transfer_channel::tests::server_messages_roundtrip_with_producer_cursor --locked`

Expected: FAIL until the enum variant has the new fields.

- [ ] **Step 3: Implement message fields**

Add `consumer_id: u64` and `consumer_slot: u64` to the variant and update all send/receive sites to compile.

- [ ] **Step 4: Run focused transfer tests**

Run: `cargo test -p capture-transfer transfer_channel::tests --locked`

Expected: PASS.

### Task 4: VideoSlotManager Mirrors Cursor State

**Files:**
- Modify: `crates/capture-transfer/src/video.rs`

- [ ] **Step 1: Write failing tests**

Add tests that acquiring a frame registers/updates a control-page consumer cursor entry, releasing mirrors the release cursor, and disconnect clears the consumer slot.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p capture-transfer video::tests::consumer_cursor --locked`

Expected: FAIL because manager integration does not exist.

- [ ] **Step 3: Implement manager integration**

Register a slot during acquisition, update acquire counters from `ConsumerRingCursorSnapshot`, mirror release on socket release, and unregister slots during `disconnect_consumer`.

- [ ] **Step 4: Run focused video tests**

Run: `cargo test -p capture-transfer video::tests --locked`

Expected: PASS.

### Task 5: Daemon Consumer Writes Release Cursor

**Files:**
- Modify: `crates/capture-transfer/src/shm.rs`
- Modify: `crates/capture-transfer/src/control_page.rs`
- Modify: `crates/capture-transfer/src/daemon.rs`

- [ ] **Step 1: Write failing daemon test**

Update or add a fake-server test where the server maps the same control page and asserts `release_cursor` is stored before the client sends `release_video_frame`.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p capture-transfer daemon::tests::daemon_consumer_writes_release_cursor_before_socket_release --locked`

Expected: FAIL because the client maps read-only and does not write release cursors.

- [ ] **Step 3: Implement writable mapping**

Add `SharedMemorySegment::map_read_write`, `VideoTrackControlPage::map_read_write`, store `consumer_slot` per track in `DaemonConsumer`, and write `release_cursor` before socket release.

- [ ] **Step 4: Run focused daemon tests**

Run: `cargo test -p capture-transfer daemon::tests --locked`

Expected: PASS.

## Chunk 3: Portholed Socket Integration

### Task 6: Send Consumer Slot Registration

**Files:**
- Modify: `crates/portholed/src/capture_registry.rs`

- [ ] **Step 1: Write failing fd-socket test**

Extend the fd-socket control page registration assertion to require non-zero `consumer_id` and a numeric `consumer_slot`.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p portholed capture_registry::tests::capture_fd_socket_registers_reusable_cpu_pool_once --locked`

Expected: FAIL until portholed sends the new fields.

- [ ] **Step 3: Implement registry integration**

Pass `consumer_id` into `control_page_registration`, include the returned slot fields in `RegisterVideoControlPage`, and preserve disconnect lease cleanup.

- [ ] **Step 4: Run focused portholed tests**

Run: `cargo test -p portholed capture_registry::tests --locked`

Expected: PASS.

## Chunk 4: Verification And Finish

### Task 7: Workspace Gates

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
