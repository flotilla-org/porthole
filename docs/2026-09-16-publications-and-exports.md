# Publications, exports and the cross-host attach

Evidence note, 2026-09-16. Porthole on the `feat/shared-native-acquisition`
working tree plus the uncommitted publications work, jackstay on the
`bridge-slice-one` working tree. Design and measurements in the project-map
report `reports/jackstay-remoting-research-2026-09-16.md`; the bridge itself in
jackstay `docs/design/bridge.md`.

## What was added

portholed is the graph manager for cross-host bridges. A *publication* is a
running frame stream with a local endpoint: every capture session is one, and a
*republication* is the local end of a bridge. An *export* spawns the
producer-side bridge half (`jackstay-bridge egress --listen`) for a native
capture session; it attaches to portholed's own attach service with the
session's token and listens on two Unix sockets under the runtime directory.
A republication runs the consumer-side half (`jackstay-bridge ingress`) as a
launchd job with its own Mach service name, because only a launchd-registered
job can vend one, and reports it as a native publication viewers attach to
exactly as they attach to a local capture.

Routes, all requiring an authenticated agent: `GET /publications`,
`GET /publications/{id}`, `POST /publications/{id}/exports` (session owner
only), `GET` and `DELETE /publications/{id}/exports/{export_id}` (owner only),
`POST /publications/republish`, `DELETE /publications/{id}` for a
republication. Capture sessions now record the surface they capture, so a
publication carries two identities: the source (a surface id) and the running
publication (the session id), kept apart as the registry note asks.

CLI: `porthole publications list | export | export-status | export-close |
republish | close | attach`. `attach --host HOST` runs the whole sequence over
`ssh -N -L` with the AES-GCM cipher: forward the remote daemon's control
socket, search and track the surface, open a native capture, create the
export, forward its two sockets, republish locally, print the viewer command,
and with `--hold` tear everything down on interrupt. It waits while the remote
reports `agent_permission_needed`, so an operator on the remote can approve the
request from `porthole agents requests`. Tender will replace the SSH forwarding;
nothing in the daemon depends on how the sockets got there.

The bridge executable is located from `JACKSTAY_BRIDGE_BIN`, then as a sibling
of the daemon binary, then on the PATH. The bundle builder copies it in as that
sibling when `JACKSTAY_BRIDGE_BIN` is set at bundle time or a `jackstay-bridge`
sits in the target profile, so an installed daemon needs no environment: a
launchd-managed daemon only sees launchd's environment, and `launchctl setenv`
from an SSH session lands in the wrong domain.

## Evidence

Consumer side alone, kiwi (M4, macOS 26.6), a development daemon under
`PORTHOLE_RUNTIME_DIR=/tmp/pdev-kiwi`: `porthole publications republish` for a
local synthetic export spawned the launchd ingress, listed it as a
`republished` publication with the viewer command, the reference viewer
presented 45 frames, and `close` removed the job.

Both sides, comte (M4, macOS 26.5.1) to kiwi: with comte's installed daemon
running this tree and its own bridge binary, one command on kiwi,
`porthole publications attach --host comte --app-name Simulator --hold`, tracked
the booted iPhone 16 Pro simulator, opened a native capture session (456x972),
created an export, forwarded it, republished on kiwi and printed the viewer
command; the viewer presented 120 frames. `publications list` on comte showed
the capture with 175 frames published and no drops; on kiwi it showed the
republication. Interrupting the command closed the republication and the
export; no launchd job on kiwi and no egress process on comte remained, and
comte's capture session showed the usual soft close with native resources
retired.

## Findings on the way

- Unix socket paths are limited to 104 bytes on macOS. Export sockets under the
  per-user temp directory exceeded it, the egress failed to bind, and ssh
  refused the forward specification for the same reason. Export directories
  and socket names are now short (`x/<12 hex>/m` and `/c`), and the attach
  command uses short local names. comte's daemon was moved to
  `PORTHOLE_RUNTIME_DIR=/tmp/p501` for the run; `attach --remote-runtime-dir`
  exists for that case.
- Permission grants bind to a surface id, and a daemon restart renews the
  tracked surface's id, so each restart costs a new approval on the remote.
  The attach command waits for it rather than failing.
- The helper does not re-register the daemon agent when the background-items
  database says enabled but launchd has no job, which is what a `bootout`
  leaves behind. comte's daemon currently runs from a temporary absolute-path
  plist in `/tmp`; a logout and login should let the helper's registration
  take over. `/Applications/Porthole.app`, a May build, was renamed to
  `Porthole-old-2026-05.app` so Spotlight opens the right bundle.
- A permission tick in System Settings is only seen after a daemon restart.

## Not verified

- A republication's width and height are reported only once its half has
  ended; the publications view shows 0x0 while it runs.
- More than one export per publication, more than one consumer per
  republication, and a second concurrent republication on one host.
- Linux hosts on either side.
