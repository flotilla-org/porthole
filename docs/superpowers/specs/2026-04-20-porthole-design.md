# Porthole — Design

Date: 2026-04-20
Status: Draft for review

## 1. Purpose

Porthole is an OS-level presentation substrate. It papers over platform differences in launching apps, identifying their windows, driving those windows with input, capturing them, and presenting them to a user. It is designed primarily for agent callers, so its API is structured, deterministic, and explicit about what it did and how confidently.

Porthole is not:

- A desktop environment
- A multiplexer (tmux, zellij, cmux remain in their own layer)
- A general GUI test framework
- A work-item or workspace manager (that belongs to flotilla)
- A single unified windowing model across every platform — it papers over differences, it does not erase them

## 2. Framing

Two source documents informed this design:

- `docs/porthole_spec.md` — an earlier ideation with ChatGPT. Ambitious cross-platform vocabulary (targets, surfaces, slots, bindings, adapters, overlays, attention model). Used here as inspiration for naming and long-term shape, not as a commitment.
- `docs/2026-04-20-window-evidence-experience-report.md` — a concrete experience report from capturing Ghostty bug-repro evidence. The strongest live pain signal: launching, identifying, driving, and capturing desktop windows as part of an agent workflow.

The design takes the report's narrower workflow as the first real use case, and shapes the API toward the spec's destination so later growth does not need renaming.

### 2.1 Use cases porthole must serve

- **Evidence collection** — the report's workflow: launch an app, drive it, capture before/after screenshots, optionally record.
- **Presentation** — an agent (a flotilla yeoman or otherwise) showing an artifact to the user, *in the right place*. Covers what `superpowers:visual-companion` and tools like [glimpse](https://github.com/hazat/glimpse) try to do, but more principled about placement and attention. The yeoman decides at a higher level whether an artifact belongs in a mux pane (flotilla's problem) or as a dedicated window alongside (porthole's problem). When porthole is chosen, it owns placement, reuse-in-place, dismiss lifecycle, and attention-aware positioning. Porthole does *not* own artifact rendering or content-type policy — that stays with the caller or a higher-level tool layered on top.
- **Cross-host delegation** — showing a window on a specific user's machine when multiple are involved. Not in v0, but the shape must not prevent it.
- **Standalone use** — a human running porthole from a shell to automate desktop work not mediated by flotilla.

### 2.2 Boundary with flotilla

Heuristic: **if a concept differs significantly by OS, compositor, or app, porthole owns it. If it is OS-agnostic domain logic, flotilla (or another caller) owns it.**

Applied:

| Concept | Owner |
|---|---|
| OS process launch, app-bundle activation | porthole |
| Top-level window identity, geometry, focus | porthole |
| Tabs where an app exposes them natively (iTerm2, Ghostty, browsers) | porthole |
| Screen / display enumeration and placement | porthole |
| Screenshot, recording | porthole |
| Input injection into a specific window | porthole |
| Multiplexer panes (tmux, zellij, cmux) | flotilla |
| Work items, correlation across providers | flotilla |
| Agent hooks, session lifecycles, repo state | flotilla |
| Nested addressing (pane inside tab inside window) | flotilla, via env-var threading |

Porthole does not model nesting. When flotilla needs to drive a specific pane inside a specific iTerm2 tab, it asks porthole to focus the window-and-tab, then uses its own multiplexer knowledge for the pane, then asks porthole to screenshot the window. Env vars set at launch thread the identity through.

### 2.3 Primary client is an agent

The API is designed for an agent caller (likely a flotilla yeoman agent with context about user intent), not a human first. This means:

- Return structured data, not formatted text.
- Prefer explicit inputs over guesswork.
- Ambiguity ("which window did you mean?") is an error the agent can see and correct, not a silent best-effort.
- Every response carries enough metadata to be self-describing — no out-of-band assembly.

## 3. System Architecture

```
┌──────────┐   HTTP over UDS    ┌──────────┐
│  porthole │ ──────────────────▶ │ portholed │
│   (CLI)   │ ◀─── SSE events ─── │ (daemon)  │
└──────────┘                     └────┬─────┘
                                      │
                                      ▼
                                 ┌──────────┐
                                 │ adapters │
                                 └────┬─────┘
                                      │
                                      ▼
                              native OS APIs
```

- `porthole-core` — library crate. Adapter trait, handle store, launch/attach state machine, event dispatch. No HTTP, no CLI.
- `portholed` — daemon crate. Thin HTTP-over-UDS adapter over `porthole-core`. Owns the socket, the event SSE stream, the handle lifetime.
- `porthole` — CLI crate. Thin HTTP client over `portholed`. Auto-spawns the daemon on first call if not running (cleat's pattern).
- Adapters — platform-specific crates. v0 ships `porthole-adapter-macos`. Future: Hyprland, browser (CDP), X11.

Workspace layout mirrors flotilla and cleat so contributors move between them without friction.

### 3.1 Transport: HTTP over Unix Domain Socket

Chosen for:

- curl-debuggable by default — agents and humans already know REST
- Standard Rust frameworks (axum + tower + hyperlocal) keep the implementation cheap
- An OpenAPI spec falls out of the route definitions — agents get a machine-readable surface
- Trivially promotable to TCP HTTP later for cross-host without wire-protocol change
- Matches the Firecracker VM pattern; nothing exotic for callers to learn

The UDS lives under the XDG runtime dir (same discovery priority as cleat: `$PORTHOLE_RUNTIME_DIR` → `$XDG_RUNTIME_DIR/porthole` → `$TMPDIR/porthole-<uid>` → `/tmp/porthole-<uid>`). Per-user, not per-session — porthole handles span tasks.

#### 3.1.1 Transport details

- **HTTP/1.1** is the v0 baseline. hyper/axum speak it natively over UDS; HTTP/2-over-UDS (h2c) adds machinery for near-zero benefit on a local socket and is not pursued in v0.
- **SSE runs on its own connection.** Under HTTP/1.1 a single connection carries one in-flight request, so an open event stream occupies that connection exclusively. Synchronous calls use a separate (pooled) connection. Connection cost is trivial on UDS. Typical client shape: one task consumes `/events`; request calls go on another connection.
- **Events are ephemeral.** If a client disconnects, events emitted while gone are lost. `Last-Event-ID` replay and an event log are deferred until a use case asks for them — the evidence-loop and presentation cases treat SSE as a live notification channel, not a durable log.

### 3.2 Embedding

`porthole-core` is a library crate so that an embedder (a future flotilla path, test harnesses) can skip the socket and call in-process. The daemon is the shipping deployment for v0; embedding is a structural property kept alive for later, not a second delivery vehicle.

## 4. Resource Model

The API is REST-shaped. Top-level resources:

| Resource | Role |
|---|---|
| `/launches` | A launch handle. The daemon tracks the link between "this launch" and "the surface(s) that appeared." |
| `/surfaces` | Windows and tabs. Core noun. Both launched and attached surfaces live here. |
| `/displays` | Monitors. Read model (topology, primary/focused flags) in v0; placement happens via launch options, not verbs on this resource in v0. |
| `/attention` | Read-only: focused surface, focused display, cursor position, recently-active surfaces. What a placement-aware caller consults before deciding where to show something. |
| `/events` | SSE stream (GET) of surface lifecycle and launch events. |
| `/info` | Read-only introspection: which adapters are loaded, what capabilities each claims, daemon version. |

Operations are verbs on surfaces:

- `POST /surfaces/{id}/key`
- `POST /surfaces/{id}/text`
- `POST /surfaces/{id}/click`
- `POST /surfaces/{id}/scroll`
- `POST /surfaces/{id}/screenshot`
- `POST /surfaces/{id}/recording` — v0.1; noun in API shape, adapter path deferred
- `POST /surfaces/{id}/wait`
- `POST /surfaces/{id}/focus`
- `POST /surfaces/{id}/close`
- `POST /surfaces/{id}/replace` — close this surface and launch a new one in its position; atomic from the caller's POV. Enables "reuse-in-place" for presentation without needing the spec's slot concept in v0.
- `POST /surfaces/search` — find by query; used for attach mode
- `POST /surfaces/{id}/track` — promote a search result to a watched handle

Sessions are an opaque tag field, not a resource. Every mutation accepts an optional `session` string; the daemon indexes by it so callers can ask "what happened under this session" without porthole competing to be the system of record. This leaves room for future scoping (agent tokens limited to a session set) without locking storage semantics now.

## 5. Handles

Handles are opaque string IDs minted by the daemon. They reference a surface in the daemon's state, not the OS-level identifier directly. The daemon:

- Watches the underlying OS surface for lifecycle events (moved, retitled, closed).
- Keeps the mapping from handle → live AX element / CGWindowID / etc. fresh across OS-level changes where possible.
- Transitions the handle to an explicit `dead` state when the surface goes away, emitting an event. Operations on a dead handle return a typed error — never a silent failure.

Launched and attached handles are indistinguishable in downstream API — same verbs, same responses.

## 6. Launch and Correlation

The central technical problem: "I just launched Ghostty. Three other Ghostty windows already exist, across two app bundles. Which one is mine?"

### 6.1 Strategy

Layered fallback with reported confidence:

1. **Strong tag** where the adapter knows how — env var readable via `ps eww`, window title token the launched command sets, AX attribute, URL-scheme echo, etc. Adapter-specific per app class.
2. **PID tree** — capture the launched PID, match windows whose owning process is in its descendant tree.
3. **Temporal window** — the next new window of the expected app that appears within Δt.

The launch response reports:

```json
{
  "launch_id": "...",
  "surface_id": "...",
  "confidence": "strong" | "plausible" | "weak",
  "correlation": "tag" | "pid_tree" | "temporal",
  "evidence": { ... }
}
```

v0 assumes a single identified surface per launch. Launches that produce multiple surfaces (multi-window apps opening several at once) are deferred: the launch returns the first correlated surface, the rest are attachable via `/surfaces/search`. A future `surface_ids: [...]` field can layer on without breaking the v0 response shape.

Callers pass `require_confidence: "strong"` to fail fast if the adapter cannot get a strong match. Default accepts `plausible`. Weak matches require opt-in.

### 6.2 Contract

`POST /launches` blocks until:

- The surface is identified, **and**
- The daemon has started watching its lifecycle

…or a timeout is hit. This means the returned handle is immediately usable for input and capture — no "did it appear yet?" poll. Timeout is a request parameter with a sane default.

### 6.3 Lifecycle policy

Chosen at launch via an explicit enum:

| Mode | Meaning |
|---|---|
| `exit_on_command_end` | Default for short repros. |
| `keep_alive_duration: N` | Linger for N milliseconds after command exit. Replaces the report's "linger for N seconds" shell plumbing. |
| `keep_alive_interactive` | Drop into a shell after the command. Replaces `exec zsh -i`. |
| `keep_alive_until_closed` | Stay up until explicit `POST /surfaces/{id}/close`. |

The intent is that callers never reconstruct lifecycle through shell wrappers again.

The lifecycle enum is command-centric and only applies to `process` launches. `artifact` launches have no command to exit, so they stay up until the user closes the window, the API closes it, or an `auto_dismiss_after_ms` fires (see §6.7).

### 6.4 Attach mode

`POST /surfaces/search` returns candidates matching a query (app bundle, title regex, PID, AX path). Caller picks one, `POST /surfaces/{id}/track` promotes it to a watched handle. From then on the surface is indistinguishable from a launched one.

Covers: "the user (or agent) opened a window manually, now porthole should manage it" and the standalone case.

### 6.5 Launch kind — `process` or `artifact`

Launch requests carry a `kind` discriminator:

- **`process`** — run a command in an app. Body: app bundle / executable, args, env, working dir, lifecycle mode. The evidence-collection case.
- **`artifact`** — show a file or URL to the user using the OS default handler. Body: a local path or URL, optional `opener` override, placement, auto-dismiss. Internally on macOS this is `open <path>` or `open <url>`, then correlation + tracking like any other launch. No rendering, no built-in webview — content-type dispatch is whatever the OS already knows.

Both kinds produce a surface handle indistinguishable downstream. A caller that needs real rendering (markdown → HTML, templated views) builds on top and still uses `artifact` once it has a concrete file or URL.

### 6.6 Placement

Placement is a field on the launch body, applied once the surface is identified:

| Field | Meaning |
|---|---|
| `on_display: <id> \| "focused" \| "primary"` | Which monitor to anchor to. `focused` uses `/attention`'s view. |
| `geometry: { x, y, w, h }` | Explicit rectangle on the chosen display, in logical points. |
| `anchor: "focused_display" \| "cursor"` | Shorthand for "wherever the user is" without an explicit display id. |

Deferred to v0.1: `relative_to: <surface_id>`, `avoid: [<surface_id>, ...]`, size hints (`compact`, `medium`, `fill`). The primitives shipped in v0 cover the common presentation cases without committing to a richer placement vocabulary before evidence.

### 6.7 Dismiss

Separate from the lifecycle enum (which is command-centric). `auto_dismiss_after_ms: N` on the launch body closes the surface N milliseconds after it was opened, regardless of whether a command has exited. Makes presentation-style "show this for 10 seconds" a one-field decision rather than a polling loop. Explicit `POST /surfaces/{id}/close` dismisses on demand. Replacement via `POST /surfaces/{id}/replace` is the third path.

## 7. Operation Loop

### 7.1 Input

Structured verbs, handle-targeted:

- `key` — named keys, modifiers, sequences
- `text` — literal string
- `click`, `scroll` — coordinate or AX element reference when available

Default: porthole brings the surface to front before injection. Terminals and most macOS apps will not process input otherwise.

Opt-in `focus: "preserve"` attempts no-focus-steal delivery via AX where the adapter supports it, and returns an explicit error when it does not. Agents never silently lose keystrokes.

### 7.2 Capture

Two verbs with rich response metadata:

- `screenshot` — PNG of the surface bounds
- `recording` — short clip, bounded duration (v0.1 — v0 may ship screenshot only; see §9)

Response metadata includes: surface handle, launch id, app bundle, window title at capture time, pixel dimensions, display scale, monotonic timestamp, session tag if present. Every artifact is self-describing — the report's "manual evidence bundle assembly" pain dissolves.

### 7.3 Sequencing — orthogonal `wait`

Not baked into capture or input. Explicit `POST /surfaces/{id}/wait` with a condition:

- `stable` — no frame change for Δt
- `dirty` — next frame change
- `title_matches(regex)`
- `exists`
- `gone`

The canonical evidence loop reads as four explicit calls:

```
screenshot (before)
key Enter
wait dirty
screenshot (after)
```

Clear in agent transcripts and shell pipelines. The alternative `screenshot --after-input=Enter` is tempting but bakes sequencing policy and hides what happened in logs.

## 8. Adapters

### 8.1 macOS (v0)

- **Identity + lifecycle + input**: Accessibility (AX) APIs via `accessibility-sys` / hand-rolled FFI.
- **Launch**: `NSWorkspace` / `open` semantics for both `process` and `artifact` kinds, with tag injection for correlation.
- **Capture**: CGWindowList for v0. ScreenCaptureKit as a later swap behind the adapter trait.
- **Placement**: CoreGraphics display enumeration + AX window geometry. `on_display` and explicit `geometry` in v0; `anchor: focused_display` uses the same signals that feed `/attention`.
- **Attention**: focused app/window via AX and NSWorkspace, focused display and cursor position via CoreGraphics. Read-only; cheap.
- **Recording**: native path (AVFoundation or ScreenCaptureKit); ships v0.1.

xcap (nashaofu/xcap) was evaluated and rejected as a dependency — the capture path is small enough to own directly, and owning it gives us a clean path to ScreenCaptureKit when CGWindowList's deprecation bites. We read xcap's macOS source as prior art for the CG bindings.

Known constraint: input and capture both need the user's Accessibility and Screen Recording permissions. Porthole detects missing permissions on startup and reports them via `/info`. Operations that require them fail with a typed permission-needed error rather than silently no-op'ing.

### 8.2 Deferred to v0.1 and later

- **Hyprland** (IPC via `hyprctl`)
- **Browser tabs** via CDP
- **Multi-display placement verbs**
- **Recording** on macOS
- **KWin**, **X11**, **Windows**
- **Overlay subsystem** (the spec's wish-list)
- **MCP server surface** — layers on top of the HTTP API without new protocol work

The noun for each is preserved in the v0 API shape (tabs exist as a surface type; `/displays` is a resource) even where no adapter implements it yet. Growth does not need renames.

## 9. v0 Scope in One Line

macOS adapter, launch (process and artifact kinds, with lifecycle and placement) / attach / find / input / screenshot / wait / replace / events / attention, handles persisted by the daemon, sessions as opaque tags, HTTP over UDS, no recording yet, no overlay, no cross-host, no other platforms.

Explicit v0.1 candidates:

- Recording on macOS
- Multi-display placement verbs
- Browser tabs via CDP

## 10. Error Model

- Typed, machine-readable error responses — every error has a code, a message, and (where relevant) structured fields.
- Distinct codes for: `surface_not_found`, `surface_dead`, `permission_needed`, `launch_correlation_failed`, `launch_timeout`, `ambiguous_search`, `adapter_unsupported`, `capability_missing`.
- Ambiguity in search returns `ambiguous_search` with the candidate list — never a silent best-effort pick.

## 11. Observability and Debuggability

- Request/response log at the HTTP layer. Because it is HTTP-over-UDS, `curl --unix-socket` gives operators a direct debug channel.
- `GET /info` surfaces adapter list, loaded capabilities, permission status, daemon version, uptime.
- `GET /events` SSE stream mirrors what the daemon saw: surface appeared, moved, retitled, died; launch correlated with confidence X.
- Planned but not required in v0: structured tracing exported via OpenTelemetry.

## 12. Testing Strategy

Mirroring flotilla's record/replay approach where possible:

- Adapter trait is mockable — most core logic (launch state machine, handle store, event dispatch) tested against an in-memory adapter.
- Real macOS adapter covered by a smaller set of integration tests that require a real desktop session — marked and skippable in CI sandboxes.
- HTTP layer tested against an in-process daemon bound to an ephemeral UDS.
- Launch correlation logic is a pure state machine over an event stream — table-testable without any OS.

## 13. Open Questions

Intentionally deferred:

- **Authentication / authorization.** v0 runs on a user-owned UDS and trusts anything that can connect. A later tighter model (agent-scoped tokens, session-scoped permissions) layers on without changing the resource shape. The `session` tag is already positioned for this.
- **Cross-host routing.** HTTP over UDS makes TCP promotion trivial at the transport layer; the policy layer (who is allowed to display on whose host, how identity flows across) is an explicit later problem.
- **Recording format and length bounds.** Defer to when we build it in v0.1.
- **How the flotilla `PresentationManager` concept integrates.** Feeds into later flotilla work. The current read: flotilla's PresentationManager composes porthole calls and multiplexer calls, with env-var threading to carry identity. Concrete trait shape on flotilla's side is out of scope here.
- **Slots** (the ChatGPT spec's named-place-where-surfaces-go concept). Considered and deferred: the v0 `replace` verb gives callers enough to reuse windows for successive artifacts without an additional abstraction. If presentation patterns emerge where the yeoman wants a persistent named reference across porthole restarts or across surfaces it didn't mint, slots come back on the table.
- **Dedicated presentation subsystem with rendering.** Considered: a full `/presentations` resource with content-type dispatch, markdown rendering, a built-in webview. Rejected for v0 because porthole has no OS-level differences to paper over for rendering — a higher-level tool layered on top (or inside the flotilla yeoman) is the better home. Porthole ships primitives (attention, placement, artifact launch, replace) rich enough that such a tool can be thin.

## 14. Success Criterion

From the experience report, preserved verbatim because it remains the right bar:

> It should make the exact workflow from this session feel boring.

If an agent can, with porthole: launch an app with a lifecycle, know which window is the one it cares about, drive it with targeted input, capture screenshots whose metadata explains what they are, and tag the whole thing under a session string — and never write a shell wrapper or enumerate the desktop — v0 has done its job.
