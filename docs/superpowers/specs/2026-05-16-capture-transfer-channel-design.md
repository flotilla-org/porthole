# Capture Transfer Channel Design

Date: 2026-05-16
Status: implemented prototype

## Context

The capture path has two daemon-facing sockets:

- the normal HTTP-over-UDS daemon API for session setup and general porthole
  commands
- the raw capture transfer channel used for frame metadata and `SCM_RIGHTS`
  handle passing

Older docs and comments call the second socket a "side channel". That name is
too vague now that this socket is becoming the main capture transport control
path. The mechanism stays the same in this slice: newline-delimited JSON plus
ancillary fd passing over a Unix-domain socket. The implementation should become
less ad hoc and start carrying the same cursor vocabulary that the in-process
ring model already owns.

## Goals

- Rename docs/comments toward "capture transfer channel" where this socket is
  meant.
- Add typed Rust message structs/enums for capture transfer channel requests and
  server messages.
- Replace scattered `serde_json::json!` construction and untyped `Value`
  parsing in the capture channel hot path.
- Carry `producer_cursor` in `video_frame` metadata.
- Keep the existing socket, JSON-line framing, fd passing, pool registration,
  lease, and release behavior unchanged.

## Non-Goals

- No binary protocol.
- No shared control mmap.
- No atomics or wake primitive work.
- No ordered recording cursor.
- No removal of `lease_id` as the pin/release authority.
- No public C ABI expansion for producer cursor in this slice.

## Protocol Shape

Client-to-daemon requests are typed as:

```json
{ "op": "latest_video_frame", "session_id": "...", "track_id": 1 }
{ "op": "release_video_frame", "lease_id": 42 }
```

Daemon-to-client messages are typed as:

```json
{
  "op": "register_cpu_pool",
  "session_id": "...",
  "track_id": 1,
  "pool_id": 7,
  "pool_generation": 3,
  "payload_map_len": 196608,
  "slot_stride": 65536,
  "slot_count": 3
}
```

```json
{
  "op": "video_frame",
  "session_id": "...",
  "track_id": 1,
  "lease_id": 42,
  "producer_cursor": 99,
  "sequence": 99,
  "pool_id": 7,
  "slot_id": 2,
  "slot_generation": 3,
  "payload_offset": 131072,
  "payload_len": 4096,
  "payload_map_len": 196608
}
```

The `producer_cursor` is the monotonic ring cursor assigned when the frame is
published. It is not used for release in this slice; `lease_id` remains the
release authority. Exposing the cursor on the channel lets daemon-backed clients
observe the same control-plane position as in-process consumers and prepares the
wire shape for a future shared control page.

## Implementation

Add a small `capture-transfer` module for channel messages. Both the daemon
client and `portholed` should use these types for serde encoding/decoding.
`porthole-protocol::LatestVideoFrameResponse` grows `producer_cursor` because it
is the current registry-to-channel carrier for frame metadata inside `portholed`.

`AcquiredVideoFrame` exposes a `producer_cursor()` accessor. The cursor remains
internal to capture-transfer frame lifetime logic, not part of the public C ABI.

## Testing

- Unit tests for channel message JSON round trips, including `producer_cursor`.
- Daemon client fake-server tests assert parsed `DaemonFrame::producer_cursor`.
- `portholed` capture transfer channel tests assert emitted `video_frame`
  messages include `producer_cursor`.
- Existing pool registration, immutable fallback, lease release, and disconnect
  cleanup tests remain green.
