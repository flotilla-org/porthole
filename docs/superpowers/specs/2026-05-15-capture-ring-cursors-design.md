# Capture Ring Cursors Design

Date: 2026-05-15
Status: implemented prototype

## Context

Reusable CPU pools are now registered once per daemon consumer connection. Frame
payloads are stable ranges inside those pools, but the control state is still
split across a private metadata ring, per-consumer skip maps, frame pin sets,
and JSON request/release messages.

The destination is a shared control/ring page shaped more like io_uring or
DPDK-style queues: producer progress, consumer positions, release state, and
counters live in one mechanically sympathetic control structure. This slice does
not expose that page cross-process. It makes the in-process model look like the
future shared structure so later mmap/atomic work is a smaller step.

## Goals

- Add an explicit in-process control model for video ring cursors.
- Track producer progress independently from frame sequence.
- Track per-consumer latest cursor, release cursor, skipped count, and acquired
  count in one place.
- Keep current latest-frame semantics: consumers acquire newest available frame,
  slow consumers skip intermediate frames, and producers do not block.
- Keep daemon capture transfer channel wire shape unchanged.
- Preserve current payload pool, lease, and pin behavior.

## Non-Goals

- No cross-process shared control mmap in this slice.
- No atomics, futexes, eventfd, kqueue, or wake primitive work.
- No ordered recording cursor.
- No binary capture transfer channel protocol change.
- No GPU synchronization or native handle transfer change.

## Model

Each video track has a `TrackRingControl` that owns:

- the fixed-size metadata ring
- a producer cursor that increments on every committed frame
- the latest published sequence

Each `(consumer_id, track_id)` has a `ConsumerRingCursor` that owns:

- last acquired producer cursor
- last acquired sequence
- release cursor for the latest released producer cursor
- skipped frame count
- acquired frame count

`VideoSlotManager::store_published_payload` appends a metadata entry and
advances the track producer cursor. `acquire_latest` resolves the latest entry
through `TrackRingControl`, pins the stored frame, and advances the consumer
cursor. If the consumer's previous producer cursor is behind the latest cursor,
the difference minus one is recorded as skipped frames.

`release` removes the pin as it does today and advances the consumer release
cursor when the released frame belongs to that consumer/track. Disconnect cleanup
removes all cursor state for that consumer.

## Data Flow

```text
producer commit
  -> write payload range
  -> TrackRingControl::push(entry)
  -> producer_cursor += 1
  -> ring entry stores producer_cursor, frame_key, sequence, pool/slot/range

consumer latest acquire
  -> latest TrackRingControl entry
  -> StoredFrame by frame_key
  -> pin StoredFrame for consumer
  -> ConsumerRingCursor::acquire(entry)
  -> returned desc includes skipped and evicted counters

consumer release
  -> unpin StoredFrame for consumer
  -> ConsumerRingCursor::release(producer_cursor)
  -> prune unpinned frames as today
```

## Error Handling

The new model should not introduce new public error cases. Existing unknown
track behavior remains: acquiring from a track with no ring entry returns
`UnknownTrack`. Release remains tolerant of already-pruned or unknown frames
because daemon disconnect cleanup and duplicate releases must stay harmless.

## Testing

- `debug_control_snapshot` exposes producer cursor, latest sequence, and ring
  capacity for tests.
- A consumer acquiring after wraparound records skipped frames from producer
  cursor movement, not from ad hoc sequence-only state.
- Releasing frames advances the consumer release cursor.
- Disconnecting a consumer removes its cursor state and pins.
- Existing reusable-pool and daemon-client tests remain green because the wire
  protocol is unchanged.
