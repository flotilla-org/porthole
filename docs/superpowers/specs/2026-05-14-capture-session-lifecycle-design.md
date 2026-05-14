# Capture Session Lifecycle Design

## Context

PR #27 proved the macOS CPU frame path:

```text
ScreenCaptureKit callback -> porthole capture registry -> capture-transfer shm pool -> fd lease -> SDL consumer
```

That path is now fast enough to keep iterating on, but the session registry is
still spike-shaped. A surface publisher session is inserted into the registry
before the first frame is ready, owned-frame sessions have a detached background
task, and there is no explicit way to stop a session. This is acceptable for the
first smoke test, but it is the wrong shape to carry into long-lived consumer
transport, Metal payloads, extraction, or session enumeration.

## Goals

- Make capture session lifecycle explicit in daemon/protocol responses.
- Ensure consumers never see a misleading `UnknownTrack` during startup.
- Add an explicit close path that drops native capture handles and aborts owned
  frame tasks.
- Keep the current one-frame-per-fd-request transport unchanged.
- Keep the slice macOS CPU-frame oriented; do not design Metal, Linux, or audio
  here.

## Non-Goals

- No long-lived consumer connection yet.
- No shared metadata ring changes yet.
- No capture session enumeration endpoint yet.
- No recording cursor or every-frame guarantee.
- No encoded video file output.

## Lifecycle Model

The daemon registry owns session state. The current externally visible states
are:

- `starting`: the session id exists, but the first frame has not committed.
- `ready`: source, track, dimensions, and latest-frame acquisition are usable.
- `closed`: the producer/source ended normally while the session remains
  inspectable.
- `failed`: startup or producer failure has been recorded. The session remains
  inspectable long enough to explain the failure, but latest-frame acquisition
  is rejected.

Explicitly closed sessions are removed from the registry. A later lookup returns
the same unknown-session behavior as any other missing id. Producer-ended
sessions use `closed` instead, so a caller can distinguish intentional API
removal from source disappearance or natural producer shutdown.

## Protocol Shape

Add a string status field to capture session responses:

```json
{
  "session_id": "...",
  "source_id": 1,
  "track_id": 1,
  "status": "ready",
  "status_message": null,
  "fd_socket_path": "..."
}
```

`CreateCaptureSessionResponse` should return `ready` for successful creates,
because the route still waits for first-frame readiness before returning.
`CaptureSessionResponse` exposes the actual registry state, including
`starting`, `closed`, or `failed` if future discovery can observe them.

`LatestVideoFrameRequest` is unchanged. If the session is not `ready`, the fd
sidecar returns an error and closes the connection without sending a frame fd.
HTTP errors for registry operations should map:

- unknown session -> `surface_not_found` / 404, matching today's behavior
- not ready -> `invalid_argument` for now, with a message naming the session
  status
- closed -> `invalid_argument` for now, with a message naming the terminal
  status
- failed -> `internal_error` if caused by producer failure

This avoids adding new public error codes until the transport shape is clearer.

## Daemon Registry Behavior

`CaptureSession` gets an explicit status and optional status message.

Publisher startup keeps the session entry in `starting` so callbacks have a
target. The first committed frame updates dimensions, pixel format, and status
to `ready`. If first-frame wait times out or the publisher returns an error
before readiness, the session is removed as part of create failure cleanup.
Startup also stores a cancellation sender. Removing the session while startup is
waiting wakes the create path, which returns a closed-startup error instead of
allowing a late first-frame result to be reported as successful.

Owned-frame fallback creates the session only after the first frame is known, so
it can start as `ready`. Its background task marks the session `failed` when
`next_frame()` returns an error and `closed` when `next_frame()` returns
`Ok(None)`.

Publisher sessions are also supervised by a registry-owned task. The task owns
the native capture handle, waits for terminal events from the adapter, marks the
session `failed` or `closed`, and is aborted when the session is removed.
Dropping a `CaptureSession` must abort any capture task and send any pending
startup cancellation signal.

## API Surface

Add:

```http
DELETE /capture-sessions/{id}
```

The response can be the existing empty OK shape. This is a developer/testing
primitive for now: it lets smokes stop sessions intentionally and proves the
registry owns native capture lifetime.

The CLI can gain:

```sh
porthole capture-session close <session_id>
```

This is small but useful for manual testing. The SDL viewer can continue to rely
on process exit and fd lease release; it does not need to call close yet.

## Testing

Core tests should stay deterministic and use the in-memory adapter.

Required tests:

- synthetic session response reports `status: ready`
- surface session response reports `status: ready`
- `GET /capture-sessions/{id}` includes status and dimensions
- `DELETE /capture-sessions/{id}` removes the session; follow-up `GET` returns
  404
- dropping/removing a session aborts an owned-frame background task
- startup close sends the pending startup cancellation signal
- owned-frame normal end marks the session `closed`
- publisher monitor errors mark the session `failed`
- latest-frame against a non-ready session does not produce `UnknownTrack`
  internally

Manual smoke:

```sh
./scripts/manual-capture-transfer-smoke.sh --surface-id <simulator-surface> --frames 120
```

This should continue to work unchanged.

## Follow-On Slices

After this lands:

1. Replace one-connection-per-frame fd transfer with a long-lived consumer
   connection.
2. Add consumer-visible frame/sequence statistics to the SDL smoke.
3. Start the macOS Metal/IOSurface payload path as a second payload kind.
4. Extract the stable subset into `jackstay`.
