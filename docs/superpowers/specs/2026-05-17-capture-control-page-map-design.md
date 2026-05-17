# Capture Control Page Map Design

Date: 2026-05-17
Status: implemented prototype

## Context

The internal video control page now has the right ring vocabulary: fixed header
fields, power-of-two capacity, per-slot publication sequence, and validated
cursor reads. It is still stored as ordinary Rust fields plus a `Vec`, so the
next mmap slice would have to change storage, layout, and cross-process
behavior at once.

This slice only changes storage. The control page becomes one contiguous
file-backed mapping owned by `VideoTrackControlPage`, but it remains private to
the in-process `VideoSlotManager`. No fd is passed to consumers yet, and the
capture transfer channel remains unchanged.

## Goals

- Back `VideoTrackControlPage` with one contiguous mapped region.
- Define explicit header and entries offsets, aligned for the future shared
  memory version.
- Preserve the existing control-page API and current video behavior.
- Keep reads and writes typed through `VideoTrackControlHeader` and
  `VideoRingEntry`, not ad hoc byte parsing.
- Add layout/byte-length tests so the next fd-passing slice has concrete
  invariants to validate.

## Non-Goals

- No cross-process control-page fd passing.
- No atomics or acquire/release loads/stores yet.
- No wake primitive.
- No consumer cursor page.
- No transfer-channel protocol change.
- No GPU, IOSurface, dmabuf, D3D, or native synchronization changes.

## Model

The page layout is:

```text
offset 0
  VideoTrackControlHeader
  padding to CONTROL_PAGE_ALIGNMENT
entries_offset
  VideoRingEntry[capacity]
```

`CONTROL_PAGE_ALIGNMENT` is 128 bytes. That is deliberately conservative for
Apple silicon and recent Intel adjacent-line prefetch behavior. This slice does
not pad each individual ring entry to a cache line because entries are still
plain structs and the immediate goal is storage migration, not false-sharing
tuning. Per-entry/cacheline padding belongs with the later atomic layout slice.

The mapped byte length is:

```text
align_up(size_of::<VideoTrackControlHeader>(), CONTROL_PAGE_ALIGNMENT)
  + capacity * size_of::<VideoRingEntry>()
```

`VideoTrackControlPage::new(capacity)` still rounds `capacity` up to a power of
two and clamps to at least one. It writes the initialized header into the mapped
region and zero-initializes all entries by relying on the new shared-memory
segment's initial zero contents.

All existing helpers operate by loading/storing typed structs at fixed offsets:

- header at `0`
- entry `i` at `entries_offset + i * entry_len`

The unsafe boundary stays inside `control_page`. Callers still receive owned
snapshots and owned `VideoRingEntry` values.

## Follow-On

- Add atomic accessor wrappers around the mapped fields.
- Pass the control-page fd on the capture transfer channel and let daemon
  consumers validate magic/version/length without using it for acquisition.
- Add shadow-read comparison against socket frame metadata.
- Make the shared control page authoritative for latest descriptor reads.

## Testing

- Layout test: byte length and entries offset match the documented formula.
- Initialization test: header fields are written into the mapped header and
  entries start zeroed.
- Existing wraparound, lapped, seqlock-read, video-slot, daemon-channel, and
  capture registry tests stay green.
