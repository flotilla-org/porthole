# Capture Registered CPU Pools Design

Date: 2026-05-15
Status: implemented prototype

## Context

The daemon capture transfer channel now has the right consumer lifetime shape: one
long-lived Unix-domain socket per daemon-backed consumer, stable daemon
`ConsumerId`, explicit `lease_id`s, and disconnect cleanup. The remaining CPU
transport cost is that every `latest_video_frame` response still passes an fd
and the consumer maps that fd for the acquired frame.

That is acceptable for proving leases, but it is not the destination. Reusable
CPU slots should behave like registered buffers: transfer the pool handle once,
then describe each frame as an offset/length inside a known pool. This is also
the next step toward an externally visible shared control/ring page. Once the
consumer already has stable pool mappings, frame metadata can later move out of
JSON responses and into shared memory without also changing handle lifetime.

## Goals

- Register daemon CPU shm pools once per capture transfer channel connection.
- Cache pool mappings in the capture-transfer daemon client.
- Keep `latest_video_frame` responses small: frame metadata names
  `pool_id`, `slot_id`, `slot_generation`, `payload_offset`, and
  `payload_len`.
- Keep the existing explicit `lease_id` release contract unchanged.
- Keep the socket as the setup/control/handle-transfer path.
- Preserve current latest-frame behavior and existing C ABI surface.

## Non-Goals

- No shared metadata/control ring exposed to consumers yet.
- No wake primitive work.
- No ordered recording cursor.
- No IOSurface, dmabuf, D3D, audio, or multi-track synchronization changes.
- No removal of immutable-per-frame fallback behavior where a frame has
  `pool_id = 0`.

## Protocol Shape

The capture transfer channel remains line-delimited JSON plus `SCM_RIGHTS`.

Pool registration is a server message with an fd:

```json
{
  "op": "register_cpu_pool",
  "session_id": "...",
  "track_id": 1,
  "pool_id": 7,
  "pool_generation": 3,
  "payload_map_len": 196608,
  "slot_stride": 65536,
  "slot_count": 3
}
```

Frame request and release stay as they are:

```json
{ "op": "latest_video_frame", "session_id": "...", "track_id": 1 }
{ "op": "release_video_frame", "lease_id": 42 }
```

For registered reusable CPU pools, the daemon ensures the consumer has the pool
before sending the frame response. If the pool is unknown on that connection, it
sends `register_cpu_pool` with the pool fd, then sends the frame metadata without
a per-frame fd:

```json
{
  "op": "video_frame",
  "lease_id": 42,
  "session_id": "...",
  "track_id": 1,
  "pool_id": 7,
  "slot_id": 2,
  "slot_generation": 3,
  "payload_offset": 131072,
  "payload_len": 4096,
  "payload_map_len": 196608
}
```

For immutable fallback frames where `pool_id = 0`, the daemon may keep the
current behavior: pass an fd with that specific frame response and let the
consumer map it for the frame lifetime. This keeps the slice focused on reusable
CPU pools and avoids forcing all backends through a pool abstraction at once.

## Data Model

`VideoSlotManager` already owns the information needed for registration:

- `pool_id`
- pool generation / `slot_generation`
- segment fd
- segment length / `payload_map_len`
- slot stride and slot count

The daemon needs a registry-facing way to acquire the latest frame while also
knowing whether the frame belongs to a reusable pool and how to clone that
pool's fd. The consumer needs a map keyed by `(track_id, pool_id,
slot_generation)` to a read-only mmap. `DaemonFrame` then borrows bytes from a
cached mapping and carries only `lease_id` plus range metadata. Releasing a frame
must not unmap the pool; disconnecting the `DaemonConsumer` drops cached pool
mappings.

## Error Handling

The consumer validates every registered pool:

- `payload_map_len > 0`
- mmap succeeds
- future frame ranges satisfy
  `payload_offset + payload_len <= registered payload_map_len`
- frame `payload_map_len` matches the registered pool length for the key

If a frame references an unknown pool without a preceding registration, the
consumer returns a daemon transport error. If registration or mapping fails, the
connection is considered unusable for this slice and the caller receives an
error from `latest_frame`.

Unknown release ids remain ignored by the daemon. Disconnect still releases
outstanding leases and disconnects the daemon consumer.

## Testing

- `capture-transfer` daemon-client test: a fake server registers one CPU pool,
  sends two frames from different offsets in that pool, and the client reads
  both without receiving a per-frame fd.
- `portholed` capture transfer channel test: one synthetic session acquires two frames on
  one connection and observes only one pool registration fd for the reusable
  pool.
- Existing one-connection multi-lease and disconnect-cleanup tests remain green.
- Full repo gates remain the same as AGENTS.md.
