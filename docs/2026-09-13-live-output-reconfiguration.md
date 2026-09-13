# Live capture output reconfiguration acceptance

Porthole `03594a1` was installed and restarted on 2026-09-13 under the existing
`work.flotilla.porthole.dev` identity, signed by Apple Development: Robert Wittams
(DYYMCPD885). Accessibility and Screen Recording remained granted. Candidate and
installed daemon SHA-256 matched:
`26d830dac849d501a0dd36995c4c8619b1d2bf7235e5a4179fefd56d1104db7c`.
Jackstay remained pinned to `f482a5ca1c0c0a3831d3b4774dc8b7110c4581eb`, C ABI 0.5.

A dedicated TextEdit document (CG window 55551) supplied actual ScreenCaptureKit
CPU and native frames. Its geometry stayed at 673×439 logical points, scale 2,
at position 338,132. Before/after screenshots confirmed the unchanged geometry
and document content. Neither synthetic capture nor SDL's dummy driver was used.

The same probe that failed the generation-change assertion during the earlier
[window-resize investigation](2026-09-13-capture-output-sizing.md) now passed.
It acquired CPU and native frames before each output request, copied the held
CPU bytes and sampled the held IOSurface through Metal after producer readiness.
After new output was published, it acquired new frames and compared the old
frames' complete descriptors and pixels again. The old leases remained held
throughout each transition.

| Round | Output before → after | CPU generation | Native generation |
| --- | --- | --- | --- |
| 1 | 1346×878 → 1700×1120 | 2 → 3 | 1 → 2 |
| 2 | 1700×1120 → 1346×878 | 3 → 4 | 2 → 3 |

All four owner-authenticated `POST /capture-sessions/{id}/output` requests
returned `202`. Published session dimensions subsequently matched each request.
Both rounds verified unchanged held descriptors, unchanged held CPU bytes and
unchanged held GPU pixels. The probe exited successfully.

CPU and native reference viewers ran alongside the probe, each with a two-frame
holding reservation and 250 ms holds. Each completed 300 frames with empty
stderr: CPU in 86.53 s, native in 81.47 s. The probe also reserved two frames per
path; each producer therefore admitted four consumer holds in total during the
transitions. Native publication recorded two drops during the requests; these
were visible in session telemetry and did not terminate acquisition.

After the viewers exited, the installed CLI successfully requested the current
size on both sessions. Both sessions then reached `closed`. Fresh CPU/native
sessions admitted new viewer processes, which completed eight frames each with
empty stderr. Those sessions also reached `closed`. The dedicated test document
was closed and the test identity revoked; the preexisting TextEdit document was
not modified or closed by this test.

This supplies the previously missing live output-replacement evidence, alongside
the [long playback and delayed Simulator runs](2026-09-12-live-acquisition-acceptance.md).
It does not establish recovery after submitted GPU work outlives a crashed
consumer. Jackstay's documented visible quarantine remains that case's supported
outcome. Process-wide graceful daemon drainage and viewer aspect-ratio handling
remain separate work.

The workspace build, tests, all-target Clippy with warnings denied and pinned
nightly formatting checks passed on macOS and Linux for `03594a1`. Linux validates
that the new optional control preserves its existing build and tests; Linux
output-size control is explicitly unsupported in this slice.

Local evidence: `/tmp/porthole-live-output-k1bmnjiq/`, including `probe.log`,
`probe-results.json`, `output-requests.json`, `resize-results.json`,
`original-cleanup-results.json`, `smoke-results.json`, `cleanup-results.json`,
and the before/after screenshots. The reused probe source is in
`/tmp/porthole-live-resize-wqbxm8zv/probe/src/main.rs`. These scratch directories
also contain private test credentials and attach capabilities; do not publish
whole directories. Gate logs are `/tmp/porthole-output-{build,tests,clippy,fmt}.log`
on each host.
