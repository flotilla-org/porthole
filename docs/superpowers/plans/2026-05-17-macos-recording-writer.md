# macOS Recording Writer Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build `porthole record surface <surface-id> --duration <duration> --output <path>.mov` on top of ordered capture-transfer frames.

**Architecture:** The CLI owns recording orchestration: it creates a surface capture session through the daemon, opens a `DaemonConsumer`, consumes frames with `next_frame_after`, releases every lease, and closes the session it created on all exit paths. A small macOS-only AVFoundation C shim writes BGRA frames into a QuickTime movie; non-macOS builds return an explicit unsupported error.

**Tech Stack:** Rust CLI, `capture-transfer::daemon::DaemonConsumer`, daemon capture-session HTTP routes, Objective-C AVFoundation/AVAssetWriter shim on macOS.

---

## Chunk 1: CLI Recording Loop

### Task 1: Add record command surface and duration parsing

**Files:**
- Modify: `crates/porthole/Cargo.toml`
- Modify: `crates/porthole/src/commands/mod.rs`
- Create: `crates/porthole/src/commands/record.rs`
- Modify: `crates/porthole/src/main.rs`
- Test: `crates/porthole/tests/record_cli.rs`

- [x] **Step 1: Write failing tests**

Add tests for:
- `parse_record_duration("5s") == Duration::from_secs(5)`
- `parse_record_duration("250ms") == Duration::from_millis(250)`
- zero durations fail
- unsupported suffixes fail
- `format_record_summary` includes output path, frame count, and lapped count

Run: `cargo test -p porthole --test record_cli --locked`

- [x] **Step 2: Implement parsing and output formatting**

Create `commands::record` with:
- `RecordArgs { surface_id, duration, output, best_effort, json, control_socket_path }`
- `parse_record_duration`
- `format_record_summary`
- public `run` entrypoint stub

Run: `cargo test -p porthole --test record_cli --locked`

- [x] **Step 3: Add clap command**

Add top-level `record surface`:

```text
porthole record surface <surface-id> --duration 5s --output out.mov
```

Flags:
- `--best-effort`
- `--json`

Run: `cargo test -p porthole --test record_cli --locked`

### Task 2: Add session lifecycle and ordered-frame loop

**Files:**
- Modify: `crates/porthole/Cargo.toml`
- Modify: `crates/porthole/src/commands/record.rs`
- Test: `crates/porthole/tests/record_cli.rs`

- [x] **Step 1: Write failing lifecycle/unit tests**

Use small fakes around traits local to `record.rs`:
- success closes the created capture session
- writer failures close the created capture session
- ordered lapping fails by default
- ordered lapping records a discontinuity only with `--best-effort`
- every acquired frame is released

Run: `cargo test -p porthole --test record_cli --locked`

- [x] **Step 2: Implement testable traits**

Add narrow traits:
- `CaptureSessionClient`
- `OrderedFrameConsumer`
- `MovieWriter`

Keep production adapters thin over `DaemonClient`, `DaemonConsumer`, and `AvMovieWriter`.

Run: `cargo test -p porthole --test record_cli --locked`

- [x] **Step 3: Implement production session and consumer adapters**

Production path:
1. `POST /capture-sessions/surfaces/{surface_id}`
2. `GET /capture-sessions/{session_id}` to retrieve dimensions/stride/pixel format
3. `DaemonConsumer::connect`
4. ordered loop using `next_frame_after(track_id, cursor)`
5. `release_frame` after appending or after append failure cleanup
6. `DELETE /capture-sessions/{session_id}`

Run: `cargo test -p porthole --test record_cli --locked`

## Chunk 2: macOS Movie Writer

### Task 3: Add AVFoundation shim

**Files:**
- Create: `crates/porthole/build.rs`
- Create: `crates/porthole/src/commands/record_av_writer.rs`
- Create: `crates/porthole/src/commands/record_av_writer_stub.rs`
- Create: `crates/porthole/src/commands/record_av_writer_shim.m`
- Modify: `crates/porthole/Cargo.toml`
- Modify: `crates/porthole/src/commands/record.rs`
- Test: `crates/porthole/tests/record_cli.rs`

- [x] **Step 1: Write failing writer construction tests**

Test Rust-side validation without requiring live capture:
- rejects non-`bgra8_unorm`
- rejects stride shorter than `width * 4`
- non-macOS writer returns unsupported

Run: `cargo test -p porthole --test record_cli --locked`

- [x] **Step 2: Implement Rust writer wrapper**

Expose `AvMovieWriter` behind `#[cfg(target_os = "macos")]` with:
- `new(path, width, height, stride, pixel_format)`
- `append(timestamp_ns, bytes)`
- `finish()`

Use presentation timestamps relative to the first appended frame.

Run: `cargo test -p porthole --test record_cli --locked`

- [x] **Step 3: Implement Objective-C shim**

Use:
- `AVAssetWriter` with `AVFileTypeQuickTimeMovie`
- `AVAssetWriterInput` with H.264 settings
- `AVAssetWriterInputPixelBufferAdaptor`
- `CVPixelBufferPoolCreatePixelBuffer`
- `appendPixelBuffer:withPresentationTime:`

Return C status codes plus a copied error string. Do not overwrite existing output files silently.

Run: `cargo build -p porthole --locked`

## Chunk 3: Verification and PR

### Task 4: Gates and manual smoke

**Files:**
- Modify docs if command behavior changes from the plan.

- [x] **Step 1: Run focused tests**

Run:
- `cargo test -p porthole --test record_cli --locked`
- `cargo test -p capture-transfer --locked`

- [x] **Step 2: Run workspace gates**

Run:
- `cargo build --workspace --locked`
- `cargo test --workspace --locked`
- `cargo clippy --workspace --all-targets --locked -- -D warnings`
- `cargo +nightly-2026-03-12 fmt --check`
- `git diff --check`

- [ ] **Step 3: Manual macOS smoke**

If Screen Recording is granted, run a short recording against a visible tracked surface and verify the `.mov` opens. If the permission is missing, stop with `BLOCKED` and ask the user to grant it; do not add code workarounds.

Status: `BLOCKED` for the freshly built temporary daemon binary because macOS
reports both Accessibility and Screen Recording missing for that binary identity.
The existing long-running daemon has grants but predates the ordered recording
protocol, so it closed the transfer channel when used for this smoke.
