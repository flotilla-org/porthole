# Capture output sizing and live reconfiguration

Status: sizing policy under discussion. This records why the live resize check
has not yet exercised Jackstay pool replacement. It does not change the agreed
acquisition lifetime contract.

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

## Decision needed

Window size and capture-buffer size are separate controls. The current
recommendation is to retain a fixed output size and add an explicit host request
to reconfigure the output. An alternative is to make output dimensions follow
the source window automatically. Neither policy has been newly agreed by this
investigation. Source selection and output policy belong to Porthole; Jackstay
owns the admitted storage and lease-safe transition when the producer changes
format.

Manual window resizing alone is therefore insufficient for the missing live
pool-replacement check. The next test needs an actual output configuration
change while the probe retains its old frames. The existing bounded CPU/native
replacement tests remain relevant, but do not replace that live evidence.

Local evidence and the probe are in `/tmp/porthole-live-resize-wqbxm8zv/`:
`probe.log`, `probe/src/main.rs`, `resize-results.json`, `final-evidence.json`,
`original-geometry.txt`, `after-geometry.txt` and the before/after screenshots.
The directory contains private test credentials; publish selected evidence only.
