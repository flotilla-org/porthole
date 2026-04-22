# Porthole Slice C — Presentation — Design

Date: 2026-04-22
Status: Draft for review
Supersedes: nothing; extends the v0 design (`docs/superpowers/specs/2026-04-20-porthole-design.md`) and builds on slices A, B, and the quality round.

## 1. Purpose

Ship the presentation story that was the other half of porthole's reason for being: show an artifact to the user, in the right place, for the right duration, and reuse the window when showing something new.

An agent (a flotilla yeoman, a test harness, anyone) can now say "here is a file — show it to the user on their focused display, replace it when I have the next one, dismiss it after 15 seconds if nothing happens." Slice A gave us the surface primitives; slice B gave us attach-and-track; slice C closes the loop by adding the actual *show* operations.

This slice's concern is the *infrastructure* for presentation, not the rendered UX. A future porthole-viewer app (for markdown, screenshots, videos, mermaid, dot, asciinema) rides on top of this slice as a normal `process` launch with geometry — everything here applies to it.

## 2. Relationship to existing design

Mostly additive. The scope on `LaunchRequest` grows, but existing `process` launches keep their contract.

- `LaunchRequest.kind` gains an `artifact` variant alongside the existing `process`.
- `LaunchRequest` gains optional `placement` and `auto_dismiss_after_ms` fields.
  - On `POST /launches`, absent → no-op (OS default). `process` launches with no placement keep the v0/slice-A/B behaviour.
  - On `POST /surfaces/{id}/replace`, an **absent `placement` key** is the signal that inherits the old surface's geometry + display (§6.2 step 3). A supplied `placement` value — including the empty object `{}` — is used verbatim, matching launch semantics. This is the one wire-level asymmetry between the two endpoints.
- `LaunchResponse` gains `surface_was_preexisting: bool` (already present per the v0 spec; this slice actually emits non-false values on artifact launches that reuse a window).
- `Adapter` trait grows `launch_artifact`, `place_surface`, `snapshot_geometry` methods.
- New route: `POST /surfaces/{id}/replace` with a full `LaunchRequest` body.
- `/info` capability additions via `Adapter::capabilities()`: `"launch_artifact"`, `"placement"`, `"replace"`, `"auto_dismiss"`.

Items from the v0 spec explicitly delivered by this slice:

- §4 `artifact` as a `LaunchKind` variant
- §6.5 artifact launch kind with document-match correlation
- §6.6 placement on launch (`on_display`, `geometry`, `anchor`)
- §6.7 `auto_dismiss_after_ms` timer
- §6.8 `POST /surfaces/{id}/replace`

Items from the v0 spec still deferred after this slice:

- URL artifacts — separate browser/CDP slice (see §10)
- Tab surface enumeration
- Events SSE stream
- Recording
- `focus: "preserve"`
- Cross-host routing
- Overlay subsystem (rejected for v0 in the main design)
- Alternative openers (`opener: "quicklook"` / porthole-viewer) — future slices

## 3. New resources and endpoints

```
POST /surfaces/{id}/replace   — close surface and launch new one in its position
```

`POST /launches` gains the `artifact` kind and new optional fields. No other new endpoints.

`/info` capability additions (each adapter declares its own):

- `"launch_artifact"` — can do file-path dispatch via OS default handler
- `"placement"` — can apply `on_display` / `geometry` / `anchor` to a freshly launched window
- `"replace"` — can close a tracked surface and launch in its geometry
- `"auto_dismiss"` — can run a timer that closes a surface after N ms

## 4. `artifact` launch kind

### 4.1 Request

`POST /launches` with the artifact variant:

```json
{
  "kind": {
    "type": "artifact",
    "path": "/Users/robert/repro.pdf"
  },
  "placement": { "on_display": "focused" },
  "auto_dismiss_after_ms": 15000,
  "require_confidence": "strong",
  "session": "demo-20260422"
}
```

- `kind.path` — absolute file path. Tilde expansion and relative paths are caller-side; porthole uses the string as-is.
- URLs are not accepted in this slice. `"path"` starting with `http://` / `https://` / `file://` returns `invalid_argument` with a message pointing to the future browser-CDP slice.

### 4.2 Dispatch

Internally the macOS adapter calls `/usr/bin/open <path>`. The OS default handler (determined by `LSCopyDefaultApplicationURLForURL`) receives the file and opens it.

**Edit-oriented apps**: the macOS `open` command is designed for launching the user's preferred editor for a file type — markdown → BBEdit/Typora, JSON → VSCode, etc. That is what gets launched. Read-only review UX is an explicit future concern (QuickLook opener, porthole-viewer) not in this slice's scope. Callers that need review-oriented display should wait for those or drive their own viewer binaries as `process` launches.

### 4.3 Correlation

Three-tier with reported confidence (v0 spec §6.1):

1. **`DocumentMatch` (strong)** — query the handler app's AX windows after the `open` call returns; look for a window whose `AXDocument` attribute (a URL string) matches `file://<path>`. This covers apps that reuse windows or open new tabs in existing windows (Preview, Safari, BBEdit).
2. **`FrontmostChanged` (plausible)** — no document match, but the handler app's frontmost window changed within Δt. Assume it's ours.
3. **`Temporal` (weak)** — fallback: first new window of the handler app appeared within Δt.

Default `require_confidence: "strong"` (per slice A's flipped default). Callers opt into plausible/weak explicitly.

The correlation writes `surface_was_preexisting: true` when the matched window was present before the `open` call. Tracked via snapshot-before + match-after.

Handler-app resolution: `LSCopyDefaultApplicationURLForURL`. If LaunchServices can't resolve a handler (unusual file types, no registered app), returns `launch_correlation_failed` immediately. If the handler app is not running, porthole waits for it to start (subject to `timeout_ms`).

### 4.4 Apps that don't populate `AXDocument`

Some editors don't expose a document URL via AX. They can still launch and track, but correlation will degrade to `plausible` (FrontmostChanged) or `weak` (Temporal). Callers requiring strong correlation against such apps will get `launch_correlation_ambiguous`; they can either downgrade `require_confidence` or pre-launch + attach instead.

## 5. Placement

### 5.1 Request shape

`LaunchRequest.placement` is an optional object:

```json
"placement": {
  "on_display": "focused" | "primary" | "disp_1",
  "geometry": { "x": 100, "y": 80, "w": 1200, "h": 800 },
  "anchor": "focused_display" | "cursor"
}
```

Every field optional. An omitted `placement` applies no positioning — the OS default is used.

### 5.2 Display selection

`on_display` accepts:

- `"focused"` — the display currently holding the focused window (or cursor if nothing tracked is focused), resolved the same way slice A's `/attention` computes `focused_display_id`.
- `"primary"` — `CGMainDisplayID`.
- `"disp_<N>"` — the stable display id string returned by `/displays`.

Unresolvable display ids return `invalid_argument` with the list of known display ids in the error details.

If neither `on_display` nor `anchor` is specified but `geometry` is, the geometry applies to the primary display. Rationale: callers passing a rect without qualifying "on which monitor" most likely want the usual one.

### 5.3 Coordinate space

**Display-local logical points.** `geometry.x = 0, y = 0` is the top-left of the selected display. The adapter maps to global screen coordinates internally using the selected display's bounds (from the same `CGDisplay` enumeration that `/displays` uses).

Rationale: callers don't need to know where each monitor sits in the global coordinate space. A layout like `{ on_display: "focused", geometry: { x: 0, y: 0, w: 800, h: 600 } }` works the same whether the focused display is primary, to the right, above, or a Retina display with negative global coordinates.

### 5.4 Anchor semantics

`anchor` is a default-placement strategy — it only applies when no explicit `geometry` is given. Explicit geometry always wins.

- `anchor: "focused_display"` — center the OS-default-sized window on the focused display. If `on_display` is also given, it overrides which display the anchor uses.
- `anchor: "cursor"` — center the window at the cursor position, on whatever display the cursor is on.

### 5.5 Preexisting-surface rule

When `surface_was_preexisting: true` (artifact launch correlated to a window that existed before the `open` call), **placement is not applied**. Porthole does not reposition a window the caller didn't previously own — see v0 spec §6.6.

The caller learns about preexisting status via the launch response flag. If they want to reposition anyway, a future `POST /surfaces/{id}/place` will let them do it explicitly on a tracked handle. Not in this slice.

Callers that want to *avoid* reusing a preexisting window entirely — for example, to guarantee a replace lands in the old window's slot — should pass `require_fresh_surface: true` on the launch request. See §5.8.

### 5.6 Implementation

Macos adapter, after correlation succeeds and `surface_was_preexisting == false`:

1. Resolve `on_display` / `anchor` to a target display.
2. If `geometry` is present, compute global screen coords from display-local + display bounds.
3. If `geometry` is absent and `anchor` resolves to focused_display/cursor, compute a centered rect using the OS-reported window size (or a conservative default).
4. Write `AXPosition` and `AXSize` on the AX window via `AxElement::set_attribute_value` (a small addition to `ax.rs`).

Some apps refuse position/size writes (non-resizable windows, modal dialogs). This is **not** an error at the launch contract — the surface is already tracked and the handle is valid. The launch response carries the outcome explicitly in `placement` (see §5.7), so callers can detect the partial success without losing the surface_id.

### 5.7 Placement outcome in `LaunchResponse`

`LaunchResponse` (and by extension the `replace` response) gains a `placement` field:

```json
{
  "surface_id": "surf_abc",
  "surface_was_preexisting": false,
  "confidence": "strong",
  "correlation": "document_match",
  "placement": { "type": "applied" }
}
```

The `placement` field is a tagged enum with four variants:

| Variant | Meaning |
|---|---|
| `{ "type": "not_requested" }` | No effective placement was requested. Covers both (a) `placement` key absent from the body, and (b) `placement: {}` / all-fields-null — the adapter had nothing to act on, so the OS default geometry was used. |
| `{ "type": "applied" }` | Placement had at least one effective field (`on_display`, `geometry`, or `anchor`) and AX writes succeeded. |
| `{ "type": "skipped_preexisting" }` | Placement was requested with effective fields but the launch correlated to a preexisting surface (§5.5) and was therefore not applied. |
| `{ "type": "failed", "reason": "..." }` | Placement was requested with effective fields and AX writes failed (e.g., non-resizable window). Free-form reason string. |

The handle is valid in all four cases. Callers that *require* placement to have applied should check `placement.type == "applied"` before treating the launch as successful for their purposes. This is strictly richer than an error return because the caller gets the surface_id regardless.

### 5.8 `require_fresh_surface`

Optional boolean on the launch body. Defaults to `false`.

When `true`, a launch that would correlate to a preexisting surface (`surface_was_preexisting: true`) returns the new `launch_returned_existing` error instead of succeeding. The caller learns the window they wanted wasn't freshly produced, and can decide what to do.

Motivating use case: `POST /surfaces/{id}/replace` relies on the replacement being a *fresh* window so that the inherited geometry actually lands there. If the replacement correlates to a preexisting window elsewhere on screen, the geometry inheritance is pointless and the user sees a confusing displaced result. Callers building replace flows should pass `require_fresh_surface: true` to get an explicit failure instead of silent displacement.

## 6. `POST /surfaces/{id}/replace`

### 6.1 Request / response

Body is a full `LaunchRequest` — whatever the caller wants the replacement to be.

```json
POST /surfaces/{id}/replace
{
  "kind": { "type": "artifact", "path": "/tmp/next.pdf" },
  "placement": { "geometry": { "x": 50, "y": 50, "w": 900, "h": 700 } },
  "auto_dismiss_after_ms": 10000
}
```

Response is a `LaunchResponse` — same shape as `POST /launches`, including the `placement` outcome field:

```json
{
  "launch_id": "...",
  "surface_id": "surf_new",
  "surface_was_preexisting": false,
  "confidence": "strong",
  "correlation": "document_match",
  "placement": { "type": "applied" }
}
```

### 6.2 Semantics

1. **Snapshot the old geometry.** Read AX position + size from the old window *plus* the display it's currently on, returning a `GeometrySnapshot { display_id, display_local: Rect }`. The adapter computes the display-local rect by subtracting the containing display's origin from the global AX position. If the adapter can't read the geometry (permission, window already gone), the snapshot is `None` and step 3 does not inject anything.
2. **Close the old surface.** Uses the existing close path (AXPress on close button, Cmd+W fallback, verified via `list_windows`). Returns `close_failed` (the slice-A error code; no new code introduced) if the window refuses to close — replace aborts, old handle stays alive.
3. **Inherit geometry — triggered only by absent `placement` key.** If the caller's replacement body **omits the `placement` key entirely** (the JSON key is not present), inject the snapshotted `display_id` and `display_local` rect as `on_display` and `geometry`. If the caller supplies any `placement` value — including an empty object `placement: {}` — the value is used verbatim and no inheritance occurs. This is a deliberate distinction: `placement: {}` on replace behaves identically to `placement: {}` on `POST /launches` (no positioning, OS default), so a caller sending the same body to either endpoint gets the same result. Callers who want inheritance omit the key entirely; callers who want OS default on replace pass `placement: {}`.

   Serde-side: the `placement` field is `Option<PlacementSpec>`. `None` means inherit; `Some(PlacementSpec::default())` means "empty spec, no inheritance."

   Callers who want to partially inherit (e.g., same display, different size) should read the old geometry via a future `GET /surfaces/{id}/geometry` (deferred) or compose from `/displays` plus their own state.
4. **Launch the replacement.** Standard launch path — artifact or process.
5. **Return the new launch response.** Same shape as `POST /launches` (includes `placement: PlacementOutcome` per §5.7). The new surface is owned by the replace caller; the old handle is dead.

### 6.3 Atomicity

Handle-atomic: either the response carries a new `surface_id` or a typed error. On error:

- `close_failed` during step 2 → old handle is **still alive**, no new surface created. This is a deliberate revision of v0 spec §6.8 (which assumed close always succeeds); slice A added close verification that can genuinely fail when apps present save/discard dialogs, so replace must handle the "close refused" case. The caller gets their old surface back and can investigate (e.g., driving the save dialog programmatically) or try again.
- `launch_correlation_failed` / `launch_correlation_ambiguous` / `launch_returned_existing` during step 4 → old handle is **dead**. The replacement couldn't be identified (or was going to reuse an existing window under `require_fresh_surface: true`), but the old window is gone. Caller gets the error and a note that the old handle is now dead.

The error body carries an explicit `old_handle_alive: bool` field so callers don't have to infer the state from the error code.

Not visually atomic. Brief gap while the OS tears down the old window and brings up the new one, during which nothing is on-screen in that slot. A future overlay subsystem could paper over this; not in any current slice.

### 6.4 Preventing displaced replacements

Replace inherits geometry so the new window lands where the old one was. But if the replacement correlates to a *preexisting* window (artifact dispatch reused some other Preview or browser window elsewhere on screen), the inherited geometry is not applied (§5.5), and the result is a displaced replacement — old window closed, new "replacement" sitting in whatever slot it was already in.

**Callers doing replace should pass `require_fresh_surface: true` on the replacement launch body.** This turns preexisting-correlated replacements into an explicit `launch_returned_existing` error rather than a silently displaced success. (The earlier proposal to require strong confidence was wrong: strong DocumentMatch *is* the path that finds reused windows. Freshness is a separate axis from confidence.)

Documented as a known limitation and the recommended caller pattern.

## 7. `auto_dismiss_after_ms`

### 7.1 Request

A new optional field on `LaunchRequest`:

```json
{ "auto_dismiss_after_ms": 15000 }
```

Must be a positive integer. Zero is rejected (`invalid_argument`). Absent means no auto-dismiss.

### 7.2 Implementation

On successful launch (surface tracked, handle inserted into store), the daemon spawns a tokio timer task:

```rust
tokio::spawn(async move {
    tokio::time::sleep(Duration::from_millis(ms)).await;
    pipeline.close(&surface_id).await.ok();
});
```

Cancellation is structural: when the surface transitions to `Dead` (via API `close`, via external detection, via `replace`, via dead-handle GC), the timer's close call is a no-op (`surface_dead` error swallowed).

A cleaner implementation cancels the timer explicitly on early close. In this slice we accept the swallow-the-error pattern — it's one extra wakeup per auto-dismissed launch, negligible.

State: no explicit timer handle stored with the surface in this slice. If a future slice needs explicit cancellation (e.g., "extend the timer"), the timer becomes a field on `TrackedSurface`.

### 7.3 Daemon restart behaviour

Timers are in-memory. If the daemon restarts before the timer fires, the artifact stays up until a user closes it or another slice adds persistence. Documented as a known limitation.

## 8. Adapter trait additions

Three new methods on `Adapter`:

```rust
async fn launch_artifact(
    &self,
    path: &Path,
    require_confidence: RequireConfidence,
    require_fresh_surface: bool,
    timeout: Duration,
) -> Result<LaunchOutcome, PortholeError>;

/// Apply a resolved placement rectangle in global screen coordinates
/// to a tracked surface. Used after correlation by the launch path,
/// and available for future POST /surfaces/{id}/place.
async fn place_surface(&self, surface: &SurfaceInfo, rect: Rect) -> Result<(), PortholeError>;

/// Read the current geometry of a tracked surface, along with the
/// display id it is currently on. Used by replace to snapshot the
/// old surface before closing, in a form that round-trips across
/// multi-monitor setups.
async fn snapshot_geometry(&self, surface: &SurfaceInfo) -> Result<GeometrySnapshot, PortholeError>;
```

Where `GeometrySnapshot` is a new type in `porthole-core`:

```rust
pub struct GeometrySnapshot {
    pub display_id: DisplayId,
    pub display_local: Rect,   // coords relative to the display's origin
}
```

No signature changes on existing methods. `launch_process` and `launch_artifact` are parallel entry points (not one method with a discriminator); the pipeline picks based on `LaunchRequest.kind`.

New core types:

```rust
pub struct PlacementSpec {
    pub on_display: Option<DisplayTarget>,
    pub geometry: Option<Rect>,
    pub anchor: Option<Anchor>,
}

pub enum DisplayTarget {
    Focused,
    Primary,
    Id(DisplayId),
}

pub enum Anchor {
    FocusedDisplay,
    Cursor,
}
```

New pipeline responsibility: `LaunchPipeline` grows logic to resolve placement after correlation and call `adapter.place_surface`. `AttachPipeline` (from slice B) is untouched.

New pipeline: `ReplacePipeline` (in `crates/porthole-core/src/replace_pipeline.rs`). Coordinates snapshot + close + launch + placement-inherit in one call, taking an `Arc<dyn Adapter>`, a `HandleStore`, and references to the existing `LaunchPipeline` / `AttachPipeline` / close path.

## 9. Error model additions

One new error code:

- `launch_returned_existing` — launch correlated to a preexisting surface and the caller set `require_fresh_surface: true`. Returned from both `POST /launches` and `POST /surfaces/{id}/replace`. The error body carries a slice-B-style opaque `ref` string (produced by the same `encode_ref(pid, cg_window_id)` helper) so the caller can directly `POST /surfaces/track` on it to attach the existing surface as a fallback — no extra `/surfaces/search` round-trip required. Body also includes human-readable `app_name`, `title`, `pid`, `cg_window_id` for logging/display.

Extended use of existing codes:

- `close_failed` (from slice A) now also returned from `POST /surfaces/{id}/replace` when step 2 (close of old surface) fails. The error body carries `old_handle_alive: true` so the caller can resume using the old handle.
- `invalid_argument` used more broadly: rejects zero `auto_dismiss_after_ms`, unknown `on_display` ids, URL paths in `artifact` requests.

**Placement failure is not an error** — §5.7 moves it into the `LaunchResponse.placement` field. Prior draft versions of this spec proposed a `placement_failed` error code; it was rejected because it lost the surface_id on a surface that was already tracked, making the handle unrecoverable to the caller. Similarly, a proposed `replace_close_failed` code was rejected in favor of reusing slice A's `close_failed` with a clear error body — the semantic (surface refused to close, still alive) is the same whether it happens during a standalone close or a replace.

Existing codes continue to apply: `surface_not_found`, `surface_dead`, `permission_needed`, `launch_correlation_failed`, `launch_correlation_ambiguous`, `launch_timeout`, `close_failed`.

## 10. Testing strategy

### 10.1 Core (in-memory adapter)

- `InMemoryAdapter` scripts `launch_artifact`, `place_surface`, `snapshot_geometry` with recorders for each.
- `LaunchPipeline` tests:
  - `artifact` launch success happy path
  - Placement resolution across `on_display: "focused" | "primary" | <id>`
  - `anchor` semantics when no explicit geometry
  - Placement skipped when `surface_was_preexisting: true` — response carries `placement: { "type": "skipped_preexisting" }` and the surface_id is still returned
  - Placement failure surfaces as `placement: { "type": "failed", "reason": "..." }` with surface_id still present — not as an HTTP error
  - `auto_dismiss_after_ms` validation (zero rejected, positive accepted)
  - URL path rejected with `invalid_argument`
  - `require_fresh_surface: true` + preexisting correlation → `launch_returned_existing` error; body carries an opaque `ref` string; feeding that ref to `/surfaces/track` mints a tracked handle for the preexisting surface
- `ReplacePipeline` tests:
  - Happy path: omitted `placement` → snapshot injected (inheritance fires) → close → launch → new surface lands in old slot
  - `placement: {}` on replace → no inheritance; new surface placed OS-default; proves symmetry with `POST /launches` for the same body
  - Caller-supplied `placement.geometry` → used verbatim, no snapshot injection (confirms all-or-nothing rule)
  - Caller-supplied `placement.on_display` only → used verbatim, no geometry injection (same rule)
  - Close failure aborts replace → `close_failed` with `old_handle_alive: true` in the body; old handle remains valid; no new surface created
  - Replacement launch returns `launch_returned_existing` → old handle is **dead** (step 4 ran after step 2); error body carries the preexisting surface's `ref`; caller can attach it as fallback
  - Preexisting replacement correlation without `require_fresh_surface` → surface_was_preexisting: true, placement skipped, user sees displaced window (documented behaviour)

### 10.2 Protocol

Serde roundtrip tests for:

- Extended `LaunchRequest` with both kinds (`process`, `artifact`), with/without `placement`, with/without `auto_dismiss_after_ms`, with/without `require_fresh_surface`.
- `PlacementSpec` serialisation: empty `{}` vs fully-populated.
- `LaunchResponse.placement: PlacementOutcome` — all four variants (`not_requested`, `applied`, `skipped_preexisting`, `failed { reason }`) round-trip via serde tag.
- `LaunchRequest.placement` as `Option<PlacementSpec>`: deserialising `{}` at the key's place gives `Some(PlacementSpec::default())`, while omitting the key gives `None`. This is the contract that replace's inheritance rule depends on — covered in a protocol-level unit test so the rule can't regress silently.
- `launch_returned_existing` error body shape: contains `ref: String`, `app_name: Option<String>`, `title: Option<String>`, `pid: u32`, `cg_window_id: u32`.
- `close_failed` error body shape when returned from replace: contains `old_handle_alive: true`.

### 10.3 Daemon routes

Oneshot axum tests for:
- `POST /launches` with `kind: "artifact"` (via in-memory adapter)
- `POST /launches` with full placement spec — asserts `LaunchResponse.placement == "applied"`
- `POST /launches` with `require_fresh_surface: true` and a scripted preexisting candidate — asserts 4xx status with error code `launch_returned_existing` and the error body contains a `ref` string
- `POST /surfaces/track` with the `ref` from the previous test's error body — asserts the preexisting surface gets tracked (the full recovery path)
- `POST /surfaces/{id}/replace` with omitted `placement` → inheritance fires; response includes `placement: "applied"` and the new surface id
- `POST /surfaces/{id}/replace` with `placement: {}` in the body → no inheritance; response includes `placement: "not_requested"`. Cross-check: identical body sent to `POST /launches` yields the same `placement` outcome
- Auto-dismiss timer: launch with small `auto_dismiss_after_ms`, wait just past it, assert the surface transitions to dead (or is `surface_dead` on subsequent operation)

### 10.4 macOS adapter

Unit tests:
- Display-local → global coordinate conversion
- Handler-app resolution (smoke test via LaunchServices — mock or `#[ignore]`'d)

`#[ignore]`'d real-macOS integration tests:
- `artifact_launch_pdf`: launch a PDF via artifact, assert tracked + screenshot-able.
- `placement_on_focused_display`: launch an artifact with `anchor: "focused_display"`, assert position is within the focused display's bounds.
- `replace_pdf_with_png`: launch PDF, replace with PNG, assert the old handle is dead and the new one is placed where the old was.
- `auto_dismiss_closes_window`: launch artifact with 2-second auto_dismiss, wait 3 seconds, assert surface is dead.

### 10.5 E2E

Extend `slice_b_e2e.rs` pattern — new `slice_c_e2e.rs`: artifact launch + placement + replace + auto-dismiss round-trip over UDS using the in-memory adapter.

## 11. Out of scope

- URL artifacts — browser/CDP slice
- `opener: "quicklook" | "app_bundle"` variants on artifact launches — future
- Porthole-owned viewer app (markdown, mermaid, dot, videos, asciinema) — future project
- `POST /surfaces/{id}/place` for post-launch repositioning — small future addition
- Named slots (the v0 spec's "slot" concept for reuse-in-place without tracking a handle) — replaced by the handle + `replace` pattern in this slice
- Overlay subsystem to mask the replace transition gap — rejected for v0
- Event-driven auto-dismiss cancellation — small future refinement
- Timer persistence across daemon restart
- Placement for preexisting surfaces (the `force_place: true` variant noted in slice A §6.6)
- Multi-surface launches (a single artifact opens several windows — adapter returns first, rest attachable)

## 12. Known limitations

- **Edit-oriented dispatch.** `open <file>` launches the user's default editor, not a review-oriented viewer. Callers who need review UX must wait for the QuickLook opener / porthole-viewer / browser-CDP slices.
- **`AXDocument` coverage varies.** Apps that don't publish document URLs via AX degrade to FrontmostChanged or Temporal correlation. Documented per-app behaviour would require real-world cataloging; not in scope here.
- **Preexisting replacement produces a displaced window.** If `replace` correlates to an already-open window elsewhere on screen, the old slot's geometry is not applied. Callers doing replace should pass `require_fresh_surface: true` on the replacement launch body so that preexisting-correlated replacements return `launch_returned_existing` explicitly instead of silently landing in the wrong place. See §6.4.
- **Auto-dismiss doesn't survive daemon restart.** Timer state is in-memory. A future slice can persist scheduled dismissals.
- **Coordinate space is display-local.** Global-coordinate callers (rare; most callers don't need it) have to read `/displays` and convert.
- **`close_failed` during replace is recoverable but surprising.** Old surface stays alive; new one never launched. Caller sees `close_failed` with `old_handle_alive: true` in the body. Retry or investigate the blocking dialog. See §6.3.
- **No batch replace.** Each replace is one surface. For "swap in a new set of presentations," callers script it.

## 13. Success criterion

A yeoman agent can run this sequence against porthole and have it feel uneventful:

```sh
# Show the user the first artifact on their focused display, 15s auto-dismiss.
SURFACE=$(porthole launch \
    --kind artifact --path /tmp/proposal.pdf \
    --on-display focused \
    --require-fresh-surface \
    --auto-dismiss-ms 15000 \
    --json | jq -r .surface_id)

# User nods. Swap in the next one without moving the window.
# Replace returns a new surface_id; capture it for any later operations.
SURFACE=$(porthole replace "$SURFACE" \
    --kind artifact --path /tmp/alternative.pdf \
    --require-fresh-surface \
    --auto-dismiss-ms 15000 \
    --json | jq -r .surface_id)

# Explicit dismiss of the replacement before the timer fires.
porthole close "$SURFACE"
```

The `--require-fresh-surface` flag ensures every step gets a freshly minted window so the replace geometry inheritance lands correctly. The `$SURFACE` re-assignment after replace captures the new handle — the old one is dead once replace returns.

When that script runs cleanly across PDFs, PNGs, markdown (even landing in editors — the bar is that it *launches and tracks*, not that the UX is pretty), and the window ends up where the caller asked — this slice has done its job.

The full presentation story then lands in follow-ups: QuickLook opener for review UX, porthole-viewer for rendered markdown/mermaid/asciinema, browser-CDP for URL artifacts.
