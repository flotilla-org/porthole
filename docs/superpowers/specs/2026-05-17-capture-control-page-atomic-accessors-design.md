# Capture Control Page Atomic Accessors Design

Date: 2026-05-17
Status: implemented prototype

## Context

The video control page is now backed by one contiguous mapped region, but the
reader and writer helpers still use ordinary typed loads/stores. Before the
control page fd is passed to another process, the mapped hot fields need an
API that expresses the intended memory ordering.

This slice adds atomic accessors while the page is still private to
`VideoSlotManager`. It does not change the transfer channel or make any
cross-process reader authoritative. The point is to force the code shape toward
release/acquire publication before fd passing enters the picture.

## Goals

- Add atomic load/store helpers for hot mapped `u64` fields.
- Use release stores for producer publication:
  - per-slot `publication_sequence`
  - header producer cursor/head
- Use acquire loads for consumer/read-side validation:
  - latest producer cursor
  - per-slot `publication_sequence` before and after descriptor read
- Keep descriptor structs plain and copied by value.
- Keep single-producer semantics: no CAS on the video hot path.
- Preserve current `VideoTrackControlPage` behavior and public shape.

## Non-Goals

- No cross-process fd passing.
- No consumer cursor page.
- No wake primitive.
- No C ABI expansion.
- No per-entry cacheline padding or false-sharing tuning yet.
- No GPU/native sync changes.

## Model

The mapped page remains physically the same:

```text
VideoTrackControlHeader
padding
VideoRingEntry[capacity]
```

The hot fields are treated as atomic `u64` values in place:

- `VideoTrackControlHeader::producer_cursor`
- `VideoTrackControlHeader::latest_sequence`
- `VideoTrackControlHeader::latest_index`
- `VideoTrackControlHeader::len`
- `VideoRingEntry::publication_sequence`

The producer publish order becomes:

1. Load current producer cursor.
2. Compute next cursor and ring index.
3. Store `publication_sequence = 0` for the target slot.
4. Store the descriptor fields.
5. Release-store `publication_sequence = cursor`.
6. Store latest metadata.
7. Release-store `producer_cursor = cursor` as the head publication.

The reader path becomes:

1. Acquire-load latest producer cursor.
2. Validate requested cursor against newest/oldest live cursor.
3. Acquire-load slot `publication_sequence`.
4. Copy the descriptor.
5. Acquire-load slot `publication_sequence` again.
6. Accept only if both sequence reads match the requested cursor.

This is still not the final cross-process protocol. It is the in-process
mechanical shape of that protocol.

## Testing

- Hot-field offsets are aligned for `AtomicU64`.
- Atomic header and slot sequence helpers roundtrip values through the mapped
  page.
- Existing empty/future/lapped/seqlock/wraparound tests continue to pass.
- Existing video-slot and daemon-channel tests continue to pass.
