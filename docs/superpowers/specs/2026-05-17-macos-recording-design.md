# macOS Recording Design

Date: 2026-05-17
Status: approved

## Context

The capture-transfer path now has live macOS ScreenCaptureKit capture, reusable
CPU payload pools, fd transfer, explicit frame leases, fixed video control-page
descriptors, and mirrored per-consumer cursor state. That is enough for an
interactive viewer, where the desired behavior is "show me the newest frame I
can display now."

Recording is a different contract. A recorder must consume frames in producer
order, know when it was lapped by the ring, and report any dropped or skipped
frames honestly. Building `porthole record` as a loop over `latest_video_frame`
would create plausible clips while hiding the exact failure mode recording is
supposed to surface.

This slice defines the CPU-side/macOS recording path as two layers:

1. ordered recorder cursor semantics in the capture-transfer channel
2. a macOS recording command/API that uses those semantics to write media

The first implementation should build the ordered cursor layer before promising
a polished recording command.

## Goals

- Add a recorder-oriented frame acquisition mode that advances from a caller's
  previous producer cursor instead of jumping to the newest frame.
- Preserve the existing latest-frame viewer behavior unchanged.
- Make ring lapping and unavailable frames explicit in the protocol response so
  a recorder can fail, warn, or continue with a known gap.
- Keep frame leases and socket release authoritative for pinning and cleanup.
- Reuse the existing CPU payload pool and video control page; do not introduce
  another transport for recording.
- Leave the eventual media writer free to choose an encoder/container without
  changing capture-transfer ownership semantics.

## Non-Goals

- No audio recording in this slice.
- No IOSurface, GPU zero-copy, or native encoder handle transfer.
- No remote recording transport.
- No promise of archival every-frame capture when the producer outruns the
  consumer and the ring laps the requested cursor.
- No compatibility shim for older channel messages; porthole is pre-release.

## Approach Options

### Option A: Record Latest Frames

The quickest path is a CLI loop that calls `latest_video_frame`, writes the
returned pixels, releases the lease, and repeats.

This is not acceptable as the product recording path. It silently converts
backpressure into frame loss and cannot distinguish "no new frame yet" from
"the recorder skipped a range." It remains useful only as a manual diagnostic.

### Option B: Ordered Cursor First

Add a protocol request that asks for the next available frame after a producer
cursor. The server returns the next ordered frame if it is still in the ring, or
an explicit lapped/unavailable result when the requested range has fallen out of
the retained window. The recorder then drives this request loop and releases
each lease after copying or encoding the frame.

This is the recommended path. It keeps the recording contract honest and builds
on the control-page cursor work already in place.

### Option C: Larger Ring as a Recording Fix

Increase the reusable pool/ring capacity and keep using existing acquisition.

This helps burst tolerance but does not define recorder semantics. A larger ring
can be part of tuning later, but it is not a substitute for ordered acquisition
and explicit gap accounting.

## Protocol Design

Add an ordered acquisition request to the raw capture transfer channel:

```json
{
  "op": "acquire_next_video_frame",
  "session_id": "capture-1",
  "track_id": 1,
  "after_producer_cursor": 42
}
```

`after_producer_cursor = 0` means "start at the earliest currently retained
frame." For nonzero cursors, the server returns the first retained frame whose
producer cursor is greater than the supplied cursor.

Successful replies can continue to use the existing `video_frame` message shape.
The reply already includes `producer_cursor`, sequence, timestamps, payload
location, producer drop counters, eviction count, and consumer skipped count.
The ordered path should tighten `consumer_skipped_count` so it reflects gaps
observed by that consumer, including a lapped range.

When the requested next cursor is no longer retained, the channel returns an
explicit status message rather than jumping to latest:

```json
{
  "op": "video_frame_unavailable",
  "session_id": "capture-1",
  "track_id": 1,
  "after_producer_cursor": 42,
  "oldest_available_cursor": 48,
  "latest_available_cursor": 57,
  "skipped_count": 5,
  "reason": "lapped"
}
```

The first implementation can model this as a channel message and Rust enum
without adding a public C ABI call until the Rust daemon path is proven. If the
C ABI is expanded in the same slice, it should be explicit:

```c
ft_status ft_consumer_acquire_next_video_frame(ft_consumer *consumer,
                                               ft_track_id track_id,
                                               uint64_t after_producer_cursor,
                                               ft_video_frame *out_frame,
                                               ft_video_frame_gap *out_gap);
```

`FT_STATUS_EMPTY` remains reserved for "no frame available right now" in
nonblocking contexts. A lapped ordered recorder is not empty; it is a data-loss
condition and must carry gap details.

## VideoSlotManager Design

`VideoSlotManager` already stores ring entries by producer cursor and tracks
per-consumer acquire/release cursors. Add an ordered helper:

```rust
pub enum OrderedAcquire {
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

The helper finds the first retained ring entry with `producer_cursor >
after_producer_cursor`. `VideoSlotManager` does not separately register empty
tracks today, so a track with no published frames still uses the existing
unknown-track error. `Empty` means the track has retained frames, but no frame
newer than `after_producer_cursor` is currently available.

- If `after_producer_cursor == 0`, return the oldest retained frame.
- If the next expected cursor has been evicted, return `Lapped` with the retained
  cursor bounds and skipped count.
- If the next expected cursor is newer than the retained latest cursor, return
  `Empty`.
- If the requested next frame is retained, acquire it through the existing
  `acquire_ring_entry` path so pinning, consumer cursor mirroring, control-page
  acquire stores, and lease release remain unchanged.

The existing `acquire_latest` and `acquire_cursor` behavior must not change.

## Recording API Shape

The user-facing recording slice should sit above ordered cursor acquisition:

```text
porthole record surface <surface-id> --duration 5s --output out.mov
```

The command starts or attaches to a capture session, opens a capture-transfer
consumer connection, records ordered frames until the requested duration or stop
condition, releases every lease, then closes the session it created.

The first useful media target should be a real macOS-friendly file, most likely
`.mov` via AVAssetWriter. If encoder integration becomes too large for the first
recording PR, the acceptable intermediate is a deliberately named raw/debug
format, not a user-facing "recording" command that implies normal playback.

## Error Handling

- Missing Screen Recording permission remains a hard `BLOCKED` condition for
  live macOS verification; do not bypass it with mocks or alternate capture.
- Unknown sessions, closed sessions, and failed producer sessions should reuse
  existing capture registry errors.
- Ordered lapping must be reported as structured data, not collapsed into an I/O
  error string.
- Disconnect cleanup must release all outstanding leases and unregister the
  consumer cursor slot as it does today.
- A recorder may choose policy on lapping, but the default CLI should fail the
  recording unless the user opts into best-effort output.

## Testing

- `capture-transfer` unit tests:
  - ordered acquire from cursor `0` returns the oldest retained frame
  - ordered acquire after a retained cursor returns the next producer cursor
  - ordered acquire reports `Empty` when no newer retained frame exists
  - ordered acquire reports `Lapped` instead of jumping to latest
  - latest-frame acquisition still skips to the newest frame
  - release after ordered acquire updates consumer release cursor

- `portholed` tests:
  - fd connection serves `acquire_next_video_frame` in order over one connection
  - lapped ordered requests return `video_frame_unavailable`
  - disconnect releases outstanding ordered leases
  - synthetic sessions exercise the ordered path without macOS permissions

- CLI/API tests for the later recording command:
  - command formatting and argument validation
  - created sessions are closed on success and failure
  - lapping fails by default and can be reported in JSON

- Manual macOS smoke, only when Screen Recording is granted:
  - start a real surface capture session
  - acquire ordered frames for a short interval
  - verify monotonic producer cursors and lease release
  - run the eventual recorder command against a visible window

## Implementation Order

1. Add ordered acquisition to `VideoSlotManager` with tests.
2. Add the raw channel request/reply types with serialization tests.
3. Wire `portholed` fd connection handling through the ordered path.
4. Add daemon consumer support for ordered acquisition.
5. Add a focused manual smoke script for ordered macOS capture.
6. Build the `porthole record` API/CLI and media writer as the next spec/plan.

This order keeps the data contract correct before exposing recording as a
product feature.
