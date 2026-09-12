# Simulator CPU capture: missing consumer authorization

The September 12 pi experiment could display synthetic Jackstay frames but its
live iPhone Simulator viewer exited with `acquire frame failed with status 3`
and `acquired_frames=0`. The same failure reproduced directly in the SDL viewer,
without Katzensteg interception. Accessibility and Screen Recording were granted;
the dedicated diagnosis identity held Observe/Record for the Simulator surface.
There were no pending requests when acquisition failed.

## Cause and fix

The viewer inherited `PORTHOLE_AGENT_TOKEN`, but `ft_consumer_connect_session`
called `get_session`, whose returned `SessionInfo` had `bearer_token: None`. The
C descriptor had no token field. The protected transfer socket therefore received
a frame request before authorization and closed the connection. The FFI discarded
the underlying error and returned status 3. Synthetic sessions were unaffected
because they do not require this authorization.

A Rust probe using the viewer's exact pinned Jackstay source distinguished the
failure from capture readiness or mapping errors. Without forwarding the token,
it returned `DaemonTransport { operation: "read-capture-transfer-channel",
message: "daemon closed capture transfer channel" }`. Supplying the same already
authorized identity acquired 1,866,240 bytes from the same live capture session.
The installed daemon was neither replaced nor restarted.

[Jackstay PR #1](https://github.com/flotilla-org/jackstay/pull/1) adds an optional
copied bearer token to the C session descriptor and has the SDL viewer pass its
environment token explicitly. The library does not read credentials from the
environment or approve requests. The 0.x C ABI advances to 0.2, and the viewer
checks an exact header/library match before using descriptors.

That check also caught a build artifact problem: Porthole's viewer build compiled
against the new header while its shared target directory supplied the old 0.1
library. The build script now separates library targets by resolved Cargo package
identity, in addition to refreshing CMake's source/header configuration. This
checkout pins Jackstay `19f3b40727c438338b6dcc2862e13ea07c6de0f3`.

## Verification

- The new C-ABI regression failed before forwarding was implemented: the server
  expected `authorize` and received `latest_video_frame`. It now verifies the
  authorization handshake, copied token lifetime, real fd-backed payload and lease
  release across a Unix socket.
- All four required Jackstay workspace gates passed, along with macOS feature
  tests/Clippy and the offline SDL viewer smoke.
- All four required Porthole workspace gates passed with the updated dependency.
- Both the standalone fixed viewer and Porthole's rebuilt pinned viewer acquired
  30 frames from the live Simulator capture and exited successfully using SDL's
  dummy presentation driver. This was real authorized capture, not synthetic input.
- A normal SDL window rendered the iPhone 17 Pro home screen. A Porthole screenshot
  of that viewer was visually inspected; the bounded run acquired 71 frames before
  it was stopped. This proves local visible presentation, not pi embedding.

Pi panel attachment, changing pixels through that panel, and
`katzensteg_observe` remain to be verified with the active interactive pi target.
The earlier synthetic pi test is not evidence that live embedding now passes.
Approval context/helper UI and richer FFI error diagnostics remain separate work.
