# Capture Publication Layout Design

Date: 2026-05-17
Status: implemented prototype

## Context

The internal `VideoTrackControlPage` now gives the capture path a concrete
control-page shaped model, but it is still only a simple ring header plus
entries. Before exposing any mmap, the layout should move closer to the
jackstay-style single-producer broadcast ring:

- monotonic producer cursor
- power-of-two ring capacity and mask indexing
- per-slot publication sequence
- explicit latest-wins lap/resync helpers
- layout identity fields for future mmap validation

This slice is still in-process Rust state. It does not add atomics, mmap
registration, futexes, wakeups, GPU sync, or a binary ABI. The goal is to make
the next mmap/atomic slice less likely to bake in the wrong shape.

## Goals

- Add control-page layout identity fields: magic, version, header length, entry
  length, capacity, and index mask.
- Round control-page ring capacity up to a power of two and use mask indexing
  instead of modulo for slot selection.
- Add a per-entry `publication_sequence` that is written as the final publication
  marker for the slot.
- Add latest-lossy helper semantics: newest cursor, oldest still-live cursor,
  and lapped detection.
- Keep `VideoSlotManager`, daemon transfer messages, leases, and CPU pool
  behavior unchanged.
- Document that future atomics are acquire/release publication barriers, not
  compare-and-swap on the video hot path.

## Non-Goals

- No shared mmap exposure.
- No atomics yet.
- No seqlock double-read reader API yet.
- No futex/eventfd/kqueue/Mach wake primitive.
- No consumer registration slots or shared consumer cursor page.
- No IOSurface, dmabuf, D3D, or native sync fields.

## Model

`VideoTrackControlHeader` grows layout identity and mask fields:

```rust
magic
version
header_len
entry_len
capacity
index_mask
producer_cursor
latest_sequence
latest_index
len
```

`capacity` is the control ring capacity and is always a power of two. It may be
larger than the current CPU payload retention count when callers ask for a
non-power-of-two size. That is acceptable for latest-frame consumers because
they only acquire the newest entry. Ordered consumers must use lapped detection
before trusting older entries.

`VideoRingEntry` grows:

```rust
publication_sequence
```

For now `publication_sequence == producer_cursor`. In the future shared-memory
version, this field becomes the per-slot release-store publication primitive
that consumers acquire-load before and after reading the slot.

## Latest-Lossy Semantics

The internal helpers should express the intended policy even before cross-process
atomics exist:

- `latest_cursor()` returns the newest published cursor.
- `oldest_live_cursor()` returns the oldest cursor still represented in the
  control ring, if any.
- `cursor_lapped(expected)` returns true when `expected` is older than the
  oldest live cursor.
- latest-lossy consumers resync to `latest_cursor()`.

The current `VideoSlotManager::acquire_latest` already behaves as latest-lossy:
it resolves only the newest entry and treats skipped frames as normal.

## Future Atomic Mapping

When this page becomes shared memory:

- producer writes slot fields
- producer release-stores `publication_sequence`
- producer release-stores `producer_cursor`/head
- consumer acquire-loads `publication_sequence`
- consumer reads the slot
- consumer acquire-loads `publication_sequence` again and compares

No CAS belongs on the video publication hot path because the ring is
single-producer. CAS may appear later in setup/registration machinery, not in
per-frame publish/consume.

## Testing

- Header test: capacity is rounded to power of two, mask is capacity minus one,
  and layout identity fields are populated.
- Publish test: pushing through wraparound writes entries using mask indexing
  and sets each entry's `publication_sequence`.
- Lap test: oldest/latest cursor helpers and lapped detection distinguish
  current, stale, and empty states.
- Existing video ring and daemon transfer tests remain green.
