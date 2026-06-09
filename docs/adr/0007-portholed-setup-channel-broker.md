# portholed is the jackstay setup-channel broker; the hot path bypasses it

The jackstay hot path — frames — is shared memory, producer to consumer, with no
daemon in the middle. But the **setup channel** (capability negotiation,
handle/fd transfer, and producer↔consumer introduction) is brokered by
`portholed`.

On macOS this is forced by the OS, not chosen. The sync primitive
(`MTLSharedEventHandle`) and even an `NSXPCListenerEndpoint` refuse plain
serialization — "this object may only be encoded by an NSXPCCoder" — so handles
move only over a live XPC connection, never as bytes on a socket or file. And a
bootstrap mach-service *name* can only be registered by a launchd-managed
process. `portholed` is launchd-managed, so it owns a `MachServices` name and is
the one discoverable endpoint. Ad-hoc producers (e.g. an injected at-source
capture runtime) and consumers (e.g. a terminal) cannot register names; they
create anonymous XPC listeners and rendezvous *through* `portholed`, which
shuttles endpoints (which a live XPC connection *can* carry) so the two can then
connect directly. IOSurfaces and the shared-event handle cross that direct
connection; frames then flow over shared memory.

The compositor-capture case is the degenerate one: `portholed` is itself the
producer and the named service, so no third-party introduction is needed —
consumers are plain XPC clients that look the name up.

This was established empirically by the #81 spike on macOS 26 / Apple M4.

## Consequences

- `portholed`'s macOS bundle plist gains a `MachServices` entry. The setup
  channel on macOS is XPC, distinct from the plain UDS + SCM_RIGHTS path the CPU
  pixel-streaming fallback uses.
- IOSurfaces are transferred as objects over the XPC attach connection (or by
  mach port), **not** looked up by global `IOSurfaceID` — global-ID lookup fails
  across the launchd ↔ user-session boundary, contra the jackstay-design-checklist
  §7 "IOSurfaceID is globally meaningful" note. Per frame the ring carries only a
  pool slot index plus a monotonic `fence_value`.
- This makes `portholed` the natural control plane for later setup-channel
  concerns the hot path must not own: interposers (producer→consumer adapters),
  network stream send/receive, and virtual display/audio device management.
- Direct producer↔consumer rings with no coordinator remain possible but are
  explicitly out of scope for v1; the broker is the supported introduction path.
- The handshake *protocol* (what crosses at attach, ordering, auth) stays
  platform-neutral behind the `NativeFrameBackend` transport seam; only the
  transport (XPC vs UDS) is platform-specific.
