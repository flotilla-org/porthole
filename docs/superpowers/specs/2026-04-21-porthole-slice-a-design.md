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

Verbs on tab surfaces continue to return `capability_missing` (unchanged from v0 spec §4.1).

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
  "condition": { "type": "stable", "window_ms": 500 }
             | { "type": "dirty" }
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

Response on timeout: `wait_timeout` error with `last_observed` diagnostics (for `stable`/`dirty`, the time since the last detected hash change; for `exists`/`gone`, whether the surface was present at timeout; for `title_matches`, the last observed title).

### 5.1 Condition semantics

- **`exists`** — returns as soon as the tracked surface is in `Alive` state. Cheap; polls the handle store at short intervals.
- **`gone`** — returns as soon as the tracked surface transitions to `Dead`. Polls the same store. Useful for "wait for this dialog to close."
- **`title_matches`** — polls AX window title, tests regex. Regex is Rust `regex` crate syntax. Compiled once per call.
- **`stable: { window_ms }`** — repeatedly screenshot and hash pixel bytes; return when the hash has not changed for `window_ms` consecutive milliseconds.
- **`dirty`** — screenshot-hash; return on first change from the initial sample.

Sampling interval for screenshot-hash waits: 100 ms fixed. This is expensive (each sample is a full window capture) and is the known-slow implementation. When SSE events land in a later slice, `stable` and `dirty` swap to AX-observer-backed implementation without any wire-contract change. `GET /info` advertises `"wait"` as a present capability and `"wait_events_native"` as absent, so callers that care can detect this.

### 5.2 Timeouts and defaults

- `timeout_ms` is required in the design intent but has a default of 10 000 ms in the wire type (`#[serde(default)]`) so callers don't have to pass it every call.
- A wait that exits via timeout returns `wait_timeout`, not success. Callers that genuinely want "wait up to N ms then proceed regardless" must handle the error explicitly.

### 5.3 Input ordering

Verbs are independent HTTP calls; there's no implicit ordering. Agents that want "send Enter, wait for repaint, screenshot" compose three calls:

```
POST /surfaces/{id}/key      { events: [{ key: "Enter" }] }
POST /surfaces/{id}/wait     { condition: { type: "dirty" }, timeout_ms: 2000 }
POST /surfaces/{id}/screenshot
```

This is the canonical evidence loop. Three HTTP calls per repaint captured. Clear in logs, deterministic, matches the v0 spec §7.3.

## 6. Running commands in a terminal

This slice does **not** add a "run this command at launch" facility to the launch path. The intended model is:

1. `POST /launches` opens the terminal with its default state — an interactive shell.
2. The agent uses `POST /surfaces/{id}/text` (and `POST /surfaces/{id}/key` for modifiers / Enter) to type commands into that shell.
3. Standard shell semantics then handle keep-alive: the shell stays interactive as long as it's open, the window stays until closed.

What the v0 spec's lifecycle modes tried to express becomes a composition of primitives shipped by this slice:

| v0 spec lifecycle intent | Realized as |
|---|---|
| `exit_on_command_end` | Type command, observe it exit, shell stays; close window via `/close` when done. |
| `keep_alive_duration: N` | Type command, `wait { stable }` to see it finish, optionally `wait` further, then `/close`. |
| `keep_alive_interactive` | Default — the shell is already interactive. |
| `keep_alive_until_closed` | Default — the window stays until `/close`. |

### 6.1 Rationale

- **Less special-casing.** No terminal-app bundle allowlist, no per-terminal `-e`/`--command`/AppleScript adapters. The same input path works for every terminal that displays an interactive shell.
- **Fewer permissions.** Input injection via AX requires Accessibility only. Per-terminal launch-with-command typically requires macOS Automation permission on top (AppleScript-driven Terminal.app and iTerm2, in particular). We don't want to ask for more than we need.
- **Closer to what a human does.** A human opens a terminal and types — porthole does the same. The mental model is direct.
- **More flexible.** The agent composes commands from running results. Lifecycle modes baked the whole interaction into one launch call.

The cost is one or two extra round trips per agent workflow (type + wait, rather than launch-with-cmd). Acceptable in every use case we've identified so far.

## 7. close and focus

### 7.1 close

`POST /surfaces/{id}/close` takes an empty body and returns `{ "surface_id": "...", "closed": true }`.

Implementation on macOS: find the AX close button (`AXCloseButton` subrole) and perform `AXPress`. If not available, fall back to focusing the window and sending `Cmd+W` via the input path. If the window hosts a terminal in a keep-alive mode, the shell's parked loop is killed with the window.

Error codes:
- `surface_dead` if already closed
- `permission_needed` if AX can't act on the element
- `capability_missing` on tab targets

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

### 8.3 No caching

Both endpoints recompute from the OS on every call. CoreGraphics + NSWorkspace reads are sub-millisecond; caching would introduce staleness without meaningful savings.

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
- **Tab-surface verbs** — still return `capability_missing` per v0 spec §4.1. Promoting tab verbs from metadata-only to first-class is a larger piece of work.
- **Native event-backed `wait`** — polling is honest v0.x; SSE-events slice (later) swaps the implementation. The wire contract does not change.
- **`/events` SSE stream** — separate slice.
- **Attach mode** — separate slice.
- **Artifact launches, placement, replace, auto-dismiss** — presentation slice.
- **Cross-host transport** — not an issue at this layer yet.
- **Lifecycle modes on launch** — per §6, replaced by input-injection composition. The v0 spec's enum stays dormant until a concrete use case arises.
- **Command-at-launch facility for terminals** — agents type commands after launch via `/text`. No per-terminal AppleScript/`-e` paths in this slice.
- **Pixel-level scroll** — line deltas are what agents need; pixel scroll can be added as a variant later.

## 12. Open questions

- Whether `unknown_key` should be a soft error (log + skip) or hard. Going hard here because silent skipping is the exact failure mode agent-first design should avoid.
- How `wait stable`/`dirty` handles mostly-unchanged frames (e.g., a blinking cursor). Pixel-hash will count that as dirty; callers that want to ignore cursor-blink will need longer `window_ms`. When event-backed wait lands, this goes away because AX doesn't fire for cursor blink.
- Whether to eventually add a `command` field to launches for the very specific case of a user wanting a one-shot non-terminal command pipeline. Currently no — input injection covers it.

## 13. Success criterion

The Ghostty kitty-graphics evidence-collection workflow from the original experience report can be expressed as a straight-line script of porthole calls, with no shell wrappers, no `sleep` commands, no global desktop enumeration, and deterministic before/after screenshots correlated by session tag.

When that script reads as boring — `launch → text "python3 repro.py cell\n" → wait stable → screenshot → key Enter → wait dirty → screenshot → close` — this slice has done its job.
