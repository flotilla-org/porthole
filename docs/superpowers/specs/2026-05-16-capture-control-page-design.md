# Capture Control Page Design

Date: 2026-05-16
Status: implemented prototype

## Context

The capture prototype now has reusable CPU payload pools, explicit leases, a
typed capture transfer channel, and an in-process ring cursor model. The next
step is to make the ring/control state look like the future shared control page
without exposing it cross-process yet.

This slice keeps the current daemon and consumer behavior unchanged. The socket
is still the setup, handle-transfer, and request/release path. The control page
is an internal fixed-layout model that `VideoSlotManager` writes through, so the
eventual mmap/atomic version starts from concrete code rather than a diagram.

## Goals

- Add an internal `capture-transfer::control_page` module with fixed-size video
  track control data and ring entries.
- Keep hot metadata struct-shaped: ids, cursors, counters, offsets, lengths,
  and fixed numeric fields only.
- Route `TrackRingControl` through that control-page model for producer cursor,
  latest sequence, ring entries, and capacity.
- Preserve current latest-frame semantics, lease behavior, registered CPU pool
  behavior, and typed capture transfer channel wire shape.
- Document the fields that are deliberately ordinary Rust values for now but
  are likely to become atomics when the page is shared.

## Non-Goals

- No cross-process shared control mmap.
- No atomics, futexes, kqueue, eventfd, Mach semaphore, or wake primitive work.
- No binary protocol.
- No ordered recording cursor.
- No GPU, IOSurface, dmabuf, D3D, or native synchronization changes.
- No C ABI expansion.

## Model

`control_page` defines the internal shape:

```rust
pub struct VideoTrackControlPage {
    header: VideoTrackControlHeader,
    entries: Vec<VideoRingEntry>,
}

pub struct VideoTrackControlHeader {
    pub capacity: u64,
    pub producer_cursor: u64,
    pub latest_sequence: u64,
    pub latest_index: u64,
    pub len: u64,
}

pub struct VideoRingEntry {
    pub producer_cursor: u64,
    pub sequence: u64,
    pub frame_key: u64,
    pub pool_id: u64,
    pub slot_id: u64,
    pub slot_generation: u64,
    pub payload_offset: u64,
    pub payload_len: u64,
}
```

The header uses `latest_sequence = 0` and `latest_index = u64::MAX` when empty
because the future shared-memory representation should avoid optional pointer
or heap-shaped fields. The public debug snapshot can still expose
`Option<u64>` for ergonomics.

`VideoTrackControlPage::push` increments the producer cursor, writes one ring
entry at `(producer_cursor - 1) % capacity`, and updates the header. Snapshot helpers
return entries in oldest-to-newest order, matching the current
`MetadataRing::snapshot` behavior.

Follow-up work hardens this internal page toward the future publication layout:
the page now carries layout identity fields, rounds its control ring capacity to
a power of two, uses a mask for ring indexing, stores a per-slot publication
sequence, and exposes latest-lossy lap/resync helpers. It remains in-process
Rust state rather than shared mmap.

Consumer cursors remain per-consumer Rust state for this slice. They already
look close to the future shared fields: last acquired cursor, release cursor,
skipped count, and acquired count. Moving those into a shared page should be a
later slice because it raises ownership questions for dynamic consumer slots and
process-exit cleanup.

## Data Flow

```text
producer commit
  -> write payload range
  -> VideoTrackControlPage::push(pending entry)
  -> producer_cursor advances in the page header
  -> latest entry names frame_key + pool/slot/range

consumer latest acquire
  -> read latest VideoRingEntry from the page model
  -> resolve frame_key to StoredFrame
  -> pin StoredFrame for consumer lease
  -> update ConsumerRingCursor

consumer release
  -> unpin StoredFrame
  -> update ConsumerRingCursor release_cursor
```

## Error Handling

The control page model should not add new public errors. Capacity is clamped to
at least one entry, as the current ring does. Numeric conversions from `usize`
capacity to fixed-width fields should be checked or use values already bounded
by allocation success.

## Testing

- `control_page` unit tests assert empty header values, push behavior, wraparound
  ordering, latest-entry resolution, and capacity clamping.
- Existing `VideoSlotManager` ring tests should continue to pass with snapshots
  sourced from `VideoTrackControlPage`.
- A video test should assert that the control-page debug snapshot is the source
  of producer cursor/latest sequence after wraparound.
- Full repo gates remain the AGENTS.md gates.

## Follow-On Slices

- Register a read-only control mmap over the capture transfer channel.
- Define atomic memory ordering for producer cursor, ring entries, and consumer
  release/watermark fields.
- Add a wake primitive per platform.
- Add native synchronization fields for IOSurface/Metal and Linux dmabuf.
- Add ordered recording cursors and dynamic consumer slots when a real recorder
  exists.
