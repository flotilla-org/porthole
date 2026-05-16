# Capture Consumer Connection Design

Date: 2026-05-14
Status: draft for implementation

## Context

The current daemon-backed capture consumer path opens one raw Unix-domain socket
connection per acquired frame. That was good enough to prove fd passing, mmap
validation, and daemon-side frame pinning, but it makes the connection itself
act as the frame lease. It also gives every `latest_frame` request a fresh
consumer id, so per-consumer cursor/drop counters cannot mean what they say.

The destination is still a shared control/ring page in the spirit of io_uring,
DPDK rings, and Disruptor-style sequence counters:

- producer publish cursor
- fixed-size frame metadata ring entries
- per-consumer cursor/read state
- release watermarks or lease state
- counters and discontinuity flags
- wake sequence numbers

This slice should not expose that shared ring yet. It should make the
capture transfer channel long-lived and explicit enough that a shared control
page can be registered later without changing the consumer identity or release
contract.

## Goals

- Use one capture transfer channel connection per daemon-backed consumer.
- Allocate one stable daemon `ConsumerId` per capture transfer channel connection.
- Replace connection-lifetime frame leases with explicit `lease_id`s.
- Let a single connection acquire and release multiple latest frames.
- Release all outstanding leases and disconnect the consumer when the side
  channel closes.
- Keep current latest-lossy semantics and fd passing.
- Update the C ABI daemon consumer to hold the connection.

## Non-Goals

- No externally mapped shared metadata ring yet.
- No subscription/wake protocol yet.
- No registered-pool handle optimization yet; frame acquisition may still send
  an fd with each acquired frame.
- No ordered recording cursor.
- No IOSurface, dmabuf, D3D, audio, or multi-track synchronization work.

## Protocol Shape

The raw capture transfer channel remains line-delimited JSON plus `SCM_RIGHTS` ancillary fd
passing. A connection owns exactly one daemon consumer id.

Client request:

```json
{ "op": "latest_video_frame", "session_id": "...", "track_id": 1 }
```

Server reply:

```json
{
  "session_id": "...",
  "track_id": 1,
  "lease_id": 42,
  "sequence": 10,
  "pool_id": 1,
  "slot_id": 2,
  "slot_generation": 1,
  "payload_offset": 128,
  "payload_len": 4096,
  "payload_map_len": 12288
}
```

The fd is sent with the frame response using `SCM_RIGHTS`, as today.

Client release:

```json
{ "op": "release_video_frame", "lease_id": 42 }
```

No release acknowledgement is required in this slice. Unknown lease ids are
ignored. Connection close releases every outstanding lease and calls
`disconnect_consumer` for cleanup of per-consumer state.

## Relationship To Shared Ring

The long-lived connection is not the future hot path. It is the durable
control/setup/handle path. Later it can carry messages such as:

```json
{
  "op": "register_control_page",
  "pool_id": 7,
  "consumer_cursor_offset": 4096,
  "producer_ring_offset": 8192,
  "ring_capacity": 256
}
```

At that point, cursor positions and publish/release watermarks can live in
shared memory while the UDS connection continues to handle setup, fd/Mach/Win32
handle transfer, terminal errors, wake primitive setup, and reconfiguration.

## Error Handling

If latest-frame acquisition fails, the server closes the connection for this
slice. That keeps the first protocol small and matches the current fd-sidecar
failure behavior. Structured terminal/error messages can be added with the
shared control-page work.

## Testing

- Daemon capture transfer channel test: one connection can acquire, release, and acquire
  again.
- Registry test: acquiring multiple frames with the same `ConsumerId` preserves
  per-consumer skip accounting.
- Registry test: disconnecting a daemon consumer releases outstanding pins.
- C ABI smoke remains green through `ft_consumer_connect_session`.
