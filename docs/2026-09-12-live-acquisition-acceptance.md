# Live acquisition acceptance

On 2026-09-12, the signed Porthole bundle at commit `67fa206` was installed and
restarted with Robert's approval. The running daemon uses Jackstay
`f482a5ca1c0c0a3831d3b4774dc8b7110c4581eb`, C ABI 0.5. The installed daemon's SHA-256
matches the verified candidate. Accessibility and Screen Recording remained
granted under `work.flotilla.porthole.dev` with the existing Apple Development
identity.

The source was the existing Simulator / iPhone 17 Pro window, CG window 52608,
at 456×972 pixels. Capture used the installed daemon's ScreenCaptureKit CPU and
native paths. The SDL viewer was built from the exact Cargo-resolved Jackstay
revision. All four viewers ran concurrently, with two admitted consumers per
capture session. These runs did not use synthetic producers or SDL's dummy driver.

| Viewer | Frames completed | Hold before consumption | Observed runtime |
| --- | ---: | ---: | ---: |
| CPU normal | 10,000 | 0 ms | 178.59 s |
| Native normal | 10,000 | 0 ms | 179.60 s |
| CPU delayed | 800 | 250 ms | 231.16 s |
| Native delayed | 800 | 250 ms | 217.02 s |

Every viewer exited successfully with empty stderr. The delayed CPU viewer
compared each frame's bytes before and after its hold and reported no mutation.
The native viewer retained frames through Metal completion. Screenshots of both
viewers showed Simulator content with the clock advanced from the initial source
screenshot. The native reference viewer stretches the source into its default
window dimensions; these checks do not establish correct aspect-ratio handling.

After the viewers exited, explicit close reached `closed` in 0.055 s for CPU and
0.170 s for native, as observed by the polling harness. Final publication counts
were 18,932 with zero drops for CPU and 18,879 with one drop for native. Fresh
CPU and native sessions then started, and new viewer processes completed eight
frames each. This verifies successful session retirement and reuse of the named
native service reservation for this run.

## Remaining resize evidence

Porthole's attempt to resize Simulator from 456×972 to 380×810 returned HTTP 501,
`capability_missing`, with `AX refused position/size write: pos=0 size=-25200`.
The restore request returned the same error. Capture dimensions remained 456×972;
Accessibility was still granted. This is not a successful live reconfiguration
check. Robert has been asked to resize the window manually; the fresh capture
sessions remain available for that check.

Budgeted replacement, pending GPU work and crash quarantine have separate
fixture coverage in Jackstay. They do not substitute for live host resize.
Neither these successful closures nor the daemon restart establish reclamation
after a GPU consumer crash. The documented quarantine/recovery limitation and
the absence of a process-wide graceful drain API remain unchanged.

## Local evidence

Artifacts are in `/tmp/porthole-live-acquisition-qrl79wnm/`:

- `manifest.json`: source, session IDs, revisions and daemon hashes.
- `acceptance-results.json`: exit status, frame counts, elapsed time and stderr.
- `session-restart-results.json`: final retirement status of the first sessions.
- `smoke-results.json`: successful viewer admission into replacement sessions.
- `resize-results.json`: unchanged capture dimensions after rejected AX writes.
- `simulator-before.png`, `cpu-viewer.png`, `native-viewer.png`: observed content.

The directory also contains private test credentials and native attach tokens;
do not publish it wholesale. The tracked report contains no bearer tokens.
