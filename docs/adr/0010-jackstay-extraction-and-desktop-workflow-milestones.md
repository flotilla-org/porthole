# Jackstay extraction and desktop workflow milestones

Status: accepted by Robert, 2026-09-05, following the project-map roadmap discussion.

## Decisions

Jackstay extraction and desktop workflow verification are separate tracks. They
can interleave; neither track waits for the other. API stability is a distant
goal and does not gate this work.

### Jackstay as a usable 0.x dependency

Extract Jackstay into `~/dev/jackstay` and initially push it to the private
`flotilla-org/jackstay` repository. Preserve the relevant source history. Porthole
will consume a pinned revision of that repository; a temporary requirement for
private-repository access in developer builds and CI is accepted. Public release
and a stable API are separate later decisions.

The extraction is complete when the independent library and reference viewer
work, porthole consumes the external dependency, and the existing macOS and Linux
native capture paths have passed real-desktop verification across that boundary.
A synthetic producer and viewer must also work without a porthole checkout or
daemon. Keep the Rust implementation and C ABI, with explicit versioning and
freedom to change 0.x contracts. Neither Windows native capture nor the 1.0 stamp
is an extraction gate.

The reference viewer lives with Jackstay. Katzensteg's existing SDL interception
of the simple viewer is sufficient for this milestone; direct integration to
reduce copies can wait. Performance measurement remains tracked by porthole #86;
moving the repository does not establish a performance claim or close #80.

### Capture mechanism and authority

Jackstay owns the transport, native handles, synchronization and buffer lifetime.
Reusable capture mechanisms may live beside them. In particular, retain the
Linux PipeWire mechanism in Jackstay: the current handoff accepts an already-open
PipeWire connection and a stream identifier supplied by the host application.

Porthole obtains OS permission, selects the capture source, applies caller
authorization and supervises the desktop capture session. Another host, such as
a compositor or desktop environment, can supply its own authorized source and
use Jackstay without adopting porthole's identity or token system. This does not
require a general coordinator framework during extraction. A future PipeWire
example can accept an authorized connection from its launcher; the synthetic
example proves independence first.

This ownership boundary is provisional. Keep the existing mechanism together
where it works and adjust it when another consumer exposes a concrete problem.
Do not move portal approval or desktop application identity into Jackstay merely
because the PipeWire implementation lives there.

"Authority in porthole" here describes the desktop host's responsibility relative
to the library. It does not reverse ADR-0006: the current mint/grant commands are
a local-trust development facility. The intended general agent authority remains
external, with porthole verifying and enforcing tokens. No new permission model,
notification approval UI or enforcement bypass belongs in these milestones.

### One desktop workflow on macOS, KWin and Windows

The acceptance flow on each platform is:

1. With the user already logged into a GUI session, porthole starts automatically
   within that session using the platform's supported startup mechanism.
2. Porthole launches a terminal hosting a cleat daemon and a coding agent inside
   that GUI session. Provision the agent's porthole token and required grants
   before launch using the existing development authority path.
3. The operator attaches to that agent's terminal using `cleat attach`. Use kiwi
   as the client for the remote KWin and Windows targets; on kiwi itself use a
   separate local client terminal. The connection must reach the cleat daemon
   in the GUI session, not start a replacement daemon in an SSH session.
4. The agent calls target-local porthole to launch an application, send input and
   save a screenshot that visibly demonstrates the input took effect.
5. Detach and reattach without losing the agent; clean up the processes and test
   surfaces created by the verification run. Record build revisions, OS/session
   details, commands and evidence without recording token values.

Attachment means terminal interaction through cleat. It does not mean viewing or
controlling the remote desktop through a live video stream. Pools and terminal
session lifetime remain cleat's responsibility; porthole supplies the GUI-session
launch context and desktop operations.

macOS and KWin get regression verification and fixes for demonstrated drift.
Windows first gets real local launch, input and screenshot operations, then the
session-startup and remote-attachment flow on gouda. Beaufort is the later physical
GPU target. Windows' existing named-pipe control plane is a starting point, not
proof of a real desktop adapter.

Existing login and necessary OS grants are prerequisites. Automatic startup
within that session is in scope; creating a login, automatic login after reboot,
continuous Windows capture, DXGI transport and a multi-GPU matrix are outside this
milestone. A missing macOS Accessibility or Screen Recording grant is BLOCKED
under AGENTS.md; resume after the user grants it, without a code workaround.

### Streaming and Tender

A future bridge can consume a local Jackstay stream, encode and transmit it,
then decode and publish into a local stream on the destination host. Native
handles remain local to their host. The network protocol and codec are undecided.

Integrated host discovery, identity and connectivity may use Tender after its
extraction from flotilla. This creates no Tender dependency for Jackstay
extraction or the desktop workflow. No Tender requirements are established now;
record specific needs only when evidence produces them. A bounded experiment
between explicitly configured endpoints remains possible later.

## Sequencing and existing work

The standalone Jackstay proof precedes porthole's pinned dependency switch. Real
Windows desktop operations precede the complete Windows workflow. macOS and KWin
verification can proceed independently, sharing the acceptance flow above.

Use existing issues for launch correlation (#89, related #10), signing identity
(#95), native attach readiness (#97), GPU failure handling (#92), native overhead
measurement (#86), dialog-free KWin capture (#108, with older blocker #78 to
revalidate), Windows test coverage (#111) and host sleep inhibition (#112).
These retain their own acceptance criteria; none is silently closed by planning
or extraction. Only demonstrated prerequisites block a particular workflow.

## Relationship to earlier plans

This decision supersedes extraction/freeze coupling in ADR-0005 and ADR-0008 and
the July 8 project-map Windows brief's rule to extract only when Windows capture
starts. Linux remains a required regression target, not a reason to wait for a
stability promise. It also replaces that brief's screenshot-only proof with the
agreed launch/input/screenshot flow on all three platforms.

The mechanisms of ADR-0007 and ADR-0009 remain applicable. A standalone example
may supply its own setup host; the supported porthole macOS desktop capture path
continues to use its launchd-owned broker and OS permissions.

## Implementation issues

- [Extract Jackstay 0.x with an independent synthetic producer and reference viewer](https://github.com/flotilla-org/porthole/issues/113).
- [Consume pinned Jackstay from porthole and verify macOS/Linux capture end to end](https://github.com/flotilla-org/porthole/issues/114).
- [Verify the macOS GUI-session agent workflow through cleat and porthole](https://github.com/flotilla-org/porthole/issues/115).
- [Verify the KWin GUI-session agent workflow with cleat attachment from kiwi](https://github.com/flotilla-org/porthole/issues/116).
- [Implement real Windows app launch, input and screenshots through porthole](https://github.com/flotilla-org/porthole/issues/117).
- [Start a Windows GUI-session agent and attach from kiwi through cleat](https://github.com/flotilla-org/porthole/issues/118).
