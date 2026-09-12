# macOS capture acquisition ownership

Status: integration in progress. Live GPU verification remains blocked by Metal
shared-event allocation on kiwi. This is not a completed acquisition rollout;
live CPU acceptance also remains outstanding. The CPU host and reference clients
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

A 100 ms host maintenance task retries allocation and polls idle cleanup.
Consumer waits remain event-driven in Jackstay. The task shares no registry
reference with the SCK callback, avoiding a registry/callback reference cycle.

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

Unit tests cover startup-error propagation and cancellation retaining an owner
until idle maintenance completes. Jackstay tests cover CPU mapping/frame drainage
and native allocation preflight; actual gated-GPU shutdown still needs execution.

Porthole's workspace build, non-ignored tests, all-target Clippy and pinned
formatting pass against Jackstay revision `7029ae5dca6d12332ddedf27c355b9fa91b9af9a`.
The updated acquisition tests and Linux Clippy also pass on paneer. Jackstay's
full suite subsequently passed after the old CPU daemon path was removed and
replaced with common session acquisition in `3fabbf1`. That synthetic coverage
does not replace the live native acceptance described below.

The ignored adapter smoke test
`sck_iosurface_stays_immutable_through_xpc_acquisition_and_delayed_release` captures
a real window, acquires through XPC, waits for readiness and checks leased pixels
before and after a delay. `PORTHOLE_SMOKE_FRAMES` and `PORTHOLE_SMOKE_HOLD_MS`
control its duration. It then closes the consumer and waits for producer drainage.
This fixed-size fixture does not prove host resize, cross-process viewer behavior
or long playback until those acceptance runs have actually been performed.
