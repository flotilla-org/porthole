# Recipe: drive a terminal end-to-end

This is a working agent-facing walkthrough for the most common porthole workload: launch a terminal emulator, run a known program inside it, take screenshots at known moments, exercise reflow / scrollback, and clean up. The motivating workload is a kitty-graphics-protocol conformance test suite, but the same shape applies to any terminal automation.

Assumptions:

- Daemon (`portholed`) is running via the bundled `.app` (so TCC grants are stable). Run `porthole onboard` first.
- A terminal emulator is installed. Examples below use Ghostty (`/Applications/Ghostty.app`), but Terminal.app, iTerm2, kitty, and Alacritty work the same way.
- You have a `surface_id` from `porthole launch`. All other verbs target that surface.

## The verb sequence

| Step | Verb | What it does | When to use |
|---|---|---|---|
| 1 | `launch --kind process --app /Applications/X.app` | Start a fresh window. Returns `surface_id`. | Once per test run. |
| 2 | `focus <surface_id>` | Bring window to front so input lands. | Defensively, before any input burst. |
| 3 | `wait <surface_id> --condition stable --window-ms 1500` | Block until the frame is unchanged for 1.5s, ignoring small pixel changes (cursor blink). | After launching, before screenshots, between input bursts. |
| 4 | `send-keys <surface_id> 'cmd' Enter` | Tmux-style sequence: text, named keys, and modifier-prefixed keys in one call (`C-c`, `Cmd-Tab`, `M-x`). The recommended input primitive. Convenience aliases: `send <id> 'cmd'` (text + Enter) and `interrupt <id>` (Ctrl+C). Lower-level: `text` and `key` map 1:1 to the wire endpoints when you don't want parsing. |
| 5 | `screenshot <surface_id> --out file.png` | PNG capture of the window. | At known points; the inner test script tells the harness when. |
| 6 | `place <surface_id> --x N --y N --w N --h N` | In-place resize/move. **Surface identity preserved**, inner process unaffected. | Reflow tests, before/after geometry comparisons. |
| 7 | `close <surface_id>` | Clean shutdown. | End of test run. |

## End-to-end shell example

This script exercises every verb above against a real Ghostty window. It assumes `porthole` is on `PATH` — either via the bundle's symlink (`~/.local/bin/porthole` once phase-1 install lands) or by adding `target/debug/Porthole.app/Contents/MacOS` to `PATH`.

```bash
#!/usr/bin/env bash
set -euo pipefail
GHOSTTY=/Applications/Ghostty.app
OUT=/tmp/porthole-recipe
mkdir -p "$OUT"

# 1. Launch a fresh window. --json gives us a stable surface_id to script against.
SID=$(porthole launch --app "$GHOSTTY" --kind process --json | jq -r .surface_id)
echo "surface_id=$SID"

# 2. Focus + wait for prompt to settle.
porthole focus "$SID"
porthole wait "$SID" --condition stable --window-ms 1500 --threshold-pct 1.0

# 3. Type a known command. The exit code from `seq` is irrelevant; we want pixels.
porthole send "$SID" 'printf "hello porthole\n"; seq 1 80'
porthole wait "$SID" --condition stable --window-ms 1500 --threshold-pct 1.0
porthole screenshot "$SID" --out "$OUT/01-after-output.png"

# 4. Scrollback: Cmd+ArrowUp three times, then capture.
porthole send-keys "$SID" Cmd-Up Cmd-Up Cmd-Up
porthole wait "$SID" --condition stable --window-ms 1000
porthole screenshot "$SID" --out "$OUT/02-scrolled.png"

# 5. Reflow: shrink the window to half-width, screenshot, restore.
porthole place "$SID" --x 100 --y 100 --w 500  --h 800
porthole wait  "$SID" --condition stable --window-ms 1000
porthole screenshot "$SID" --out "$OUT/03-narrow.png"

porthole place "$SID" --x 100 --y 100 --w 1200 --h 800
porthole wait  "$SID" --condition stable --window-ms 1000
porthole screenshot "$SID" --out "$OUT/04-wide.png"

# 6. Done.
porthole close "$SID"
echo "outputs: $OUT"
```

The script in the repo at `scripts/manual-terminal-smoke.sh` is essentially this, runnable. Use it as a one-shot sanity check before assuming porthole is healthy.

## Inner-script ↔ harness signalling

For automated test suites, you usually want the *inner* process to tell the *outer* harness when a test phase has settled — independent of frame stability, because some content changes (animated cursors, network spinners) never settle. Standard patterns:

- **UDS callback channel**: the harness opens a Unix Domain Socket the inner test connects to. Inner sends `{"phase": "before-screenshot", "name": "test-12-image"}`; harness calls `porthole screenshot` and replies `ok`.
- **Sentinel files**: inner writes `/tmp/porthole-test-N.ready`; harness watches via `inotify`/`fsevents` (or a polling loop) and calls `screenshot` on appearance.
- **Log markers**: inner emits `=== porthole-mark: phase-N ===` to stdout; harness tails the terminal's output stream and reacts on the marker.

**This is outside porthole's scope.** Porthole's contract is "given a surface_id, give me a PNG." The when, the why, and the inter-process signal are the harness's design space. The recipe above shows every porthole verb the harness needs; the rest is glue.

## Common pitfalls

- **Forgetting `focus`**. AX input goes to the focused app. If another window steals focus between `text` calls, your text lands in the wrong place. Re-`focus` defensively.
- **Confusing `place` with `replace`**. `replace` *closes* the surface and launches a new one in its slot, minting a new `surface_id`. The inner shell process dies. For reflow, use `place`.
- **Tight `wait` thresholds on terminals with cursor blink**. `--threshold-pct 1.0` ignores the blinking-cursor diff. Lower than 0.1 will time out forever.
- **`scroll` vs scrollback keys**. `scroll` simulates a mouse wheel. Most terminals' scrollback is keyboard-bound (`Cmd+ArrowUp`, `Shift+PageUp`), so prefer `key` with modifiers for that.
- **Screen Recording permission**. Wait conditions that read pixels (`Stable`, `Dirty`, `TitleMatches`) need it; `Exists` and `Gone` do not. If a wait fails with `system_permission_needed`, run `porthole onboard`.
- **`send-keys` token semantics for single chars**. A bare single ASCII character or digit (e.g. `send-keys SID c` or `send-keys SID 5`) is typed as **literal text**, not a keypress. Use `KeyA`/`KeyC`/`Digit5` (DOM names) when you want a key event without a modifier, or `C-c`/`Cmd-1` when you have a modifier. Multi-token sequences coalesce text fragments with single-space separators, so `send-keys SID hello world Enter` types "hello world" then presses Enter.
