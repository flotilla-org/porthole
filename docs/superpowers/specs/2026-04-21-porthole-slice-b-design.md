# Porthole Slice B — Attach Mode — Design

Date: 2026-04-21
Status: Draft for review
Supersedes: nothing; extends the v0 design (`docs/superpowers/specs/2026-04-20-porthole-design.md`) and builds on slice A (`docs/superpowers/specs/2026-04-21-porthole-slice-a-design.md`)

## 1. Purpose

Let a caller take an already-running OS window that porthole did not launch and promote it to a tracked surface. After that promotion, the window is indistinguishable from a launched one — every verb in the existing API works against it.

First-class tested use case: a script finding its own containing terminal window so a test harness can screenshot and drive the window it's running in.

## 2. Relationship to existing design

Mostly additive, with one deliberate rename during the pre-release development phase.

- `Adapter` trait gains two methods (`search`, `window_alive`). Existing methods unchanged.
- `HandleStore` gains one method (`track_or_get`). Existing methods unchanged.
- `WireError` / `ErrorCode` unchanged; the existing `CandidateRefUnknown` error code, reserved by the v0 foundation, is now actually emitted.
- **Rename:** the v0 foundation's `SurfaceInfo.app_bundle` and slice-A's `AttentionInfo.focused_app_bundle` both become `app_name` / `focused_app_name` (they always held `kCGWindowOwnerName`, a human display name, not a true bundle identifier — the old names were a misnomer). Callers of `/attention` from slice-A see the field rename. Acceptable because porthole has not shipped a stable release and both fields just landed; the `no backwards compatibility` phase this workspace is in explicitly permits renames like this. A future additive `app_bundle_id` field via `NSRunningApplication` is not in this slice.
- New wire types for search and track.
- New route endpoints under `/surfaces`.
- `/info` capability list grows `"search"` and `"track"` entries (each adapter declares its own).
- CLI grows `search`, `track`, and `attach` subcommands.

Items from the v0 design §6.4 explicitly delivered by this slice:

- `POST /surfaces/search` returns candidates with opaque `ref` descriptors
- `POST /surfaces/track` promotes a candidate to a tracked surface handle
- Tracked and launched handles are indistinguishable in downstream API

Items from the v0 design still deferred after this slice:

- Events SSE stream
- Artifact launches, placement, replace, auto-dismiss
- Tab surface enumeration — this slice's search returns only window candidates
- Recording
- AX-element-reference targeting, focus-preserve
- Cross-host routing
- Advanced search filters: `on_display`, `bounds_in`, AX-path predicate (`frontmost` is in this slice — see §4.1)

## 3. New resources and endpoints

```
POST /surfaces/search  — enumerate candidates matching a query
POST /surfaces/track   — promote a candidate ref to a tracked surface handle
```

`/info` capability additions (each adapter declares its own via `Adapter::capabilities()`):

- `"search"` — adapter can enumerate matching windows
- `"track"` — adapter can verify a window is alive and mint a tracked handle

## 4. Search

### 4.1 Request

`POST /surfaces/search` body:

```json
{
  "app_name": "Ghostty",
  "title_pattern": "^demo-",
  "pids": [12345, 9876, 1234],
  "cg_window_ids": [42, 58],
  "frontmost": true,
  "session": "optional-tag"
}
```

Every field is optional. Matching is **AND across fields**, **OR within a list**.

- `app_name` — exact match against the OS-reported display name of the owning app (`kCGWindowOwnerName` on macOS, e.g. `"Ghostty"`, `"Safari"`, not a bundle identifier). Note §11 on the deliberate naming and deferred `app_bundle_id` field.
- `title_pattern` — Rust `regex` crate syntax, anchored only if the pattern includes `^`/`$`. Compiled once per call.
- `pids` — list of PIDs; a window matches if its owner_pid is in the list.
- `cg_window_ids` — list of CGWindowID u32s; a window matches if its CGWindowID is in the list.
- `frontmost` — when `true`, narrows the results to the single highest-Z-order window among matches (the one the user is most likely looking at). Common disambiguator for the "my terminal window" case where a single terminal-app PID owns many windows.

Empty-lists mean "no filter for that field." An entirely empty query returns all currently-visible (on-screen) windows. For off-screen / hidden / other-Space windows, see §5.2 on track — a tracked handle remains valid through hide/minimize cycles even though search won't surface hidden candidates. A future `include_hidden` flag can expose offscreen search in a later slice if needed.

### 4.2 Response

```json
{
  "candidates": [
    {
      "ref": "ref_eyJwaWQiOjk4NzYsImNnX3dpbmRvd19pZCI6NDJ9",
      "app_name": "Ghostty",
      "title": "demo-terminal",
      "pid": 9876,
      "cg_window_id": 42
    }
  ]
}
```

Zero candidates returns `{ "candidates": [] }` — never an error. The caller decides what ambiguity or absence means for them.

Candidate ordering follows the adapter's underlying enumeration order. On macOS this is `CGWindowListCopyWindowInfo`'s on-screen-only-above-window order (approximately front-to-back). Callers that need determinism should filter tightly.

### 4.3 Ref format

**Self-encoded**, not cache-backed. The ref carries everything needed to re-identify the surface:

```
ref_<urlsafe_base64(json{"pid": <u32>, "cg_window_id": <u32>, "v": 1})>
```

- `v: 1` is a schema version for forward compatibility.
- No daemon-side cache — refs are stateless, survive daemon restarts, and require no TTL or eviction logic.
- Track decodes the ref, asks the adapter whether the window is still alive, mints a handle if so.

The ref is not intended for direct human consumption or manipulation; callers treat it as opaque. Security is not a concern in v0 (UDS is already per-user and trusts any connected caller).

## 5. Track

### 5.1 Request / response

`POST /surfaces/track` body:

```json
{ "ref": "ref_eyJwaWQ...", "session": "optional-tag" }
```

Response:

```json
{
  "surface_id": "surf_abc",
  "cg_window_id": 42,
  "pid": 9876,
  "app_name": "Ghostty",
  "title": "demo-terminal",
  "reused_existing_handle": false
}
```

### 5.2 Semantics

1. **Decode the ref.** Malformed or unknown-version refs return `candidate_ref_unknown` (existing error code).
2. **Verify the window is alive.** Adapter's `window_alive(pid, cg_window_id)` returns `Some(SurfaceInfo)` if the window still exists, `None` if it's genuinely gone. **Critically, "alive" is broader than "currently on-screen."** The adapter enumerates all windows (via `CGWindowListCopyWindowInfo` *without* `kCGWindowListOptionOnScreenOnly`), so a hidden, minimized, or other-Space window still returns `Some`. Only a closed-and-reaped window returns `None`. This keeps tracked handles valid through normal OS hide/minimize/Space-switch cycles — a handle doesn't silently die when the user `Cmd+H`s the app.
3. **Atomic get-or-insert, keyed by `cg_window_id`.** A single `HandleStore::track_or_get(cg_window_id, SurfaceInfo)` call, holding the store's write lock for the duration, checks for an alive tracked surface with the matching `cg_window_id`:
   - If one exists → return it with `reused_existing_handle: true`.
   - Otherwise → insert the new `SurfaceInfo` and return it with `reused_existing_handle: false`.
   Holding the lock across both steps is load-bearing — two concurrent `POST /surfaces/track` calls for the same window must not both mint fresh handles. The existing `find_by_cg_window_id` + `insert` composition is **not** safe under concurrent requests and is not used here.

### 5.3 Tracked vs launched

The only difference between a tracked and a launched handle is attribution at the creation boundary:

- Launched handles carry `confidence` + `correlation` fields in the launch response.
- Tracked handles don't — there's no correlation to report; the caller explicitly committed to this window via the ref.

After creation, both kinds are equivalent. Every verb — screenshot, key, text, click, scroll, wait, close, focus — works identically.

### 5.4 One handle per window

Idempotent track (§5.2 step 3) enforces a one-to-one rule: at any time there is at most one *alive* tracked handle per `cg_window_id` in a given daemon. If caller A launched the window (minting `H1`) and caller B later tracks it via search, caller B gets back `H1` with `reused_existing_handle: true`. If the window's handle has transitioned to `dead`, a subsequent track mints a new handle — the constraint is on alive handles only.

`HandleStore::find_by_cg_window_id` (used by `/attention` focused-surface resolution) is therefore deterministic: at most one alive handle per CGWindowID, so it either returns that handle or `None`.

Dead handles linger in the store until garbage-collected by a future sweep (not in this slice); a caller that sees `surface_dead` on a handle and wants to re-track should call `/surfaces/search` + `/surfaces/track` again.

## 6. Ancestry walking

The script-finding-its-own-window use case requires walking up the process tree: a bash script's PID is not the terminal window's owning PID. Porthole keeps the daemon simple (accepts a list of PIDs) and does the walk caller-side.

### 6.1 Daemon contract

The daemon's search API accepts `pids: Vec<u32>`. It does not walk process trees. Every caller — CLI, yeoman agent, test harness — is responsible for providing the full list of PIDs they want to match.

This is intentional: walk policies vary by caller (skip through `sudo`, cross user boundaries, stop at session leaders, handle mux-reparenting) and the caller has the context to decide. Keeping the daemon agnostic avoids baking one policy into the substrate.

### 6.2 CLI convenience

Two flag styles on `porthole search` and `porthole attach`:

- `--pid <PID>` — adds a single PID to the list. Repeatable.
- `--containing-pid <PID>` — walks ancestry starting at `<PID>`, adds every ancestor (including `<PID>` itself) to the list.

Example from a shell script:

```sh
surface=$(porthole attach --containing-pid $$ --frontmost --json | jq -r .surface_id)
porthole screenshot "$surface" --out my-window.png
```

The `--frontmost` flag is canonical for the "my terminal window" case because a single terminal-app PID typically owns multiple windows — see §11 for the full rationale and non-frontmost fallback patterns.

### 6.3 Library helper

`porthole::ancestry::containing_ancestors(pid: u32) -> Vec<u32>` lives in the `porthole` CLI crate as a public function. Non-CLI Rust callers (agents, test harnesses) import it directly.

The signature is best-effort infallible — returns a `Vec<u32>` containing as many ancestors as could be walked. Starts with the given pid and appends each ancestor found. Stops at PID 1, at a bounded depth of 128 hops, or at the first `ps` failure. Failures during the walk are logged via `tracing::warn!` but do not abort — a short partial chain is almost always more useful to the caller than an error.

Implementation on macOS / Linux: iteratively shell out to `/bin/ps -o ppid= -p <pid>`, accumulate ancestors. Future optimisation to `libc::proc_pidinfo` on macOS and `/proc/<pid>/stat` on Linux deferred.

### 6.4 Muxes and reparenting

Multiplexers (tmux, zellij, screen) typically reparent their children via `setsid` or daemonization, which breaks the parent chain between the user's shell and the terminal window process. In that situation, `--containing-pid $$` from inside the mux will not find the terminal window because there's no PID ancestry linking them.

This is not a porthole bug — it's a correct reflection of how process trees actually work under muxes. Recovery is caller-side: the caller needs domain knowledge (which mux is running, which session's outer terminal hosts it) to construct the right PID list. The yeoman agent is the natural home for that knowledge; for simple test scripts, running without a mux is the easy path.

## 7. Adapter trait additions

```rust
#[async_trait]
pub trait Adapter: Send + Sync {
    // ... existing methods ...

    /// Enumerate candidate surfaces matching the query.
    async fn search(&self, query: &SearchQuery) -> Result<Vec<Candidate>, PortholeError>;

    /// Return a live SurfaceInfo for the window identified by (pid, cg_window_id)
    /// if it still exists, None if it does not.
    async fn window_alive(&self, pid: u32, cg_window_id: u32)
        -> Result<Option<SurfaceInfo>, PortholeError>;
}
```

New core types (`porthole-core`):

```rust
pub struct SearchQuery {
    pub app_name: Option<String>,
    pub title_pattern: Option<String>,     // regex source; pipeline compiles
    pub pids: Vec<u32>,
    pub cg_window_ids: Vec<u32>,
    pub frontmost: Option<bool>,
}

pub struct Candidate {
    pub r#ref: String,          // self-encoded
    pub app_name: Option<String>,
    pub title: Option<String>,
    pub pid: u32,
    pub cg_window_id: u32,
}
```

New pipeline (`porthole-core`): `AttachPipeline`. Owns:
- Query validation (regex compile, reasonable-size bounds on input lists)
- Candidate ref encoding/decoding
- `window_alive` dispatch
- Atomic idempotent-track via a new `HandleStore::track_or_get` method (see §5.2 step 3). The caller-facing API is a single call that returns `(SurfaceInfo, reused: bool)`.

HandleStore extension (`porthole-core`):

```rust
impl HandleStore {
    /// Atomic get-or-insert keyed by cg_window_id. Holds the write lock
    /// for the whole operation. If a handle with this cg_window_id already
    /// exists in the Alive state, returns it with `reused=true`. Otherwise
    /// inserts `candidate` and returns it with `reused=false`.
    pub async fn track_or_get(&self, candidate: SurfaceInfo) -> (SurfaceInfo, bool);
}
```

The macOS adapter's `search` is built on `list_windows()` (on-screen enumeration) with in-memory filtering; no new OS calls. `window_alive` uses a broader enumeration — `CGWindowListCopyWindowInfo` *without* `kCGWindowListOptionOnScreenOnly` — so hidden, minimized, and other-Space windows still resolve as alive (per §5.2 step 2). Slice A's `wait_for_surface_gone` continues to use the on-screen enumeration it already uses (its semantic is "visible presence changed," which is what callers want for wait `gone`); `window_alive` is a distinct liveness check used only by `/surfaces/track`.

Rename on existing types: the v0 foundation's `SurfaceInfo.app_bundle: Option<String>` is renamed to `app_name` to reflect that it holds `kCGWindowOwnerName` (a human display name), not a bundle identifier. The matching field on `AttentionInfo.focused_app_bundle` is likewise renamed to `focused_app_name`. A true bundle-id field (`app_bundle_id`, populated via `NSRunningApplication.bundleIdentifier`) is a future addition; not in this slice.

This is a wire-compatibility break for anything consuming `AttentionInfo.focused_app_bundle`. Acceptable because slice A's `/attention` has only just shipped and is still explicitly called out as v0.1-ish in the slice-A spec §4.

## 8. Error model additions

No new error codes. Uses existing:

- `candidate_ref_unknown` (already defined in v0 foundation, now actually emitted) — malformed ref, unknown schema version, or payload that isn't a valid base64-encoded JSON blob.
- `surface_dead` — ref decoded fine, but the (pid, cg_window_id) no longer resolves to a live window.
- `invalid_argument` — query contains a malformed regex or unreasonable input size.

Empty search results are **not** an error. An empty `candidates` list is a valid outcome.

## 9. Testing strategy

### 9.1 Core (in-memory adapter)

- `InMemoryAdapter` gains scripting hooks: `set_next_search_result`, `set_next_window_alive_result`, recorders for both.
- `AttachPipeline` unit-tested against the in-memory adapter: ref encode/decode round-trip, malformed-ref error, surface-dead on disappeared window, idempotent track (same window tracked twice → same surface_id with `reused_existing_handle: true`), regex validation.
- HandleStore's `track_or_get`: unit test the single-call happy path, alive-match reuse path, and a concurrent-race test that spawns N tokio tasks all calling `track_or_get` with the same `cg_window_id` and asserts exactly one ends up with `reused=false` and all others with `reused=true`, all returning the same `surface_id`.

### 9.2 Protocol

Serde roundtrip tests for `SearchRequest`, `SearchResponse`, `Candidate`, `TrackRequest`, `TrackResponse`.

### 9.3 Daemon routes

Oneshot axum router tests: search returns candidates, empty search returns empty list, track on a fresh ref mints a handle, track on a stale ref returns `surface_dead`, track on an already-tracked window reuses the handle.

### 9.4 macOS adapter

- Unit-testable: ref encoding, query filter application on a scripted window list.
- `#[ignore]`-gated integration tests:
  - `search_and_track_textedit`: launch TextEdit, search by `app_name: "TextEdit"`, track the candidate, assert the tracked handle is driveable (screenshot + close).
  - `self_find_via_ancestry`: spawn a subprocess that calls `porthole attach --containing-pid $$ --frontmost` against the test daemon and asserts it gets a handle — verifies the ancestry walk + frontmost disambiguation work end-to-end.

### 9.5 CLI

- `porthole search --pid X` and `--containing-pid X` flag parsing tests.
- `porthole attach` exit-code behavior: zero matches (exit 1), one match (exit 0, prints surface_id), multiple matches (exit 1 with error).

## 10. Out of scope

- **`on_display` / `bounds_in` filters** — deferred. The shipped query dimensions cover the important cases; richer filters can layer on without breaking wire compatibility.
- **`include_hidden` search flag** — deferred. Search returns on-screen candidates only; tracked handles remain valid for hidden windows (per §5.2 step 2), but finding a hidden window via search is a future addition.
- **True bundle-id field (`app_bundle_id`)** — deferred. Current slice ships `app_name` (human display name from `kCGWindowOwnerName`). Adding `NSRunningApplication.bundleIdentifier` is a future additive field.
- **AX-path predicate** — deferred; no concrete use case yet.
- **Tab candidates** — search returns only windows. Tab surface enumeration is a separate later slice.
- **Cache-backed refs** — rejected in favor of self-encoded; could swap later if the ref becomes a security boundary.
- **Cross-search correlation** — e.g. "find all surfaces of this app and track them all as a set." Callers do their own batch tracking.
- **Watching for new matches** — e.g. "notify me when a new window matches this query." Belongs to the events slice.
- **Dead-handle garbage collection** — dead handles accumulate in the HandleStore until a later slice adds sweep logic. Memory footprint is minor (one `SurfaceInfo` per dead handle) but noted.

## 11. Known limitations

- **`app_name` is a display name, not a bundle identifier.** `kCGWindowOwnerName` gives "Ghostty", "Safari", "Terminal" — human strings chosen by the app. They're stable in practice but not guaranteed, and they're localized. A future `app_bundle_id` field populated from `NSRunningApplication.bundleIdentifier` will give true stable IDs; for this slice, callers match on display names.
- **Multi-window terminal apps need `frontmost: true` for the "my window" case.** A single terminal app PID commonly owns several windows. `porthole attach --containing-pid $$` without `--frontmost` will often return multiple candidates. The canonical invocation is `porthole attach --containing-pid $$ --frontmost`; the success criterion uses that form. For non-interactive contexts where no window is "frontmost" (e.g., a script running over SSH into a background session), disambiguation is caller-side — usually via title matching or an out-of-band hint the caller planted at launch.
- **Search enumerates every on-screen window** — O(windows) per call. Cheap on typical desktops but could become noticeable on heavily-populated systems. Unlikely to matter.
- **Ref decode reveals (pid, cg_window_id)** — trivially decodable by anyone who can base64-decode. No secrets; see §4.3. If cross-user or cross-process trust boundaries appear later, refs become cache-backed opaque IDs.
- **Mux reparenting breaks ancestry walks** — see §6.4. Caller's problem; documented as a caller-side concern.
- **`reused_existing_handle: true` can be surprising** — the caller asked to track a fresh candidate and got back something that was already tracked. Idempotency is almost always what the caller wants, but the flag is explicit in the response so callers can detect the reuse.

## 12. Success criterion

From the script-finding-its-own-window use case, which this slice makes into a few lines:

```sh
# My script wants to screenshot its own terminal window.
# --frontmost narrows to the single topmost window when the terminal
# app owns several (the common case on macOS).
surface=$(porthole attach --containing-pid $$ --frontmost --json | jq -r .surface_id)
porthole screenshot "$surface" --out repro-state.png
porthole close "$surface"   # optional — script's terminal goes away with it
```

When that runs cleanly from inside Ghostty, iTerm2, Terminal.app, or Wezterm on macOS — and when a test harness running in CI can do the equivalent against a fresh terminal it launched — this slice has done its job.
