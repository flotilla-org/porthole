# Capture Transfer Producer Slot Claim Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let CPU producers write directly into capture-transfer managed shm slots so the transport no longer requires a producer-owned `Vec<u8>` followed by a copy into shm.

**Architecture:** Add a `claim_video_slot` / `commit_video_slot` path to `VideoSlotManager`. Claim selects or allocates a reusable pool slot, returns a writable payload slice plus slot metadata, and commit publishes the already-filled slot into the metadata ring. The existing `publish(&[u8])` API becomes a convenience wrapper over claim + copy + commit, preserving current callers while exposing the lower-copy path for adapters.

**Tech Stack:** Rust workspace, existing mmap-backed CPU shm pools, per-track metadata ring.

---

## Scope

This plan implements:

- producer claim/fill/commit API inside `capture-transfer`
- tests proving claimed slots publish without requiring a source pixel slice
- reuse of the existing metadata ring and pool identity fields
- documentation of the remaining adapter-level `Vec<u8>` copy

This plan does not implement:

- changing `porthole_core::VideoCaptureFrame` away from `Vec<u8>`
- direct ScreenCaptureKit callback-to-slot fill
- IOSurface/native handles
- cross-process shared control blocks
- wake primitives

## File Structure

- Modify `crates/capture-transfer/src/video.rs`
  - Add `ClaimedVideoSlot`.
  - Add `claim_video_slot(track_id, desc, len)` and `commit_video_slot(claim)`.
  - Route reusable-pool `publish` through claim + copy + commit.
- Modify `docs/superpowers/specs/2026-05-12-capture-transfer-protocol-design.md`
  - Note that CPU producers can now write directly into managed slots, while porthole adapter integration still owns bytes today.
- Modify `docs/superpowers/specs/2026-05-13-capture-transfer-architecture-refinement-design.md`
  - Capture the claim/fill/publish shape as the current CPU baseline direction.

## Tasks

- [x] Add failing tests for claim/fill/commit publishing bytes and slot identity.
- [x] Implement claim/commit for reusable pool mode.
- [x] Preserve immutable per-frame `publish` behavior for non-pool managers.
- [x] Update docs.
- [x] Run full repo verification.
