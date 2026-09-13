# Capture output sizing and live reconfiguration

Status: fixed output with explicit reconfiguration agreed on 2026-09-13.
This does not change the agreed acquisition lifetime contract. The live evidence
below predates the explicit output control. The subsequent
[output reconfiguration acceptance](2026-09-13-live-output-reconfiguration.md)
passed both transitions with old CPU/native frames held.

On 2026-09-13, Porthole's installed `67fa206` build captured a dedicated TextEdit
window through both CPU and native ScreenCaptureKit paths. Accessibility and
Screen Recording were granted. The source started at 673×439 logical points,
scale 2, and both streams produced 1346×878 pixels.

`POST /surfaces/{id}/place` successfully changed the window to 850×560 points.
A fresh screenshot confirmed that geometry at scale 2. Both existing capture
sessions continued producing 1346×878 buffers. A separate Rust probe had already
acquired CPU and native frames before the resize. It observed the same allocation
generation afterward and failed its required generation-change assertion. That
is a failed reconfiguration test, not a demonstrated stale-frame or metadata bug.
Its subsequent old-pixel comparisons were not reached.

CPU and native reference viewers nevertheless completed 300 frames each with
250 ms holds and empty stderr, in 86.15 s and 80.58 s respectively. After closing
those sessions, fresh captures of the larger window produced 1700×1120 pixels.
Restoring the window to its original size left those replacement streams at
1700×1120. All sessions then reached `closed`; the test document window was
closed and the test identity revoked. The preexisting TextEdit document was not
modified or closed.

## Mechanism

The shared CPU/native setup in `sck_capture_shim.m` assigns
`SCStreamConfiguration.width` and `.height` once, from the initial content
geometry and scale. It does not call `updateConfiguration` while capturing.
Callbacks and session metadata report the actual pixel-buffer dimensions.

Apple describes window content being scaled into a largely fixed stream output,
with content rectangle and scale metadata describing the result. It also warns
that frequent output-size changes allocate additional storage. This matches the
live observations. See [Take ScreenCaptureKit to the next level, WWDC22](https://developer.apple.com/videos/play/wwdc2022/10155/).

## Agreed policy

Window size and capture-buffer size are separate controls. Keep a fixed output
size, initially chosen from the source, and let the session owner explicitly
request new pixel dimensions. Source selection and output policy belong to
Porthole; Jackstay owns the admitted storage and lease-safe transition when the
producer changes format. A successful backend request is not evidence that a
new pool or frame has already been published. Report actual dimensions from
published frames throughout the transition.

Support is backend-specific. A backend that cannot honor output size requests
must return unsupported. A later capability/request model should distinguish
what a source/backend offers, what the caller requests and what was obtained,
without silently weakening requirements or imposing a lowest common denominator.
That model remains future work.

For applications intended only for Porthole/Jackstay consumption, Linux may
eventually use a small purpose-built or repurposed compositor. macOS and Windows
will have different constraints. Controlled displays and compositor integration
are future directions, not requirements for this output control.

Manual window resizing alone was therefore insufficient for the live
pool-replacement check. The subsequent test used actual output configuration
changes while retaining old frames and passed. The existing bounded CPU/native
replacement tests remain separate coverage for admission under pressure.

Local evidence and the probe are in `/tmp/porthole-live-resize-wqbxm8zv/`:
`probe.log`, `probe/src/main.rs`, `resize-results.json`, `final-evidence.json`,
`original-geometry.txt`, `after-geometry.txt` and the before/after screenshots.
The directory contains private test credentials; publish selected evidence only.
