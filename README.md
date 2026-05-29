# porthole

OS-level presentation substrate for agents: launch apps and artifacts, drive them, capture them, and show them to the user — through a typed HTTP-over-UDS API.

Status: v0, macOS only, pre-release. Wire contract may still change between slices. Part of [flotilla-org](https://github.com/flotilla-org) — designed to be the substrate flotilla's yeoman uses for desktop orchestration, but usable standalone.

## What is porthole?

Most agent workflows that need to touch the desktop end up reinventing the same fragile stack: `open`, AppleScript, PID scraping, sleep loops. Porthole is that stack, written once and tested. It gives you:

- Stable handles for windows you launched or attached to — no global enumeration needed after the fact
- Structured input (`key`, `text`, `click`, `scroll`) targeted at a specific handle
- Typed waits (`stable`, `dirty`, `exists`, `gone`, `title_matches`) that don't livelock on blinking cursors
- Capture primitives (screenshot and bounded recording) with self-describing metadata
- Presentation: launch a file artifact with explicit placement, auto-dismiss, and in-place replacement

Everything happens over HTTP-over-UDS — same protocol Firecracker uses for VM control, curl-debuggable by default, with an OpenAPI-able surface for agents.

## Install & run

Recommended sequence: build the bundle, install it (helper + daemon + CLI go in one place under launchd's control), then run `porthole onboard` against the installed daemon to grant TCC.

```sh
git clone https://github.com/flotilla-org/porthole
cd porthole
cargo build --workspace --release
cargo xtask bundle --platform macos --release

# 1. Install: copies Porthole.app to /Applications, symlinks the CLI into
#    ~/.local/bin/porthole, registers a LaunchAgent so the helper auto-starts
#    at login and supervises the daemon. Pass --user to install per-user
#    without admin.
./target/release/Porthole.app/Contents/MacOS/porthole install

# 2. Onboard: walks through each ungranted permission, fires the OS prompt,
#    waits for you to grant, restarts the daemon, verifies. One permission
#    at a time — TCC coalesces simultaneous prompts and AX/SR trust state
#    is cached per process, so each grant gets its own restart cycle.
porthole onboard

# 3. From now on the helper is ambient and keeps the daemon running. Verify:
porthole status
```

Why install before onboard: TCC grants attach to bundle path. Granting in the build location and then moving the bundle to `/Applications` resets the grants, and onboarding the build location twice does no good. Install first → onboard against the final location → done.

`Porthole.app` holds the native helper, daemon, and CLI in `Contents/MacOS/`, sharing one TCC bundle identity so a single Privacy & Security entry covers all three. The helper owns native startup/status UI and daemon supervision; the daemon still owns HTTP-over-UDS, desktop actions, and permission truth.

To reverse: `porthole uninstall` removes the LaunchAgent, the symlink, and the bundle. TCC grants persist; clear with `tccutil reset Accessibility work.flotilla.porthole.dev` (and `ScreenCapture`) if needed.

Pre-release builds before the `flotilla.work` identity rename used
`org.flotilla.porthole*`. If one of those is installed locally, remove the old
LaunchAgent and stale TCC grants once:

```sh
launchctl bootout "gui/$(id -u)" "$HOME/Library/LaunchAgents/org.flotilla.porthole.plist" 2>/dev/null || true
rm -f "$HOME/Library/LaunchAgents/org.flotilla.porthole.plist"
tccutil reset Accessibility org.flotilla.porthole.dev
tccutil reset ScreenCapture org.flotilla.porthole.dev
```

The daemon listens on a UDS under `$XDG_RUNTIME_DIR/porthole/porthole.sock` (or `$TMPDIR/porthole-<uid>/porthole.sock` as a fallback). Override with `PORTHOLE_RUNTIME_DIR`.

### Cargo install (CLI only)

If you just want the CLI to poke at a daemon someone else is running:

```sh
cargo install --git https://github.com/flotilla-org/porthole porthole --locked
```

This lands `porthole` in `~/.cargo/bin/`. **The daemon (`portholed`) must still be launched from the `.app` bundle, normally by `PortholeHelper`** — TCC keys off bundle identity, so a daemon started from `~/.cargo/bin/portholed` would have no stable identity for grants and prompts would re-fire on every cargo install.

### Permissions (macOS)

The macOS adapter needs **Accessibility** and **Screen Recording** for input injection, screenshots, and frame-diff waits. The dev bundle gives a stable identity so grants persist across rebuilds.

```sh
porthole onboard       # interactive grant flow; opens System Settings as needed
```

For development builds, `scripts/dev-bundle.sh` requires a local Apple
Development identity and fails hard if none is available. Ad-hoc signatures are
not usable for normal Porthole development because their designated requirement
changes on every rebuild, which invalidates TCC grants while System Settings may
still show the toggles as enabled. If you previously granted permissions to an
ad-hoc build and `porthole onboard` still reports missing permissions, reset TCC
once after rebuilding with a certificate:

```sh
tccutil reset Accessibility work.flotilla.porthole.dev
tccutil reset ScreenCapture work.flotilla.porthole.dev
```

`scripts/dev-bundle.sh` remains as a compatibility wrapper around
`cargo xtask bundle --platform macos`.

`onboard` walks through each ungranted permission one at a time: fires the OS prompt, waits for you to press Enter once you've granted in System Settings, restarts the daemon (via `launchctl kickstart -k`) so its cached AX/SR trust state refreshes, then re-reads `/info` to confirm. Serial because TCC silently coalesces simultaneous prompt requests from one process and the trust state is cached per-process; each grant needs its own daemon lifetime.

Exit codes:

- **0** — all granted and verified post-restart
- **1** — at least one still missing (dismissed, or daemon not under launchd so we can't verify) or a request to fire the prompt errored
- **3** — `--no-wait` mode; prompts fired, no Enter wait, no restart, no verification — caller handles the rest

See `docs/development.md` for first-time setup, rebuild workflow, and TCC reset commands.

## Core concepts

- **Surface** — a window or tab porthole knows about. Identified by a `surface_id` (string). All verbs operate on surfaces.
- **Launch** — starts a process or opens an artifact, correlates the resulting OS window to a surface id, tracks it for you.
- **Attach** — takes an already-running window, hands you a handle to manage it. `search` lists candidates; `track` mints the handle.
- **Session** — an optional opaque tag you attach to calls. Propagates into capture metadata. No query endpoint; just correlation-by-tag.
- **Agent identity** — a local bearer-token principal used by protected routes. Surface and desktop-control routes default-deny until the operator approves a matching request.
- **Capability** — adapters declare what they actually support via `/info`. Probe before you commit to a workflow.

## Using porthole from an agent (HTTP)

The daemon speaks HTTP/1.1 over a Unix Domain Socket. SSE is available at `/events` for agent-permission and policy events. Requests are JSON; responses are JSON; errors are `{"code": "snake_case_code", "message": "...", "details": {...}?}` with a matching HTTP status.

### Agent permissions

Protected routes require an agent bearer token and a matching local grant. The current route map covers launch/search/track/attention/display metadata, surface observe routes (`screenshot`, `wait`, `content-rect`), drive routes (`focus`, `key`, `text`, `click`, `scroll`, pointer movement), manage routes (`place`, `replace`, `close`), surface capture-session creation, and protected capture-transfer fd-channel consumption. Operator commands create identities, inspect pending requests, approve/deny requests, and revoke grants:

```sh
porthole agents create --name "My Agent" --json
# Pass the returned token as: Authorization: Bearer <token>
# CLI drive commands read it from PORTHOLE_AGENT_TOKEN or --agent-token.
# Prefer the environment variable so tokens don't appear in process listings.
export PORTHOLE_AGENT_TOKEN=pta_agent_...

porthole agents requests --json
porthole agents approve <request_id> --duration until-surface-gone
porthole agents grants --json
porthole agents grant revoke <grant_id>
```

When a protected route has no matching grant, the daemon returns `403 agent_permission_needed` with a `details.request_id`, `details.target`, and requested actions. Agent clients can retry after approval. Helper/UI clients should subscribe to `/events` for `agent_permission_requested` and `agent_permission_resolved` in the steady state; the CLI commands above provide a polling/operator fallback.

### Discover capabilities

```sh
curl --unix-socket $XDG_RUNTIME_DIR/porthole/porthole.sock \
     http://localhost/info | jq
```

Response includes `adapters[0].capabilities: [...]` — a list of strings like `"launch_process"`, `"launch_artifact"`, `"placement"`, `"screenshot"`, `"wait"`, `"attention_focused_surface"`, etc. If a capability is absent, the corresponding endpoint will return `adapter_unsupported` or `capability_missing`.

### Launch a process

```sh
curl --unix-socket .../porthole.sock \
     -X POST http://localhost/launches \
     -H 'Content-Type: application/json' \
     -d '{
       "kind": { "type": "process", "app": "/Applications/Ghostty.app" },
       "require_confidence": "strong",
       "timeout_ms": 10000
     }'
```

Returns:

```json
{
  "launch_id": "launch_...",
  "surface_id": "surf_...",
  "surface_was_preexisting": false,
  "confidence": "strong",
  "correlation": "tag",
  "placement": { "type": "not_requested" }
}
```

`confidence: "strong"` is the default; fail fast on ambiguous correlation. Pass `plausible` or `weak` to accept weaker matches.

### Launch an artifact (file path)

```sh
curl --unix-socket .../porthole.sock \
     -X POST http://localhost/launches \
     -H 'Content-Type: application/json' \
     -d '{
       "kind": { "type": "artifact", "path": "/tmp/demo.pdf" },
       "placement": { "on_display": "focused" },
       "auto_dismiss_after_ms": 30000
     }'
```

Opens the file via the OS default handler (macOS `open`, backed by LaunchServices). Returns the same shape as a process launch, plus:

- `surface_was_preexisting: true` if the OS reused an already-open window (common for apps like Preview that consolidate into tabs). When true, **placement is not applied** — porthole won't reposition a window you didn't previously own.
- `placement.type: "skipped_preexisting"` when placement was requested but skipped for the above reason.

To fail fast on a preexisting match instead of silently taking one, pass `"require_fresh_surface": true`:

```json
{
  "kind": { "type": "artifact", "path": "/tmp/demo.pdf" },
  "require_fresh_surface": true
}
```

On preexisting correlation this returns HTTP 409 `launch_returned_existing` with a `ref` in the body you can `POST /surfaces/track` to attach it if you still want it:

```json
{
  "code": "launch_returned_existing",
  "message": "launch correlated to a preexisting surface (require_fresh_surface: true)",
  "details": {
    "ref": "ref_eyJwaWQiOjk4NzYsIm...",
    "app_name": "Preview",
    "title": "demo.pdf",
    "pid": 9876,
    "cg_window_id": 42
  }
}
```

### Drive a surface

All input is handle-targeted:

```sh
# Type a command
curl --unix-socket .../porthole.sock \
     -X POST http://localhost/surfaces/$SURFACE/text \
     -H 'Content-Type: application/json' \
     -H "Authorization: Bearer $PORTHOLE_AGENT_TOKEN" \
     -d '{"text": "python3 repro.py\n"}'

# Press Enter with modifiers
curl --unix-socket .../porthole.sock \
     -X POST http://localhost/surfaces/$SURFACE/key \
     -H 'Content-Type: application/json' \
     -H "Authorization: Bearer $PORTHOLE_AGENT_TOKEN" \
     -d '{"events": [{"key": "Enter"}, {"key": "KeyA", "modifiers": ["Cmd"]}]}'

# Click at window-local coordinates
curl --unix-socket .../porthole.sock \
     -X POST http://localhost/surfaces/$SURFACE/click \
     -H 'Content-Type: application/json' \
     -H "Authorization: Bearer $PORTHOLE_AGENT_TOKEN" \
     -d '{"x": 120, "y": 440, "button": "left", "count": 1}'
```

Key names are DOM `KeyboardEvent.code` style: `"Enter"`, `"KeyA"`, `"Digit5"`, `"F5"`, `"ArrowUp"`, `"BracketLeft"`, etc. Modifiers: `"Cmd"`, `"Ctrl"`, `"Alt"`, `"Shift"`. Unknown names return `unknown_key` with the supported set in the error body.

Input focuses the target surface first (by design). There's no no-focus-steal mode yet.

### Wait on conditions

```sh
# Wait until the window stops changing (stable for 1.5s, tolerating cursor blink)
curl --unix-socket .../porthole.sock \
     -X POST http://localhost/surfaces/$SURFACE/wait \
     -H 'Content-Type: application/json' \
     -d '{
       "condition": {"type": "stable", "window_ms": 1500, "threshold_pct": 1.0},
       "timeout_ms": 10000
     }'

# Wait for any repaint larger than 1% of pixels
-d '{"condition": {"type": "dirty", "threshold_pct": 1.0}, "timeout_ms": 5000}'

# Wait for a title match
-d '{"condition": {"type": "title_matches", "pattern": "^Done:"}, "timeout_ms": 30000}'

# Wait for the surface to disappear
-d '{"condition": {"type": "gone"}, "timeout_ms": 5000}'
```

`threshold_pct` is the percentage of pixels that must differ to count as a change — defaults to `1.0` so a blinking terminal cursor doesn't livelock `stable` or spuriously satisfy `dirty`.

On timeout you get a `wait_timeout` error with a `details.last_observed` payload (presence, title, or last-change metrics).

### Capture

```sh
curl --unix-socket .../porthole.sock \
     -X POST http://localhost/surfaces/$SURFACE/screenshot \
     -H 'Content-Type: application/json' \
     -d '{}'
```

Returns:

```json
{
  "surface_id": "...",
  "png_base64": "iVBORw0KGgo...",
  "window_bounds": {"x": 100, "y": 80, "w": 1200, "h": 800},
  "content_bounds": {"x": 100, "y": 110, "w": 1200, "h": 770},
  "scale": 2.0,
  "captured_at_unix_ms": 1714000000000
}
```

The metadata is self-describing — no out-of-band assembly. `content_bounds` is omitted when the adapter can't determine it.

### Close or focus

```sh
curl --unix-socket .../porthole.sock -X POST http://localhost/surfaces/$SURFACE/close -H ... -d '{}'
curl --unix-socket .../porthole.sock -X POST http://localhost/surfaces/$SURFACE/focus -H ... -d '{}'
```

Close verifies the window actually went away (returns `close_failed` with `old_handle_alive: true` if an app refused, e.g., unsaved-changes dialog).

### Replace (swap artifact in the same slot)

```sh
curl --unix-socket .../porthole.sock \
     -X POST http://localhost/surfaces/$SURFACE/replace \
     -H 'Content-Type: application/json' \
     -d '{"kind": {"type": "artifact", "path": "/tmp/next.pdf"}}'
```

Omitting the `placement` key inherits the old surface's geometry. Include `"placement": {}` (or any populated value) to use it verbatim. Response is a fresh `launch` response — capture the new `surface_id`; the old one is dead.

Errors carry `details.old_handle_alive: bool` so you know whether the old surface survived a partial replace (e.g., a save-dialog refused the close).

### Attach existing windows

```sh
# List candidates
curl --unix-socket .../porthole.sock \
     -X POST http://localhost/surfaces/search \
     -H 'Content-Type: application/json' \
     -d '{"app_name": "Ghostty", "title_pattern": "^demo-", "frontmost": true}'

# Promote one to a tracked handle
curl --unix-socket .../porthole.sock \
     -X POST http://localhost/surfaces/track \
     -H 'Content-Type: application/json' \
     -d '{"ref": "<candidate ref from search>"}'
```

`track` is idempotent by `cg_window_id` — if another caller (or an earlier call) already tracked the same window, you get back the existing `surface_id` with `reused_existing_handle: true`.

### Attention and displays

```sh
curl --unix-socket .../porthole.sock http://localhost/attention     # focused surface/display/cursor
curl --unix-socket .../porthole.sock http://localhost/displays      # monitor topology
```

Use these before choosing a placement — e.g., "show this on whatever display the user is looking at" = `placement: { "on_display": "focused" }`.

## Using porthole from the shell (CLI)

The `porthole` CLI wraps the HTTP API with ergonomic flags. Examples:

```sh
# Launch an app
porthole launch --app /Applications/Ghostty.app

# Drive a surface
porthole text   $SURFACE "python3 repro.py"
porthole key    $SURFACE --key Enter
porthole wait   $SURFACE --condition dirty --timeout-ms 5000
porthole screenshot $SURFACE --out repro.png

# Attach to your own containing terminal window (walks $$ ancestors via ps)
SURFACE=$(porthole attach --containing-pid $$ --frontmost --json | jq -r .surface_id)

# Show an artifact with placement and auto-dismiss
porthole launch --kind artifact --app /tmp/proposal.pdf \
                --on-display focused --auto-dismiss-ms 15000

# Replace in place
porthole replace $SURFACE --kind artifact --app /tmp/alt.pdf --inherit-placement

# Move a tracked surface to the focused display using default sizing
porthole place $SURFACE --anchor focused-display

# Close
porthole close $SURFACE
```

Run `porthole --help` or `porthole <subcommand> --help` for the full flag set.

## Recipes

For longer end-to-end walkthroughs, see [`docs/recipes/`](docs/recipes/) — currently:

- [`terminal-orchestration.md`](docs/recipes/terminal-orchestration.md) — drive a terminal end-to-end (launch / focus / wait / type / screenshot / scrollback / reflow via `place` / close), with notes on inner-script ↔ harness signalling. Companion smoke script: `scripts/manual-terminal-smoke.sh`.

### Evidence-bundle for a terminal bug

The original motivating workflow. Full sequence:

```sh
# 1. Launch the terminal
SURFACE=$(porthole launch --app /Applications/Ghostty.app --json | jq -r .surface_id)

# 2. Type the repro command
porthole text $SURFACE "python3 repro.py"
porthole key  $SURFACE --key Enter

# 3. Wait for the command output to settle
porthole wait $SURFACE --condition stable --window-ms 1500 --timeout-ms 10000

# 4. Before-screenshot
porthole screenshot $SURFACE --out before.png --session ghostty-bug

# 5. Trigger the buggy behaviour (e.g., press Enter on the final prompt)
porthole key  $SURFACE --key Enter

# 6. Wait for the frame to dirty
porthole wait $SURFACE --condition dirty --timeout-ms 5000

# 7. After-screenshot
porthole screenshot $SURFACE --out after.png --session ghostty-bug

# 8. Clean up
porthole close $SURFACE
```

Both screenshots carry enough metadata (`window_bounds`, `scale`, `captured_at_unix_ms`, the `session` tag) to be self-describing in a bug report.

### A script finding its own terminal window

Useful for test harnesses or self-documenting workflows:

```sh
SURFACE=$(porthole attach --containing-pid $$ --frontmost --json | jq -r .surface_id)
porthole screenshot $SURFACE --out self.png
```

`--containing-pid $$ --frontmost` walks the process ancestry (shell → terminal-helper → terminal-app) and picks the frontmost matching window. `--frontmost` disambiguates when a terminal app owns multiple windows; without it, a multi-window match returns an error.

### Presenting something to the user

```sh
# Show a document on whatever display the user is currently looking at,
# auto-dismiss after 15 seconds if they don't act on it.
SURFACE=$(porthole launch --kind artifact --app /tmp/proposal.pdf \
                          --on-display focused \
                          --require-fresh-surface \
                          --auto-dismiss-ms 15000 --json | jq -r .surface_id)

# User nods. Swap in the next artifact in the same slot.
SURFACE=$(porthole replace $SURFACE --kind artifact --app /tmp/alternative.pdf \
                                     --require-fresh-surface \
                                     --auto-dismiss-ms 15000 --json | jq -r .surface_id)

# Done early.
porthole close $SURFACE
```

`--require-fresh-surface` ensures each step gets a freshly minted window, which is necessary for `replace`'s geometry inheritance to land in the right slot (reused windows from the OS default-handler may already be elsewhere on screen).

### Running the same TUI across multiple terminals

```sh
for APP in "Ghostty" "iTerm" "Terminal"; do
  SURFACE=$(porthole launch --app "/Applications/$APP.app" --json | jq -r .surface_id)
  porthole text   $SURFACE "htop"
  porthole key    $SURFACE --key Enter
  porthole wait   $SURFACE --condition stable --window-ms 800 --timeout-ms 5000
  porthole screenshot $SURFACE --out "htop-$APP.png"
  porthole close $SURFACE
done
```

The geometry will differ per terminal since placement isn't set. For consistent frames, pass `--on-display` and `--geom-x/y/w/h` on each launch.

## Error shapes

Every error response follows the same JSON body:

```json
{
  "code": "snake_case_code",
  "message": "...",
  "details": { ... }
}
```

`details` is optional and carries structured extras when relevant (last-observed state on `wait_timeout`, `ref` + metadata on `launch_returned_existing`, `old_handle_alive` on replace errors, etc.). HTTP status codes are typed per error — see the spec docs for the full matrix, but briefly:

- `404` — surface not found (stale id, never existed)
- `410` — surface dead (handle exists but window gone)
- `403` — permission needed (approve agent request, or grant Accessibility / Screen Recording)
- `409` — correlation ambiguous, launch returned existing, close failed, etc.
- `400` — invalid argument (unknown key name, out-of-range coord, URL instead of file path, unknown display id, etc.)
- `501` — adapter can't do this on this platform (capability_missing)
- `504` — wait or launch timed out

## Design and spec

The full design is split into layered specs:

- [v0 design](docs/superpowers/specs/2026-04-20-porthole-design.md) — the substrate story, architecture, resource model
- [Slice A](docs/superpowers/specs/2026-04-21-porthole-slice-a-design.md) — input, wait, close, focus, attention, displays
- [Slice B](docs/superpowers/specs/2026-04-21-porthole-slice-b-design.md) — search, track, PID-ancestry attach
- [Slice C](docs/superpowers/specs/2026-04-22-porthole-slice-c-design.md) — artifact launches, placement, replace, auto-dismiss

For the experience report that motivated porthole in the first place, see [docs/2026-04-20-window-evidence-experience-report.md](docs/2026-04-20-window-evidence-experience-report.md).

## What's not here yet

Deferred to future slices:

- **Events SSE expansion** — `/events` currently carries agent-permission and policy events. Surface lifecycle and launch notifications are still planned; those will enable event-native `wait` for `exists`/`gone`/`title_matches` and populate `recently_active_surface_ids`.
- **Browser / CDP** — URL artifact support, tab-as-first-class-surface, richer browser automation.
- **Tabs** — native-app tab enumeration (iTerm2, Safari, Ghostty, Preview) with the restricted verb matrix from the v0 spec §4.1.
- **Porthole-viewer app** — a canonical review-oriented display for agent-common content types (markdown, mermaid, dot, asciinema, videos). Slice C's `open`-based dispatch lands agents in edit-oriented apps; the viewer will be the right primitive for "show this for review."
- **Hyprland / Linux / Windows** adapters.
- **Cross-host routing** — same HTTP protocol over TCP; no policy yet.
- **`focus: "preserve"`** no-focus-steal input.
- **AX-element-reference targeting** for click/scroll.

See each slice's spec for what's in vs. deferred.

## License

Dual-licensed under Apache 2.0 or MIT at your option.
