# Capture Transfer Protocol Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build an implemented capture-transfer protocol draft with porthole as a video producer and a standalone SDL viewer as a C-ABI consumer.

**Architecture:** Add a `capture-transfer` Rust crate that models sessions, sources, tracks, events, shared-memory video payloads, and `SCM_RIGHTS` fd passing over a raw Unix-domain side channel, then expose a narrow C ABI. Integrate porthole as the first real producer and add a small SDL viewer that attaches to a session and displays latest video frames.

**Tech Stack:** Rust workspace, C ABI, local shared memory, ScreenCaptureKit via the existing macOS adapter path, SDL for the standalone viewer.

---

## Chunk 1: Protocol Core And State Machine

### Task 1: Add the crate skeleton

**Files:**
- Modify: `Cargo.toml`
- Create: `crates/capture-transfer/Cargo.toml`
- Create: `crates/capture-transfer/src/lib.rs`
- Create: `crates/capture-transfer/src/model.rs`
- Create: `crates/capture-transfer/src/error.rs`

- [ ] **Step 1: Add the new workspace member**

Add `crates/capture-transfer` to the workspace members in `Cargo.toml`.

- [ ] **Step 2: Create the crate manifest**

Create `crates/capture-transfer/Cargo.toml` with a Rust library target. Keep dependencies minimal until the shared-memory layer needs a platform crate.

- [ ] **Step 3: Define the initial modules**

Create `lib.rs`, `model.rs`, and `error.rs`. Export the model and error types from `lib.rs`.

- [ ] **Step 4: Run the narrow check**

Run: `cargo test -p capture-transfer --locked`

Expected: the new crate compiles and has zero tests.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml crates/capture-transfer
git commit -m "feat(capture-transfer): add protocol crate skeleton"
```

### Task 2: Model sessions, sources, tracks, and video metadata

**Files:**
- Modify: `crates/capture-transfer/src/model.rs`
- Modify: `crates/capture-transfer/src/error.rs`
- Create: `crates/capture-transfer/src/state.rs`

- [ ] **Step 1: Write model tests**

Add tests for id allocation, registering a source, registering a video track for a source, rejecting duplicate or unknown ids, and replaying current registration state.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p capture-transfer --locked`

Expected: tests fail because the state machine does not exist yet.

- [ ] **Step 3: Implement model types**

Define typed ids for sessions, sources, tracks, and frame sequences. Define enums for `TrackType`, `PayloadKind`, `PixelFormat`, `ColorSpace`, and `EventKind`.

- [ ] **Step 4: Implement the state machine**

Implement source registration, track registration, source updates, track updates, unregister, and replay events.

- [ ] **Step 5: Run tests**

Run: `cargo test -p capture-transfer --locked`

Expected: all `capture-transfer` tests pass.

- [ ] **Step 6: Commit**

```bash
git add crates/capture-transfer
git commit -m "feat(capture-transfer): model sessions sources and tracks"
```

## Chunk 2: Shared-Memory Video Payloads

### Task 3: Implement latest-frame video slots

**Files:**
- Create: `crates/capture-transfer/src/video.rs`
- Create: `crates/capture-transfer/src/shm.rs`
- Modify: `crates/capture-transfer/src/lib.rs`

- [ ] **Step 1: Write slot tests**

Add tests for publishing frames, acquiring the latest frame, skipping stale frames, pinning acquired frames, releasing frames, and cleaning up pins on consumer disconnect.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p capture-transfer --locked`

Expected: tests fail because the video slot manager is not implemented.

- [ ] **Step 3: Implement an in-process slot manager first**

Implement the latest-frame semantics without OS shared memory. This isolates lifecycle behavior from platform details.

- [ ] **Step 4: Run tests**

Run: `cargo test -p capture-transfer --locked`

Expected: slot lifecycle tests pass.

- [ ] **Step 5: Add the local shared-memory backing**

Replace or extend the in-process backing with local shared memory while preserving the same tests. Add platform-specific tests only where they can run without macOS privacy permissions.

- [ ] **Step 6: Commit**

```bash
git add crates/capture-transfer
git commit -m "feat(capture-transfer): publish latest video frames through shared memory"
```

## Chunk 3: C ABI

### Task 4: Expose producer and consumer handles

**Files:**
- Create: `crates/capture-transfer/src/ffi.rs`
- Modify: `crates/capture-transfer/src/lib.rs`
- Modify: `crates/capture-transfer/Cargo.toml`
- Create: `crates/capture-transfer/include/capture_transfer.h`

- [ ] **Step 1: Write Rust ABI smoke tests**

Add tests that call the extern functions through Rust declarations: create producer, register source, register video track, publish a frame, create consumer, poll replay events, acquire latest frame, release frame, and destroy handles.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p capture-transfer --locked`

Expected: tests fail because the FFI layer is missing.

- [ ] **Step 3: Implement opaque handles and status codes**

Add `ft_producer`, `ft_consumer`, `ft_status`, options structs, descriptor structs, and explicit destroy functions. Keep every allocation and release path owned by the library.

- [ ] **Step 4: Add the C header**

Add `capture_transfer.h` with the v1 ABI declarations and version constants.

- [ ] **Step 5: Run tests**

Run: `cargo test -p capture-transfer --locked`

Expected: ABI smoke tests pass.

- [ ] **Step 6: Commit**

```bash
git add crates/capture-transfer
git commit -m "feat(capture-transfer): expose v1 C ABI"
```

## Chunk 4: SDL Viewer Dogfood

### Task 5: Add the standalone viewer

**Files:**
- Create: `tools/capture-viewer-sdl/README.md`
- Create: `tools/capture-viewer-sdl/CMakeLists.txt` or equivalent local build file
- Create: `tools/capture-viewer-sdl/src/main.c`
- Modify: `docs/development.md`

- [ ] **Step 1: Write down the manual command shape**

Document how the viewer will receive a session descriptor or UDS path from the producer.

- [ ] **Step 2: Create the viewer skeleton**

Implement argument parsing, connection setup through the C ABI, SDL window creation, event polling, and clean shutdown.

- [ ] **Step 3: Implement video display**

On source and video-track registration, create or resize an SDL texture. On each loop, acquire the latest video frame, upload the CPU pixels, render, and release the frame.

- [ ] **Step 4: Run the viewer against a synthetic producer**

Run the producer test fixture and viewer locally.

Expected: the SDL window displays changing frames without relying on Screen Recording permission.

- [ ] **Step 5: Commit**

```bash
git add tools/capture-viewer-sdl docs/development.md
git commit -m "feat(capture-transfer): add SDL viewer consumer"
```

## Chunk 5: FD Transfer And Porthole Producer Integration

### Task 6: Add SCM_RIGHTS fd-passing primitives

**Files:**
- Create: `crates/capture-transfer/src/fdpass.rs`
- Modify: `crates/capture-transfer/src/lib.rs`

- [ ] **Step 1: Write fd-passing tests**

Use `UnixStream::pair()` to verify a file descriptor can be sent with `SCM_RIGHTS`, received as an owned fd, and read from the receiving side.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p capture-transfer --locked`

Expected: tests fail because the fd-passing module is not implemented.

- [ ] **Step 3: Implement send/receive helpers**

Use `sendmsg` / `recvmsg` with `SCM_RIGHTS`. Keep the API low-level and Unix-only for now.

- [ ] **Step 4: Run tests**

Run: `cargo test -p capture-transfer --locked`

Expected: fd-passing tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/capture-transfer
git commit -m "feat(capture-transfer): add fd passing primitives"
```

### Task 7: Add a daemon-owned synthetic capture session registry

**Files:**
- Modify: `crates/portholed/src/state.rs`
- Modify: `crates/portholed/src/server.rs`
- Create: `crates/portholed/src/routes/capture_sessions.rs`
- Modify: `crates/portholed/src/routes/mod.rs`
- Modify: `crates/portholed/Cargo.toml`
- Modify: `crates/porthole-protocol` if wire structs are needed
- Modify: `docs/recipes/terminal-orchestration.md` or create a capture recipe if a separate doc is clearer

- [ ] **Step 1: Inspect current CLI and daemon route patterns**

Use `rg` to find the existing launch, screenshot, and route test structure before choosing exact wire files.

- [ ] **Step 2: Add tests for synthetic session creation and discovery**

Write daemon tests that create a synthetic capture session, return a session id, expose source/track metadata, and return a raw fd-transfer socket path or token. Do not require macOS Screen Recording permission.

- [ ] **Step 3: Implement registry state**

Add a daemon-owned capture session registry to `AppState`. The first registry can own synthetic video producers and latest-frame slots.

- [ ] **Step 4: Implement HTTP registry routes**

Add control-plane routes for synthetic session creation and metadata discovery. These routes do not carry file descriptors.

- [ ] **Step 5: Implement the raw fd-transfer side channel**

Add a raw UDS listener for capture-transfer handle requests. The first operation is consumer-initiated latest-frame acquisition: request `{ session_id, track_id }`, reply with metadata plus one shared-memory fd via `SCM_RIGHTS`.

- [ ] **Step 6: Run targeted tests**

Run the portholed package tests that cover synthetic session creation and fd side-channel transfer.

- [ ] **Step 7: Commit**

```bash
git add crates docs
git commit -m "feat(portholed): add synthetic capture session registry"
```

### Task 8: Wire SDL viewer to daemon sessions

**Files:**
- Modify: `tools/capture-viewer-sdl/src/main.c`
- Modify: `tools/capture-viewer-sdl/README.md`
- Modify: `docs/development.md`

- [ ] **Step 1: Add viewer argument parsing tests or smoke coverage**

Keep the in-process synthetic mode, but add a `--session` or equivalent descriptor mode that connects to portholed.

- [ ] **Step 2: Implement descriptor mode**

Use the HTTP-over-UDS registry for metadata and the raw fd-transfer side channel for latest-frame descriptors.

- [ ] **Step 3: Run the viewer against daemon synthetic session**

Run the daemon, create a synthetic session, and verify the viewer displays or dummy-smokes frames from the daemon.

- [ ] **Step 4: Commit**

```bash
git add tools docs
git commit -m "feat(capture-transfer): attach SDL viewer to daemon sessions"
```

### Task 9: Wire real ScreenCaptureKit frames

**Files:**
- Modify: `crates/porthole-adapter-macos` capture-related files after inspecting current adapter layout
- Modify: relevant daemon/CLI files for the chosen command shape
- Create or modify: `scripts/manual-capture-transfer-smoke.sh`

- [ ] **Step 1: Confirm permission state before real capture**

Run the existing permission/status command. If Accessibility or Screen Recording is missing, stop with `BLOCKED` and ask the user to grant permissions. Do not add bypasses.

- [ ] **Step 2: Write or update ignored/manual tests**

Add manual or ignored integration coverage for real ScreenCaptureKit publishing.

- [ ] **Step 3: Implement ScreenCaptureKit publication**

Copy captured frames into capture-transfer video payloads, preserving width, height, stride, timestamp, format, and sequence.

- [ ] **Step 4: Run manual smoke**

Run the porthole producer and SDL viewer against a real window or display.

Expected: the SDL viewer displays real porthole-captured frames and exits cleanly when the producer stops.

- [ ] **Step 5: Commit**

```bash
git add crates scripts docs
git commit -m "feat(porthole): publish ScreenCaptureKit frames"
```

## Chunk 6: Final Verification

### Task 10: Run repository gates

**Files:**
- No source edits expected unless checks fail.

- [ ] **Step 1: Build**

Run: `cargo build --workspace --locked`

Expected: success.

- [ ] **Step 2: Test**

Run: `cargo test --workspace --locked`

Expected: success. Ignored permission-dependent macOS tests do not run here.

- [ ] **Step 3: Clippy**

Run: `cargo clippy --workspace --all-targets --locked -- -D warnings`

Expected: success.

- [ ] **Step 4: Format check**

Run: `cargo +nightly-2026-03-12 fmt --check`

Expected: success.

- [ ] **Step 5: Update the design spec if implementation changed semantics**

If the implementation forced protocol changes, update `docs/superpowers/specs/2026-05-12-capture-transfer-protocol-design.md` before finishing.

- [ ] **Step 6: Commit any final docs updates**

```bash
git add docs
git commit -m "docs: update capture transfer protocol notes"
```
