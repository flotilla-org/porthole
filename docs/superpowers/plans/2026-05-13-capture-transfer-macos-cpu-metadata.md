# Capture Transfer macOS CPU Metadata Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add forward-compatible frame metadata to the existing macOS CPU shared-memory capture path without changing the payload transport yet.

**Architecture:** Keep the current `CVPixelBuffer -> Vec<u8> -> shared-memory fd` path and thread explicit metadata through the Rust model, daemon wire structs, C ABI, porthole adapter frame type, and SDL consumer. Default fields conservatively: BGRA CPU frames use `cpu_copy_complete` sync, full-frame damage, unknown color unless the producer knows better, and explicit timestamp clock domains for CoreGraphics seed frames versus ScreenCaptureKit live frames. Add observable loss counters for dropped capture frames and latest-frame skips, but defer reusable shm pools and IOSurface handle transfer to later plans.

**Tech Stack:** Rust workspace, C ABI header, serde JSON daemon structs, ScreenCaptureKit Objective-C shim, SDL C viewer.

---

## Scope

This plan implements Phase 1 from the architecture refinement spec:

- explicit frame metadata fields with conservative defaults
- timestamp clock-domain clarification
- basic loss/drop counters
- CPU shm remains the baseline

This plan does not implement:

- reusable CPU shared-memory pools
- fixed hot shared control blocks
- IOSurface/Mach/XPC handle transfer
- dmabuf/Linux support
- audio or ordered recording cursors

## File Structure

- Modify `crates/capture-transfer/src/model.rs`
  - Add protocol enums for timestamp clock domain, sync kind, damage kind, and frame color metadata defaults.
- Modify `crates/capture-transfer/src/video.rs`
  - Extend `VideoFrameDesc`.
  - Track latest-frame skip/loss counters in `VideoSlotManager`.
  - Keep current per-frame shared-memory segment behavior.
- Modify `crates/capture-transfer/src/ffi.rs`
  - Add C ABI constants and fields.
  - Preserve explicit conversion functions.
  - Update ABI tests.
- Modify `crates/capture-transfer/include/capture_transfer.h`
  - Mirror ABI constants and struct fields.
- Modify `crates/capture-transfer/src/daemon.rs`
  - Parse new daemon response fields and fill `VideoFrameDesc`.
- Modify `crates/porthole-protocol/src/capture_sessions.rs`
  - Add new serde fields to `LatestVideoFrameResponse` and `CaptureSessionResponse` where needed.
- Modify `crates/porthole-core/src/adapter.rs`
  - Add capture-frame metadata fields used by adapters.
- Modify `crates/porthole-core/src/in_memory.rs`
  - Fill metadata defaults for deterministic tests.
- Modify `crates/porthole-adapter-macos/src/sck_capture.rs`
  - Mark seed frames and live SCK frames with explicit clock domains and default sync/damage/color metadata.
  - Count callback drops from bounded channel `try_send`.
- Modify `crates/porthole-adapter-macos/src/sck_capture_shim.m`
  - Leave pixel-copy behavior unchanged; only add shim fields if a concrete SCK metadata field is needed.
- Modify `crates/portholed/src/capture_registry.rs`
  - Store and return the new frame metadata.
- Modify `tools/capture-viewer-sdl/src/main.c`
  - Compile against the expanded C ABI; no rendering behavior change expected.
- Modify tests under the touched crates.
  - Keep real ScreenCaptureKit tests manual/ignored per repo permission rules.

## Chunk 1: Capture-Transfer Metadata Model

### Task 1: Add protocol metadata enums

**Files:**
- Modify: `crates/capture-transfer/src/model.rs`

- [ ] **Step 1: Add tests for metadata enum defaults**

Add tests in `model.rs` covering the exact default values expected for CPU BGRA frames:

```rust
#[test]
fn frame_metadata_defaults_are_explicit() {
    assert_eq!(ClockDomain::Unknown as u32, 0);
    assert_eq!(ClockDomain::UnixTime as u32, 1);
    assert_eq!(ClockDomain::MediaTime as u32, 2);
    assert_eq!(FrameSyncKind::CpuCopyComplete as u32, 1);
    assert_eq!(DamageKind::FullFrame as u32, 1);
    assert_eq!(ColorSpace::Unknown as u32, 0);
}
```

- [ ] **Step 2: Run the narrow failing test**

Run:

```bash
cargo test -p capture-transfer model::tests::frame_metadata_defaults_are_explicit --locked
```

Expected: FAIL because the new enums do not exist yet.

- [ ] **Step 3: Implement model enums**

Add these enums with explicit discriminants:

```rust
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClockDomain {
    Unknown = 0,
    UnixTime = 1,
    MediaTime = 2,
    HostTime = 3,
}

#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameSyncKind {
    Unknown = 0,
    CpuCopyComplete = 1,
    SckSampleReady = 2,
    NativeTimeline = 3,
}

#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DamageKind {
    Unknown = 0,
    FullFrame = 1,
    None = 2,
    InlineRects = 3,
    SidecarRects = 4,
}
```

Keep `ColorSpace::Unknown` and `ColorSpace::Srgb`; add explicit discriminants if needed.

- [ ] **Step 4: Run model tests**

Run:

```bash
cargo test -p capture-transfer model::tests::frame_metadata_defaults_are_explicit --locked
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/capture-transfer/src/model.rs
git commit -m "feat(capture-transfer): add frame metadata enums"
```

### Task 2: Extend `VideoFrameDesc`

**Files:**
- Modify: `crates/capture-transfer/src/video.rs`

- [ ] **Step 1: Write tests for default metadata propagation**

Update the `frame_desc()` test helper in `video.rs` to include:

```rust
clock_domain: ClockDomain::MediaTime,
color_space: ColorSpace::Unknown,
sync_kind: FrameSyncKind::CpuCopyComplete,
damage_kind: DamageKind::FullFrame,
damage_base_sequence: sequence,
dropped_before_publish: 0,
producer_drop_count: 0,
evicted_count: 0,
consumer_skipped_count: 0,
```

Add a test:

```rust
#[test]
fn acquiring_latest_preserves_frame_metadata() {
    let mut slots = VideoSlotManager::new(2);
    let track = TrackId::new(1);
    let mut desc = frame_desc(7);
    desc.damage_base_sequence = 3;
    desc.producer_drop_count = 2;

    slots.publish(track, desc.clone(), &[1, 2, 3, 4]).unwrap();

    let frame = slots.acquire_latest(ConsumerId::new(7), track).unwrap();
    assert_eq!(frame.desc, desc);
}
```

- [ ] **Step 2: Run the failing video tests**

Run:

```bash
cargo test -p capture-transfer video::tests --locked
```

Expected: FAIL until `VideoFrameDesc` is extended.

- [ ] **Step 3: Add fields to `VideoFrameDesc`**

Extend `VideoFrameDesc` with fixed-size scalar metadata only:

```rust
pub clock_domain: ClockDomain,
pub color_space: ColorSpace,
pub sync_kind: FrameSyncKind,
pub damage_kind: DamageKind,
pub damage_base_sequence: u64,
pub dropped_before_publish: u64,
pub producer_drop_count: u64,
pub evicted_count: u64,
pub consumer_skipped_count: u64,
```

Do not add variable-size damage arrays or sidecar refs in this slice.

- [ ] **Step 4: Add basic skip/eviction accounting**

In `VideoSlotManager`, maintain per-track state sufficient to report:

- `consumer_skipped_count`: when a consumer acquires latest and its previous acquired sequence for that track was older than `latest.sequence - 1`.
- `evicted_count`: number of unpinned frames pruned for the track.

Use simple maps keyed by `(ConsumerId, TrackId)` and `TrackId`; avoid shared atomics in this prototype.

- [ ] **Step 5: Run capture-transfer tests**

Run:

```bash
cargo test -p capture-transfer --locked
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/capture-transfer/src/video.rs
git commit -m "feat(capture-transfer): carry explicit video metadata"
```

## Chunk 2: C ABI And Daemon Wire Metadata

### Task 3: Extend the C ABI structs and conversions

**Files:**
- Modify: `crates/capture-transfer/include/capture_transfer.h`
- Modify: `crates/capture-transfer/src/ffi.rs`

- [ ] **Step 1: Add ABI test expectations**

Update `producer_consumer_smoke_through_c_abi` in `ffi.rs` so `FtVideoFrameDesc` includes the new fields and asserts they survive acquire:

```rust
assert_eq!(frame.desc.clock_domain, FT_CLOCK_DOMAIN_MEDIA_TIME);
assert_eq!(frame.desc.color_space, FT_COLOR_SPACE_UNKNOWN);
assert_eq!(frame.desc.sync_kind, FT_FRAME_SYNC_CPU_COPY_COMPLETE);
assert_eq!(frame.desc.damage_kind, FT_DAMAGE_FULL_FRAME);
assert_eq!(frame.desc.damage_base_sequence, 1);
```

- [ ] **Step 2: Run the failing ABI test**

Run:

```bash
cargo test -p capture-transfer ffi::tests::producer_consumer_smoke_through_c_abi --locked
```

Expected: FAIL because the C ABI fields/constants are missing.

- [ ] **Step 3: Update the C header**

Add constants:

```c
#define FT_CLOCK_DOMAIN_UNKNOWN 0
#define FT_CLOCK_DOMAIN_UNIX_TIME 1
#define FT_CLOCK_DOMAIN_MEDIA_TIME 2
#define FT_CLOCK_DOMAIN_HOST_TIME 3

#define FT_COLOR_SPACE_UNKNOWN 0
#define FT_COLOR_SPACE_SRGB 1

#define FT_FRAME_SYNC_UNKNOWN 0
#define FT_FRAME_SYNC_CPU_COPY_COMPLETE 1
#define FT_FRAME_SYNC_SCK_SAMPLE_READY 2
#define FT_FRAME_SYNC_NATIVE_TIMELINE 3

#define FT_DAMAGE_UNKNOWN 0
#define FT_DAMAGE_FULL_FRAME 1
#define FT_DAMAGE_NONE 2
#define FT_DAMAGE_INLINE_RECTS 3
#define FT_DAMAGE_SIDECAR_RECTS 4
```

Extend `ft_video_frame_desc` after existing fields:

```c
uint32_t clock_domain;
uint32_t color_space;
uint32_t sync_kind;
uint32_t damage_kind;
uint64_t damage_base_sequence;
uint64_t dropped_before_publish;
uint64_t producer_drop_count;
uint64_t evicted_count;
uint64_t consumer_skipped_count;
```

This is a breaking pre-release ABI change; no compatibility shim is needed.

- [ ] **Step 4: Update Rust FFI conversions**

Mirror the constants and fields in `ffi.rs`. Add conversion helpers for the new enums.

- [ ] **Step 5: Run ABI tests**

Run:

```bash
cargo test -p capture-transfer ffi::tests --locked
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/capture-transfer/include/capture_transfer.h crates/capture-transfer/src/ffi.rs
git commit -m "feat(capture-transfer): expose frame metadata in C ABI"
```

### Task 4: Extend daemon JSON metadata

**Files:**
- Modify: `crates/porthole-protocol/src/capture_sessions.rs`
- Modify: `crates/capture-transfer/src/daemon.rs`
- Modify: `crates/portholed/src/capture_registry.rs`

- [ ] **Step 1: Add daemon response tests**

In the existing daemon/capture registry tests, assert that latest frame JSON includes and round-trips:

```rust
clock_domain
color_space
sync_kind
damage_kind
damage_base_sequence
dropped_before_publish
producer_drop_count
evicted_count
consumer_skipped_count
```

If there is not a narrow existing test, add one near the synthetic session tests in `crates/portholed/src/server.rs` or the relevant test module that currently covers `capture_sessions`.

- [ ] **Step 2: Run the failing daemon tests**

Run:

```bash
cargo test -p portholed capture --locked
```

Expected: FAIL until wire structs and registry responses include the fields.

- [ ] **Step 3: Extend protocol structs**

Add the new serde fields to `LatestVideoFrameResponse`. Add session-level defaults to `CaptureSessionResponse` only if the consumer needs initial metadata before first frame; otherwise keep the new fields frame-level.

Use string enum names for JSON where the current protocol already uses strings, for example:

```rust
pub clock_domain: String,
pub color_space: String,
pub sync_kind: String,
pub damage_kind: String,
```

Use integers for counters and sequence references.

- [ ] **Step 4: Update capture registry response construction**

In `CaptureRegistry::latest_frame`, populate the new response fields from `frame.desc`.

Add string conversion helpers next to `pixel_format_name`.

- [ ] **Step 5: Update daemon client parser**

In `capture-transfer/src/daemon.rs`, extend `LatestFrameWire` and parse string enum names back into `VideoFrameDesc`.

- [ ] **Step 6: Run targeted tests**

Run:

```bash
cargo test -p capture-transfer daemon::tests --locked
cargo test -p portholed capture --locked
```

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add crates/porthole-protocol/src/capture_sessions.rs crates/capture-transfer/src/daemon.rs crates/portholed/src/capture_registry.rs crates/portholed/src/server.rs
git commit -m "feat(capture-transfer): carry frame metadata over daemon transport"
```

## Chunk 3: macOS Producer Metadata

### Task 5: Extend porthole capture frame metadata

**Files:**
- Modify: `crates/porthole-core/src/adapter.rs`
- Modify: `crates/porthole-core/src/in_memory.rs`
- Modify: `crates/porthole-adapter-macos/src/sck_capture.rs`

- [ ] **Step 1: Add core tests/defaults**

Update in-memory video capture fixture tests so synthetic `VideoCaptureFrame` values include explicit metadata:

```rust
timestamp_clock: VideoCaptureClockDomain::UnixTime,
color_space: VideoCaptureColorSpace::Unknown,
sync_kind: VideoCaptureSyncKind::CpuCopyComplete,
damage_kind: VideoCaptureDamageKind::FullFrame,
damage_base_sequence: sequence,
dropped_before_publish: 0,
producer_drop_count: 0,
```

- [ ] **Step 2: Run the failing core tests**

Run:

```bash
cargo test -p porthole-core in_memory --locked
```

Expected: FAIL until the core frame type is extended.

- [ ] **Step 3: Add core capture metadata enums**

In `adapter.rs`, add:

```rust
pub enum VideoCaptureClockDomain { Unknown, UnixTime, MediaTime, HostTime }
pub enum VideoCaptureColorSpace { Unknown, Srgb }
pub enum VideoCaptureSyncKind { Unknown, CpuCopyComplete, SckSampleReady, NativeTimeline }
pub enum VideoCaptureDamageKind { Unknown, FullFrame, None }
```

Extend `VideoCaptureFrame` with matching fields and counters.

- [ ] **Step 4: Update in-memory fixtures**

Fill the new fields wherever `VideoCaptureFrame` is constructed in `in_memory.rs`.

- [ ] **Step 5: Run core tests**

Run:

```bash
cargo test -p porthole-core --locked
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/porthole-core/src/adapter.rs crates/porthole-core/src/in_memory.rs
git commit -m "feat(porthole-core): add video capture metadata fields"
```

### Task 6: Mark seed versus SCK live frame clocks and drops

**Files:**
- Modify: `crates/porthole-adapter-macos/src/sck_capture.rs`
- Modify: `crates/porthole-adapter-macos/src/sck_capture_shim.m` only if needed

- [ ] **Step 1: Add focused unit-testable helper**

In `sck_capture.rs`, add a small pure helper for constructing metadata from frame source:

```rust
enum CaptureFrameSource { CoreGraphicsSeed, ScreenCaptureKitLive }
```

Test that:

- seed frames use `UnixTime`
- live SCK frames use `MediaTime`
- both default to `FullFrame` damage and `Unknown` color
- seed frames use `CpuCopyComplete`
- live frames use `SckSampleReady` after the callback copy completes

- [ ] **Step 2: Run the failing macOS adapter test**

Run:

```bash
cargo test -p porthole-adapter-macos sck_capture --locked
```

Expected: FAIL until helper and metadata fields exist.

- [ ] **Step 3: Implement metadata defaults in seed frame**

In `capture_initial_window_frame`, fill the new `VideoCaptureFrame` fields:

```rust
timestamp_clock: VideoCaptureClockDomain::UnixTime,
color_space: VideoCaptureColorSpace::Unknown,
sync_kind: VideoCaptureSyncKind::CpuCopyComplete,
damage_kind: VideoCaptureDamageKind::FullFrame,
damage_base_sequence: 1,
dropped_before_publish: 0,
producer_drop_count: 0,
```

- [ ] **Step 4: Track callback drops**

Add `dropped_before_publish: AtomicU64` to `CallbackState`.

When `try_send` fails for a live frame, increment the counter. When publishing a live frame succeeds, use `swap(0, Ordering::Relaxed)` for `dropped_before_publish` and increment a cumulative `producer_drop_count` if needed.

Do not block the SCK callback to preserve latest-frame behavior.

- [ ] **Step 5: Implement live frame metadata**

In `frame_callback`, fill:

```rust
timestamp_clock: VideoCaptureClockDomain::MediaTime,
color_space: VideoCaptureColorSpace::Unknown,
sync_kind: VideoCaptureSyncKind::SckSampleReady,
damage_kind: VideoCaptureDamageKind::FullFrame,
damage_base_sequence: sequence,
```

Keep SCK dirty rect extraction for a later slice.

- [ ] **Step 6: Run adapter tests**

Run:

```bash
cargo test -p porthole-adapter-macos --locked
```

Expected: PASS for non-ignored tests. Permission-dependent ignored tests are not part of this command.

- [ ] **Step 7: Commit**

```bash
git add crates/porthole-adapter-macos/src/sck_capture.rs
git commit -m "feat(porthole): annotate macOS capture frame metadata"
```

### Task 7: Convert porthole capture metadata into capture-transfer metadata

**Files:**
- Modify: `crates/portholed/src/capture_registry.rs`

- [ ] **Step 1: Add registry conversion tests**

Add tests for converting `VideoCaptureFrame` metadata into `VideoFrameDesc`, covering:

- seed-style `UnixTime`
- live-style `MediaTime`
- drop counters
- full-frame damage base sequence

- [ ] **Step 2: Run the failing registry tests**

Run:

```bash
cargo test -p portholed capture_registry --locked
```

Expected: FAIL until conversion helpers exist.

- [ ] **Step 3: Add conversion helpers**

Add focused helper functions near `capture_pixel_format`:

```rust
fn capture_clock_domain(domain: VideoCaptureClockDomain) -> ClockDomain
fn capture_color_space(space: VideoCaptureColorSpace) -> ColorSpace
fn capture_sync_kind(kind: VideoCaptureSyncKind) -> FrameSyncKind
fn capture_damage_kind(kind: VideoCaptureDamageKind) -> DamageKind
```

Update `publish_capture_frame_to_video` to fill all fields in `VideoFrameDesc`.

- [ ] **Step 4: Run registry tests**

Run:

```bash
cargo test -p portholed capture_registry --locked
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/portholed/src/capture_registry.rs
git commit -m "feat(portholed): publish capture frame metadata"
```

## Chunk 4: Consumer And Documentation Updates

### Task 8: Keep SDL viewer compiling

**Files:**
- Modify: `tools/capture-viewer-sdl/src/main.c`

- [ ] **Step 1: Build the C ABI library**

Run:

```bash
cargo build -p capture-transfer --locked
```

Expected: PASS.

- [ ] **Step 2: Build SDL viewer**

Run the documented viewer build:

```bash
cmake -S tools/capture-viewer-sdl -B target/capture-viewer-sdl \
  -DCAPTURE_TRANSFER_LIB="$PWD/target/debug/libcapture_transfer.dylib"
cmake --build target/capture-viewer-sdl
```

Expected: FAIL if the viewer assumes the old `ft_video_frame_desc` shape or includes stale header behavior.

- [ ] **Step 3: Update viewer if needed**

If compilation fails, update the viewer to initialize/consume the expanded struct. Rendering should still use only:

- `frame.desc.width`
- `frame.desc.height`
- `frame.desc.stride`
- `frame.desc.pixel_format`
- `frame.data`

Do not add color conversion or damage rendering in this slice.

- [ ] **Step 4: Run dummy smoke**

Run:

```bash
SDL_VIDEODRIVER=dummy target/capture-viewer-sdl/capture-viewer-sdl --frames 3
```

Expected: exits successfully after 3 frames.

- [ ] **Step 5: Commit**

```bash
git add tools/capture-viewer-sdl/src/main.c
git commit -m "fix(capture-transfer): update SDL viewer for frame metadata ABI"
```

### Task 9: Update docs for metadata semantics

**Files:**
- Modify: `docs/superpowers/specs/2026-05-12-capture-transfer-protocol-design.md`
- Modify: `docs/superpowers/specs/2026-05-13-capture-transfer-architecture-refinement-design.md` if implementation semantics differ.
- Modify: `tools/capture-viewer-sdl/README.md` only if command behavior changes.

- [ ] **Step 1: Update protocol spec**

Add a short subsection under video frame metadata explaining:

- timestamp clock domain
- color defaults
- sync kind defaults
- full-frame damage default
- basic drop/loss counters

- [ ] **Step 2: Check docs diff**

Run:

```bash
git diff -- docs/superpowers/specs tools/capture-viewer-sdl/README.md
```

Expected: docs describe actual fields implemented in this plan.

- [ ] **Step 3: Commit docs**

```bash
git add docs/superpowers/specs tools/capture-viewer-sdl/README.md
git commit -m "docs(capture-transfer): document frame metadata semantics"
```

## Chunk 5: Final Verification

### Task 10: Run required repository gates

**Files:**
- No source edits expected unless checks fail.

- [ ] **Step 1: Build workspace**

Run:

```bash
cargo build --workspace --locked
```

Expected: PASS.

- [ ] **Step 2: Test workspace**

Run:

```bash
cargo test --workspace --locked
```

Expected: PASS. Ignored ScreenCaptureKit integration tests are not run here.

- [ ] **Step 3: Run clippy**

Run:

```bash
cargo clippy --workspace --all-targets --locked -- -D warnings
```

Expected: PASS.

- [ ] **Step 4: Check formatting**

Run:

```bash
cargo +nightly-2026-03-12 fmt --check
```

Expected: PASS.

- [ ] **Step 5: Optional manual real-capture smoke**

Only run this if Accessibility and Screen Recording are already granted. If either permission is missing, stop with `BLOCKED` per `AGENTS.md`; do not invent a code bypass.

Run:

```bash
./scripts/manual-capture-transfer-smoke.sh --frames 300
```

Expected: SDL viewer receives real frames and exits cleanly.

- [ ] **Step 6: Final commit if checks required fixes**

If verification required follow-up edits:

```bash
git add <changed-files>
git commit -m "fix(capture-transfer): address metadata verification issues"
```

Expected: no uncommitted source changes remain except explicitly accepted local drift such as unrelated `Cargo.lock` changes.
