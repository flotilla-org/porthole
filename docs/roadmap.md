# Porthole Roadmap

Living document. Each phase lists concrete deliverables as a checklist; PRs that land items tick the boxes. Phase order reflects dependencies — phase N+1 generally needs the design surface set by phase N — but phase 4 (product slices) is orthogonal to the platform/UX track and can interleave freely.

Last revised: 2026-04-26.

---

## Current state

What has shipped on `main`:

- macOS adapter for launch (process + artifact), input (key/text/click/scroll), wait (Stable / Dirty / Exists / Gone / TitleMatches), screenshot, focus, close, attention, displays, search, attach, replace, snapshot_geometry.
- HTTP-over-UDS daemon (`portholed`) and CLI (`porthole`) talking via Unix Domain Socket.
- System-permissions slice: `porthole onboard` flow, `/info` permission status, `/system-permissions/request` route, capability-aware error mapping (`SystemPermissionNeeded`, `SystemPermissionRequestFailed`).
- Dev bundle script (`scripts/dev-bundle.sh`) producing an ad-hoc-signed `.app` for stable TCC identity across rebuilds.
- CI: `cargo build --workspace --locked` / `cargo test --workspace --locked` / `cargo clippy --workspace --all-targets --locked -- -D warnings` / `cargo +nightly-2026-03-12 fmt --check`.

What's known missing or rough:

- No `POST /surfaces/{id}/place` route — the adapter method exists, but the only HTTP path to in-place resize goes through `/replace`, which destroys the surface.
- No agent-facing recipe / walkthrough doc for end-to-end terminal automation.
- No installable shipping path — users build from source and run the dev bundle manually. No `cargo install`-mediated install, no LaunchAgent, no auto-start.
- CLI binary lives outside the bundle; TCC identity is daemon-only.
- No native UI — onboard flow is CLI-only, no menu bar presence, no notification surface for future agent-permission approvals.

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
    - [x] Drop `~/Library/LaunchAgents/org.flotilla.porthole.plist` (`RunAtLoad=true`, `KeepAlive(Crashed=true)`, `LimitLoadToSessionType=Aqua`, `Program` pointing at `Porthole.app/Contents/MacOS/portholed`, stdout/stderr to `~/Library/Logs/porthole/portholed.log`).
    - [x] `launchctl bootstrap gui/$UID <plist>`. Idempotent: bootouts any prior load before writing.
- [x] `porthole uninstall` subcommand: reverse of the above. `--keep-bundle` to leave the `.app` for the user to manage manually.
- [x] Recommended sequence documented in README: install bundle → `porthole onboard` → `porthole install`. Order matters because TCC dialogs need an active user; auto-start before grants exist queues prompts the user has no context for.
- [ ] Optional: `porthole status` — daemon up/down, socket path, version, surface count.

---

## Phase 2 — agent-permissions design spec

**Goal:** Pin down the API and UX for agent-X-asks-permission-to-drive-window-Y *before* the helper app is built, so the helper isn't rewritten when the model crystallises. No code in this phase.

- [ ] `docs/superpowers/specs/YYYY-MM-DD-porthole-agent-permissions-design.md` covering:
    - Identity: how an agent identifies itself (token? tag? bundle path?).
    - Scope: per-surface, per-app, per-action-class, time-bounded?
    - Approval flow: who prompts, who decides, where decisions are persisted.
    - Wire shape: new endpoints, new event types on `/events`, new error codes.
    - Relationship to system-permissions: what's separate, what overlaps.
    - Default-deny vs default-allow tradeoffs.

---

## Phase 3 — `PortholeHelper.app` (Swift menu-bar app)

**Goal:** Native UX for the parts of porthole that benefit from being native. Same bundle as the daemon — `CFBundleExecutable` flips from `portholed` to `PortholeHelper`, helper spawns daemon as a child.

- [ ] Xcode project under `apps/porthole-helper-mac/` (Swift / SwiftUI). Build output assembles the final bundle by copying the Rust-built `portholed` and `porthole` from `target/release/` into `Contents/MacOS/`.
- [ ] `NSStatusItem` with monochrome glyph + optional badge (surface count, "broken" state).
- [ ] Helper spawns `portholed` on launch via `Process` if not already running; restarts on crash.
- [ ] Onboard UI flow — native equivalent of `porthole onboard`. Pulls grant state from `/info`, deep-links to System Settings panes via `x-apple.systempreferences:` URLs, "re-arm prompt" actions POST to `/system-permissions/request`.
- [ ] Notification surface for agent-permission approvals (depends on phase 2). `UNUserNotificationCenter` actions Allow / Deny POST back to `/agent-permissions/{id}/approve|deny`.
- [ ] `SMAppService.daemon(plistName:)` registration so the user gets a System Settings → General → Login Items entry. Subsumes phase 1's CLI-installed LaunchAgent for users who have the helper.
- [ ] Migration: helper's first launch detects and removes any phase-1 LaunchAgent plist at `~/Library/LaunchAgents/org.flotilla.porthole.plist` (and `launchctl bootout`s it) before registering its own, so the user doesn't end up with two start mechanisms competing.
- [ ] Quit / Restart daemon menu items.

---

## Phase 4 — v0.1 product slices (parallel)

**Goal:** Expand what porthole *does*. These are independent of the platform/UX track and can interleave freely. Pick one at a time; each gets its own design spec under `docs/superpowers/specs/`.

Candidates, roughly ordered by leverage:

- [ ] **Recording on macOS** — AVFoundation or ScreenCaptureKit. Biggest user-visible feature gap.
- [ ] **Multi-display placement verbs** — extends the phase-0 `/place` route (which takes explicit geometry) with anchor-based placement (e.g. `anchor: focused_display`, `anchor: by_id`).
- [ ] **Browser tabs via CDP** — Chrome / Edge tab coverage that AX can't reach. Expanded tab verb set (input, wait, replace) and content-area screenshot crop.
- [ ] **`force_place: true` launch option** — placement on preexisting surfaces.
- [ ] **Hyprland adapter (Linux)** — second platform via `hyprctl` IPC.

Long-horizon, beyond v0.1: KWin, X11, Windows adapters; overlay/annotation subsystem; MCP server surface; remote multi-machine presentation; record/replay integration.

---

## Bundle architecture

End-state layout (post-phase 3):

```
/Applications/Porthole.app/
  Contents/
    Info.plist            # CFBundleIdentifier = org.flotilla.porthole
    MacOS/
      PortholeHelper      # SwiftUI menu-bar app (CFBundleExecutable)
      portholed           # daemon, spawned by helper
      porthole            # CLI, same bundle so TCC matches
~/.local/bin/porthole -> /Applications/Porthole.app/Contents/MacOS/porthole
```

TCC, `UNUserNotificationCenter`, and `SMAppService` are all keyed off `CFBundleIdentifier`, not executable path. Three binaries inside one `.app` = one Privacy & Security entry, one Notifications entry, one Login Item. The user manages "Porthole" as one thing.

Phase 0–2 transitional layout: `CFBundleExecutable = portholed`, no helper present. Phase 3 flips `CFBundleExecutable` to `PortholeHelper` and adds the helper binary; bundle ID stays constant so existing TCC grants survive the transition.

---

## Amendment policy

This doc is the single source of truth for phase ordering. Updates land as part of the PR that changes the plan, not as separate "let's update the roadmap" commits. When a phase finishes, leave the checklist as a record of what shipped — don't delete it.
