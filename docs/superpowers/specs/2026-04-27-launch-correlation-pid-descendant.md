# Launch correlation: PID-descendant fallback

**Status:** Spec.
**Closes:** #10.
**Author:** rjwittams.
**Date:** 2026-04-27.

## Problem

`porthole launch --app /Applications/WezTerm.app` fails with `launch_correlation_failed` even though the window opens, is owned by a known PID, and is straightforwardly findable via `porthole search`. The current correlation strategy (env-var tag injection, surfaced via `ps eww`) doesn't see WezTerm's window because the window-owning process either doesn't carry the launch env or is reached via macOS XPC plumbing rather than a direct fork.

This pattern is the rule, not the exception, for terminals: WezTerm, kitty, iTerm2 (`single_instance` mode), and Terminal.app all have either multi-process architectures or LaunchServices-mediated activation that breaks naive env-var inheritance assumptions. The kitty-graphics-protocol harness (the immediate use case for porthole) lives or dies on launching arbitrary terminals reliably.

The current workarounds — `--require-confidence weak` and "launch externally then `attach`" — both work but neither is a sane default. An agent author should not need to know that "terminal" is a special case.

## Current strategy

`crates/porthole-adapter-macos/src/launch.rs::launch_process`:

1. Generate a random tag (e.g. `plt_a3f8c1...`).
2. Spawn the app with `PORTHOLE_LAUNCH_TAG=<tag>` in its environment via `NSWorkspace.openApplication` (or direct `Command::spawn` for non-bundle paths).
3. Loop until the launch deadline:
   - Enumerate windows via `CGWindowListCopyWindowInfo`.
   - For each window's `owner_pid`, run `ps eww -o command= -p <pid>` and look for `PORTHOLE_LAUNCH_TAG=<tag>` in the printed env block.
4. First match wins → `Confidence::Strong`, `Correlation::Tag`. No match by deadline → `launch_correlation_failed`.

The tag-injection plumbing is fine. The "find a window whose PID owns this env var" step is what breaks for apps where the window's PID is not the spawned PID's direct line.

## Design options considered

### A. PID-descendant correlation
After spawn, get the launched root PID. Walk descendant PID tree (`proc_listpids` / `proc_pidinfo` filtered by `PROC_PIDLISTCHILDREN`). Filter `CGWindowListCopyWindowInfo` by `owner_pid ∈ descendants`. Unique match → strong; multiple → ambiguous; none → fail.

- **Works for**: terminals that fork the shell as a child, any normally-launched GUI app where the window-owning process is the launched PID itself, multi-process terminals where the worker PID is a descendant of the launcher.
- **Breaks on**: terminals where the worker is a *sibling* of the launched PID (rare on macOS — the macOS process model is hierarchical), or where the window appears before the descendant tree settles (race).

### B. NSWorkspace PID + window watch
`NSWorkspace.openApplication` returns an `NSRunningApplication` with an authoritative `processIdentifier`. Watch CGWindowList for new windows whose `owner_pid == that PID`.

- **Pros**: zero PID-tree gymnastics.
- **Cons**: doesn't help WezTerm specifically. The launcher PID is the stub; the window owner is the long-lived worker. Strategy reduces to "exact PID match" which fails for the same reason as the tag strategy fails — neither sees the worker.

This is a strict subset of (A) for most cases and a broken case for terminals. Drop.

### C. AX `AXWindowCreatedNotification` observer
Subscribe via `AXObserver` for the launched PID. React to the notification with the new window's `AXUIElement`, extract `kAXWindowAttribute` chain, attach surface_id.

- **Pros**: authoritative — the OS itself signals the new window. No CGWindow polling, no PID-tree walking.
- **Cons**: requires AX permission (we have it), needs CFRunLoop integration in our tokio runtime (solvable but non-trivial), has subscribe-after-window-created races, and the notification target is the *application* — the window owner. Same problem as (B): for WezTerm, subscribing to the launched PID's AX observer doesn't see windows created by the worker process.

Could be combined with (A): subscribe to the AX observer of *every descendant PID* as we discover them. Heavier plumbing, marginal improvement over polling CGWindowList. Defer.

### D. Tag injection (current)
Existing strategy. Works for apps that surface env vars in AX or whose window-owning PID has the env in its `ps eww` output. Specifically broken for terminals.

## Chosen design

Layered fallback within a single launch attempt, ordered by signal quality:

1. **Tag injection (D)** runs first. Cheap, fast for apps that surface the tag. If a window with the tag is found within `tag_deadline_ms` (initial budget: 1500ms; tunable via spec field on `ProcessLaunchSpec`), return early with `Confidence::Strong`, `Correlation::Tag`.

2. **PID-descendant fallback (A)** kicks in if (1) hasn't matched by `tag_deadline_ms`. Compute the descendant PID set (recursive `proc_listpids(PROC_PIDLISTCHILDREN, ...)`) of the spawned root PID. Filter the current `CGWindowListCopyWindowInfo` snapshot by `owner_pid ∈ descendants`:
   - Exactly one window-owning descendant PID with at least one window → `Confidence::Strong`, `Correlation::PidDescendant`.
   - Multiple descendant PIDs each owning windows → `Confidence::Plausible`, `Correlation::PidDescendant`, with the candidate list returned in the launch outcome (same shape as the existing ambiguous case). The caller can pick via `attach`.
   - Zero matches → keep waiting until the launch timeout, then `launch_correlation_failed` (current behaviour).

3. **`--require-confidence` continues to gate** the failure threshold:
   - `strong` (default) — accept tag match OR single descendant match. Reject multiple-descendant ambiguity.
   - `plausible` — also accept multiple-descendant ambiguity (returns the strongest candidate; caller can re-attach to a different one if wrong).
   - `weak` — accept anything CGWindow lists with descendant ownership, even if multiple. Returns the topmost.

This gives terminals a reliable default path without requiring agent authors to know the WezTerm-specific quirk. `--require-confidence weak` stops being a workaround for "the tag isn't there" and goes back to its intended meaning of "I'll take any reasonable match."

## New types and wire shape

### `Correlation` enum gains `PidDescendant`
Currently:
```rust
pub enum Correlation {
    Tag,
    AxDocumentUrl,
    AxDocumentPath,
    Frontmost,
}
```
Add `PidDescendant`. Wire serialisation `pid_descendant` (snake_case to match existing variants).

### `ProcessLaunchSpec` gains `tag_deadline_ms: Option<u64>`
Default `1500` if `None`. Lower for fast-path apps (tag-only callers); higher to be patient with slow-launching ones. Capped at `timeout_ms`.

### Per-window descendant lookup helper
`crates/porthole-adapter-macos/src/correlation.rs::descendant_pids(root: u32) -> Result<HashSet<u32>, PortholeError>`. Recursive `proc_listpids` until convergence. Bounded depth (16) to avoid pathological PID-cycle tar pits (which shouldn't exist but cheap to guard).

## Edge cases

### Apps that already had a window before we launched
LaunchServices may activate an existing window instead of creating a new one. Today this is detected via `surface_was_preexisting` based on the tag's absence in any newly-spawned process. With descendant correlation:

- If the matched window's `owner_pid` is *not* a descendant of our spawned root (because the spawn didn't happen — LaunchServices reused), we should not claim the match. The descendant-set membership check naturally rejects pre-existing windows.
- If the user passed `require_fresh_surface: true` and we get zero descendants with windows after the deadline, the launch fails with `launch_correlation_failed` — same as today.

### Multi-window apps
Some apps (Finder, Preview, browsers) spawn multiple windows on launch. The descendant set may have one PID owning multiple windows. Pick:
- The newest window (by some heuristic like `kCGWindowMemoryUsage` ordering, which approximates creation order in CG's list — fragile but widely used).
- Or the one that became frontmost (NSWorkspace's `frontmostApplication` cross-reference).

For v0 of this fix: pick the most recently-created window in the matched PID's window list. Document the heuristic and the workaround (`--require-confidence weak` returns the candidate list).

### Fork-then-exit launchers
Some apps (older `open` shims, third-party launchers) `fork` to do the real launch and the parent exits. The "spawned root PID" is gone before correlation runs.

Mitigation: the descendant set computation should treat a missing root PID as "use the most-recently-spawned process group" — fall back to the launcher's PID lineage via `getpgid`. If even that fails, `launch_correlation_failed` is the right answer; the caller should `attach` instead.

For v0: if the spawned PID is gone within the tag deadline, don't run descendant correlation; emit `launch_correlation_failed` with a hint in the error details. Better fallback heuristics are follow-up work.

### `require_fresh_surface: true` interaction
The descendant-set check naturally satisfies "fresh surface" — a window owned by a non-descendant PID can't be a fresh launch we made. No special-case needed.

## Implementation outline

Single PR, atomic commits:

1. **`feat(adapter-macos): descendant_pids helper + correlation::Correlation::PidDescendant`** — new pure FFI helper using `proc_listpids` (libproc). Add the `Correlation::PidDescendant` variant with serde rename, update wire `Correlation` mirror in protocol crate. Unit tests against a mocked `proc_listpids` (hand-rolled fake).

2. **`feat(adapter-macos): wire PID-descendant fallback into launch_process`** — add the layered fallback after the tag-injection deadline. Honor `tag_deadline_ms` from `ProcessLaunchSpec` (defaulting to 1500ms). Return the new `Correlation` in the `LaunchOutcome`. Per-confidence-level acceptance/rejection logic.

3. **`docs(recipes): drop the WezTerm workaround notes`** — update `docs/recipes/terminal-orchestration.md` and `scripts/manual-terminal-smoke.sh` to assume default `--require-confidence strong` works for any terminal.

## Test plan

- **Unit (in-memory)**: scriptable correlation outcomes — `InMemoryAdapter::set_next_descendant_pids(pids)` returning the descendant set; route the launch through the same correlation logic; assert per-confidence-level acceptance.
- **Unit (real FFI, ignored by default)**: `proc_listpids` against `std::process::id()` finds at least the current process. Smoke check; not a real-world coverage test.
- **Manual on real hardware**: `porthole launch --app /Applications/WezTerm.app` with default flags succeeds and returns a usable `surface_id`. Repeat for Ghostty, kitty, iTerm2, Terminal.app. The kitty-harness recipe runs end-to-end without `--require-confidence weak`.

## Out of scope

- AX `AXWindowCreatedNotification` observer (strategy C). Heavier plumbing; the descendant fallback should cover the kitty-harness use case. Revisit if real-world testing surfaces correlation flakiness this design doesn't fix.
- LaunchServices "reuse existing window" handling beyond the `surface_was_preexisting` flag. That's a deeper rework of the launch flow; this PR scopes to "broaden what counts as a fresh match."
- Cross-platform: Hyprland/X11/Wayland correlation strategies are different problems entirely. This spec is macOS-specific.
