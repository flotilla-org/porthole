# PipeWire dmabuf handback validation

Issue: #104, Linux PipeWire producer handback must be gated by native leases.

This note records the source-level validation performed before changing the
porthole PipeWire shim. The first live retry reached the ScreenCast portal but
timed out waiting for interactive approval, so the live helper now requests a
persistent ScreenCast grant before the live gates are rerun.

## Source evidence

Primary sources inspected:

- KWin `src/plugins/screencast/screencaststream.cpp` and
  `screencastbuffer.cpp` from upstream `plasma/kwin`.
- xdg-desktop-portal-kde `src/screencast.cpp` and `src/screencasting.cpp`
  from upstream `plasma/xdg-desktop-portal-kde`.
- PipeWire `src/pipewire/stream.c` plus SPA `spa/node/io.h` and
  `spa/node/node.h` from upstream `pipewire/pipewire`.

Findings:

1. KWin is the ScreenCast PipeWire producer and allocator. The portal backend
   only brokers the Wayland `zkde_screencast_unstable_v1` stream and returns
   the node id/object metadata; it does not allocate PipeWire buffers or alter
   buffer negotiation.
2. KWin advertises `SPA_PARAM_Buffers` with
   `SPA_PARAM_BUFFERS_buffers = SPA_POD_CHOICE_RANGE_Int(3, 2, 4)`.
   Therefore the path can negotiate deeper than the default, but KWin's current
   source-level maximum is 4 buffers.
3. KWin records only when `dequeueBuffer()` returns a usable buffer. If
   `pw_stream_dequeue_buffer()` returns null, or if all dequeued dmabufs are
   waiting on release sync, `record()` returns without publishing a frame. In
   practice, holding every negotiated buffer makes KWin skip/withhold frames
   until a buffer is returned; it does not allocate beyond the negotiated pool.
4. PipeWire's stream queues match this model. Input consumers receive producer
   buffers on their `dequeued` queue and return them with
   `pw_stream_queue_buffer()`. For output producers, buffers become available
   for reuse when the graph recycles them; the SPA node docs describe `-EPIPE`
   / no-more-buffer behavior when no buffer is available.
5. Explicit sync is negotiable on the KWin dmabuf path when both sides request
   it. KWin offers a SyncObj buffer layout when `supportsSyncObjTimelines()` is
   true: it adds two extra `SPA_DATA_SyncObj` blocks, requires
   `SPA_META_SyncTimeline`, waits on the previous `release_point`, then writes
   a new acquire/release point pair. Porthole did not request that layout before
   this change, and its descriptor parser currently expects only dmabuf data
   planes, so the first fix remains hold-until-release rather than adopting
   explicit-sync handback.

## Design decision

Use hold-until-release for #104.

The porthole consumer asks for 4 PipeWire buffers, publishes a dmabuf frame by
descriptor, then keeps the corresponding `pw_buffer` dequeued until the native
lease book reports that all leases for the slot have resolved. This gates
`pw_stream_queue_buffer()` on the same lease lifetime that gates porthole's own
slot reuse.

Bounded-staleness fallback: the shim will not hold the whole negotiated pool.
It holds at most `buffer_count - 1`; if a new frame arrives while the hold queue
is full, it queues the oldest held buffer back to PipeWire and records a forced
handback count in the shim. That preserves compositor liveness under an
indefinitely held lease. It also means a pathological consumer can force the
oldest slot into the documented torn/stale-risk fallback rather than stalling
KWin. The live KDE probe must confirm this behaves as source inspection
predicts.

Deferred explicit-sync work: once porthole can parse KWin's SyncObj layout and
publish acquire/release sync descriptors, explicit sync can replace the
bounded-staleness fallback where supported. That is a separate ABI/wire design
slice, because it changes the accepted PipeWire buffer layout and frame sync
contract.

## Live ScreenCast approval

The live KDE tests use the same ScreenCast portal path as production. Agent-run
tests can easily miss the default one-minute portal chooser window, so the
adapter now uses a five-minute default `Response` timeout for KDE ScreenCast
requests. The timeout can be overridden with
`PORTHOLE_KDE_PORTAL_RESPONSE_TIMEOUT_SECS`.

The freedesktop portal API shape requested in #104/#101 discussions referred
to a `restore_token`, but the KDE backend source inspected here consumes and
returns `restore_data` for ScreenCast persistence. Porthole therefore sends
`persist_mode = 2`, caches returned KDE `restore_data` at
`${XDG_CACHE_HOME:-$HOME/.cache}/porthole/screencast-restore-token`, and passes
that value back to `SelectSources` on later runs. If KDE rejects or cancels a
run while cached restore data is present, porthole removes the cache and retries
once with a fresh chooser prompt, overwriting the cache when KDE returns new
restore data.

KDE may key the persisted grant to the requesting app or binary identity. Cargo
test binaries include hash-like names that can change after rebuilds; if the
cached `restore_data` fails after a rebuild, treat that as binary-identity
fragility and rely on the retry path rather than adding more session
configuration.

Live retry status on 2026-07-06: porthole reached `CreateSession`,
`SelectSources`, and `Start`, then blocked waiting for KDE's `Start` `Response`
signal. The adapter's new five-minute timeout fired correctly with phase
`Response signal`, but KDE did not return approval and therefore no
`restore_data` cache file was written. The live dmabuf/buffer-count checks
remain blocked until the desktop returns a ScreenCast approval response.

## Required live checks before push

- Confirm negotiated buffer count on paneer's live KDE session after porthole
  requests 4 buffers.
- Hold a lease and confirm the corresponding `pw_buffer` is not queued until
  `ft_native_release_frame`.
- Hold enough leases to fill the porthole hold budget and confirm KWin does not
  allocate beyond its negotiated pool; it should skip/withhold until a buffer is
  returned, with porthole's forced handback preserving liveness.
- Confirm whether KWin offers the SyncObj layout to porthole only after
  porthole requests `SPA_META_SyncTimeline`; the current implementation should
  stay on the implicit-sync dmabuf layout.
