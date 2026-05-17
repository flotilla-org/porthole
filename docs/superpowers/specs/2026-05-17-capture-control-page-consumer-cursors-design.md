# Capture Control Page Consumer Cursors Design

Status: approved

## Context

The video control page now carries the latest producer cursor and the full fixed
frame descriptor. Daemon-backed consumers map that page, choose the latest cursor
from shared memory, and acquire that exact cursor through the fd socket. The
socket still mints leases and remains authoritative for release.

The remaining ownership gap is consumer position. `VideoSlotManager` already
tracks per-consumer acquired and release cursors in process, but that state is
not visible in the mapped control page. The next useful step is to expose a
fixed consumer cursor table and let the consumer write its release cursor there,
while keeping the socket lease release path as the correctness fallback.

## Goals

- Add a fixed-size consumer cursor region to the video track control page.
- Allocate one consumer cursor slot per `(consumer, track)` on the daemon side.
- Include the assigned consumer cursor slot in the control-page registration
  message.
- Map the control page writable in the daemon-backed consumer so it can publish
  its release cursor before sending `release_video_frame`.
- Mirror server-side acquire and socket release state into the consumer cursor
  slot for debugging and future producer-side decisions.
- Keep socket leases authoritative for pinning and cleanup in this slice.

## Consumer Cursor Entry

Each consumer cursor entry is a fixed `repr(C)` integer record:

- `consumer_id`
- `slot_generation`
- `last_acquired_cursor`
- `last_acquired_sequence`
- `release_cursor`
- `skipped_count`
- `acquired_count`

`consumer_id == 0` means the slot is free. `slot_generation` gives a future
reader a way to distinguish a reused slot from an old mapping. This slice does
not trust `slot_generation` for correctness yet; it is included now so the ABI
does not have to grow immediately when release cursors become authoritative.

## Data Flow

1. A daemon fd-socket connection gets a non-zero `ConsumerId`.
2. On first frame for a track, `VideoSlotManager` allocates a consumer cursor
   slot in that track's control page for the connection's consumer id.
3. The daemon sends `register_video_control_page` with `map_len`,
   `consumer_id`, and `consumer_slot`, then passes the same control-page fd.
4. The consumer maps the page writable, validates the header, and records the
   slot assignment locally.
5. On acquire, the server updates `last_acquired_cursor`,
   `last_acquired_sequence`, `skipped_count`, and `acquired_count` in the slot.
6. On release, the consumer first stores `release_cursor` to the mapped slot,
   then sends the existing `release_video_frame { lease_id }` request.
7. The server handles the socket release as today and mirrors the release cursor
   into the slot. Pin cleanup still comes from the lease map and disconnect
   cleanup.

## ABI Shape

This slice changes the control-page header and mapped byte layout, so
`VIDEO_TRACK_CONTROL_VERSION` should advance. Header validation must check:

- `consumer_entries_offset`
- `consumer_entry_len`
- `consumer_capacity`
- total map length

The ring entry layout remains the fixed descriptor shape from the previous
slice. The consumer region starts after the ring entries and is aligned to
`CONTROL_PAGE_ALIGNMENT`.

## Non-goals

- Do not remove socket lease ids.
- Do not let producer slot reuse trust the writable release cursor yet.
- Do not add blocking waits or wake primitives.
- Do not add dynamic consumer table resizing.
- Do not add shared acquire cursors or per-consumer polling APIs beyond the
  fixed cursor entry.
- Do not add variable-length damage metadata or native GPU payloads.

## Error Handling

- If the consumer cursor table is full, acquisition fails with a capture transfer
  shared-memory error.
- Invalid header fields or insufficient map length remain shared-memory
  validation errors.
- A consumer with no mapped control page still releases through the socket path.
- Disconnect cleanup continues to release outstanding leases and unregister the
  consumer cursor slot.

## Testing

- Control-page unit tests cover layout/header fields, cursor slot allocation,
  writable mapping, release cursor stores, and unregister cleanup.
- Transfer-channel tests cover the expanded `register_video_control_page`
  message.
- Video slot manager tests cover server-side acquire/release mirroring into the
  consumer cursor slot and disconnect clearing the slot.
- Daemon fake-server tests cover a client writing the release cursor before
  sending `release_video_frame`.
- Portholed fd-socket tests cover registration messages carrying the assigned
  consumer slot and disconnect cleanup retaining socket lease behavior.
