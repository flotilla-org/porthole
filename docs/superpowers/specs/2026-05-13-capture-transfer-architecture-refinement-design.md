# Capture Transfer Architecture Refinement

Date: 2026-05-13
Status: working design note

## Context

The first capture-transfer spike proved an end-to-end path:

```text
ScreenCaptureKit
  -> porthole capture session
  -> CPU shared-memory frame payload
  -> fd transfer over Unix-domain socket
  -> SDL / external consumer
```

That spike should remain understood as a proof of shape, not the final transport
architecture. It made sessions, sources, tracks, frame metadata, fd passing,
consumer attach, and latest-frame behavior concrete. It also intentionally left
performance, native handle transfer, multi-channel synchronization, and library
extraction unresolved.

This document refines the target direction after reviewing the implementation
report, the external texture notes, and reference systems including Wayland
dmabuf/syncobj, PipeWire, OBS ScreenCaptureKit, WebRTC Metal rendering, LMAX
Disruptor, DPDK rings, io_uring, Chronicle Queue, GStreamer, and OBS media
sync.

The goal is not to nail down every field now. Some details need experiments and
profiling. The goal is to give the next few phases a shape that will not block
native macOS frames, Linux dmabuf, other data channels, or later extraction into
a standalone library.

## Near-Term Priorities

The practical order is:

1. Move toward a final-looking macOS frame path.
2. Keep CPU memory buffers working as the baseline transport.
3. Add a macOS native texture path, most likely IOSurface plus Metal metadata.
4. Extract the capture-transfer library from porthole when its boundary is
   useful enough.
5. Port the same model to Linux with dmabuf/memfd.
6. Reassess Windows, audio, recording, and richer session setup once the macOS
   and Linux paths expose the real constraints.

The design should therefore be macOS-driven in the next phase, but not
macOS-shaped at the protocol layer.

## Architectural Position

The future transport should be a registered payload-pool protocol:

```text
setup/control:
  negotiate session, tracks, formats, payload kinds, handle transports
  register reusable payload slots and native handles

steady state:
  producer writes or references payload slot
  producer publishes frame metadata in shared/control state
  producer wakes consumers
  consumers acquire by sequence/slot/cursor mode
  consumers release or advance watermarks
```

The current per-frame fd path is acceptable for the prototype but should not
become the display-rate path. Native systems generally treat fds, Mach ports,
and Windows handles as buffer lifetime handles, not frame packets. Per-frame
metadata should identify an already-registered slot and the synchronization
state that makes that slot safe to read.

## Separation Of Concerns

A frame loop has three distinct mechanisms:

```text
shared/control state: what changed
wake primitive: stop sleeping and inspect state
native synchronization: prove payload contents are ready
```

These must not be conflated.

Futexes, Mach semaphores, eventfds, pipes, kqueue, Win32 events, and similar
primitives are wake mechanisms. They should mean only "something changed."
They should not carry frame metadata, native handles, or GPU correctness.

GPU/native synchronization is separate. Examples:

- Linux: sync file or DRM syncobj timeline.
- macOS: Metal command-buffer completion locally, or MTLSharedEvent for
  transferable timeline synchronization.
- Windows: shared fences or platform-specific D3D synchronization primitives.

Correctness comes from the shared metadata and native sync fields. The wake
primitive exists to avoid polling.

## Payload And Handle Model

Payload kind and handle transport are separate axes.

```text
payload_kind:
  cpu_shm
  macos_iosurface
  linux_dmabuf
  d3d_shared_resource

handle_transport:
  scm_rights_fd
  mach_port
  xpc
  win32_handle
  broker_token
  in_process
```

This distinction matters immediately. CPU shared memory on Unix can use
SCM_RIGHTS. IOSurface is not fd-like; it should be shared with Mach/XPC
semantics such as an IOSurface Mach port. D3D shared resources are also not
fd-like; they need Win32 handle duplication, inheritance, or brokered transfer.

The protocol should therefore describe handles abstractly and let each platform
binding define the actual transfer mechanism.

## Registered Buffer Pools

The target steady-state model is:

```text
register pool
register slot 0..N with payload handles or pool offsets
publish frame: pool_id + slot_id + generation + sequence + metadata + sync
consumer releases slot use or advances cursor/watermark/native fence
producer reuses slot only when allowed by policy and release state
```

The current prototype now includes the first internal version of that steady
state: each track has a fixed-size metadata ring inside `VideoSlotManager`.
Frame publication writes a registered CPU slot, appends a ring entry naming the
slot and payload range, and `acquire_latest` resolves the newest ring entry.
This proves the metadata/control shape before exposing a shared ring mapping,
wake primitive, or platform-native synchronization field.

The CPU path also now has a producer claim/fill/commit API. Producers can claim
a writable slot from the pool, fill the mapped bytes directly, and commit the
metadata ring entry without first allocating a separate frame-sized buffer. The
slot manager reserves outstanding claims so multiple producer-side claims cannot
alias the same writable slot. The existing slice-based publish API is just a
compatibility wrapper over that shape for callers that already own bytes.
Porthole's macOS live SCK path uses this through a publisher interface: the
callback supplies a borrowed frame view, and the daemon claims a slot and copies
the callback bytes directly into the shared-memory payload. This removes the
per-live-frame `Vec<u8>` allocation from the SCK callback path. The owned
`VideoCaptureFrame` path remains for adapters that have not implemented the
publisher interface and for startup/fallback paths.

For CPU memory buffers, this means preallocated shared-memory slots rather than
one mmap file per frame. For IOSurface or dmabuf, it means registering a small
swapchain of native buffers. For D3D, it means registering shared resources and
fences through the Windows binding.

CPU shared memory should preferably be one or a few registered regions, not one
file per slot. A setup message transfers the region handle once, then slots and
sidecar blobs are described by offset and length:

```text
AddPool {
  pool_id
  payload_kind = cpu_shm
  handle_transport = scm_rights_fd | platform equivalent
  total_len
  mmap_permissions
  alignment
  generation
  seal_state
}

AddSlot {
  slot_id
  pool_id
  offset
  capacity
  alignment
  intended_kind = video_pixels | audio_samples | metadata_sidecar
}
```

On Linux, a CPU pool should usually be a memfd transferred once and sealed
against resize after sizing. `F_SEAL_SHRINK`, `F_SEAL_GROW`, and `F_SEAL_SEAL`
protect consumers from mmap/truncate hazards. A reusable producer-written pool
cannot be fully write-sealed, because future frames still need writes. Immutable
one-shot blobs may use stronger sealing if a backend supports it.

Every consumer must validate `offset + length <= pool.total_len`, slot
generation, alignment, and declared access before mapping or reading. The pool
base is page-aligned by the OS mapping. Slot offsets should be at least
cache-line aligned for shared metadata and may need page, plane, texture, or
backend-specific alignment for pixel data.

The current immutable-per-frame fd clone path can release daemon-side pins
immediately after sending the fd because each frame owns its backing storage.
Reusable slots cannot do that. They need explicit leases, release messages,
consumer watermarks, or native release sync before overwrite.

The current daemon CPU path uses a deliberately small first lease: one
fd-side-channel connection owns one acquired frame, and closing that connection
releases the daemon-side pin. That is good enough for safe reusable CPU pools in
the prototype, but it is not the final control-plane shape. Streaming consumers,
recording cursors, and native GPU handles should move to explicit lease ids,
watermarks, or native release fences.

Pool retirement is also generational. A producer can announce a new pool or slot
generation during resize/reconfigure, but the old generation remains live until
no acquired frames, sidecar references, or native fences can still reference it.

## Fixed Metadata And Sidecar Data

Hot/shared metadata should be fixed-size and struct-shaped.

It should contain only:

- ids
- enum values
- counters
- timestamps
- offsets
- lengths
- fixed-size rectangles or small arrays
- generation numbers
- flags

It should not contain raw pointers, unbounded inline arrays, heap-owned strings,
or variable-size blobs. Variable-size data belongs in registered sidecar storage
and is referenced by id, offset, and length.

Examples of sidecar data:

- long damage rectangle lists
- ICC profiles or color-space blobs
- platform metadata blobs
- window titles or source labels
- cursor bitmaps
- codec/private metadata
- accessibility event payloads

The reference shape is:

```text
SidecarRef {
  pool_id
  offset
  length
  kind
  generation
}
```

The same CPU shared-memory pool machinery can back both pixel payloads and
metadata sidecars, as long as slots declare their intended kind and consumers
validate bounds and generations.

## Frame Metadata

Frame metadata should converge toward:

```text
track_id
sequence
slot_id
slot_generation
timestamp
clock_domain
duration_or_rate_hint
dimensions
visible/content rect
scale/content scale
format descriptor
color descriptor
damage rects
damage_base_sequence
payload descriptor
sync descriptor
flags/discontinuities
loss counters or references
```

Many of these can be defaulted initially. For example, CPU BGRA frames can use
`clock_domain = unknown_or_session_default`, `damage = full_frame`, and
`sync = cpu_copy_complete`. The important thing is that the protocol has named
places for the information before consumers start making incompatible
assumptions.

### Format

Do not treat format as only `Bgra8Unorm`.

The descriptor should be able to represent:

- OS pixel format, such as CoreVideo fourcc or DRM fourcc.
- GPU format, such as Metal or Vulkan format.
- Plane count.
- Per-plane width, height, stride, offset, and modifier where relevant.
- Alpha mode.
- Range and subsampling for YUV formats.

The macOS path should expect at least BGRA, higher-depth RGB, NV12/video-range
YUV, full-range YUV, and HDR-capable formats over time.

### Color And HDR

Color metadata should be explicit even when initially defaulted:

- primaries
- transfer function
- matrix
- range
- ICC/color-space reference or blob where available
- dynamic range / HDR mode
- content headroom where available

The immediate value may often be `unknown`. That is still better than silently
implying sRGB.

### Damage

Damage should be buffer-coordinate and sequence-relative:

```text
damage_kind = none | full_frame | inline_rects | sidecar_rects
damage_base_sequence
inline_rect_count
inline_rects[small_fixed_limit]
damage_ref = SidecarRef
```

If a consumer skips frames and the producer cannot accumulate damage since that
consumer's last acquired sequence, the producer must publish full-frame damage
for that consumer or mark the damage unusable. First frames and format/size
changes should default to full-frame damage.

The common cases should stay cheap and struct-shaped: no damage, full-frame
damage, or a few inline rectangles. Large damage lists should use a sidecar
reference instead of making the frame metadata variable-length.

## Cursor And Delivery Modes

Video preview and recording are different contracts.

Initial cursor modes:

- `latest_lossy`: consumer wants the newest usable frame; skipped frames are
  normal.
- `ordered_bounded`: consumer wants ordered delivery within a bounded backlog;
  loss is explicit.
- `recording`: future specialization of ordered delivery with stronger loss,
  timestamp, and retention reporting.

The current SDL viewer maps to `latest_lossy`. It should not block capture when
slow. A recorder should not silently skip frames under the same API.

Loss accounting should be observable:

```text
produced_count
published_count
dropped_before_publish_count
evicted_count
consumer_skipped_count
last_sequence
discontinuity flags
```

These counters can be approximate at first, but the contract should state what
they mean.

## Multi-Channel Sessions

The protocol should remain session/source/track based, not video-frame based.
Likely track families:

- video
- audio
- metadata/source updates
- accessibility events
- input-correlation events
- cursor or pointer state

Tracks should share a session clock model, not a sequence number. Video frame
sequence 100 and audio buffer sequence 100 do not imply synchronization. Sync
comes from timestamps in known clock domains and optional session-clock
conversion.

Different channels need different defaults:

```text
video preview: latest_lossy
audio: ordered bounded, low jitter, explicit underrun/overrun
metadata: replayable latest state plus ordered changes
accessibility/input events: ordered bounded, explicit loss
recording: ordered, timestamped, discontinuity-aware
```

This probably means session setup will not be one-size-fits-all. A terminal
viewer, recorder, accessibility observer, and remote bridge may negotiate
different cursors, payload kinds, and retention policies.

## macOS Frame Direction

The next macOS work should keep the CPU path while making room for native
IOSurface frames.

CPU baseline:

- Reduce avoidable copies where possible.
- Move toward reusable CPU shared-memory slots.
- Prefer claim/fill/commit into managed slots over producer-owned frame buffers.
- Keep BGRA working with explicit stride, format, color, timestamp, and damage
  defaults.

Native path:

- Extract IOSurface from ScreenCaptureKit CVPixelBuffer where available.
- Register IOSurface-backed slots using Mach/XPC handle transfer.
- Let Metal consumers create textures from IOSurface or CVPixelBuffer-derived
  surfaces.
- Preserve ScreenCaptureKit metadata such as frame status, display time,
  content rect, content scale, dirty rects, and dynamic range where practical.
- Add a sync descriptor that can start as `sck_sample_ready` and later support
  `metal_shared_event`.

The first IOSurface implementation does not need to solve every GPU pipeline
case. It should prove handle transfer, validation, metadata, and latest-frame
slot reuse.

## Linux Direction

Linux should follow the same model with different handles:

- CPU baseline: memfd-backed slots.
- Native path: dmabuf slots with DRM fourcc, modifier, planes, device
  negotiation, and explicit fallback.
- Sync: implicit sync where unavoidable, sync file or DRM syncobj timeline where
  supported.
- Damage: buffer-coordinate damage with base sequence.

dmabuf should not be assumed mmapable. Consumers import it through EGL, Vulkan,
VA-API, or another native path unless a negotiated CPU mapping path exists.

## Windows Direction

Windows should be left as a later phase, but the protocol should avoid blocking
it now.

Expected ingredients:

- CPU baseline: shared memory section or equivalent.
- Native path: D3D shared texture/resource handles.
- Handle transfer: Win32 handle duplication, inheritance, or broker token.
- Sync: D3D shared fence or keyed mutex depending on API level and resource
  type.

The shared protocol should not assume fd passing, mmap, POSIX permissions, or
Unix-domain sockets as universal concepts.

## Mechanical Sympathy Guidance

Use mechanical sympathy as design pressure, not cargo cult.

Useful ideas:

- single writer per track
- fixed-size rings/pools
- monotonic sequences
- claim/fill/publish ordering
- gated slot reuse
- explicit overflow/drop policy
- cache-line separation for hot producer and consumer counters
- release/acquire memory ordering in shared control blocks

Not default:

- busy spinning
- MPMC lock-free queues
- per-frame syscalls in the hot path
- allocator work in the frame loop
- durable logs for interactive preview

Cache-line padding and false-sharing work matter once shared atomics exist. They
do not matter much in the current mutex plus mmap-per-frame prototype, where
allocation and copying dominate.

Current CPU hot-path notes:

- The live macOS CPU path has one expected copy: SCK-owned callback memory into
  a reusable shared-memory slot.
- The ring/pool shape should not be assumed slower than a normal bounded
  thread-safe queue. It gives the producer bounded ownership and makes
  allocation policy explicit.
- The producer must not block on slow consumers. Consumers detect missed frames
  through monotonic sequences, slot generations, and ring wraparound.
- Drop policy remains ours to tune: overwrite old slots, add staging slots,
  grow a pool, or introduce backpressure only if deliberately chosen.
- If callback latency becomes visible, the first refinement should be a
  producer ingress thread or staging policy, not abandoning the ring.

## Library Extraction And Language

The protocol should be specified independently from the current Rust
implementation.

The likely extracted library name is `jackstay`: in keeping with porthole,
cleat, and flotilla, it names the rope used to transfer cargo between ships.
Until extraction, `capture-transfer` remains the in-repo crate name.

The current Rust crate is a good prototype vehicle because it gives tests,
clear ownership, and a C ABI quickly. It should not force the final extracted
library language. A future standalone library could plausibly be:

- Rust: strong internal safety and tests, heavier toolchain/runtime concerns.
- Zig: precise low-level control and good C ABI story, younger ecosystem.
- C: maximal ABI portability, highest manual safety burden.

For now, continue in Rust while keeping:

- a small C ABI
- dependency count low
- protocol types language-neutral
- unsafe/platform boundaries isolated
- tests focused on ownership, lifetimes, and wire compatibility

When extraction happens, the repository boundary should separate:

- protocol specification
- core transport library
- platform handle-transfer backends
- porthole producer/consumer integration
- dogfood tools

## Realistic Phases

### Phase 1: macOS CPU Path Cleanup

- Add explicit frame metadata fields with conservative defaults.
- Clarify timestamp clock domains, especially seed frame versus SCK PTS.
- Add basic loss/drop counters.
- Keep CPU shm as the stable baseline.
- Use reusable CPU slots and the live SCK borrowed-frame publisher path as the
  baseline.
- Add lightweight instrumentation for published frames, dropped frames, skipped
  frames, slot wraps, publish duration, and maximum callback publish time.
- Tighten wrap/miss detection semantics in tests and docs.

### Phase 2: macOS Native IOSurface Experiment

- Prototype IOSurface extraction from SCK frames.
- Define and test Mach/XPC handle transfer.
- Register a small IOSurface slot pool.
- Publish latest frames by slot id, sequence, metadata, and sync descriptor.
- Validate received native handles before exposing them to consumers.

### Phase 3: Library Boundary

- Identify which APIs belong to capture-transfer rather than porthole.
- Use `jackstay` as the working name for the extracted low-level transfer
  library unless a better name appears.
- Extract only after the macOS CPU/native split proves the abstraction.
- Keep porthole as a producer/consumer integration, not the owner of protocol
  semantics.

### Phase 4: Linux Port

- Implement memfd CPU slots.
- Add dmabuf slot registration and format/modifier negotiation.
- Add sync-file or syncobj timeline support when available.
- Validate against Wayland/PipeWire expectations rather than inventing a
  conflicting local model.

### Phase 5: Additional Channels

- Add audio or metadata/accessibility tracks once frame sessions are stable.
- Make clock domains and ordered cursor behavior real, not just documented.
- Add discontinuity and loss reporting suitable for recording.

### Phase 6: Windows And Remote/Mux Work

- Define the Windows handle-transfer backend.
- Explore D3D shared resource/fence support.
- Revisit remote transport and muxing once local native-handle semantics are
  clear.

## Open Questions

- What is the first macOS native handle transfer mechanism: direct Mach messages,
  XPC, or a daemon/broker abstraction?
- Should CPU shared-memory slots move to a shared control block before
  IOSurface, or should IOSurface experiments drive the control-block shape?
- How much of ScreenCaptureKit's metadata should be normalized versus preserved
  as platform-specific metadata?
- What is the minimum C ABI that remains stable across CPU shm and native
  handles?
- When extracted, should the low-level library remain Rust, move to Zig, move to
  C, or split protocol/core/backend layers by language?
- Which channel should come after video: audio, metadata, accessibility events,
  or recording cursors?

## Design Bias

Default the fields; do not erase them.

The next implementation does not need perfect color, HDR, native sync, damage,
or multi-channel semantics. It does need named slots in the protocol where those
facts will live. A vague but honest field with `unknown`, `none`, or
`full_frame` is better than a compact v1 shape that teaches consumers the wrong
assumptions.
