# Jackstay extraction status, 2026-09-07

Jackstay now lives in private `flotilla-org/jackstay` at revision
`16c4d24bc2e2d6820c316a55eb3c4ea3967c2890` (package 0.1.0). The filtered history
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
  https://github.com/flotilla-org/jackstay/actions/runs/34115549960
- Porthole's workspace build, tests, clippy and pinned format check passed after
  switching to the external dependency. Its viewer helper resolved Cargo's pinned
  Git checkout and passed the standalone CTest smoke from that source.

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
private-dependency CI and real macOS/KWin capture, screenshot and recording
verification against this pin. The Windows and desktop workflow issues #115–#118
have not been implemented by this extraction.
