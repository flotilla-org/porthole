# Jackstay's transport is native-handle-first, not CPU-baseline

Jackstay (today the in-repo `capture-transfer` crate) exists to move captured
surfaces to heterogeneous consumers at the lowest possible overhead. The primary
transport is therefore the **native handle path**: the producer passes an
OS-native GPU surface handle (macOS `IOSurface`, Linux dmabuf, Windows D3D-shared)
plus an explicit sync primitive, and a capable local consumer presents it
zero-copy. **Pixel streaming is the fallback** — for consumers that cannot
receive a handle (ssh, tmux, terminals without the handle-passing extension, the
conformance harness) — not the baseline.

This supersedes the phase framing in
`docs/superpowers/specs/2026-05-13-capture-transfer-architecture-refinement-design.md`,
which called CPU shm the "stable baseline" and native `IOSurface` an
"experiment". That ordering makes pixel streaming the product and native an
optional upgrade. The whole point of jackstay is the opposite: if it doesn't get
to native zero-copy, there is no reason for it to exist over existing pixel
pipelines.

## Consequences

- **The explicit sync primitive ships in v1.** Each ring message carries both a
  surface reference and a fence (`MTLSharedEvent` + value on macOS; drm_syncobj /
  timeline on Linux; `ID3D12Fence` on Windows). Publishing a surface id with no
  accompanying fence is the "implicit GPU sync" anti-pattern — it appears to work
  and tears under load. It is not a later addition.
- **"Done" for the macOS path is the full chain, end to end:** SCK frame →
  `IOSurface` extracted → handle + `MTLSharedEvent` + value transferred once at
  attach over the setup socket → surface pool with OS-refcount as source of truth
  → a real consumer latches the surface, waits on the fence, presents zero-copy →
  overhead measured against the CPU path. A producer that merely publishes
  `IOSurfaceID`s into the ring with nothing latching them has proven nothing.
- CPU shm remains, but as the documented fallback class, exercised by the same
  producer code with different consumer behaviour.
