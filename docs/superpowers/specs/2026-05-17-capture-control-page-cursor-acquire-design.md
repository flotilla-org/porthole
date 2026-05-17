# Capture Control Page Cursor Acquire Design

Status: implemented prototype

## Context

The daemon consumer can now map the video track control page read-only and
verify that its ring entry agrees with the socket `video_frame` metadata. That
proves the mapped page shape, but the socket still chooses the frame by asking
for "latest" and receiving whatever the daemon selected.

The next useful step is to let the mapped control page choose the producer
cursor while keeping the socket as the pin, lease, metadata, and release
authority. This moves toward the shared-ring destination without adding shared
consumer cursors, wake primitives, or binary queue ownership yet.

## Goals

- Add a capture transfer request that asks the daemon to acquire a specific
  `producer_cursor`.
- Let daemon-backed consumers read the latest cursor from the mapped control
  page and request that exact cursor on subsequent frames.
- Keep `latest_video_frame` as the bootstrap and fallback request, especially
  before the control page is registered.
- Keep socket `video_frame` metadata and `lease_id` authoritative for frame
  lifetime and release.
- Continue comparing the returned socket metadata against the mapped control
  page entry.

## Non-goals

- Do not move lease ids, release cursors, or consumer positions into shared
  memory.
- Do not add futexes, eventfd, kqueue, or blocking waits.
- Do not add a binary capture transfer protocol.
- Do not make consumers retain shm slots without an explicit socket lease.
- Do not add IOSurface, Metal, dmabuf, D3D, or native sync in this slice.

## Wire Shape

Add one request:

```json
{
  "op": "acquire_video_frame_by_cursor",
  "session_id": "session-1",
  "track_id": 7,
  "producer_cursor": 42
}
```

The response remains the existing sequence of optional registration messages
followed by `video_frame`. The response metadata must describe the requested
cursor. If the cursor is not live, the server side treats this as a capture
ring read error and closes the connection as existing transport errors do.

## Data Flow

1. Consumer connects with no control page.
2. First `latest_frame(track_id)` sends `latest_video_frame`.
3. Daemon returns `register_video_control_page` plus fd, optional pool
   registration, then `video_frame`.
4. Consumer maps and validates the control page, then shadow-checks the returned
   frame as today.
5. On later `latest_frame(track_id)` calls, if a control page exists and has a
   latest cursor, the consumer sends `acquire_video_frame_by_cursor` for that
   cursor.
6. Daemon pins exactly that cursor, mints a lease id, returns `video_frame`, and
   the consumer still releases by lease id.

## Error Handling

- Empty control page on the consumer side falls back to `latest_video_frame`.
- Lapped, future, or slot-mismatched cursor acquisition on the daemon side is a
  capture transport failure for this prototype.
- A successful exact-cursor response is still shadow-compared against the mapped
  page entry. Mismatch remains a daemon transport error.

## Testing

- Transfer channel JSON round trip for `acquire_video_frame_by_cursor`.
- `VideoSlotManager` exact-cursor acquisition returns the requested older live
  frame and records consumer cursor state.
- `VideoSlotManager` exact-cursor acquisition reports lapped cursors after
  wraparound.
- Daemon consumer fake-server test verifies that after registering a control
  page, the next `latest_frame` request uses `acquire_video_frame_by_cursor`.
- Portholed fd-socket test verifies the server can serve an exact cursor and
  still release by lease id.
