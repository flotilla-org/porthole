# Capture Control Page FD Shadow Design

Date: 2026-05-17
Status: draft

## Context

The video control page now has a mapped storage layout, identity fields,
per-slot publication sequence, and acquire/release accessors for hot fields. It
is still private to the producer process. The next useful proof is to pass that
mapped page to the daemon consumer, map it read-only, and verify that the ring
metadata agrees with the existing socket-delivered frame metadata.

This slice keeps the current request/lease path authoritative. The shared
control page is a shadow channel only: if it validates and agrees with the
socket metadata, the consumer records that fact; if it disagrees, the consumer
returns an error so experiments catch protocol drift early.

## Goals

- Clone and pass a video track control-page fd on the capture transfer channel.
- Register each control page at most once per consumer connection and track.
- Map the page read-only on the daemon consumer side.
- Validate layout identity before any shadow read:
  - magic
  - version
  - header length
  - entry length
  - capacity is non-zero and power-of-two
  - index mask matches capacity minus one
  - declared mapped length is large enough for the header and entries
- Shadow-read the latest ring entry with the seqlock-style acquire/read/acquire
  path and compare it with `video_frame` metadata.
- Keep socket metadata, lease ids, release messages, and payload mappings
  authoritative.

## Non-Goals

- Do not make the shared ring authoritative for acquisition.
- Do not add consumer cursors, release cursors, or wake primitives.
- Do not add fd passing to the C ABI.
- Do not add IOSurface, Metal, dmabuf, D3D, or native synchronization fields.
- Do not add backwards-compatibility shims for older transfer-channel messages.

## Wire Shape

Add one capture transfer message:

```text
register_video_control_page {
  session_id,
  track_id,
  map_len
} + fd
```

The daemon sends this before the first `video_frame` for a track on a consumer
connection. Later `latest_video_frame` requests on the same connection reuse the
mapped page and do not resend it.

The message is setup/control-plane traffic. It is ordered ahead of the frame
metadata on the same Unix-domain socket, and the fd travels as ancillary data
with the message.

## Consumer Shadow Read

After receiving `video_frame`, the daemon consumer:

1. Resolves the registered control page for `track_id`.
2. Reads the latest cursor from the mapped page.
3. Reads the ring entry for the socket frame's `producer_cursor` using:
   - acquire-load slot `publication_sequence`
   - copy descriptor fields
   - acquire-load slot `publication_sequence`
   - require both reads to match the requested cursor
4. Compares ring fields with socket metadata:
   - `producer_cursor`
   - `sequence`
   - `pool_id`
   - `slot_id`
   - `slot_generation`
   - `payload_offset`
   - `payload_len`

The comparison intentionally excludes lease id, timestamps, dimensions, pixel
format, damage fields, and skip counters because those are not currently part of
the ring descriptor.

## Error Handling

Control page validation or shadow mismatch is a daemon transport error on the
consumer side. The producer/daemon keeps existing behavior; it still pins frames
and expects explicit release messages. A shadow-read failure means the prototype
has found an inconsistent control page, not that the consumer should silently
fall back.

## Testing

- Transfer-channel serde covers the new registration message.
- `capture-transfer` tests cover mapping/validation from a passed fd and
  shadow comparison against socket frame metadata.
- `portholed` fd-socket tests cover control-page registration before frame
  metadata, one registration per connection/track, and matching ring metadata
  for the synthetic capture path.
- Existing reusable CPU pool and lease tests remain green.
