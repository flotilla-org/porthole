# Capture Transfer Protocol Design

Date: 2026-05-12
Status: draft

## Context

The Katzensteg graphics-session roadmap calls for a frame transfer protocol after
Metal capture and porthole recording expose the real constraints. A markdown-only
protocol draft is too likely to stay unproven, so this phase should produce a
small implemented protocol draft with one real producer and one real consumer.

The first end-to-end path is:

```text
porthole ScreenCaptureKit capture
  -> capture-transfer producer
  -> local shared-memory video payloads passed by fd over UDS
  -> SDL viewer consumer
```

This is deliberately not the final zero-copy design. The point is to make the
session model, event model, metadata, ownership, and consumer behavior concrete
before adding IOSurface, dmabuf, remote transport, or mux semantics.

## Goals

- Model capture sessions, with video frames as the first implemented track type.
- Prove the model with real porthole-produced macOS frames.
- Prove cross-language consumption through a small C ABI.
- Use CPU shared memory as the first payload transport.
- Pass shared-memory file descriptors with `SCM_RIGHTS` on a raw Unix-domain
  side channel rather than trying to encode descriptors in HTTP bodies.
- Make latest-frame viewing behavior explicit: slow interactive consumers skip
  stale frames rather than blocking the producer.
- Leave room for audio, accessibility events, metadata events, native handles,
  ordered recording cursors, muxed consumers, and remote transport.

## Non-Goals

- No IOSurface, dmabuf, D3D shared resources, or native sync primitives in v1.
- No audio capture or accessibility-event capture in v1.
- No remote transport in v1.
- No reliable every-frame recording cursor in v1.
- No extraction to a separate repository in v1. The code should live inside
  porthole until the boundary is proven by Katzensteg or another consumer.

## Vocabulary

The protocol should not be named or modeled as "frames only." It is a capture
session protocol with tracks.

- **Session:** A producer-owned capture session that consumers can attach to.
- **Source:** A captured thing, such as a porthole-known window, surface, or
  display. Source identity is separate from OS window ids and transport handles.
- **Track:** A typed stream associated with a source. V1 implements only video.
- **Event:** Control-plane state such as session start, source registration,
  track registration, source updates, and producer shutdown.
- **Payload:** Data-plane storage for media data. V1 implements CPU shared
  memory. Native handles are future payload kinds.
- **Cursor:** A consumer's read position or read mode. V1 implements latest-frame
  semantics for video.

Reserved track types:

- video
- audio
- accessibility events
- metadata / source updates
- input-correlation events

Only `video` is implemented in v1.

## Architecture

The first implementation should have five pieces:

```text
crates/capture-transfer/
  Rust protocol model, shared-memory transport, fd passing, C ABI

portholed HTTP-over-UDS registry
  session discovery, source/track metadata, producer lifecycle

porthole producer integration
  ScreenCaptureKit frames copied into capture-transfer video payloads

raw capture-transfer UDS side channel
  framed metadata plus SCM_RIGHTS descriptors for payload handles

tools/capture-viewer-sdl/
  standalone SDL consumer using only the C ABI

docs/superpowers/specs/
  protocol draft, updated as implementation pressure reveals constraints
```

The `capture-transfer` crate owns the protocol model and transport. It must not
know about porthole, ScreenCaptureKit, SDL, kitty graphics, ffmpeg, or
Katzensteg.

Porthole integration should be a producer. It discovers or receives a porthole
surface/window/display, starts a ScreenCaptureKit session, registers a source and
video track, and publishes frames.

The SDL viewer should be a separate executable and must consume through the C
ABI. This keeps the first dogfood honest: if the ABI cannot support the viewer,
the boundary is not real yet.

HTTP-over-UDS remains the control plane. It can create sessions, list sessions,
return source/track metadata, and provide the fd-transfer socket path or token.
It must not pretend that an HTTP body carries a file descriptor. `SCM_RIGHTS` is
socket ancillary data, so fd transfer needs a raw Unix-domain socket path where
the implementation controls `sendmsg` and `recvmsg`.

## V1 Lifecycle

The minimal lifecycle is:

```text
producer creates local session
producer registers source
producer registers video track for source
consumer discovers session through portholed
consumer connects to raw fd-transfer side channel
consumer receives replayed source and track registration events
producer publishes video frames to the track
consumer requests or subscribes to the latest video frame
daemon replies with frame metadata plus shared-memory fd via SCM_RIGHTS
producer updates source/track metadata if size or format changes
producer unregisters source or exits
consumer receives terminal event and cleans up
```

Consumers attach to a session, not directly to OS windows. Session identity must
be distinct from the local transport address so the model can later survive mux
or remote transport.

The first interaction mode is consumer-initiated: a viewer asks for the latest
frame for a track. The model must also leave room for producer-initiated offers:
a terminal or other sink may later advertise that it can accept an image source,
after which a producer can offer or push handles to that sink. Both flows use the
same session/source/track vocabulary; they differ in who starts the handle
exchange.

## C ABI Shape

The C ABI should be small, explicit, and versioned. It is not the whole protocol;
the protocol semantics live in this document and in Rust model types.

Producer side:

```c
ft_status ft_producer_create(const ft_producer_options*, ft_producer**);
ft_status ft_producer_register_source(ft_producer*, const ft_source_desc*, ft_source_id*);
ft_status ft_producer_register_track(ft_producer*, ft_source_id, const ft_track_desc*, ft_track_id*);
ft_status ft_producer_publish_video_frame(ft_producer*, ft_track_id, const ft_video_frame_desc*, const void* pixels, size_t len);
ft_status ft_producer_unregister_source(ft_producer*, ft_source_id);
void      ft_producer_destroy(ft_producer*);
```

Consumer side:

```c
ft_status ft_consumer_connect(const ft_consumer_options*, ft_consumer**);
ft_status ft_consumer_poll_event(ft_consumer*, ft_event*);
ft_status ft_consumer_acquire_latest_video_frame(ft_consumer*, ft_track_id, ft_video_frame*);
void      ft_consumer_release_video_frame(ft_consumer*, ft_video_frame*);
void      ft_consumer_destroy(ft_consumer*);
```

The exact names can change during implementation, but the shape should remain:
session-oriented handles, explicit ids, explicit acquire/release ownership, and
typed video functions for the only implemented track type.

The in-process producer-pointer connection is only a bootstrap/test shape. The
cross-process C ABI should grow a descriptor-based connection:

```c
typedef struct ft_session_descriptor {
  const char *control_socket_path;
  const char *session_id;
} ft_session_descriptor;
```

That descriptor lets a consumer use HTTP-over-UDS for registry metadata and the
raw UDS side channel for descriptor transfer.

## Event Model

The event stream carries replayable control-plane state. A consumer that connects
after a producer has already started must be able to reconstruct current source
and track registrations before receiving live updates.

V1 event types:

- producer started
- source registered
- source updated
- track registered
- track updated
- source unregistered
- producer stopped

Frame payloads are not events. They are data-plane state acquired by track id.

## Video Frame Metadata

Each video frame must carry:

- track id
- monotonically increasing sequence number
- timestamp
- timestamp clock domain
- width
- height
- stride
- pixel format
- colorspace or `unknown`
- sync kind, initially `cpu_copy_complete` for copied CPU payloads
- damage kind and damage base sequence, initially full-frame
- damage regions, initially optional and usually full-frame
- basic loss counters
- payload kind, initially `cpu_shm`
- payload offset, payload length, and mapped-region length

The first implementation should support the smallest practical pixel-format set.
It is acceptable to start with BGRA or RGBA if that is what the ScreenCaptureKit
copy path and SDL upload path can use cleanly. The metadata should still model
format and stride honestly.

The CPU shared-memory implementation carries conservative metadata defaults.
CoreGraphics seed frames use a Unix-time timestamp clock. Live ScreenCaptureKit
frames use the media-time clock reported by the sample buffer. Color space is
`unknown` until the producer can report it explicitly. CPU-copied frames use
`cpu_copy_complete` or `sck_sample_ready` as their sync kind; neither implies a
future native GPU timeline. Damage defaults to `full_frame` with
`damage_base_sequence` equal to the frame sequence until SCK dirty rects or
accumulated damage are implemented. Loss counters are surfaced as scalar
metadata so consumers can distinguish normal latest-frame skips from a reliable
recording cursor.

## Shared-Memory Transport

V1 uses local CPU shared memory. The producer copies ScreenCaptureKit output into
bounded shared-memory slots owned by the capture-transfer library. Cross-process
consumers receive file descriptors for those slots with `SCM_RIGHTS`.

Frame payload metadata is range-based even when the first daemon path still uses
one immutable file per frame:

```text
payload_offset
payload_len
payload_map_len
```

Consumers must validate `payload_offset + payload_len <= payload_map_len` before
reading. In-process producers may use reusable pool slots today because
`ft_consumer_release_video_frame` is a real release point. Daemon-backed capture
sessions still use immutable per-frame files: the current one-shot fd request
does not tell the daemon when a remote consumer has finished reading, so reusing
that slot immediately after sending the fd would be unsafe. Cross-process
reusable pools need an explicit lease/release, cursor watermark, or native fence
protocol before they become the daemon default.

Interactive consumers use latest-frame semantics:

- producers publish monotonically increasing frame sequences
- consumers acquire the newest available frame for a video track
- skipped intermediate frames are normal
- slow consumers do not block the producer
- acquired frames are pinned until release or consumer disconnect cleanup
- the producer may reuse unpinned older slots
- fd passing is explicit metadata plus ancillary data, not HTTP body content

This is the correct behavior for an SDL viewer and future terminal bridge. A
recording consumer can later add ordered cursor semantics and explicit drop
accounting.

## FD Transfer Side Channel

The fd-transfer side channel is a raw Unix-domain socket separate from
portholed's Axum HTTP socket. Messages on the side channel should be small
length-prefixed JSON or binary frames that describe the operation and the
metadata. Handles travel as ancillary data with `SCM_RIGHTS`.

Initial consumer-pull request:

```text
consumer -> daemon: latest_video_frame { session_id, track_id }
daemon -> consumer: video_frame_metadata { sequence, width, height, stride, format, len } + fd
```

Reserved producer-offer flow:

```text
sink -> daemon: register_sink_capability { accepted_track_types, accepted_payload_kinds }
producer/daemon -> sink: offer_source_or_track { session_id, source_id, track_id }
sink -> daemon: accept/reject offer
producer/daemon -> sink: metadata + fd/native handle
```

The content-type idea still has value as an operation marker, but only at a
layer that owns the raw socket. A header such as
`application/vnd.flotilla.capture-transfer.fd+json` means "this framed message
is accompanied by SCM_RIGHTS ancillary data"; it does not mean the JSON or HTTP
body contains a descriptor.

## Future Extensions

Native payloads:

- macOS IOSurface, with Metal synchronization where needed
- Linux dmabuf, with explicit sync where needed
- Windows D3D shared resources

Additional tracks:

- audio samples with format, rate, channel layout, timestamp, and ordered cursor
  behavior
- accessibility events with structured payloads and bounded reliable delivery
- metadata streams for title, app identity, geometry, display, and colorspace
  changes

Additional consumer modes:

- latest-frame cursor for interactive display
- ordered cursor for recording or analysis
- replay cursor for mux attach

Additional transports:

- local FD-passing over raw UDS side channel
- mux side channel
- remote out-of-band frame or video data

## Testing

The first implementation should have tests at three levels:

- `capture-transfer` unit tests for ids, state replay, source/track lifecycle,
  frame slot acquisition/release, and latest-frame skipping.
- porthole daemon/core tests that can exercise producer wiring with a synthetic
  frame source without macOS permissions.
- manual macOS smoke test for the real ScreenCaptureKit producer and SDL viewer.

The real-capture smoke command is `scripts/manual-capture-transfer-smoke.sh`.
It attaches the frontmost tracked surface by default, creates a
`capture-session surface`, parses `session_id` and `porthole_socket` from the
descriptor, and starts the SDL consumer for a bounded frame count.

The real ScreenCaptureKit path is permission-dependent. If Accessibility or
Screen Recording permission is missing, work must stop with `BLOCKED` and ask
the user to grant the permission. Do not add mock bypasses or code-level
workarounds for missing OS permissions.

## Open Questions

- Should porthole call the Rust API directly while external consumers use the C
  ABI, or should porthole also call the C ABI to maximize dogfood?
- What is the first shared-memory primitive on macOS: POSIX shared memory,
  memfd-like temporary files where available, mmap files, or a platform wrapper?
- Should session discovery be by explicit UDS path, session id through the
  porthole daemon, or both?
- Should the fd-transfer side channel use one long-lived connection per
  consumer, one connection per acquired frame, or both?
- What is the first producer-initiated offer scenario worth implementing:
  terminal image-source registration, recorder subscription, or mux replay?
- Which initial pixel format best minimizes conversion while keeping SDL upload
  simple?
- How much frame-drop accounting belongs in v1 if recording is deferred?
