# Bridge runtime acceptance on kiwi

2026-09-21, Apple M4, installed signed release bundle from Porthole `9bbce57`,
with Jackstay `b1b95ca`. Accessibility and Screen Recording were granted.
The prior system-wide shared-event allocation failure no longer reproduced.

## Installed daemon

A dedicated AppKit window changed colour and a counter at 10 Hz. A temporary
agent identity received bounded Observe/Record grants for that window. The
installed daemon created its native ScreenCaptureKit capture session and an
in-process export and republication.

A separate launchd-hosted generated source and standalone egress fed a second
in-process republication in the same daemon. Both republications used
`work.flotilla.porthole.attach` with distinct tokens, alongside the original
capture registration. SDL native viewers presented 30 frames from each;
a CPU viewer acquired 30 from the generated publication. After the real-capture
republication was closed, the generated one still presented 15 native frames.

The real capture was then exported and republished with `execution: "worker"`.
Native and CPU viewers each consumed 30 frames. The worker's launchd job was
absent after close. Both execution modes removed their export and CPU runtime
directories after close. The test finally closed the capture, revoked its agent
identity, and stopped its source processes and synthetic launchd job.

The in-process egress acquired and encoded 279 frames; the worker egress
acquired and encoded 87. Both reports recorded hardware HEVC 4:4:4, no encoder
errors and no sender drops. These are short local acceptance runs, not a
throughput or long-duration stability claim. Input relay was not exercised.

## Transport and codec checks

The standalone native loopback verified 25 decoded frames against its generated
source, with maximum mean absolute error 0.10 and no codec errors or drops.

Jackstay's cross-process launchd/Metal test
`named_publications_route_independently_and_retired_connections_cannot_rebind`
passed with three registrations, including an untokened one. It verifies pixel
readback, unknown-token rejection, independent retirement/replacement, rejection
of old connections, and listener recreation after every registration drops.
The full native-feature suite and offline SDL smoke also passed.

Both repositories passed their four required workspace gates: locked build,
locked tests, locked all-target Clippy with warnings denied, and pinned-nightly
formatting. Local logs and the test driver are under
`/tmp/porthole-146-validation/`; these temporary artifacts are not required by CI.

## Existing admission limits

The capture registry still admits only one native capture session globally.
A second request returns the first session's status as an invalid-argument error,
which does not explain the limit. Two simultaneous egress consumers on the same
capture also exceed its reservation budget: each asks for three holding slots,
but only five are available after history and producer reserves. The second
export records that cause while republish reports only `link closed before hello`.

The concurrent republication check therefore used independent real and generated
sources. It does not claim multiple native captures or two exports from the same
capture work. [Issue #147](https://github.com/flotilla-org/porthole/issues/147)
tracks the errors and the separate admission-policy decisions.
