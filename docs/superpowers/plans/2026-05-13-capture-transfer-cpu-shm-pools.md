# Capture Transfer CPU SHM Pools Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add reusable fixed-size CPU shared-memory pool slots for in-process consumers, while initially keeping daemon-backed sessions on immutable per-frame files until cross-process release semantics exist.

**Architecture:** `VideoSlotManager` can run in immutable per-frame mode or reusable-pool mode. Reusable mode owns one CPU shared-memory pool per video track; each pool is a single mmap-backed file divided into fixed-size slots. Publishing copies pixels into an available slot, records `payload_offset`, `payload_len`, and `payload_map_len`, and latest acquisition returns a slice into the pool. The daemon wire also carries offset and map length. This is not a shared ring/control-block protocol.

**Tech Stack:** Rust workspace, Unix mmap/fd cloning, serde JSON daemon structs, C ABI header.

---

## Scope

This plan implements the next CPU-memory step from the architecture refinement spec:

- one reusable shared-memory region per track for in-process consumers
- fixed-size producer-chosen slots inside that region
- explicit payload offset/length/mapping metadata
- existing latest-lossy API behavior preserved
- daemon sessions require a separate lease/release slice before using reusable pools

This plan does not implement:

- shared producer/consumer rings
- futex/eventfd/Mach wake primitives
- cross-process pool registration messages
- cross-process slot release or lease tracking
- IOSurface, dmabuf, or D3D handles
- sidecar damage buffers
- ordered/recording cursors

## File Structure

- Modify `crates/capture-transfer/src/shm.rs`
  - Keep `SharedMemorySegment` as the mmap/file owner and add fd cloning plus range helpers usable by pooled slots.
- Modify `crates/capture-transfer/src/video.rs`
  - Add payload placement fields to `VideoFrameDesc`.
  - Add explicit immutable and reusable-pool storage modes.
  - Keep acquired frames pinned until release so slots are not overwritten while a consumer holds a frame.
- Modify `crates/capture-transfer/src/ffi.rs`
  - Extend `FtVideoFrameDesc` with payload offset/length/map length and keep `frame.data` pointing at the acquired payload slice.
- Modify `crates/capture-transfer/include/capture_transfer.h`
  - Mirror C ABI fields.
- Modify `crates/porthole-protocol/src/capture_sessions.rs`
  - Add daemon response fields for payload offset/length/map length.
- Modify `crates/capture-transfer/src/daemon.rs`
  - Map the advertised shared region and expose `frame.bytes()` as the valid payload slice.
- Modify `crates/portholed/src/capture_registry.rs`
  - Include payload mapping metadata in latest-frame responses.
- Modify docs under `docs/superpowers/specs/`.
  - Clarify that current reusable pools are internal storage, not the final cross-process registration protocol.

## Chunk 1: Internal Pool Storage

### Task 1: Add payload placement metadata

- [ ] **Step 1: Write tests**

Add `VideoFrameDesc` assertions proving acquired frames include `payload_offset`, `payload_len`, and `payload_map_len`.

- [ ] **Step 2: Verify red**

Run `cargo test -p capture-transfer video::tests --locked`.

Expected: FAIL because the fields do not exist yet.

- [ ] **Step 3: Implement fields**

Add scalar fields:

```rust
pub payload_offset: u64,
pub payload_len: u64,
pub payload_map_len: u64,
```

`payload_len` must match the published byte count. `payload_offset + payload_len <= payload_map_len`.

### Task 2: Replace per-frame allocation with reusable per-track pools

- [ ] **Step 1: Write tests**

Add tests proving two sequential unpinned frames reuse the same map length with different slot offsets, and that a pinned frame remains readable after newer publishes.

- [ ] **Step 2: Verify red**

Run `cargo test -p capture-transfer video::tests --locked`.

Expected: FAIL until pooling is implemented.

- [ ] **Step 3: Implement pool slots**

For reusable mode, lazily allocate a `SharedMemorySegment` sized to `slot_capacity * capacity_per_track`, rounded up to a conservative 64-byte alignment. Choose a free slot if possible. If every slot is pinned, allocate a replacement pool generation and keep old acquired frames alive through their `Arc`. Keep immutable mode as the default for daemon-backed sessions.

### Task 3: Thread metadata through C ABI and daemon wire

- [ ] **Step 1: Write tests**

Extend ABI and daemon tests to assert payload offset/length/map length survive round-trip.

- [ ] **Step 2: Verify red**

Run focused tests for `capture-transfer` and `portholed`.

- [ ] **Step 3: Implement conversions**

Map the full pool fd in daemon consumers and expose the payload slice from `payload_offset..payload_offset + payload_len`.

## Final Verification

- [ ] Run `cargo build --workspace --locked`
- [ ] Run `cargo test --workspace --locked`
- [ ] Run `cargo clippy --workspace --all-targets --locked -- -D warnings`
- [ ] Run `cargo +nightly-2026-03-12 fmt --check`
