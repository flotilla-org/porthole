# Capture Control Page FD Shadow Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Pass video control-page fds over the capture transfer channel and verify them as a shadow metadata path.

**Architecture:** Add a `register_video_control_page` setup message paired with an fd. The producer side sends one control page per connection/track before frame metadata; the daemon consumer maps it read-only, validates layout identity, and compares a seqlock read of the ring entry with the socket-delivered `video_frame`. Socket metadata and lease release remain authoritative.

**Tech Stack:** Rust, Unix-domain `SCM_RIGHTS`, mmap, existing `capture-transfer` and `portholed` tests.

---

## Chunk 1: Protocol and Mapping

### Task 1: Add wire message and tests

**Files:**
- Modify: `crates/capture-transfer/src/transfer_channel.rs`

- [x] **Step 1: Write the failing serde test**

Add `RegisterVideoControlPage` to `server_messages_roundtrip_with_producer_cursor` and assert:

```rust
let control = CaptureTransferMessage::RegisterVideoControlPage {
    session_id: "session-1".to_string(),
    track_id: 7,
    map_len: 4096,
};
let control_json = serde_json::to_value(&control).unwrap();
assert_eq!(control_json["op"], "register_video_control_page");
assert_eq!(control_json["map_len"], 4096);
assert_eq!(serde_json::from_value::<CaptureTransferMessage>(control_json).unwrap(), control);
```

- [x] **Step 2: Run test to verify failure**

Run: `cargo test -p capture-transfer transfer_channel --locked`

Expected: fail because `RegisterVideoControlPage` does not exist.

- [x] **Step 3: Add the enum variant**

Add:

```rust
RegisterVideoControlPage {
    session_id: String,
    track_id: u64,
    map_len: u64,
},
```

- [x] **Step 4: Run focused test**

Run: `cargo test -p capture-transfer transfer_channel --locked`

Expected: pass.

### Task 2: Expose and map control page fds

**Files:**
- Modify: `crates/capture-transfer/src/shm.rs`
- Modify: `crates/capture-transfer/src/control_page.rs`

- [x] **Step 1: Write failing tests**

In `control_page.rs`, add tests that:

- publish one entry, clone the control-page fd, map it read-only, and validate
  the header
- shadow-read cursor `1` from the mapped page and compare the ring fields

- [x] **Step 2: Run test to verify failure**

Run: `cargo test -p capture-transfer control_page --locked`

Expected: fail because fd clone/read-only mapping helpers do not exist.

- [x] **Step 3: Add read-only shared-memory mapping support**

Add a `SharedMemorySegment::map_read_only(fd: &OwnedFd, len: usize) -> Result<Self>`
or equivalent constructor that mmaps `PROT_READ | MAP_SHARED`. Preserve drop
unmapping. The mapped consumer segment must not remove the backing file on drop.

- [x] **Step 4: Add control page fd and mapped consumer API**

Add:

```rust
pub fn try_clone_fd(&self) -> Result<OwnedFd>
pub fn map_read_only(fd: OwnedFd, map_len: usize) -> Result<Self>
pub fn validate_header(&self) -> Result<VideoTrackControlHeader>
pub fn shadow_read_entry_for_cursor(&self, cursor: u64) -> Result<VideoRingEntry>
```

Use the existing `read_entry_for_cursor` logic internally where possible.

- [x] **Step 5: Run focused tests**

Run: `cargo test -p capture-transfer control_page --locked`

Expected: pass.

## Chunk 2: Daemon Consumer Shadow Path

### Task 3: Receive and compare control pages in `DaemonConsumer`

**Files:**
- Modify: `crates/capture-transfer/src/daemon.rs`

- [x] **Step 1: Write failing daemon consumer test**

Add a test server that sends:

1. `register_video_control_page` plus fd
2. `register_cpu_pool` plus fd
3. `video_frame`

Assert `DaemonConsumer::latest_frame` succeeds and uses the same payload bytes
as before.

- [x] **Step 2: Run test to verify failure**

Run: `cargo test -p capture-transfer daemon::tests::daemon_consumer_maps_control_page_shadow --locked`

Expected: fail because the consumer ignores/does not understand the message.

- [x] **Step 3: Add registered control page state**

Add a `BTreeMap<u64, VideoTrackControlPage>` or focused mapped-control wrapper
keyed by `track_id` in `DaemonConsumer`.

- [x] **Step 4: Handle registration message**

On `RegisterVideoControlPage`, receive the fd, map the page read-only, validate
it, and store it for the track.

- [x] **Step 5: Compare on video frame**

Before returning the `DaemonFrame`, if a control page exists for the track,
read the entry at `producer_cursor` and compare ring fields with the socket
metadata. Return a `DaemonTransport` error on mismatch.

- [x] **Step 6: Run focused test**

Run: `cargo test -p capture-transfer daemon::tests::daemon_consumer_maps_control_page_shadow --locked`

Expected: pass.

## Chunk 3: Daemon FD Socket Wiring

### Task 4: Send control page registrations from `portholed`

**Files:**
- Modify: `crates/capture-transfer/src/video.rs`
- Modify: `crates/portholed/src/capture_registry.rs`
- Modify: `crates/portholed/src/server.rs`

- [x] **Step 1: Write failing fd-socket tests**

Add/extend tests that assert:

- the first frame request sends `register_video_control_page` before
  `register_cpu_pool` or `video_frame`
- the control page fd maps and contains a matching cursor/entry
- repeated latest requests on the same connection do not resend the control page

- [x] **Step 2: Run test to verify failure**

Run: `cargo test -p portholed capture_fd_socket --locked`

Expected: fail because no control page registration is sent.

- [x] **Step 3: Expose control page fd from video manager**

Add a method on `VideoSlotManager`, through `TrackRingControl`, to clone the
track control-page fd and report mapped length.

- [x] **Step 4: Include control page fd in latest frame reply**

Extend internal `LatestFrameReply` with optional control page registration data.
This does not change HTTP protocol types.

- [x] **Step 5: Register once per connection/track**

In `handle_fd_connection`, track registered control pages in a `BTreeSet<u64>`.
Before sending frame metadata, send `RegisterVideoControlPage` and its fd if
the track has not been registered on this connection.

- [x] **Step 6: Run focused tests**

Run:

```bash
cargo test -p portholed capture_fd_socket --locked
cargo test -p capture-transfer daemon --locked
```

Expected: pass.

## Chunk 4: Verification and Publish

### Task 5: Final gates

**Files:**
- Modify: `docs/superpowers/specs/2026-05-17-capture-control-page-fd-shadow-design.md`
- Modify: `docs/superpowers/plans/2026-05-17-capture-control-page-fd-shadow.md`

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
git commit -m "feat(capture): shadow read mapped control pages"
```
