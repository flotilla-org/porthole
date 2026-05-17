# macOS Recording Ordered Cursor Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the ordered frame acquisition prerequisite for honest macOS recording semantics.

**Architecture:** Add ordered acquisition below the recorder API: `VideoSlotManager` exposes next-after-cursor acquisition with explicit empty/lapped states, the raw capture-transfer channel carries an `acquire_next_video_frame` request plus `video_frame_unavailable` replies, and `portholed` leases ordered frames through the existing fd socket. The latest-frame viewer path remains unchanged.

**Tech Stack:** Rust workspace, `capture-transfer`, `portholed`, JSON-over-UDS capture transfer channel, CPU shared memory payload pools, existing lease release path.

---

Spec: `docs/superpowers/specs/2026-05-17-macos-recording-design.md`

## File Structure

- Modify `crates/capture-transfer/src/video.rs`
  - Add `OrderedVideoAcquire` result enum.
  - Add `VideoSlotManager::acquire_next_after`.
  - Add ring helper for retained cursor bounds.
  - Add unit tests for oldest/next/no-newer-frame/lapped/latest unchanged/release cursor behavior.

- Modify `crates/capture-transfer/src/transfer_channel.rs`
  - Add `CaptureTransferRequest::AcquireNextVideoFrame`.
  - Add `CaptureTransferMessage::VideoFrameUnavailable`.
  - Add serialization roundtrip tests.

- Modify `crates/portholed/src/capture_registry.rs`
  - Add registry path for ordered acquisition.
  - Add fd connection handling for `acquire_next_video_frame`.
  - Add lapped/unavailable reply serialization.
  - Add tests for ordered fd serving and lapped response.

- Modify `crates/capture-transfer/src/daemon.rs`
  - Add Rust daemon consumer method for ordered acquisition.
  - Add gap type for unavailable frames.
  - Add tests for request shape and unavailable parsing.

- Optional later follow-up, not in this plan:
  - Add public C ABI for ordered acquisition.
  - Add `porthole record`.
  - Add AVAssetWriter media output.

## Chunk 1: VideoSlotManager Ordered Acquisition

### Task 1: Add ordered-acquire tests

**Files:**
- Modify: `crates/capture-transfer/src/video.rs`

- [ ] **Step 1: Write failing test for cursor 0 returning the oldest retained frame**

Add near the existing `VideoSlotManager` tests:

```rust
#[test]
fn acquire_next_after_zero_returns_oldest_retained_frame() {
    let mut slots = VideoSlotManager::new_reusable_pool(3);
    let track = TrackId::new(7);
    let consumer = ConsumerId::new(11);
    publish_test_frame(&mut slots, track, 1, b"aaaa");
    publish_test_frame(&mut slots, track, 2, b"bbbb");

    let frame = match slots.acquire_next_after(consumer, track, 0).unwrap() {
        OrderedVideoAcquire::Frame(frame) => frame,
        other => panic!("expected frame, got {other:?}"),
    };

    assert_eq!(frame.producer_cursor(), 1);
    assert_eq!(frame.desc.sequence, 1);
    slots.release(frame);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p capture-transfer acquire_next_after_zero_returns_oldest_retained_frame --locked`

Expected: compile failure because `acquire_next_after` and `OrderedVideoAcquire` do not exist.

- [ ] **Step 3: Write failing tests for next retained, no newer frame, lapped, latest unchanged, and release cursor**

Add tests:

```rust
#[test]
fn acquire_next_after_retained_cursor_returns_next_frame() {
    let mut slots = VideoSlotManager::new_reusable_pool(3);
    let track = TrackId::new(7);
    let consumer = ConsumerId::new(11);
    publish_test_frame(&mut slots, track, 1, b"aaaa");
    publish_test_frame(&mut slots, track, 2, b"bbbb");
    publish_test_frame(&mut slots, track, 3, b"cccc");

    let frame = match slots.acquire_next_after(consumer, track, 1).unwrap() {
        OrderedVideoAcquire::Frame(frame) => frame,
        other => panic!("expected frame, got {other:?}"),
    };

    assert_eq!(frame.producer_cursor(), 2);
    assert_eq!(frame.desc.sequence, 2);
    slots.release(frame);
}

#[test]
fn acquire_next_after_latest_cursor_returns_empty() {
    let mut slots = VideoSlotManager::new_reusable_pool(3);
    let track = TrackId::new(7);
    publish_test_frame(&mut slots, track, 1, b"aaaa");

    let result = slots
        .acquire_next_after(ConsumerId::new(11), track, 1)
        .unwrap();

    assert_eq!(result, OrderedVideoAcquire::Empty);
}

#[test]
fn acquire_next_after_reports_lapped_cursor() {
    let mut slots = VideoSlotManager::new_reusable_pool(2);
    let track = TrackId::new(7);
    publish_test_frame(&mut slots, track, 1, b"aaaa");
    publish_test_frame(&mut slots, track, 2, b"bbbb");
    publish_test_frame(&mut slots, track, 3, b"cccc");
    publish_test_frame(&mut slots, track, 4, b"dddd");

    let result = slots
        .acquire_next_after(ConsumerId::new(11), track, 1)
        .unwrap();

    assert_eq!(
        result,
        OrderedVideoAcquire::Lapped {
            after_producer_cursor: 1,
            oldest_available_cursor: 3,
            latest_available_cursor: 4,
            skipped_count: 1,
        }
    );
}

#[test]
fn acquire_next_after_does_not_change_latest_semantics() {
    let mut slots = VideoSlotManager::new_reusable_pool(2);
    let track = TrackId::new(7);
    let consumer = ConsumerId::new(11);
    publish_test_frame(&mut slots, track, 1, b"aaaa");
    publish_test_frame(&mut slots, track, 2, b"bbbb");

    let latest = slots.acquire_latest(consumer, track).unwrap();

    assert_eq!(latest.producer_cursor(), 2);
    slots.release(latest);
}

#[test]
fn acquire_next_after_release_updates_consumer_release_cursor() {
    let mut slots = VideoSlotManager::new_reusable_pool(2);
    let track = TrackId::new(7);
    let consumer = ConsumerId::new(11);
    publish_test_frame(&mut slots, track, 1, b"aaaa");

    let frame = match slots.acquire_next_after(consumer, track, 0).unwrap() {
        OrderedVideoAcquire::Frame(frame) => frame,
        other => panic!("expected frame, got {other:?}"),
    };
    slots.release(frame);

    let registration = slots.control_page_registration(track, consumer).unwrap();
    let page = VideoTrackControlPage::map_writable(registration.fd, registration.map_len as usize).unwrap();
    assert_eq!(
        page.consumer_cursor_entry(registration.consumer_slot as usize)
            .release_cursor,
        1
    );
}
```

- [ ] **Step 4: Run focused tests to verify expected failures**

Run: `cargo test -p capture-transfer acquire_next_after --locked`

Expected: compile failure until production code exists.

### Task 2: Implement ordered acquire in VideoSlotManager

**Files:**
- Modify: `crates/capture-transfer/src/video.rs`

- [ ] **Step 1: Add public result enum**

Near `VideoStorageMode`, add:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OrderedVideoAcquire {
    Frame(AcquiredVideoFrame),
    Lapped {
        after_producer_cursor: u64,
        oldest_available_cursor: u64,
        latest_available_cursor: u64,
        skipped_count: u64,
    },
    Empty,
}
```

If deriving `PartialEq, Eq` fails because `AcquiredVideoFrame` does not support
them, manually implement test assertions with pattern matching and derive only
`Debug`.

- [ ] **Step 2: Add ring bounds helper**

Inside `TrackRingControl`, add:

```rust
fn retained_cursor_bounds(&self) -> Option<(u64, u64)> {
    let entries = self.ring_snapshot();
    let oldest = entries.first()?.producer_cursor;
    let latest = entries.last()?.producer_cursor;
    Some((oldest, latest))
}
```

- [ ] **Step 3: Add `acquire_next_after`**

Inside `impl VideoSlotManager`, near `acquire_cursor`, add:

```rust
pub fn acquire_next_after(
    &mut self,
    consumer_id: ConsumerId,
    track_id: TrackId,
    after_producer_cursor: u64,
) -> Result<OrderedVideoAcquire> {
    let control = self
        .controls_by_track
        .get(&track_id)
        .ok_or(CaptureTransferError::UnknownTrack { track_id })?;
    let Some((oldest, latest)) = control.retained_cursor_bounds() else {
        return Ok(OrderedVideoAcquire::Empty);
    };
    let target = if after_producer_cursor == 0 {
        oldest
    } else {
        after_producer_cursor.saturating_add(1)
    };
    if target < oldest {
        return Ok(OrderedVideoAcquire::Lapped {
            after_producer_cursor,
            oldest_available_cursor: oldest,
            latest_available_cursor: latest,
            skipped_count: oldest.saturating_sub(target),
        });
    }
    if target > latest {
        return Ok(OrderedVideoAcquire::Empty);
    }
    let entry = control
        .entry_for_cursor(target)
        .map_err(|source| CaptureTransferError::VideoControlRingRead { track_id, source })?;
    self.acquire_ring_entry(consumer_id, track_id, entry)
        .map(OrderedVideoAcquire::Frame)
}
```

- [ ] **Step 4: Run focused tests**

Run: `cargo test -p capture-transfer acquire_next_after --locked`

Expected: all ordered acquire tests pass.

- [ ] **Step 5: Run existing capture-transfer video tests**

Run: `cargo test -p capture-transfer video::tests --locked`

Expected: pass.

- [ ] **Step 6: Commit chunk**

```bash
git add crates/capture-transfer/src/video.rs
git commit -m "feat(capture): add ordered video acquisition"
```

## Chunk 2: Transfer Channel Messages

### Task 3: Add request and unavailable reply types

**Files:**
- Modify: `crates/capture-transfer/src/transfer_channel.rs`

- [ ] **Step 1: Write failing serialization tests**

Extend `request_messages_roundtrip_with_snake_case_ops`:

```rust
let next = CaptureTransferRequest::AcquireNextVideoFrame {
    session_id: "session-1".to_string(),
    track_id: 7,
    after_producer_cursor: 42,
};
let next_json = serde_json::to_value(&next).unwrap();
assert_eq!(next_json["op"], "acquire_next_video_frame");
assert_eq!(next_json["after_producer_cursor"], 42);
assert_eq!(serde_json::from_value::<CaptureTransferRequest>(next_json).unwrap(), next);
```

Extend `server_messages_roundtrip_with_producer_cursor`:

```rust
let unavailable = CaptureTransferMessage::VideoFrameUnavailable {
    session_id: "session-1".to_string(),
    track_id: 7,
    after_producer_cursor: 42,
    oldest_available_cursor: 48,
    latest_available_cursor: 57,
    skipped_count: 5,
    reason: "lapped".to_string(),
};
let unavailable_json = serde_json::to_value(&unavailable).unwrap();
assert_eq!(unavailable_json["op"], "video_frame_unavailable");
assert_eq!(unavailable_json["oldest_available_cursor"], 48);
assert_eq!(
    serde_json::from_value::<CaptureTransferMessage>(unavailable_json).unwrap(),
    unavailable
);
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p capture-transfer transfer_channel::tests --locked`

Expected: compile failure because variants are missing.

- [ ] **Step 3: Add enum variants**

Add to `CaptureTransferRequest`:

```rust
AcquireNextVideoFrame {
    session_id: String,
    track_id: u64,
    after_producer_cursor: u64,
},
```

Add to `CaptureTransferMessage`:

```rust
VideoFrameUnavailable {
    session_id: String,
    track_id: u64,
    after_producer_cursor: u64,
    oldest_available_cursor: u64,
    latest_available_cursor: u64,
    skipped_count: u64,
    reason: String,
},
```

- [ ] **Step 4: Run serialization tests**

Run: `cargo test -p capture-transfer transfer_channel::tests --locked`

Expected: pass.

- [ ] **Step 5: Commit chunk**

```bash
git add crates/capture-transfer/src/transfer_channel.rs
git commit -m "feat(capture): add ordered frame channel messages"
```

## Chunk 3: portholed fd Connection

### Task 4: Add registry ordered acquisition path

**Files:**
- Modify: `crates/portholed/src/capture_registry.rs`

- [ ] **Step 1: Write failing registry test**

Add a unit test near `fd_connection_acquires_requested_producer_cursor`:

```rust
#[test]
fn fd_connection_acquires_next_video_frame_in_order() {
    let (client, server) = UnixStream::pair().unwrap();
    let registry = CaptureRegistry::new_for_tests();
    let mut video = VideoSlotManager::new_reusable_pool(3);
    let track = TrackId::new(9);
    publish_test_frame_to_video(&mut video, track, 1, b"aaaa");
    publish_test_frame_to_video(&mut video, track, 2, b"bbbb");
    let session_id = "capture-ordered".to_string();
    registry.insert_ready_test_session(session_id.clone(), track, video);

    let registry_for_server = registry.clone();
    let server_thread = thread::spawn(move || handle_fd_connection(server, registry_for_server).unwrap());

    writeln!(
        client.try_clone().unwrap(),
        "{}",
        serde_json::json!({
            "op": "acquire_next_video_frame",
            "session_id": session_id,
            "track_id": 9,
            "after_producer_cursor": 1
        })
    )
    .unwrap();

    let mut reader = BufReader::new(client.try_clone().unwrap());
    let response = read_next_video_frame_message(&mut reader);
    assert_eq!(response["op"], "video_frame");
    assert_eq!(response["producer_cursor"], 2);

    let lease_id = response["lease_id"].as_u64().unwrap();
    writeln!(
        client.try_clone().unwrap(),
        "{}",
        serde_json::json!({ "op": "release_video_frame", "lease_id": lease_id })
    )
    .unwrap();
    drop(client);
    server_thread.join().unwrap();
}
```

Adjust helper names to match existing test helpers; keep this test synthetic so
it does not require macOS permissions.

- [ ] **Step 2: Write failing lapped response test**

Add a second fd connection test that publishes three frames into a capacity-two
ring, requests after cursor `1`, and asserts:

```rust
assert_eq!(response["op"], "video_frame_unavailable");
assert_eq!(response["reason"], "lapped");
assert_eq!(response["after_producer_cursor"], 1);
assert_eq!(response["oldest_available_cursor"], 2);
assert_eq!(response["latest_available_cursor"], 3);
assert_eq!(response["skipped_count"], 1);
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p portholed fd_connection_acquires_next_video_frame_in_order --locked`

Expected: request deserializes only after channel variants exist, then fails
because `handle_fd_connection` does not handle ordered acquisition.

- [ ] **Step 4: Add registry helper**

Import `OrderedVideoAcquire` and add a method similar to `frame_by_cursor_for_consumer`:

```rust
fn next_frame_for_consumer(
    &self,
    request: &LatestVideoFrameRequest,
    consumer_id: ConsumerId,
    include_control_page: bool,
    after_producer_cursor: u64,
) -> Result<OrderedFrameReply, CaptureRegistryError>
```

`OrderedFrameReply` should be:

```rust
enum OrderedFrameReply {
    Frame(LatestFrameReply),
    Unavailable {
        session_id: String,
        track_id: u64,
        after_producer_cursor: u64,
        oldest_available_cursor: u64,
        latest_available_cursor: u64,
        skipped_count: u64,
    },
    Empty,
}
```

For `Empty`, return a `video_frame_unavailable` message with reason `"empty"` if
the channel needs an immediate reply. Do not block in this slice.

- [ ] **Step 5: Wire fd connection request handling**

In `handle_fd_connection`, add:

```rust
CaptureTransferRequest::AcquireNextVideoFrame {
    session_id,
    track_id,
    after_producer_cursor,
} => {
    let include_control_page = !connection.registered_control_pages.contains(&track_id);
    let request = LatestVideoFrameRequest {
        session_id: session_id.clone(),
        track_id,
    };
    match registry.next_frame_for_consumer(&request, consumer_id, include_control_page, after_producer_cursor)? {
        OrderedFrameReply::Frame(reply) => {
            send_frame_reply(&mut stream, &request.session_id, include_control_page, reply, &mut connection)?;
        }
        OrderedFrameReply::Unavailable { .. } | OrderedFrameReply::Empty => {
            send_unavailable_reply(&mut stream, ...)?;
        }
    }
}
```

Use a small helper to serialize `CaptureTransferMessage::VideoFrameUnavailable`.

- [ ] **Step 6: Run focused portholed tests**

Run: `cargo test -p portholed fd_connection_acquires_next_video_frame_in_order --locked`

Run: `cargo test -p portholed fd_connection_reports_lapped_ordered_video_frame --locked`

Expected: pass.

- [ ] **Step 7: Run capture registry test group**

Run: `cargo test -p portholed capture_registry --locked`

Expected: pass.

- [ ] **Step 8: Commit chunk**

```bash
git add crates/portholed/src/capture_registry.rs
git commit -m "feat(capture): serve ordered video frames over fd channel"
```

## Chunk 4: Daemon Consumer Ordered API

### Task 5: Add daemon consumer parsing and method

**Files:**
- Modify: `crates/capture-transfer/src/daemon.rs`

- [ ] **Step 1: Add failing request-shape test**

Add a daemon consumer test that calls:

```rust
let result = consumer.next_frame_after(7, 42);
```

The fake server should assert the request:

```json
{
  "op": "acquire_next_video_frame",
  "session_id": "session-1",
  "track_id": 7,
  "after_producer_cursor": 42
}
```

Then return a normal `video_frame`; assert the returned frame has producer cursor
`43` and release still sends `release_video_frame`.

- [ ] **Step 2: Add failing unavailable parsing test**

Fake server returns:

```json
{
  "op": "video_frame_unavailable",
  "session_id": "session-1",
  "track_id": 7,
  "after_producer_cursor": 42,
  "oldest_available_cursor": 48,
  "latest_available_cursor": 57,
  "skipped_count": 5,
  "reason": "lapped"
}
```

Assert `next_frame_after` returns a structured `DaemonFrameUnavailable` value,
not a generic transport error.

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p capture-transfer daemon_consumer_requests_next_frame_after_cursor --locked`

Expected: compile failure because `next_frame_after` does not exist.

- [ ] **Step 4: Add public unavailable type**

In `daemon.rs`, add:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DaemonFrameUnavailable {
    pub track_id: u64,
    pub after_producer_cursor: u64,
    pub oldest_available_cursor: u64,
    pub latest_available_cursor: u64,
    pub skipped_count: u64,
    pub reason: String,
}

#[derive(Debug)]
pub enum DaemonFrameAcquire {
    Frame(DaemonFrame),
    Unavailable(DaemonFrameUnavailable),
}
```

- [ ] **Step 5: Implement `DaemonConsumer::next_frame_after`**

Factor the common receive loop currently inside `latest_frame` so both latest
and ordered acquisition can share registration handling and `video_frame`
mapping. The new method should write `CaptureTransferRequest::AcquireNextVideoFrame`
and return `DaemonFrameAcquire`.

- [ ] **Step 6: Run focused daemon tests**

Run: `cargo test -p capture-transfer daemon_consumer_requests_next_frame_after_cursor --locked`

Run: `cargo test -p capture-transfer daemon_consumer_parses_video_frame_unavailable --locked`

Expected: pass.

- [ ] **Step 7: Run daemon module tests**

Run: `cargo test -p capture-transfer daemon::tests --locked`

Expected: pass.

- [ ] **Step 8: Commit chunk**

```bash
git add crates/capture-transfer/src/daemon.rs
git commit -m "feat(capture): add ordered daemon consumer acquire"
```

## Chunk 5: Verification and Documentation

### Task 6: Verify workspace gates

**Files:**
- Modify if needed: `docs/superpowers/specs/2026-05-17-macos-recording-design.md`
- Modify if needed: `docs/superpowers/plans/2026-05-17-macos-recording-ordered-cursor.md`

- [ ] **Step 1: Run formatting**

Run: `cargo +nightly-2026-03-12 fmt`

Expected: formats Rust files only.

- [ ] **Step 2: Run full CI gates**

Run:

```bash
cargo build --workspace --locked
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo +nightly-2026-03-12 fmt --check
git diff --check
```

Expected: all pass. Permission-dependent ignored macOS integration tests remain
ignored.

- [ ] **Step 3: Manual smoke if permissions are already granted**

Run: `porthole info`

If Screen Recording is not granted, stop with `BLOCKED` and state the missing
permission. Do not add a mock or bypass.

If granted, run an existing synthetic or live capture smoke and manually request
ordered frames through the daemon consumer path. Record exact command output in
the PR summary.

- [ ] **Step 4: Commit any verification/doc updates**

```bash
git status --short
git add <changed-files>
git commit -m "docs(capture): record ordered acquisition verification"
```

Only commit if files changed.
