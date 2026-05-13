# Capture Transfer Metadata Ring Prototype Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Prove the metadata-ring shape over registered CPU shm slots without waiting for a cross-process shared control block.

**Architecture:** `VideoSlotManager` keeps per-track reusable payload pools and appends each published frame to a fixed-size per-track metadata ring. Ring entries name `frame_key`, `pool_id`, `slot_id`, `slot_generation`, sequence, and payload range. `acquire_latest` resolves the latest ring entry back to the stored frame, so the current request/lease path exercises the ring while wake primitives and a real shared-memory control block remain deferred.

**Tech Stack:** Rust workspace, existing mmap-backed CPU shm pools, serde JSON daemon metadata, C ABI header.

---

## Scope

This plan implements:

- explicit `pool_id`, `slot_id`, and `slot_generation` frame metadata
- a per-track fixed-size metadata ring inside `VideoSlotManager`
- latest-frame acquisition through the ring entry rather than `frames.last()`
- ABI and daemon wire propagation for the new slot identity
- docs explaining where the ring fits relative to CPU pools and future IOSurface/Linux work

This plan does not implement:

- a cross-process shared-memory control block
- futex/eventfd/kqueue wakeups
- ordered recording cursors
- native GPU synchronization
- IOSurface or dmabuf handles

## File Structure

- Modify `crates/capture-transfer/src/video.rs`
  - Add slot identity fields to `VideoFrameDesc`.
  - Add `TrackFrameRing` / `FrameRingEntry`.
  - Append ring entries on publish and acquire latest through the ring.
- Modify `crates/capture-transfer/src/ffi.rs`
  - Add C ABI fields and round-trip assertions.
- Modify `crates/capture-transfer/include/capture_transfer.h`
  - Mirror new metadata fields.
- Modify `crates/capture-transfer/src/daemon.rs`
  - Parse slot identity from daemon latest-frame responses.
- Modify `crates/porthole-protocol/src/capture_sessions.rs`
  - Add slot identity JSON fields.
- Modify `crates/portholed/src/capture_registry.rs`
  - Return slot identity from latest-frame responses.
- Modify `crates/porthole-core/src/adapter.rs`, `crates/porthole-core/src/in_memory.rs`, and macOS capture defaults if needed.
  - Keep producer-side input metadata defaulted to zero; slot identity is assigned by `VideoSlotManager`.
- Modify docs under `docs/superpowers/specs/`.

## Tasks

- [ ] Write failing tests for ring-backed latest acquisition and explicit slot identity.
- [ ] Implement ring structs and publish/acquire integration.
- [ ] Thread `pool_id`, `slot_id`, and `slot_generation` through ABI and daemon wire.
- [ ] Update docs.
- [ ] Run full repo verification.
