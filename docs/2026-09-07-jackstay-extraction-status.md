# Jackstay extraction status, 2026-09-07

Jackstay now lives in private `flotilla-org/jackstay` at revision
`1d0770e943d01fc4a59c26c17162377f7e817a82` (package 0.1.0). The filtered history
preserves 68 commits affecting the original library and viewer; a committed map
records the source commit IDs. The Rust implementation and optional PipeWire
mechanism moved together. Capture authority stays with the host.

## Completed verification

- Jackstay workspace build, tests, clippy and pinned format check passed locally.
  The backend-macos tests and clippy checks also passed.
- The SDL viewer consumed and validated 30 synthetic frames in both the offline
  dummy-video run and a normal desktop-window run. This is an in-process C-ABI
  proof, not a live desktop capture or cross-process native-handle claim.
- Jackstay CI passed on macOS and Linux (including native-feature tests and the
  SDL smoke), Windows compilation, and C11/Zig public-header guards:
  https://github.com/flotilla-org/jackstay/actions/runs/34117394704
- Porthole's workspace build, tests, clippy and pinned format check passed after
  switching to the external dependency. Its viewer helper resolved Cargo's pinned
  Git checkout and passed the standalone CTest smoke from that source.

## Live macOS verification

The installed development bundle kept its existing Apple Development signing
identity and both OS permissions. A separately launched SDL window was attached
as a surface using a temporary test agent with approved Observe/Record grants.
The test agent was revoked and its viewer/session cleaned up afterward.

Screenshot capture produced the expected 640x424 window image. A two-second
recording decoded as H.264 with 117 frames and no laps. A later native viewer run
reported `presented_frames=30` with successful Metal submission and lease release.
This does not measure GPU completion, copy overhead or the cleat agent workflow.

Live recording exposed two inherited Jackstay retention bugs. The storage
capacity now matches the ring's power-of-two capacity, and pinned frames cannot
cause another advertised frame's storage to be reused or pruned. Both fixes have
regression tests that failed before the fixes. One repeat recording reported a
ring overrun; later runs succeeded, so this is evidence of functional capture,
not a sustained throughput or stability claim.

Local evidence is under
`/var/folders/6z/8gmpf02s6gz_9zfgj292bpy00000gn/T/porthole-live-extraction-f_71txov`
(screenshot and decoded recording) and sibling `porthole-live-extraction-0gdkh67m`
(native frame-count log). The daemon used Jackstay `e08db67` for these runs;
`1d0770e` adds only the viewer's stricter success checks and documentation.

## Integration state

Porthole's source and lockfile pin Jackstay; the duplicate crate and viewer source
are removed. The viewer build discovers the dependency through Cargo metadata.
Native capture producers, portal consent and porthole's integration tests stay in
porthole. Jackstay owns its C/Zig header checks and native-library CI.

GitHub rejected a read-only deploy key because deploy keys are disabled for this
repository. Porthole CI now expects `JACKSTAY_READ_TOKEN`, a repository-scoped
read-only Contents token for Jackstay. That secret still needs operator setup;
no personal login token was copied into CI. Local Git access used the existing
GitHub CLI credential helper.

Issue #113's independent extraction is complete. Issue #114 remains open pending
private-dependency CI and real KWin capture, screenshot and recording
verification. Paneer is at the SDDM greeter; this needs an existing logged-in
KWin desktop. No automatic login was configured. The Windows and desktop workflow issues #115–#118
have not been implemented by this extraction.
