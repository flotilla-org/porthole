# Capture Control Page Full Descriptor Design

Status: approved

## Context

The daemon consumer can map a read-only video control page, read the latest
producer cursor, and request that exact cursor through the fd socket. The
socket still carries the full `video_frame` metadata and remains authoritative
for frame lifetime, lease ids, fd passing, and release.

The mapped ring entry is still only a narrow slot locator: sequence, frame key,
pool id, slot id, slot generation, payload offset, and payload length. That is
enough to choose a frame, but not enough to make the control page the future
hot metadata path. The next useful step is to put the full fixed-size frame
descriptor in the shared ring while leaving ownership and release on the socket.

## Goals

- Expand `VideoRingEntry` and `PendingVideoRingEntry` to carry the fixed frame
  descriptor fields already present in `VideoFrameDesc`.
- Publish those descriptor fields into the mapped control page when a frame is
  stored.
- Keep the control-page shadow comparison strict enough to catch mismatches in
  descriptor metadata, not only cursor and payload locator fields.
- Keep `entry_len` validation as the layout guard for stale or corrupt mappings.
- Use stable numeric values for enum-like fields in shared memory.

## Descriptor Fields

The shared ring entry should include:

- `timestamp_ns`
- `width`, `height`, `stride`
- `pixel_format` as a stable numeric value
- `payload_map_len`
- `clock_domain`, `color_space`, `sync_kind`, `damage_kind` as stable numeric
  values
- `damage_base_sequence`
- `dropped_before_publish`
- `producer_drop_count`

The entry already includes:

- `publication_sequence`
- `producer_cursor`
- `sequence`
- `frame_key`
- `pool_id`
- `slot_id`
- `slot_generation`
- `payload_offset`
- `payload_len`

## Non-goals

- Do not add `lease_id` to shared memory. Socket replies remain authoritative
  for leases and release.
- Do not add variable-length damage rects or sidecar metadata.
- Do not move consumer release cursors into shared memory.
- Do not add wake primitives, futexes, kqueue, eventfd, or blocking waits.
- Do not change the JSON fd-socket protocol in this slice.
- Do not add native IOSurface, Metal, dmabuf, D3D, or native timeline payloads
  beyond preserving the existing fixed `sync_kind` metadata value.
- Do not publish `consumer_skipped_count`; it is per-consumer acquisition state,
  not a producer-published frame descriptor.
- Do not publish `evicted_count`; it is manager/socket accounting, not an
  immutable descriptor for the published frame.

## Data Flow

1. Producer publishes or commits a `VideoFrameDesc`.
2. `VideoSlotManager` builds a `PendingVideoRingEntry` from the full fixed
   descriptor.
3. `VideoTrackControlPage::push` copies that descriptor into the ring slot,
   bracketed by the existing publication sequence stores.
4. A mapped consumer reads the latest cursor and requests that cursor through
   the fd socket, as in the previous slice.
5. The daemon returns the existing `video_frame` socket metadata and lease id.
6. The consumer shadow-reads the mapped entry for the returned cursor and
   verifies all fixed descriptor fields match the socket metadata.

## ABI Shape

The control page remains a local, pre-release shared-memory ABI. Compatibility
shims are not needed, but stale mappings must fail closed. The existing
`version`, `header_len`, and `entry_len` validation are the right guard for this
slice: growing `VideoRingEntry` changes `entry_len`, and read-only mappings with
an old entry size must be rejected during header validation.

Enum values in the ring must be numeric rather than strings. Existing
`#[repr(u32)]` metadata enums can be copied as `u32`. `PixelFormat` should get
the same explicit representation and default value discipline before it enters
the shared page.

## Error Handling

- Invalid control-page header or entry length remains a shared-memory validation
  error.
- Shadow comparison mismatch remains a daemon transport error with operation
  `control-page-shadow`.
- Exact cursor reads continue to report empty, lapped, future, and slot sequence
  mismatch states through `VideoRingReadError`.

## Testing

- Control-page unit test pushes an entry with every fixed descriptor field set
  and verifies a read-only mapped shadow read preserves them.
- Model unit test verifies stable numeric values for `PixelFormat` alongside
  the existing metadata enum defaults.
- Video slot manager unit test verifies publishing a `VideoFrameDesc` writes
  the full fixed descriptor into the debug control-page snapshot.
- Daemon consumer fake-server test verifies shadow comparison rejects a
  mismatch in a newly covered descriptor field such as `timestamp_ns` or
  `payload_map_len`.
- Existing fd-socket and control-page tests are updated to construct the
  expanded `PendingVideoRingEntry`.
