# Porthole Slice A — Evidence Loop Completion — Design

Date: 2026-04-21
Status: Draft for review
Supersedes: nothing; extends `docs/superpowers/specs/2026-04-20-porthole-design.md`
Builds on: `docs/superpowers/plans/2026-04-21-porthole-v0-foundation.md` (merged into `main`)

## 1. Purpose

Complete the agent-drives-and-captures loop that motivated porthole in the first place. The v0 foundation shipped `launch` and `screenshot`. This slice adds the input, timing, and surface-lifecycle primitives (close, focus) an agent needs to run the actual evidence-collection workflow from the original experience report — plus two cheap read-only resources (`/attention`, `/displays`) that unblock future presentation work.

Scope is "Profile 2 of Slice A" as agreed in brainstorm, refined during spec review to drop lifecycle modes: the full set of input verbs, a usable but deliberately polling-based `wait`, `close`, `focus`, and the two read-only resources. `focus: "preserve"` is explicitly out.

**Refinement from brainstorm:** the v0 design spec's lifecycle modes (§6.3 of that spec — `keep_alive_duration`, `keep_alive_interactive`, `keep_alive_until_closed`) are *not* delivered by this slice and are likely not needed at all. With input injection landing here, an agent runs commands in a terminal by `POST /surfaces/{id}/text`ing them into an interactive shell — exactly how a human does it. The shell's natural behavior subsumes all three keep-alive variants; "wait N ms before closing" is `POST /surfaces/{id}/wait { stable } ; POST /surfaces/{id}/close`. Avoiding lifecycle wrapping also avoids per-terminal launch paths (Ghostty `-e`, Terminal.app AppleScript, iTerm2 scripting bridge), each of which would have required additional macOS permissions (Automation, not just Accessibility). The v0 design spec's lifecycle enum is revisited if a use case emerges that input injection cannot express.

## 2. Relationship to existing design

This slice adds verbs and resources. It does **not** change any contract already shipped:

- Existing error codes, `SurfaceId`, `LaunchRequest`/`LaunchResponse`, `ScreenshotResponse`, HTTP-over-UDS transport, handle lifecycle — unchanged.
- One additive schema extension: `AdapterInfo` grows an optional `permissions: Vec<PermissionStatus>` field. See §8.3. Existing callers that don't read it are unaffected (serde default).
- `Adapter` trait gains new methods, but existing methods keep their signatures. The in-memory adapter grows new scripting hooks alongside the existing ones.
- The v0 design spec's "out of scope" list contracts; items it pulls in are explicitly named below.

Items from the v0 spec explicitly delivered by this slice:

- §4 operations: `key`, `text`, `click`, `scroll`, `wait`, `close`, `focus`
- §4 resources: `/attention` (read-only v0.1-ish), `/displays` (read model)
- §7.1 input model: as specified, minus the `focus: "preserve"` opt-in
- §7.3 sequencing: orthogonal `wait` verb, polling implementation

Items from the v0 spec still deferred after this slice:

- Events SSE stream, `/events` resource
- Attach mode (`/surfaces/search` + `/surfaces/track`)
- Artifact launch kind, placement, `replace`, `auto_dismiss_after_ms`
- Tab verbs (including any input/wait/replace on tab surfaces)
- Recording
- `focus: "preserve"` no-focus-steal input
- AX-element-reference targeting for click/scroll
- Cross-host routing

Items from the v0 spec likely not needed given input injection:

- §6.3 lifecycle modes (`keep_alive_duration`, `keep_alive_interactive`, `keep_alive_until_closed`). Kept on the deferred list, not the explicitly-planned list. If a concrete use case emerges that input injection does not cover, the v0 spec revisits the enum; until then it's dormant.

## 3. New resources and endpoints

```
POST /surfaces/{id}/key        — keyboard events
POST /surfaces/{id}/text       — literal text
POST /surfaces/{id}/click      — mouse click
POST /surfaces/{id}/scroll     — scroll
POST /surfaces/{id}/wait       — wait on condition
POST /surfaces/{id}/close      — close surface
POST /surfaces/{id}/focus      — focus surface
GET  /attention                — focus / cursor / recent
GET  /displays                 — monitors
```

`LaunchRequest` is unchanged by this slice. `InfoResponse.adapters[].capabilities` grows these entries: `input_key`, `input_text`, `input_click`, `input_scroll`, `wait`, `close`, `focus`, `attention`, `displays`. Capability advertisement lets callers detect what an adapter actually supports without probe-and-fail.

Tab surface handling: tab surfaces are still deferred (not enumerated by the foundation; no tab enumeration added here either), so in practice this slice's verbs only ever operate on window surfaces. When tab surfaces do land in a later slice, the v0 spec's §4.1 matrix applies — `focus` activates the tab and brings its window forward, `screenshot` activates the tab and captures the whole window, and `close` closes just the tab (not the window); the input verbs and `wait` return `capability_missing` on tabs. This slice does not change that matrix.

## 4. Input model

### 4.1 Keyboard

`POST /surfaces/{id}/key` body:

```json
{
  "events": [
    { "key": "KeyA", "modifiers": ["Cmd"] },
    { "key": "Enter" }
  ],
  "session": "optional-tag"
}
```

- `key` is a DOM `KeyboardEvent.code`-style identifier: `"Enter"`, `"Escape"`, `"Space"`, `"Tab"`, `"Backspace"`, `"ArrowUp"`/`"ArrowDown"`/`"ArrowLeft"`/`"ArrowRight"`, `"Home"`/`"End"`, `"PageUp"`/`"PageDown"`, `"F1"`–`"F12"`, `"KeyA"`–`"KeyZ"`, `"Digit0"`–`"Digit9"`, plus punctuation codes (`"Minus"`, `"Equal"`, `"Comma"`, `"Period"`, `"Slash"`, `"Semicolon"`, `"Quote"`, `"Backquote"`, `"BracketLeft"`, `"BracketRight"`, `"Backslash"`).
- `modifiers` is a subset of `["Cmd", "Ctrl", "Alt", "Shift"]`. Order does not matter.
- `events` is applied in order; each event fires down + up before the next. Single-event calls are common; the array accommodates sequences like `Ctrl+C` follow-ups without a round trip per keystroke.

Rationale for DOM-style names: stable across keyboard layouts (`"KeyA"` always targets the physical A-position regardless of AZERTY/QWERTY), already familiar to every web-capable agent, and cleanly mappable to macOS virtual key codes via a short table.

### 4.2 Text

`POST /surfaces/{id}/text` body:

```json
{ "text": "hello, world", "session": "optional-tag" }
```

Literal Unicode text typed as if the user typed it. Implementation uses `CGEventKeyboardSetUnicodeString` — handles non-ASCII, emoji, paste-style input that would be awkward to express as a sequence of `key` events.

### 4.3 Click and scroll

`POST /surfaces/{id}/click` body:

```json
{
  "x": 120.0,
  "y": 440.0,
  "button": "left" | "right" | "middle",
  "count": 1,
  "modifiers": [],
  "session": "optional-tag"
}
```

- Coordinates are **window-local logical points** (not pixels, not screen-global). The adapter translates to screen coordinates using the current window geometry.
- `count` supports double-click (2) and triple-click (3).
- `button` defaults to `"left"`, `count` to `1`.

`POST /surfaces/{id}/scroll` body:

```json
{
  "x": 120.0,
  "y": 440.0,
  "delta_x": 0.0,
  "delta_y": -120.0,
  "session": "optional-tag"
}
```

- Coordinate is where the scroll happens (window-local logical points).
- Deltas are in **line units** (positive = down/right, negative = up/left). One line ≈ the OS's native scroll-wheel click. Pixel-level scroll is deferred; line-level is what every agent workflow needs for list/log navigation.

AX-element-reference targeting is not exposed in this slice. Coordinates are sufficient for the evidence-collection and presentation uses on the immediate horizon. When element-reference targeting does land, it will be a new optional field on these verbs; coordinates stay.

### 4.4 Focus behavior

All four input verbs **focus the target surface before injecting**. Focus = AX raise on the window + `NSRunningApplication.activate`. The focus change is a user-visible side effect; callers who want to avoid it cannot in this slice.

`focus: "preserve"` is out of scope (see §11). Injecting without focus on macOS is unreliable — most apps (notably terminals) won't process keystrokes sent to a non-focused window — and the adapter work to do it correctly is not justified by evidence yet.

### 4.5 Errors

- `permission_needed` if Accessibility permission is missing. Returned with a message telling the caller where to grant it. Porthole also surfaces the missing permission in `GET /info`.
- `surface_dead` if the target is dead when the call arrives.
- `capability_missing` on tab targets.

## 5. Wait

`POST /surfaces/{id}/wait` body:

```json
{
  "condition": { "type": "stable", "window_ms": 1500, "threshold_pct": 1.0 }
             | { "type": "dirty", "threshold_pct": 1.0 }
             | { "type": "exists" }
             | { "type": "gone" }
             | { "type": "title_matches", "pattern": "regex" },
  "timeout_ms": 10000,
  "session": "optional-tag"
}
```

Response on success:

```json
{ "surface_id": "...", "condition": "stable", "elapsed_ms": 823 }
```

Response on timeout: `wait_timeout` error with `last_observed` diagnostics (for `stable`/`dirty`, the time since the last significant frame change and the observed change fraction; for `exists`/`gone`, whether the surface was present at timeout; for `title_matches`, the last observed title).

### 5.1 Condition semantics

- **`exists`** — returns as soon as the tracked surface is in `Alive` state. Cheap; polls the handle store at short intervals.
- **`gone`** — returns as soon as the tracked surface transitions to `Dead`. Polls the same store. Useful for "wait for this dialog to close."
- **`title_matches`** — polls AX window title, tests regex. Regex is Rust `regex` crate syntax. Compiled once per call.
- **`stable: { window_ms, threshold_pct }`** — repeatedly screenshot and compare frames. Returns when no frame in the last `window_ms` differed from its predecessor by more than `threshold_pct` of pixels.
- **`dirty: { threshold_pct }`** — repeatedly screenshot. Returns on the first frame whose difference from the initial sample exceeds `threshold_pct`.

### 5.2 Why `threshold_pct`

Terminals, video players, progress indicators, and anything else with a blinking cursor or animated glyph continuously change a tiny fraction of pixels. A naive "any hash change counts" policy would see those as perpetually dirty and never stable, livelocking the canonical evidence loop the moment it targets a terminal.

`threshold_pct` is the fraction of pixels (0–100) that must change between consecutive samples to count as a real change. Defaults:

- `stable.window_ms`: 1500 ms
- `stable.threshold_pct`: 1.0
- `dirty.threshold_pct`: 1.0

A cursor cell is well under 1% of a typical terminal window; a typical command output burst is well over 1%. The defaults work for the canonical loop without ceremony. Callers that need finer sensitivity (e.g., a game or a busy dashboard) tune explicitly.

Implementation: each sample produces a downsampled grayscale fingerprint of the window; consecutive-sample diff is counted at that resolution, then converted to a percentage. Sampling interval is 100 ms fixed.

### 5.3 Future implementation notes

`stable` and `dirty` are pixel-based by nature. AX observers do not fire a "window contents changed" notification, so a future events slice will **not** swap the implementation of these two conditions — they stay pixel-based. Events land for `exists`, `gone`, and `title_matches`, where AX has natural notifications. `GET /info` advertises `"wait"` as a present capability; a future `"wait_events_native"` flag, when added, covers only the conditions events actually can cover.

### 5.4 Timeouts and defaults

- `timeout_ms` is required in the design intent but has a default of 10 000 ms in the wire type (`#[serde(default)]`) so callers don't have to pass it every call.
- A wait that exits via timeout returns `wait_timeout`, not success. Callers that genuinely want "wait up to N ms then proceed regardless" must handle the error explicitly.

### 5.5 Input ordering

Verbs are independent HTTP calls; there's no implicit ordering. Agents that want "send Enter, wait for repaint, screenshot" compose three calls:

```
POST /surfaces/{id}/key      { events: [{ key: "Enter" }] }
POST /surfaces/{id}/wait     { condition: { type: "dirty" }, timeout_ms: 2000 }
POST /surfaces/{id}/screenshot
```

This is the canonical evidence loop. Three HTTP calls per repaint captured. Clear in logs, deterministic, matches the v0 spec §7.3.

## 6. Running commands in a terminal

This slice does **not** add a "run this command at launch" facility to the launch path. `LaunchRequest` already has no lifecycle field (the v0 foundation shipped without one), and this slice keeps it that way. The intended model:

1. `POST /launches` opens the terminal with its default state — typically an interactive shell, determined entirely by how the user has configured that terminal app.
2. The agent uses `POST /surfaces/{id}/text` (and `POST /surfaces/{id}/key` for modifiers / Enter) to type commands into that shell.
3. The window's lifetime is controlled by the agent: `POST /surfaces/{id}/close` when done, or let the user close it, or leave it open.

### 6.1 Why this is different from a lifecycle enum

This is **not** a "realization" of the v0 spec's `keep_alive_*` modes. Those modes were specifically about launching with an embedded command that would otherwise exit-and-close the window. This slice takes a different shape: launch never embeds a command, so there's no command-exit event to wrap around.

The practical consequence for agent workflows:

- **Short repro followed by screenshot:** type the repro command, `wait { stable }` until output stops, `screenshot`, then `close` (or `key` another step).
- **Interactive multi-step:** type a command, read results, decide what to type next. No pre-planning needed.
- **Fire-and-forget command with a linger:** type the command, `wait { stable }` to confirm it ran, then `wait` a fixed window or just `close`.

None of these require the launch call to know anything about a command. The shell handles its own lifetime; the agent handles the window's.

### 6.2 Rationale

- **Less special-casing.** No terminal-app bundle allowlist, no per-terminal `-e`/`--command`/AppleScript adapter.
- **Fewer permissions.** Input injection via AX requires Accessibility only. Per-terminal launch-with-command typically requires macOS Automation permission on top (AppleScript-driven Terminal.app and iTerm2, in particular). We don't ask for more than we need.
- **Closer to what a human does.** A human opens a terminal and types — porthole does the same. The mental model is direct.
- **Composable.** The agent can react to intermediate results and decide what to type next; this isn't possible with a lifecycle-baked-into-launch model.

The cost is one or two extra round trips per agent workflow (type + wait, rather than launch-with-cmd). Acceptable in every use case we've identified so far. If a compelling use case for command-at-launch emerges, it comes back as a separate launch variant — likely a terminal-specific path that explicitly declares the Automation-permission requirement.

## 7. close and focus

### 7.1 close

`POST /surfaces/{id}/close` takes an empty body and returns `{ "surface_id": "...", "closed": true }`.

Implementation on macOS: find the AX close button (`AXCloseButton` subrole) and perform `AXPress`. If not available, fall back to focusing the window and sending `Cmd+W` via the input path. Closing a terminal window terminates the hosted shell the same way the user dismissing the window would — no porthole-specific logic needed.

Error codes:
- `surface_dead` if already closed
- `permission_needed` if AX can't act on the element

Per the v0 spec §4.1 matrix, `close` works on tab targets (activate the tab, then close just the tab) — but tab surfaces are deferred to a later slice, so in this slice `close` only ever runs on window targets.

### 7.2 focus

`POST /surfaces/{id}/focus` takes an empty body and returns `{ "surface_id": "...", "focused": true }`.

Implementation: `AXRaise` on the window + `NSRunningApplication(processIdentifier:).activate(options:)` on the owning process. Idempotent. Succeeds quietly if the surface is already focused.

This is a no-op candidate for later optimization but not in this slice.

## 8. /attention and /displays

### 8.1 GET /attention

Response:

```json
{
  "focused_surface_id": "surf_abc" | null,
  "focused_app_bundle": "com.mitchellh.ghostty" | null,
  "focused_display_id": "disp_1",
  "cursor": { "x": 1200.5, "y": 640.0, "display_id": "disp_1" },
  "recently_active_surface_ids": ["surf_a", "surf_b"]
}
```

- `focused_surface_id` is null when the frontmost OS window isn't one porthole tracks.
- `focused_app_bundle` is always filled when the frontmost app is known (via `NSWorkspace.frontmostApplication`).
- `cursor.display_id` is computed from the cursor position against the current display bounds.
- `recently_active_surface_ids` is ordered most-recent-first, tracks only porthole-managed surfaces. Updated whenever a tracked surface gains focus. Capped at 16; older entries drop.

### 8.2 GET /displays

Response:

```json
{
  "displays": [
    {
      "id": "disp_1",
      "bounds": { "x": 0, "y": 0, "w": 3008, "h": 1692 },
      "scale": 2.0,
      "primary": true,
      "focused": true
    },
    {
      "id": "disp_2",
      "bounds": { "x": 3008, "y": 0, "w": 2560, "h": 1440 },
      "scale": 1.0,
      "primary": false,
      "focused": false
    }
  ]
}
```

- `id` is a stable string per display (backed by CGDirectDisplayID, so stable within a boot).
- `bounds` is logical points; `scale` is the points→pixels factor.
- `primary` matches `CGMainDisplayID`.
- `focused` is derived from `/attention`: the display containing the focused window (or, if no tracked focus, the one containing the cursor).

### 8.3 /info — permission reporting (additive schema change)

The macOS adapter needs Accessibility permission for input and some wait conditions, and Screen Recording permission for capture. This slice extends `AdapterInfo` additively so callers can detect the current grant state without probing. The new field:

```rust
pub struct AdapterInfo {
    pub name: String,
    pub loaded: bool,
    pub capabilities: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub permissions: Vec<PermissionStatus>,
}

pub struct PermissionStatus {
    pub name: String,       // e.g., "accessibility", "screen_recording"
    pub granted: bool,
    pub purpose: String,    // short string explaining why this adapter needs it
}
```

Back-compat: existing callers that don't know about `permissions` are unaffected (`#[serde(default)]` on deserialize, omitted when empty on serialize).

macOS adapter reports:

- `{ name: "accessibility", granted: ?, purpose: "input injection and some wait conditions" }`
- `{ name: "screen_recording", granted: ?, purpose: "window screenshot capture" }`

Permission state is recomputed per `/info` call (calls into `AXIsProcessTrusted` / `CGPreflightScreenCaptureAccess`, both sub-millisecond). No caching.

### 8.4 No caching (attention / displays)

`/attention` and `/displays` recompute from the OS on every call. CoreGraphics + NSWorkspace reads are sub-millisecond; caching would introduce staleness without meaningful savings.

## 9. Error model additions

New error codes:

- `wait_timeout` — `POST /surfaces/{id}/wait` hit the timeout without the condition being satisfied. Body includes `last_observed` diagnostics.
- `unknown_key` — a `key` event passed a name not in the supported set. Body lists the supported set.
- `invalid_coordinate` — `click`/`scroll` coordinates fall outside the window bounds (with tolerance).

Existing codes continue to apply: `surface_not_found`, `surface_dead`, `permission_needed`, `capability_missing`.

## 10. Testing

### 10.1 Core (in-memory adapter)

`InMemoryAdapter` gains scripting hooks for each new verb, mirroring the existing pattern:

- `set_next_key_result` / `key_calls` (recording)
- `set_next_text_result` / `text_calls`
- `set_next_click_result` / `click_calls`
- `set_next_scroll_result` / `scroll_calls`
- `set_next_wait_result` / `wait_calls`
- `set_next_close_result` / `close_calls`
- `set_next_focus_result` / `focus_calls`
- `set_next_attention` / `attention_calls` (zero-arg; counter)
- `set_next_displays` / `displays_calls`

LaunchPipeline-style wrappers for each verb land in `porthole-core`, owning input validation (key-name lookup, coordinate validation), timeout propagation, and error mapping. Each wrapper has table tests against the in-memory adapter.

### 10.2 Protocol

Serde roundtrip tests for every new wire type. Particular attention to the `condition` discriminated union on wait (and its `window_ms` on `stable`) and the `key` event shape.

### 10.3 Daemon

Oneshot `axum::Router` tests exercise each new endpoint against the in-memory adapter. Confirm status codes, error codes, header handling.

### 10.4 macOS adapter

- Unit-testable without a desktop: key-name → virtual-key-code table, coordinate conversion between window-local and screen-global.
- `#[ignore]`-gated integration tests for what needs a real desktop: one per verb, targeting TextEdit for text/key/click/scroll/close/focus and Ghostty or iTerm2 for wait (types text into a shell, waits for dirty/stable, takes a screenshot). These do not run in CI; they are checked manually.

### 10.5 Permissions

The macOS adapter detects missing Accessibility permission on startup and surfaces it in `/info`. Integration tests document the required permission set in their docstrings. CI does not run the gated integration tests, so permission state is irrelevant there.

## 11. Out of scope

Explicitly deferred to later slices (not this one):

- **`focus: "preserve"` no-focus-steal input** — requires adapter-specific paths that vary by app and doesn't have a concrete use case yet.
- **AX-element-reference targeting** for click/scroll — coordinates cover the near-term needs; element refs can be added as an optional alternative field later.
- **Tab surface enumeration and the tab-verb matrix** — the v0 spec §4.1 permits `focus`/`close`/`screenshot` on tabs and returns `capability_missing` for input verbs/`wait`/`replace`. Landing tab surfaces as first-class (enumeration via AX `AXTabs`, activate-the-tab semantics for the supported verbs) is a separate slice. Until then, no tab surfaces exist for any verb to act on.
- **Native event-backed `wait`** for `exists` / `gone` / `title_matches` — polling is honest v0.x. When the SSE-events slice lands, these three conditions can switch to AX-observer-driven notifications without changing the wire contract. `stable` and `dirty` stay pixel-based permanently (AX has no window-contents-changed signal, per §5.3).
- **`/events` SSE stream** — separate slice.
- **Attach mode** — separate slice.
- **Artifact launches, placement, replace, auto-dismiss** — presentation slice.
- **Cross-host transport** — not an issue at this layer yet.
- **Lifecycle modes on launch** — per §6, replaced by input-injection composition. The v0 spec's enum stays dormant until a concrete use case arises.
- **Command-at-launch facility for terminals** — agents type commands after launch via `/text`. No per-terminal AppleScript/`-e` paths in this slice.
- **Pixel-level scroll** — line deltas are what agents need; pixel scroll can be added as a variant later.

## 12. Open questions

- Whether `unknown_key` should be a soft error (log + skip) or hard. Going hard here because silent skipping is the exact failure mode agent-first design should avoid.
- Whether the `threshold_pct` defaults (1.0%) work in practice across the apps we care about. Cursor-blink is a single cell (well under 1%); typical terminal output bursts are well over 1%. A progress bar that flickers only its last character may be near the boundary — a real workflow will tell us whether the defaults need revisiting or whether we should expose finer-grained controls.
- Whether to eventually add a `command` field to launches for the very specific case of a user wanting a one-shot non-terminal command pipeline. Currently no — input injection covers it.

## 13. Success criterion

The Ghostty kitty-graphics evidence-collection workflow from the original experience report can be expressed as a straight-line script of porthole calls, with no shell wrappers, no `sleep` commands, no global desktop enumeration, and deterministic before/after screenshots correlated by session tag.

When that script reads as boring — `launch → text "python3 repro.py cell\n" → wait stable → screenshot → key Enter → wait dirty → screenshot → close` — this slice has done its job.
