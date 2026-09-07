# Porthole Roadmap

Living document. The active milestones below govern current work. The original phases remain as a delivery record; their numbering no longer implies the next task. Jackstay extraction and desktop workflow verification can interleave, subject to the explicit dependencies below.

Last revised: 2026-09-07. Decisions: [ADR-0010](adr/0010-jackstay-extraction-and-desktop-workflow-milestones.md).

---

## Current state

What has shipped on GitHub `main` (through PR #110; a checkout at #109 predates the Windows CI job):

- macOS adapter for launch (process + artifact), input (key/text/click/scroll), wait (Stable / Dirty / Exists / Gone / TitleMatches), screenshot, focus, close, attention, displays, search, attach, replace, placement, snapshot_geometry.
- Shared local HTTP transport between `portholed` and `porthole`: Unix Domain Sockets on macOS/Linux and named pipes on Windows (#109). Windows still uses the in-memory desktop adapter; real Windows desktop operations remain open work.
- System-permissions slice: `porthole onboard` flow, `/info` permission status, `/system-permissions/request` route, capability-aware error mapping (`SystemPermissionNeeded`, `SystemPermissionRequestFailed`).
- macOS bundle install/uninstall and helper onboarding. The helper registers `portholed` as its own launchd agent so the daemon owns its native capture attach MachService (#98).
- Dev bundle script (`scripts/dev-bundle.sh`) producing an Apple Development signed `.app` with `portholed` and `porthole` in one bundle for stable TCC identity across rebuilds; ad-hoc signing is a hard failure because it invalidates TCC grants on rebuild.
- macOS recording command (`porthole record surface ... --duration ... --output ...`) built on ordered capture-transfer cursors and AVFoundation `.mov` writing.
- Agent-permissions enforcement foundation: daemon-owned identities/tokens/grants/denials/pending requests/audit store, `/events` policy publication, default-deny drive-route guard for input and pointer movement, and `porthole agents ...` operator commands.
- KWin compositor, input and screenshot foundation (#79); Linux native PipeWire/dmabuf transport and lease-handback work (#100, #107). Live checks still require a real KWin session and capture consent.
- Native macOS IOSurface/Metal producer and reference viewer, explicit synchronization, Linux native C ABI and ABI version guardrails. Jackstay now has a public standalone 0.1.0 repository; native macOS/KWin capture is verified and the pinned integration is awaiting CI (#114).
- Required repository checks: `cargo build --workspace --locked`, `cargo test --workspace --locked`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, and `cargo +nightly-2026-03-12 fmt --check`. GitHub also has Linux gates and Windows compile/portable-logic coverage (#110); broader Windows tests remain #111.

What's known missing or rough:

- General agent authority and native notification approvals are deferred under [ADR-0006](adr/0006-agent-permission-authority-deferred.md). The existing local-trust mint/grant path supports development and launch-time provisioning; it is not a separate agent/operator security boundary.
- macOS launch correlation now uses the native launch result's PID in the #89 implementation, with live launch/input/screenshot checks passing; merge is pending. [Validation and limits](2026-09-07-macos-launch-correlation.md). Brokered and descendant processes remain #10. Deterministic signing (#95) and false native attach readiness (#97) have separate open fixes.
- KWin unattended capture remains #108; older portal-start blocker #78 needs revalidation against the chosen installed-app path. Native performance measurement (#86) and GPU-error handling (#92) remain open.
- Agent-permission enforcement covers drive routes plus most observe/manage/record HTTP routes. Protected surface capture-session fd-socket consumption requires a bearer-token handshake on the capture-transfer channel; synthetic capture sessions remain public because they do not touch the real desktop.
- Recording live-smoke remains permission-blocked for freshly built daemon identities unless the installed bundle has Accessibility and Screen Recording grants.

---

## Active milestones

### A. Extract and consume Jackstay 0.x

**Outcome:** an independent library and reference viewer, consumed by porthole
through a pinned Git dependency, with the existing macOS and Linux paths
verified. API stability, Windows capture and direct Katzensteg integration are
not completion requirements.

- [x] Extract to `~/dev/jackstay` and public `flotilla-org/jackstay`, preserving
  relevant history. Keep the Rust implementation, C ABI, version checks and
  reusable PipeWire mechanism. Demonstrate synthetic producer → standalone viewer
  without a porthole checkout or daemon. [#113](https://github.com/flotilla-org/porthole/issues/113).
- [ ] Switch porthole to a pinned Jackstay revision, verify dependency
  access for builds/CI, and remove the duplicated transport implementation.
  Verify native desktop capture → extracted viewer on macOS and KWin, plus the
  existing screenshot/recording paths. [#114](https://github.com/flotilla-org/porthole/issues/114); depends on the
  standalone extraction.

Jackstay owns mechanism: transport, handles, synchronization and buffer lifetime.
Its host obtains capture authority, selects sources and supervises desktop
sessions. Keep PipeWire's already-authorized-connection handoff; another compositor
or desktop environment may supply that connection. This boundary is provisional
and can change when a real consumer exposes a problem. Preserve existing behavior
through extraction; #86 remains the separate measured-performance acceptance.

### B. Verify the desktop agent workflow on each platform

**Outcome:** in an already logged-in GUI session, porthole starts automatically,
launches a terminal hosting cleat plus a coding agent with its porthole token
provisioned, and the operator can `cleat attach`. The agent uses target-local
porthole to launch an app, send input and capture a screenshot showing the result.
Detach/reattach preserves the agent. Clean up only the verification run's resources
and record the build revisions, commands and screenshot evidence.

- [ ] macOS regression verification and demonstrated drift fixes, using a separate
  client terminal on kiwi. [#115](https://github.com/flotilla-org/porthole/issues/115); #89 blocks the full launch
  proof. Retain the installed bundle, launchd ownership and real OS grants.
  [Comte verification](2026-09-07-comte-desktop-workflow.md) now demonstrates the
  supervised flow from kiwi; pre-granting access to future windows remains open.
- [ ] KWin regression verification and demonstrated drift fixes, attaching from
  kiwi to cleat in the target's GUI session. [#116](https://github.com/flotilla-org/porthole/issues/116). Establish
  the current need for #108/#78 from the tested screenshot path; do not assume a
  continuous-capture portal issue blocks every desktop operation.
- [ ] Windows real local launch, input and screenshot operations on gouda through
  the daemon and CLI. [#117](https://github.com/flotilla-org/porthole/issues/117). A named-pipe `/info` response or
  an in-memory adapter does not satisfy this milestone.
- [ ] Windows startup within the logged-in session, porthole-launched cleat/agent,
  pre-provisioned token and kiwi → gouda terminal attachment. Demonstrate the full
  desktop workflow and detach/reattach. [#118](https://github.com/flotilla-org/porthole/issues/118); depends on the
  real Windows desktop operations. Beaufort is a later physical GPU target.

Startup means starting porthole within the existing desktop login. Creating a
login or automatically logging in after reboot is out of scope. `cleat attach`
connects to the agent's terminal; it does not provide a remote desktop video view.
Cleat owns Pools and terminal session lifetime. Keep agent authority under
ADR-0006; no new approval UI or daemon authorization bypass is required.

### Existing work and later directions

Use the existing issues for #89/#10 launch correlation, #95 signing, #97 native
readiness, #92 GPU errors, #86 measurement, #108/#78 KWin capture, #111 Windows
coverage and #112 sleep inhibition. The new milestones link to these rather than
repeat them. Broader capture lifecycle follow-ups (#19, #20, #28, #31, #33) need
reconciliation against the current implementation before treating them as new
work. No existing issue is closed merely because the roadmap changed.

Windows continuous capture/DXGI, browser CDP, additional compositors, presentation
hierarchy integration and direct Katzensteg integration remain later work. The
existing SDL interception of the simple viewer is sufficient for this extraction.

A future network bridge can consume a local Jackstay stream and publish a new
local stream at the destination after encoding/transmission/decoding. Leave the
protocol open. Tender may provide integrated host identity and connectivity later;
there is no Tender dependency or newly specified Tender requirement now. Record
concrete requirements only if implementation reveals them. General streaming,
coordinator/translator machinery and API stability remain outside these milestones.

---

## Phase 0 — kitty-harness unblocker

**Goal:** Unblock the kitty-graphics-protocol conformance test harness use case. End of this phase, an agent can install porthole, drive a real terminal end-to-end, and the resize gap is closed.

- [x] `POST /surfaces/{id}/place` route + handler + InMemoryAdapter e2e test (adapter method already exists at `crates/porthole-adapter-macos/src/placement.rs:9`)
- [x] CLI `porthole place <surface_id> --x --y --w --h` subcommand
- [x] `docs/recipes/terminal-orchestration.md` — agent-facing walkthrough: launch → focus → wait-stable → text/key → screenshot → scrollback → resize → close. Notes the inner-script ↔ harness UDS pattern as out-of-scope (not porthole's job).
- [x] `scripts/manual-terminal-smoke.sh` — runnable shell script exercising launch / focus / text / key / wait-stable / screenshot / scrollback / close on Ghostty (or any installed terminal).
- [x] README **Install** section documenting `cargo install --git ... porthole --locked` for the CLI, with the explicit caveat that the daemon needs the `.app` bundle to satisfy TCC.
- [x] `dev-bundle.sh`: rename output from `Portholed.app` to `Porthole.app`, copy the `porthole` CLI into `Contents/MacOS/` alongside `portholed` so the CLI shares the daemon's TCC identity.

---

## Phase 1 — installable, ambient daemon

**Goal:** No more "did you remember to start the daemon?". Single canonical install location, daemon auto-starts on login, CLI on PATH.

- [x] `porthole install` subcommand:
    - [x] Copy bundle to `/Applications/Porthole.app` (fall back to `~/Applications/Porthole.app` with `--user`).
    - [x] Symlink `~/.local/bin/porthole` → bundle's CLI.
    - [x] Detect whether `~/.local/bin` is on `PATH`; print a copy-pasteable export line if missing. (No auto-edit of dotfiles — too intrusive; the print-and-let-the-user-paste shape was a deliberate scope decision.)
    - [x] Drop `~/Library/LaunchAgents/work.flotilla.porthole.plist` (`RunAtLoad=true`, `KeepAlive(Crashed=true)`, `LimitLoadToSessionType=Aqua`, `Program` pointing at `Porthole.app/Contents/MacOS/portholed`, stdout/stderr to `~/Library/Logs/porthole/portholed.log`).
    - [x] `launchctl bootstrap gui/$UID <plist>`. Idempotent: bootouts any prior load before writing.
- [x] `porthole uninstall` subcommand: reverse of the above. `--keep-bundle` to leave the `.app` for the user to manage manually.
- [x] Recommended sequence documented in README: install bundle → `porthole onboard` → `porthole install`. Order matters because TCC dialogs need an active user; auto-start before grants exist queues prompts the user has no context for.
- [x] Optional: `porthole status` — daemon up/down, socket path, version, surface count.

---

## Phase 2 — agent-permissions design spec

**Goal:** Pin down the API and UX for agent-X-asks-permission-to-drive-window-Y *before* the helper app is built, so the helper isn't rewritten when the model crystallises. No code in this phase.

- [x] `docs/superpowers/specs/2026-05-17-porthole-agent-permissions-design.md` covering:
    - Identity: how an agent identifies itself (token? tag? bundle path?).
    - Scope: per-surface, per-app, per-action-class, time-bounded?
    - Approval flow: who prompts, who decides, where decisions are persisted.
    - Wire shape: new endpoints, new event types on `/events`, new error codes.
    - Relationship to system-permissions: what's separate, what overlaps.
    - Default-deny vs default-allow tradeoffs.

## Phase 2.5 — agent-permissions enforcement foundation

**Goal:** Ship the daemon-owned policy core and first protected vertical slice before helper notification UI consumes it.

- [x] Pure Rust policy model for agent identities, target selectors, action classes, durations, constraints, grants, denials, and authorization decisions.
- [x] SQLite policy store for identities, token hashes, grants, denials, pending requests, and execution audit rows.
- [x] Daemon identity/request/grant endpoints and `/events` publication for requested/resolved/policy-changed state.
- [x] Default-deny route guard for the first `drive` vertical slice: key, text, click, scroll, and pointer movement.
- [x] CLI operator commands: `porthole agents create/list/show/revoke`, token mint/revoke, pending request list/show/approve/deny, grant list/revoke.
- [x] Map remaining HTTP route classes: `observe`, `manage`, and surface-session `record` creation.
- [x] Add bearer-auth handshaking to raw capture-transfer fd-socket consumption, or replace it with a daemon-mediated read path.
- Deferred under ADR-0006: helper/private operator authority for identity and policy mutation. The current CLI operator path relies on the local-user trust boundary.

---

## Phase 3 — platform UI apps, starting with macOS `Porthole.app`

**Goal:** Native UX for the parts of porthole that benefit from being native. macOS ships one `Porthole.app` bundle with `PortholeHelper` as its main executable. The original child-process design below was superseded by #98: the helper now registers a separate launchd job for `portholed` and its attach MachService. Future platform UI apps use native shell conventions.

- [x] `docs/superpowers/specs/2026-05-17-platform-ui-apps-bundle-design.md` — platform UI app contract and macOS bundle build architecture.
- [x] `docs/superpowers/specs/2026-05-17-macos-onboarding-ui-design.md` — native onboarding UI contract for system-permission state, Settings deep links, prompt requests, and daemon restart verification.
- [x] Repo-native bundle builder (`cargo xtask bundle --platform macos`) that assembles `target/<profile>/Porthole.app` from checked-in macOS bundle metadata and Rust binaries.
- [x] Swift / SwiftUI macOS helper under `apps/macos/PortholeHelper/`; build output copies `PortholeHelper`, `portholed`, and `porthole` into `Contents/MacOS/`.
- [x] `NSStatusItem` with monochrome glyph + optional badge (surface count, "broken" state).
- [x] Original helper child-process startup and restart. Superseded by #98: the helper registers the bundled daemon LaunchAgent with `SMAppService.agent`.
- [x] Onboard UI flow — native equivalent of `porthole onboard`. Pulls grant state from `/info`, deep-links to System Settings panes via `x-apple.systempreferences:` URLs, "re-arm prompt" actions POST to `/system-permissions/request`.
- Deferred under ADR-0006: notification surface for agent-permission approvals, pending the general authority-model decision.
- [x] `SMAppService.mainApp` registration so the user gets a System Settings → General → Login Items entry for the per-user helper app. Subsumes phase 1's CLI-installed LaunchAgent for users who have the helper.
- [x] Migration: helper's first launch detects and removes any phase-1 LaunchAgent plist at `~/Library/LaunchAgents/work.flotilla.porthole.plist` (and `launchctl bootout`s it) before registering its own, so the user doesn't end up with two start mechanisms competing.
- [x] Helper passively re-probes an externally running daemon so the `runningExternal` status recovers if that daemon exits after helper launch.
- [x] Quit / Restart daemon menu items.

---

## Phase 4 — v0.1 product slices (parallel)

**Goal:** Expand what porthole *does*. These are independent of the platform/UX track and can interleave freely. Pick one at a time; each gets its own design spec under `docs/superpowers/specs/`.

Historical product slices; the active milestones above govern current ordering:

- [x] **Recording on macOS** — AVFoundation or ScreenCaptureKit. Biggest user-visible feature gap.
- [x] **Multi-display placement verbs** — extends the phase-0 `/place` route (which takes explicit geometry) with anchor-based placement (e.g. `anchor: focused_display`, display id targeting).
- [ ] **Browser tabs via CDP** — Chrome / Edge tab coverage that AX can't reach. Expanded tab verb set (input, wait, replace) and content-area screenshot crop.
- [x] **`force_place: true` launch option** — placement on preexisting surfaces.
- [x] **KWin foundation (Linux)** — compositor, input and screenshot support landed in #79. Native capture followed; unattended capture and full workflow verification remain open in the active milestones.

Later directions: Hyprland, X11, overlay/annotation, MCP, remote multi-machine presentation and record/replay integration. Windows desktop operations are now an active milestone; Windows continuous capture remains deferred.

---

## Bundle architecture

Current helper bundle and daemon job (#98):

```
/Applications/Porthole.app/
  Contents/
    Info.plist            # CFBundleIdentifier = work.flotilla.porthole
    MacOS/
      PortholeHelper      # SwiftUI menu-bar app (CFBundleExecutable)
      portholed           # daemon, its own launchd job
      porthole            # bundled CLI
    Library/LaunchAgents/
      work.flotilla.porthole.daemon.plist  # daemon job + attach MachService
~/.local/bin/porthole -> /Applications/Porthole.app/Contents/MacOS/porthole
```

The helper registers its login item and the bundled daemon agent separately.
The daemon owns `work.flotilla.porthole.attach`; native consumers connect to that
MachService. Keep the bundle's signing identity and installation path stable to
preserve OS grants, and verify permissions after installation or rebuilds.

The phase 0–2 daemon-only bundle and the original phase-3 child-process startup
are historical layouts, not the target for new session-bootstrap work.

---

## Amendment policy

This doc records active milestones and their dependencies. Update it when implementation or an explicit planning decision changes the plan. Preserve completed phase checklists as the delivery history; mark superseded or deferred items explicitly. ADR-0010 and this revision record the roadmap discussion accepted on 2026-09-05.
