# Jackstay is a general app-to-app transport; topology stays non-precluded

Current milestone: [ADR-0010](0010-jackstay-extraction-and-desktop-workflow-milestones.md)
permits breaking 0.x changes and separates extraction from stability. The general
shapes below remain design intent; they are not a current ABI-freeze requirement.

Jackstay's reason to exist is not surface capture specifically — it is the
**lowest-cost cross-platform transport between apps for any stream**. Video was
the *hardest* case and was done first (zero-copy GPU handle + explicit fence, see
ADR-0004); it is not the whole scope. Audio (clock-synchronised with video),
input (flowing in the *reverse* direction, consumer→producer), and arbitrary byte
buffers (e.g. a pty stream) are all in scope, and any stream may eventually be
bidirectional (microphone, control). "Frame" is really "buffer
`{payload_kind, timestamp on a shared clock, handle-or-bytes}`".

The governing design rule for everything below: **design so the full vision is
never precluded, without implementing every topology now.** Simple, directly
described cases work first; the *shapes* (descriptors, roles, the C ABI) must not
bake in assumptions that a later negotiated/translated/remoted/multi-modal graph
would have to fight. This is explicitly *not* a request to build the general
machinery now — it is a constraint on the v1 shapes so the general machinery
remains addable without an ABI break.

## Scope of the transport

- **Three payload tiers, all in scope:** raw bytes / shared-memory streams; OS
  handles (fds, mach ports, NT handles); and GPU surfaces + sync fences. The
  handle tier exists to enable the GPU tier (ADR-0004); the byte tier carries
  arbitrary formats (pty, PCM audio, …).
- **Multi-modal sync is a shared-clock concern, already in the wire.**
  `jackstay_frame_slot` carries `timestamp_ns` "in the config's `clock_domain`",
  and `jackstay_stream_config` carries `clock_domain`, `sync_kind`,
  `pixel_format`, `color_space`, and dmabuf `modifier`. So synchronised audio and
  format/colorspace generality are *not* later wire breaks — the ring was cut for
  them. Producers on related streams must stamp the same clock domain so
  consumers can align A/V/input.
- **Roles are symmetric.** A consumer can re-export as a producer. Reverse-
  direction streams (input) and full bidirectionality are the *same* mechanism,
  not a new one — and this is precisely what lets a translator be "just a peer".

## Coordinator is a role; porthole is one instance

- **"Coordinator" is a jackstay concept**, not a porthole feature. porthole is
  one coordinator *instance* that holds the required desktop permissions and
  delegation capability. The current macOS desktop capture integration stays in
  porthole with its installed identity and launchd broker (ADR-0007). Another host
  can supply its own authorized source; Jackstay does not require porthole as its
  authority. Reusable capture mechanisms such as PipeWire may live in Jackstay
  while the host obtains permission and supervises the session (ADR-0010).
- **Broker-less floor: trust is environmental.** In the component case — a
  consumer forks a producer and they share inherited fds — naming, auth, and
  trust are entirely OS-provided (permissions, namespaces). Jackstay's *core* adds
  nothing mandatory here; names/tokens are a coordinator-supplied layer on top
  (consistent with ADR-0006: authority lives outside, porthole only verifies).
  This is already the right shape: `AttachEndpoint::new` takes
  `expected_bearer: Option<String>` — auth is optional today. Keep it optional.
  ADR-0010 now requires an independent synthetic producer/viewer example using
  the existing setup mechanisms. The porthole macOS desktop path keeps its broker;
  a general coordinator or topology framework remains outside the milestone.

## Setup is capability + expectation declaration (deferred, not precluded)

- Each side declares its **capabilities** and possibly an **end that does not
  exist yet** (one side may be absent at first). The library and/or coordinator
  then instantiates *enough* — including **translator processes** — to satisfy
  the declared expectation, choosing the **cheapest mutually-viable path** and
  never imposing more cost than the producer/consumer constraints require.
- **Translators are just peers** (consume one side, produce the other; no special
  API). **Remoting is the translator that serializes** because handles can't
  cross the wire. The coordinator (porthole) knows how to *instantiate and run*
  translator processes; the fuller jackstay likely owns the translation-pipeline
  machinery (kin to the translation bits in katzensteg).
- **Explicitly not designed now:** the full negotiation/topology resolution and
  the translation-pipeline machinery. v1 may resolve only the both-ends-present,
  directly-described case (plus brokered introduction per ADR-0007). The
  descriptor shape simply must not assume that is all there will ever be.

## Non-preclusion invariants (the expensive-to-reverse shapes)

Once katzensteg (Zig), the libghostty-vt fork (Zig), cleat, and `capture-viewer-sdl`
(C) link the C ABI (ADR-0005), these are costly to change, so get them right in v1:

1. **Every buffer carries `{payload_kind, shared-clock timestamp, handle-or-bytes}`** —
   already true in the ring; it must also be true at the public consumer C ABI.
2. **Producer/consumer roles are symmetric** — a consumer can re-export as a
   producer, so translators need no special API and reverse/bidirectional streams
   are not a separate mechanism.
3. **No mandatory auth/naming in the core** — a coordinator-supplied layer only
   (`expected_bearer: Option`); consistent with ADR-0006 (jackstay core stays
   enforcement-free; authority/verification is a coordinator/platform-helper
   concern).
4. **Setup is a capability/expectation *descriptor*, not "connect to this concrete
   surface"** — so a coordinator can interpose translators later without either
   endpoint changing.
5. **Negotiation is a real exchange** (declare capabilities → pick cheapest
   viable), not a hardcoded path — even if v1's only branch is "same-platform GPU
   zero-copy, else fall back / fail".

## Where the work actually concentrates

- **The ring is already general; the load-bearing freeze is the public consumer
  C ABI** (`capture_transfer.h` / `ffi_native.rs`). It must carry **handle-type
  discrimination** — IOSurface vs multi-plane dmabuf + `modifier` vs D3D shared
  handle — plus the fence, because the consumers above all import handles into
  *their* renderers across two GPU APIs.
- **One macOS-ism remains in the producer trait:** reuse gating via
  `IOSurfaceGetUseCount` (`NativeFrameBackend::surface_use_count`) has no dmabuf
  equivalent; a Linux backend will re-cut that contract.
- **Validate against a second real backend before freezing the ABI.** ADR-0005
  says extract only after the macOS native path is proven; this adds: also stress
  the *consumer* ABI with a second platform first. The right stressor is **Linux
  (PipeWire/dmabuf + drm_syncobj), producer and consumer reference both** —
  maximally different from macOS (multi-plane + modifiers + no use-count +
  fd-passing transport). **Windows is shape-compatible** with macOS (single
  shareable NT handle + counter fence) and validates little, so it comes later.

## Sequencing: extract at 0.x; stability comes later

Updated 2026-09-05 by [ADR-0010](0010-jackstay-extraction-and-desktop-workflow-milestones.md).
The July amendment deferred only the 1.0 stamp, but left contradictory statements
that tied extraction to freezing the ABI. Those statements are superseded.

Extract Jackstay as a usable 0.x dependency in its own private repository.
Porthole consumes a pinned revision. The standalone synthetic producer/viewer
proof and real macOS/Linux native integration checks establish the extraction's
completion; no Windows capture implementation or API stability promise is needed.
The C ABI stays versioned and may change during 0.x development.

Linux has already exercised the handle, modifier and lease contracts that macOS
alone could not validate. Preserve that implementation and verify the real
PipeWire producer and native consumer across the new repository boundary.
This is regression coverage, not an instruction to implement Linux again or
freeze the ABI before moving it.

Windows desktop launch/input/screenshots proceed independently. Windows native
capture can reshape the transport contracts later. The existing C header checks
and cross-language guards remain regression checks; direct Katzensteg integration
is not an extraction gate.

Audio, input streams, translation and network streaming remain later additions
when a concrete consumer needs them. Tender introduces no dependency or new
requirements for this extraction. API stability and public distribution remain
separate future decisions.

## Relationship to other ADRs

Extends ADR-0004 (handle-first + fence in v1), ADR-0005 (one library / stable C
ABI / extract-after-proven / descriptor general from day one), and ADR-0007
(coordinator/broker split; hot path bypasses the broker; direct no-coordinator
rings out of scope for v1). Invariant #3 is consistent with ADR-0006 (authority
is a coordinator/platform-helper concern; jackstay core stays enforcement-free).
Consistent with ADR-0002 / ADR-0003 (name wire types after concepts, not one
OS's vocabulary).
