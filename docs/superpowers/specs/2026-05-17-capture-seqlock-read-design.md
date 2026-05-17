# Capture Seqlock Read Design

Date: 2026-05-17
Status: implemented prototype

## Context

The control page now has a publication layout: power-of-two capacity, mask
indexing, layout identity, and per-slot `publication_sequence`. The jackstay
checklist calls out the next correctness rule before mmap exposure: consumers
must validate a slot by reading the per-slot sequence before and after reading
the descriptor.

This slice models that reader contract inside ordinary Rust state. It does not
add atomics or shared mmap. The double-read is not yet a real concurrent memory
barrier, but the API shape and error cases become concrete before the storage is
shared.

## Goals

- Add a validated cursor read API to `VideoTrackControlPage`.
- Detect empty rings, future cursors, lapped cursors, and slot sequence
  mismatches.
- Model the seqlock-style read shape: read `publication_sequence`, clone/read
  the entry, read `publication_sequence` again, compare.
- Add a latest-lossy helper that resolves the current latest cursor through the
  validated read path.
- Route `VideoSlotManager::acquire_latest` through that helper while preserving
  current external behavior.

## Non-Goals

- No atomics yet.
- No acquire/release ordering yet.
- No shared mmap exposure.
- No wait/wake primitive.
- No ordered recording cursor.
- No daemon transfer protocol change.

## Model

`control_page` adds:

```rust
pub enum VideoRingReadError {
    Empty,
    NotPublished { requested_cursor, latest_cursor },
    Lapped { requested_cursor, oldest_live_cursor, latest_cursor },
    SlotSequenceMismatch { requested_cursor, first_sequence, second_sequence },
}

pub fn read_entry_for_cursor(cursor: u64) -> Result<VideoRingEntry, VideoRingReadError>
pub fn read_latest_lossy_entry() -> Result<Option<VideoRingEntry>, VideoRingReadError>
```

The read helper validates in this order:

1. Empty page -> `Empty`.
2. Requested cursor newer than latest -> `NotPublished`.
3. Requested cursor older than oldest live -> `Lapped`.
4. Load `publication_sequence` from the indexed slot.
5. Clone/read the entry.
6. Load `publication_sequence` again.
7. If either sequence differs from the requested cursor -> mismatch/lapped.
8. Return the entry.

In the current in-process implementation the two reads should always match
unless the caller requests an overwritten cursor or the slot has been corrupted
inside the model. In the future shared-memory version the producer must make the
slot temporarily unpublished while writing the descriptor, then publish the final
cursor with a release store; readers use acquire loads around descriptor reads.
This avoids accepting a descriptor whose payload fields are from a newer write
while the old publication cursor is still visible.

`read_latest_lossy_entry` returns `Ok(None)` only for an empty control page.
Sequence mismatches stay visible to callers so the future shared-memory reader
can retry a write-in-progress slot instead of collapsing it into "no frame".

## Testing

- Empty read returns `Empty`.
- Cursor newer than latest returns `NotPublished`.
- Cursor older than oldest live returns `Lapped`.
- Wrapped slot mismatch is detected through `publication_sequence`.
- `read_latest_lossy_entry` returns the newest entry after wraparound.
- Existing `VideoSlotManager` latest/skip tests remain green.
