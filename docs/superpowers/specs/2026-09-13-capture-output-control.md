# Explicit capture output size

The [sizing decision](../../2026-09-13-capture-output-sizing.md) keeps source
geometry and capture output separate. This slice adds output requests for macOS
CPU and native SCK sessions, using Jackstay's existing frame-driven pool changes.

`POST /capture-sessions/{id}/output` accepts `width` and `height` in pixels. It
requires a live agent identity and, for a surface session, the session owner.
A viewer's ability to acquire frames does not give it output control. Starting,
closed and failed sessions reject requests. Sessions without backend control
return `adapter_unsupported`, including synthetic and current Linux sessions.

The backend completion produces `202` with the session ID and requested size.
It does not change reported dimensions. `GET /capture-sessions/{id}` continues
to describe published output and any pending admission. Jackstay retains old
leases while admitting replacement storage within the session's existing budget.
Requests are bounded to fit an eight-resource, 512 MiB BGRA pool, reserving 1 MiB
for metadata and rounding row size to 256 bytes for the request check. This is a
host request ceiling; backend allocation and outstanding leases still determine
when a format can be installed.

The core exposes an optional owned `VideoCaptureOutputControl`. It does not
pretend that all capture backends offer the same controls. macOS shares the
control implementation between CPU and native streams. One complete SCK config
constructor is used for both initial setup and resize, preserving other settings.

The stream owns shutdown. A mutex serializes submitting an update with stopping
the SCK handle, while the asynchronous completion owns its request state and a
single-update reservation. The completion does not borrow the stream's frame
callback context. Cancelling the HTTP future leaves the backend operation and
reservation alive; another update is rejected until completion. Stream shutdown
still clears and joins frame callbacks before freeing their Rust state, without
waiting for an output-update completion. An update completing after shutdown
returns a closed-stream error to a still-waiting caller.

Validation covers owner checks, invalid dimensions, unsupported sessions, closed
sessions, unchanged published dimensions on acceptance, and reservation lifetime
when a request is cancelled. Live acceptance must change the actual SCK output
while retaining acquired CPU and native frames, then compare their descriptors
and pixels across the replacement. Merely resizing a source window is insufficient.

A future capability/request model and controlled displays/compositors are outside
this slice. The capability model should describe backend differences, requested
requirements and obtained results without silent substitutions.
