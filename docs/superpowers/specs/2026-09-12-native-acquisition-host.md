# macOS capture acquisition ownership

Status: integration in progress. Metal shared-event allocation on kiwi recovered;
native arena, XPC and isolated cross-process viewer tests now pass. Live desktop
CPU/GPU acceptance remains outstanding, so this is not a completed rollout. The CPU host and reference clients
now use the common arena; see [CPU integration](2026-09-12-cpu-acquisition-host.md).

Porthole retains capture authority: source selection, Screen Recording permission
and per-session authorization. Jackstay owns storage admission, acquisition,
readiness, completion and mapping retirement. Native sessions use
`NativeArenaProducer` and `XpcArenaServer`; the reference viewer uses the same
acquisition implementation through the C API.

## Capacity and resize

Each native session reserves at most 512 MiB for Jackstay storage. Its pool has
eight resources, two retained history entries and one producer working resource.
The remaining five resources cover admitted holding reservations. A consumer
restart receives a new incarnation; unresolved old holdings still count.

Initial allocation checks the native layout bound plus arena metadata before
allocating surfaces. During resize, both old and new generations count against
the budget. An accepted transition without enough overlap capacity pauses
publication. Incoming frames are dropped while the host retries the pending
transition. A proposal that cannot fit even after all old storage retires is
rejected and reported as a session failure.

A 100 ms host maintenance worker retries allocation and polls idle cleanup.
Consumer waits remain event-driven in Jackstay. The worker owns the runtime
independently of the session registry and Tokio runtime, and exits only after
retirement. It shares no registry reference with the SCK callback.

Session queries report installed dimensions and distinguish `starting`, `ready`,
`paused`, `failed`, `draining`, `recovery_required` and `closed`. Their message
includes the budget and publication/drop counts. Pending dimensions are not
reported as installed.

## Close, cancellation and recovery

The registry installs its teardown owner before starting SCK. An error or
cancelled startup closes that owner through the startup reservation's destructor.
Explicit session close follows the same path: stop acquisition/publication,
invalidate the listener and stop SCK outside the callback's mutex.

The closed session stays queryable while maintenance waits for every consumer
mapping and lease, retired pool and actual producer GPU write to drain. XPC
setup owners must also retire before the producer is destroyed. Only then can
a new native session reuse the single named service reservation and budget.

Five seconds without proof of retirement produces an explicit recovery status.
It never permits resource reuse. A publication fault can leave GPU use
uncertain; the host retains that producer and refuses a replacement session.
The daemon's process-wide shutdown does not yet provide a graceful drain API.

## Verification

Unit tests cover startup-error propagation, cancellation and cleanup after both
the session and its async runtime are dropped. An isolated Metal test submits a
producer write behind a GPU gate, drops the session, runtime and consumer, then
checks that the producer remains owned until the gate opens and cleanup finishes.
It passed on 2026-09-12; log: `/tmp/porthole-native-owner-retirement-metal.log`.
This tests retained cleanup within a live process. It does not establish GPU
completion after daemon process death or provide a graceful process exit API.

Porthole pins Jackstay `f482a5ca1c0c0a3831d3b4774dc8b7110c4581eb` (C ABI 0.5).
Workspace build, non-ignored tests, all-target Clippy and pinned formatting pass
on macOS and Linux. Logs for this cleanup-owner change are
`/tmp/porthole-native-owner-tests.log` and, on paneer,
`/tmp/porthole-native-owner-linux-tests.log`. Jackstay's native allocation,
replacement and GPU-readiness/completion tests also passed; see its runtime
verification record below. Generated fixtures do not establish live capture.

The ignored adapter smoke test
`sck_iosurface_stays_immutable_through_xpc_acquisition_and_delayed_release` captures
a real window, acquires through XPC, waits for readiness and checks leased pixels
before and after a delay. `PORTHOLE_SMOKE_FRAMES` and `PORTHOLE_SMOKE_HOLD_MS`
control its duration. It then closes the consumer and waits for producer drainage.
This fixed-size fixture does not prove host resize, cross-process viewer behavior
or long playback until those acceptance runs have actually been performed.


The resumed runtime checks and their crash-quarantine limitation are recorded in
Jackstay's `docs/design/acquisition-runtime-verification.md`. Generated test
pixels exercise actual GPU work but do not substitute for the live capture runs.
