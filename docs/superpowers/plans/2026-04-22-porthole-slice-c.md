# Porthole Slice C — Presentation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add the `artifact` launch kind (file paths via `open`), placement on launches (`on_display` / `geometry` / `anchor`), `POST /surfaces/{id}/replace` with geometry inheritance, and `auto_dismiss_after_ms` timers — the full v0 presentation story minus URL artifacts.

**Architecture:** Extends the existing slice-B workspace. New core types for placement and geometry snapshots; three new adapter trait methods (`launch_artifact`, `place_surface`, `snapshot_geometry`); `LaunchPipeline` grows placement resolution and `PlacementOutcome` reporting; new `ReplacePipeline` coordinates snapshot → close → launch → geometry inheritance; auto-dismiss implemented as detached tokio timer tasks. Uses the slice A+B foundations: `HandleStore::track_or_get`, `AxElement` RAII, deadline-aware `wait`, `close` verification, `containing_ancestors`.

**Tech Stack:** Same as slice B.

---

## Out of Scope for This Plan

Per the slice-C spec (`docs/superpowers/specs/2026-04-22-porthole-slice-c-design.md`, §11):

- URL artifacts (deferred to browser-CDP slice)
- QuickLook / porthole-viewer openers
- `POST /surfaces/{id}/place` for post-launch repositioning
- Tab surface enumeration
- Events SSE
- Recording
- `focus: "preserve"`, AX-element targeting
- Cross-host transport
- Persistent timers across daemon restart
- `force_place: true` for preexisting surfaces
- Multi-surface single-launch

---

## File Structure

```
crates/porthole-core/
  src/
    adapter.rs                     # modify: add launch_artifact/place_surface/snapshot_geometry; GeometrySnapshot type
    display.rs                     # modify: add DisplayTarget + Anchor enums
    error.rs                       # modify: add LaunchReturnedExisting
    in_memory.rs                   # modify: script new adapter methods
    launch.rs                      # modify: LaunchPipeline grows placement resolution + outcome reporting + require_fresh
    placement.rs                   # NEW: PlacementSpec + PlacementOutcome + Resolved helpers
    replace_pipeline.rs            # NEW: ReplacePipeline (snapshot + close + launch + inheritance)
    lib.rs                         # modify: declare new modules + re-exports

crates/porthole-protocol/
  src/
    launches.rs                    # modify: ArtifactLaunch variant, placement, auto_dismiss, require_fresh_surface, PlacementOutcome
    error.rs                       # modify: rich LaunchReturnedExisting body type + old_handle_alive on close_failed

crates/portholed/
  src/
    routes/
      launches.rs                  # modify: artifact branch + new fields + placement outcome in response
      replace.rs                   # NEW: POST /surfaces/{id}/replace
      errors.rs                    # modify: map LaunchReturnedExisting → 409, wire error body formatters
      mod.rs                       # modify: declare replace module
    server.rs                      # modify: wire replace route + tests
    state.rs                       # modify: add ReplacePipeline; dismiss-timer bookkeeping
  tests/
    slice_c_e2e.rs                 # NEW: end-to-end artifact launch + placement + replace + auto-dismiss

crates/porthole/
  src/
    commands/
      launch.rs                    # modify: --kind artifact + placement flags + auto-dismiss + require-fresh
      replace.rs                   # NEW: porthole replace <id> ...
      mod.rs                       # modify: declare replace module
    main.rs                        # modify: add Replace variant + extend Launch args

crates/porthole-adapter-macos/
  src/
    ax.rs                          # modify: add AxElement::set_attribute_value and set_position_size helpers
    lib.rs                         # modify: impl the 3 new trait methods; update capabilities()
    launch.rs                      # modify: extract handler-app resolution via LaunchServices; add launch_artifact path
    artifact.rs                    # NEW: artifact correlation (DocumentMatch / FrontmostChanged / Temporal)
    placement.rs                   # NEW: display resolution + AX position/size writes
    snapshot.rs                    # NEW: read AX geometry + resolve which display
  tests/
    slice_c_integration.rs         # NEW: ignored real-macOS integration tests
```

---

## Task 1: Core types — `PlacementSpec`, `DisplayTarget`, `Anchor`, `PlacementOutcome`, `GeometrySnapshot`

**Files:**
- Create: `crates/porthole-core/src/placement.rs`
- Modify: `crates/porthole-core/src/display.rs`
- Modify: `crates/porthole-core/src/adapter.rs`
- Modify: `crates/porthole-core/src/lib.rs`

- [ ] **Step 1: Write `placement.rs`**

Create `crates/porthole-core/src/placement.rs`:

```rust
use serde::{Deserialize, Serialize};

use crate::display::{DisplayRect as Rect, DisplayId};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PlacementSpec {
    #[serde(default)]
    pub on_display: Option<DisplayTarget>,
    #[serde(default)]
    pub geometry: Option<Rect>,
    #[serde(default)]
    pub anchor: Option<Anchor>,
}

impl PlacementSpec {
    /// True when the spec has no effective field — PlacementOutcome::NotRequested applies.
    pub fn is_effectively_empty(&self) -> bool {
        self.on_display.is_none() && self.geometry.is_none() && self.anchor.is_none()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DisplayTarget {
    Focused,
    Primary,
    Id(DisplayId),
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Anchor {
    FocusedDisplay,
    Cursor,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PlacementOutcome {
    NotRequested,
    Applied,
    SkippedPreexisting,
    Failed { reason: String },
}

/// Snapshot of a window's current geometry, display-local.
/// Used by ReplacePipeline to inject inherited placement into the
/// replacement launch.
#[derive(Clone, Debug, PartialEq)]
pub struct GeometrySnapshot {
    pub display_id: DisplayId,
    pub display_local: Rect,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placement_spec_empty_by_default() {
        let p = PlacementSpec::default();
        assert!(p.is_effectively_empty());
    }

    #[test]
    fn placement_spec_with_any_field_not_empty() {
        let p = PlacementSpec { on_display: Some(DisplayTarget::Primary), ..Default::default() };
        assert!(!p.is_effectively_empty());
    }

    #[test]
    fn placement_outcome_roundtrip() {
        let o = PlacementOutcome::Applied;
        let s = serde_json::to_string(&o).unwrap();
        assert_eq!(s, r#"{"type":"applied"}"#);

        let o = PlacementOutcome::Failed { reason: "AX denied".into() };
        let s = serde_json::to_string(&o).unwrap();
        assert_eq!(s, r#"{"type":"failed","reason":"AX denied"}"#);
    }

    #[test]
    fn display_target_id_serializes_transparently() {
        let t = DisplayTarget::Id(DisplayId::new("disp_1"));
        let s = serde_json::to_string(&t).unwrap();
        assert_eq!(s, r#"{"id":"disp_1"}"#);
    }

    #[test]
    fn display_target_focused_serializes_as_string_tag() {
        let t = DisplayTarget::Focused;
        let s = serde_json::to_string(&t).unwrap();
        assert_eq!(s, r#""focused""#);
    }
}
```

Note: the DisplayTarget::Id variant serializes as `{"id":"disp_1"}` because of the snake_case + untagged enum behaviour. For the wire shape `on_display: "disp_1"` (plain string), we'd need a custom serde impl. For this slice keep the tagged form for simplicity; callers use `{"on_display": {"id": "disp_1"}}` or `{"on_display": "focused"}` / `{"on_display": "primary"}`. Document this in the spec review if surprising.

Actually — the spec example shows `"on_display": "disp_1"` as a plain string. To get that, we need a custom serialize/deserialize. Add them:

After the `DisplayTarget` enum, add:

```rust
impl Serialize for DisplayTarget {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            DisplayTarget::Focused => s.serialize_str("focused"),
            DisplayTarget::Primary => s.serialize_str("primary"),
            DisplayTarget::Id(id) => s.serialize_str(id.as_str()),
        }
    }
}

impl<'de> Deserialize<'de> for DisplayTarget {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Ok(match s.as_str() {
            "focused" => DisplayTarget::Focused,
            "primary" => DisplayTarget::Primary,
            _ => DisplayTarget::Id(DisplayId::new(s)),
        })
    }
}
```

And **remove** the `#[derive(Serialize, Deserialize)]` on `DisplayTarget` and the `#[serde(rename_all = "snake_case")]`.

Update the test:

```rust
    #[test]
    fn display_target_id_serializes_as_plain_string() {
        let t = DisplayTarget::Id(DisplayId::new("disp_1"));
        let s = serde_json::to_string(&t).unwrap();
        assert_eq!(s, r#""disp_1""#);
    }

    #[test]
    fn display_target_focused_serializes_as_focused_string() {
        let t = DisplayTarget::Focused;
        let s = serde_json::to_string(&t).unwrap();
        assert_eq!(s, r#""focused""#);
    }

    #[test]
    fn display_target_deserializes_plain_string() {
        let t: DisplayTarget = serde_json::from_str(r#""disp_1""#).unwrap();
        assert_eq!(t, DisplayTarget::Id(DisplayId::new("disp_1")));
        let t: DisplayTarget = serde_json::from_str(r#""focused""#).unwrap();
        assert_eq!(t, DisplayTarget::Focused);
    }
```

- [ ] **Step 2: Add `GeometrySnapshot` to `adapter.rs`**

Edit `crates/porthole-core/src/adapter.rs`. Import `PlacementSpec`, `PlacementOutcome`, `GeometrySnapshot` from `crate::placement` (added in Step 1). Re-export `GeometrySnapshot` from placement.rs — it's defined there; adapter.rs uses it in trait signatures.

The re-export chain: `crate::placement` defines the types; `crate::lib` re-exports them; `adapter.rs` imports from `crate::placement` directly.

- [ ] **Step 3: Register module**

Edit `crates/porthole-core/src/lib.rs`. Add:

```rust
pub mod placement;
```

And extend re-exports:

```rust
pub use placement::{Anchor, DisplayTarget, GeometrySnapshot, PlacementOutcome, PlacementSpec};
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p porthole-core --lib placement`
Expected: 6 passes (4 main + 2 roundtrip variants).

- [ ] **Step 5: Commit**

```bash
git add crates/porthole-core/src/placement.rs crates/porthole-core/src/lib.rs
git commit -m "feat(core): add PlacementSpec, DisplayTarget, Anchor, PlacementOutcome, GeometrySnapshot"
```

---

## Task 2: Core — extend `LaunchKind` with `Artifact`, grow `LaunchSpec` variants

**Files:**
- Modify: `crates/porthole-core/src/adapter.rs`

Context: slice-B's v0 foundation defined `ProcessLaunchSpec` with process-specific fields. Slice C adds an `ArtifactLaunchSpec` and an enum wrapper so the adapter trait can dispatch on kind.

- [ ] **Step 1: Add `ArtifactLaunchSpec` and `LaunchSpec` wrapper**

Edit `crates/porthole-core/src/adapter.rs`. Near `ProcessLaunchSpec`, add:

```rust
#[derive(Clone, Debug)]
pub struct ArtifactLaunchSpec {
    pub path: std::path::PathBuf,
    pub require_confidence: RequireConfidence,
    pub require_fresh_surface: bool,
    pub timeout: std::time::Duration,
}

#[derive(Clone, Debug)]
pub enum LaunchSpec {
    Process(ProcessLaunchSpec),
    Artifact(ArtifactLaunchSpec),
}

impl LaunchSpec {
    pub fn require_confidence(&self) -> RequireConfidence {
        match self {
            LaunchSpec::Process(p) => p.require_confidence,
            LaunchSpec::Artifact(a) => a.require_confidence,
        }
    }

    pub fn require_fresh_surface(&self) -> bool {
        match self {
            LaunchSpec::Process(_) => false, // v0 process launches don't support the flag
            LaunchSpec::Artifact(a) => a.require_fresh_surface,
        }
    }
}
```

Also extend `ProcessLaunchSpec` with `require_fresh_surface: bool` (default false) for symmetry:

```rust
#[derive(Clone, Debug)]
pub struct ProcessLaunchSpec {
    pub app: String,
    pub args: Vec<String>,
    pub cwd: Option<String>,
    pub env: Vec<(String, String)>,
    pub timeout: std::time::Duration,
    pub require_confidence: RequireConfidence,
    pub require_fresh_surface: bool,
}
```

Update `LaunchSpec::require_fresh_surface` accordingly:

```rust
    pub fn require_fresh_surface(&self) -> bool {
        match self {
            LaunchSpec::Process(p) => p.require_fresh_surface,
            LaunchSpec::Artifact(a) => a.require_fresh_surface,
        }
    }
```

- [ ] **Step 2: Update test site and InMemoryAdapter's default outcome**

Find all construction of `ProcessLaunchSpec` in core tests (`launch.rs`, `input_pipeline.rs`, `wait_pipeline.rs`, `attach_pipeline.rs`) and add `require_fresh_surface: false` to each. Use `rg 'ProcessLaunchSpec \{' crates/` to find them.

For in-memory adapter default: `InMemoryAdapter::make_default_launch_outcome` doesn't build a spec, just an outcome. Leave unchanged.

- [ ] **Step 3: Run tests**

Run: `cargo test -p porthole-core --lib`
Expected: all existing tests pass. No test additions in this task — the structural change is validated when tasks that use it run.

- [ ] **Step 4: Commit**

```bash
git add crates/porthole-core/src/adapter.rs crates/porthole-core/src/launch.rs crates/porthole-core/src/input_pipeline.rs crates/porthole-core/src/wait_pipeline.rs crates/porthole-core/src/attach_pipeline.rs
git commit -m "feat(core): add ArtifactLaunchSpec, LaunchSpec wrapper, require_fresh_surface"
```

---

## Task 3: Core — new error code `LaunchReturnedExisting`

**Files:**
- Modify: `crates/porthole-core/src/error.rs`

- [ ] **Step 1: Add variant**

Edit `crates/porthole-core/src/error.rs`. Add to `ErrorCode`:

```rust
    LaunchReturnedExisting,
```

Update the `Display` impl arm:

```rust
            Self::LaunchReturnedExisting => "launch_returned_existing",
```

- [ ] **Step 2: Add test**

Append to the existing test module:

```rust
    #[test]
    fn launch_returned_existing_display_is_snake_case() {
        assert_eq!(ErrorCode::LaunchReturnedExisting.to_string(), "launch_returned_existing");
    }
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p porthole-core --lib error`
Expected: one additional pass.

- [ ] **Step 4: Commit**

```bash
git add crates/porthole-core/src/error.rs
git commit -m "feat(core): add LaunchReturnedExisting error code"
```

---

## Task 4: Adapter trait extension + InMemoryAdapter scripting

**Files:**
- Modify: `crates/porthole-core/src/adapter.rs`
- Modify: `crates/porthole-core/src/in_memory.rs`
- Modify: `crates/porthole-adapter-macos/src/lib.rs`

Context: three new trait methods. The macOS adapter gets `todo!()` stubs in this task to keep the workspace building; Tasks 15–18 implement them.

- [ ] **Step 1: Add trait methods**

Edit `crates/porthole-core/src/adapter.rs`. Add at the top of the imports block if not present:

```rust
use std::path::Path;

use crate::placement::GeometrySnapshot;
```

Extend the `Adapter` trait inside the `#[async_trait]` impl:

```rust
    /// Launch a file artifact via OS default handler (macOS: `open <path>`).
    /// Correlates via DocumentMatch (strong) / FrontmostChanged (plausible) /
    /// Temporal (weak) as described in the spec §4.3.
    async fn launch_artifact(&self, spec: &ArtifactLaunchSpec) -> Result<LaunchOutcome, PortholeError>;

    /// Apply a resolved placement rectangle in **global screen coordinates**
    /// to a tracked surface. The pipeline resolves on_display/anchor/geometry
    /// to a global rect and passes it here; adapter writes AXPosition + AXSize.
    async fn place_surface(&self, surface: &SurfaceInfo, rect: Rect) -> Result<(), PortholeError>;

    /// Read current geometry of a tracked surface, along with which display it's on.
    /// Returns display-local coords — caller (ReplacePipeline) uses both fields to
    /// inject inheritance into the replacement launch's placement.
    async fn snapshot_geometry(&self, surface: &SurfaceInfo) -> Result<GeometrySnapshot, PortholeError>;
```

- [ ] **Step 2: Extend `InMemoryAdapter`**

Edit `crates/porthole-core/src/in_memory.rs`. Add to `Script`:

```rust
    next_launch_artifact_outcome: Option<Result<LaunchOutcome, PortholeError>>,
    next_place_surface_result: Option<Result<(), PortholeError>>,
    next_snapshot_geometry: Option<Result<GeometrySnapshot, PortholeError>>,
    launch_artifact_calls: Vec<ArtifactLaunchSpec>,
    place_surface_calls: Vec<(SurfaceId, Rect)>,
    snapshot_geometry_calls: Vec<SurfaceId>,
```

Add the scripting setters and recorders in the impl block (mirror existing patterns):

```rust
    pub async fn set_next_launch_artifact_outcome(&self, v: Result<LaunchOutcome, PortholeError>) {
        self.script.lock().await.next_launch_artifact_outcome = Some(v);
    }
    pub async fn set_next_place_surface_result(&self, v: Result<(), PortholeError>) {
        self.script.lock().await.next_place_surface_result = Some(v);
    }
    pub async fn set_next_snapshot_geometry(&self, v: Result<GeometrySnapshot, PortholeError>) {
        self.script.lock().await.next_snapshot_geometry = Some(v);
    }
    pub async fn launch_artifact_calls(&self) -> Vec<ArtifactLaunchSpec> {
        self.script.lock().await.launch_artifact_calls.clone()
    }
    pub async fn place_surface_calls(&self) -> Vec<(SurfaceId, Rect)> {
        self.script.lock().await.place_surface_calls.clone()
    }
    pub async fn snapshot_geometry_calls(&self) -> Vec<SurfaceId> {
        self.script.lock().await.snapshot_geometry_calls.clone()
    }
```

Implement the three trait methods (at the bottom of the `impl Adapter for InMemoryAdapter` block):

```rust
    async fn launch_artifact(&self, spec: &ArtifactLaunchSpec) -> Result<LaunchOutcome, PortholeError> {
        let mut s = self.script.lock().await;
        s.launch_artifact_calls.push(spec.clone());
        s.next_launch_artifact_outcome
            .take()
            .unwrap_or_else(|| Ok(Self::make_default_launch_outcome(7777)))
    }

    async fn place_surface(&self, surface: &SurfaceInfo, rect: Rect) -> Result<(), PortholeError> {
        let mut s = self.script.lock().await;
        s.place_surface_calls.push((surface.id.clone(), rect));
        s.next_place_surface_result.take().unwrap_or(Ok(()))
    }

    async fn snapshot_geometry(&self, surface: &SurfaceInfo) -> Result<GeometrySnapshot, PortholeError> {
        let mut s = self.script.lock().await;
        s.snapshot_geometry_calls.push(surface.id.clone());
        s.next_snapshot_geometry.take().unwrap_or_else(|| {
            Ok(GeometrySnapshot {
                display_id: DisplayId::new("in-mem-display-0"),
                display_local: Rect { x: 0.0, y: 0.0, w: 800.0, h: 600.0 },
            })
        })
    }
```

Update imports at top of `in_memory.rs`:

```rust
use crate::adapter::{ArtifactLaunchSpec, Rect};
use crate::display::DisplayId;
use crate::placement::GeometrySnapshot;
```

(Check existing imports; only add what's missing.)

Note on `Rect`: the adapter has a `Rect` type in `adapter.rs` that's distinct from the `display::Rect` (aliased as `DisplayRect`). Use the same `Rect` the existing `Screenshot` type uses — it's just `{ x, y, w, h: f64 }`. `GeometrySnapshot.display_local` uses the display crate's Rect because it's display-local; if the two types have different paths, unify via alias.

Concretely: the foundation has `crate::adapter::Rect` (in `adapter.rs`) and `crate::display::Rect` (aliased as `DisplayRect`). Both are `{x,y,w,h: f64}`. For simplicity, use `crate::display::Rect` everywhere in the new code — add a re-export `pub use crate::display::Rect;` in `adapter.rs` if it helps, or replace `adapter::Rect` with `DisplayRect` in the adapter module (small refactor). For this task, use whichever type is in scope already; unify in a cleanup step during Task 22 if it matters for clippy.

- [ ] **Step 3: Add tests**

Append to the `tests` module in `in_memory.rs`:

```rust
    #[tokio::test]
    async fn launch_artifact_records_call_and_returns_default() {
        let adapter = InMemoryAdapter::new();
        let spec = ArtifactLaunchSpec {
            path: "/tmp/test.pdf".into(),
            require_confidence: crate::adapter::RequireConfidence::Strong,
            require_fresh_surface: false,
            timeout: std::time::Duration::from_secs(5),
        };
        let outcome = adapter.launch_artifact(&spec).await.unwrap();
        assert_eq!(outcome.confidence, Confidence::Strong);
        let calls = adapter.launch_artifact_calls().await;
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].path.to_str(), Some("/tmp/test.pdf"));
    }

    #[tokio::test]
    async fn place_surface_records_call() {
        let adapter = InMemoryAdapter::new();
        let info = SurfaceInfo::window(SurfaceId::new(), 1);
        let rect = crate::display::Rect { x: 10.0, y: 20.0, w: 300.0, h: 400.0 };
        adapter.place_surface(&info, rect).await.unwrap();
        assert_eq!(adapter.place_surface_calls().await.len(), 1);
    }

    #[tokio::test]
    async fn snapshot_geometry_returns_default() {
        let adapter = InMemoryAdapter::new();
        let info = SurfaceInfo::window(SurfaceId::new(), 1);
        let snap = adapter.snapshot_geometry(&info).await.unwrap();
        assert_eq!(snap.display_local.w, 800.0);
    }
```

- [ ] **Step 4: Add `todo!()` stubs to macOS adapter**

Edit `crates/porthole-adapter-macos/src/lib.rs`. In the `impl Adapter for MacOsAdapter` block:

```rust
    async fn launch_artifact(
        &self,
        _spec: &porthole_core::adapter::ArtifactLaunchSpec,
    ) -> Result<porthole_core::adapter::LaunchOutcome, porthole_core::PortholeError> {
        todo!("implemented in slice-C Task 18")
    }

    async fn place_surface(
        &self,
        _surface: &porthole_core::surface::SurfaceInfo,
        _rect: porthole_core::display::Rect,
    ) -> Result<(), porthole_core::PortholeError> {
        todo!("implemented in slice-C Task 16")
    }

    async fn snapshot_geometry(
        &self,
        _surface: &porthole_core::surface::SurfaceInfo,
    ) -> Result<porthole_core::placement::GeometrySnapshot, porthole_core::PortholeError> {
        todo!("implemented in slice-C Task 17")
    }
```

- [ ] **Step 5: Build + test**

Run:
```
cargo test -p porthole-core --lib in_memory
cargo build --workspace --locked
```

Expected: in-memory tests pass (3 new + existing), workspace builds with stubs.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "feat(core): extend Adapter with launch_artifact, place_surface, snapshot_geometry"
```

---

## Task 5: `LaunchPipeline` grows placement resolution + `PlacementOutcome` + `require_fresh_surface`

**Files:**
- Modify: `crates/porthole-core/src/launch.rs`

Context: the existing `LaunchPipeline::launch_process` dispatches to the adapter, enforces confidence, and inserts into HandleStore. Slice C extends it to:
1. Handle both `process` and `artifact` kinds (via `LaunchSpec` enum).
2. Check `require_fresh_surface` against `surface_was_preexisting`, converting to `LaunchReturnedExisting` on violation.
3. Resolve `PlacementSpec` to a global rect and call `adapter.place_surface`.
4. Track the outcome and return it alongside the `LaunchOutcome`.

- [ ] **Step 1: Define `LaunchPipelineOutcome`**

Edit `crates/porthole-core/src/launch.rs`. Add new public types for the pipeline's richer return value and the existing-surface error body:

```rust
use crate::placement::{Anchor, DisplayTarget, PlacementOutcome, PlacementSpec};
use crate::display::{DisplayId, Rect};

pub struct LaunchPipelineOutcome {
    pub outcome: LaunchOutcome,
    pub placement: PlacementOutcome,
}

/// Error body for LaunchReturnedExisting. Callers get everything needed to
/// attach the preexisting surface as a fallback without re-running search.
#[derive(Debug)]
pub struct ExistingSurfaceInfo {
    pub ref_: String,       // slice-B opaque ref via porthole_core::search::encode_ref
    pub app_name: Option<String>,
    pub title: Option<String>,
    pub pid: u32,
    pub cg_window_id: u32,
}
```

- [ ] **Step 2: Extend pipeline with artifact + placement dispatch**

Replace the existing `launch_process` body with a unified entry point. Keep the old method as a thin wrapper for backward compatibility so existing callers (no placement) keep working:

```rust
impl LaunchPipeline {
    pub async fn launch(
        &self,
        spec: &LaunchSpec,
        placement: Option<&PlacementSpec>,
    ) -> Result<LaunchPipelineOutcome, LaunchPipelineError> {
        // 1. Dispatch to the right adapter method.
        let outcome = match spec {
            LaunchSpec::Process(p) => self.adapter.launch_process(p).await?,
            LaunchSpec::Artifact(a) => self.adapter.launch_artifact(a).await?,
        };

        // 2. Confidence gate.
        if !outcome.confidence.meets(spec.require_confidence()) {
            return Err(LaunchPipelineError::Porthole(PortholeError::new(
                ErrorCode::LaunchCorrelationAmbiguous,
                format!(
                    "launch correlation returned confidence {:?}; required {:?}",
                    outcome.confidence, spec.require_confidence()
                ),
            )));
        }

        // 3. Fresh-surface gate (requires slice B's search::encode_ref).
        if spec.require_fresh_surface() && outcome.surface_was_preexisting {
            let ref_ = crate::search::encode_ref(
                outcome.surface.pid.unwrap_or(0),
                outcome.surface.cg_window_id.unwrap_or(0),
            );
            return Err(LaunchPipelineError::ReturnedExisting(ExistingSurfaceInfo {
                ref_,
                app_name: outcome.surface.app_name.clone(),
                title: outcome.surface.title.clone(),
                pid: outcome.surface.pid.unwrap_or(0),
                cg_window_id: outcome.surface.cg_window_id.unwrap_or(0),
            }));
        }

        // 4. Insert the handle.
        self.handles.insert(outcome.surface.clone()).await;

        // 5. Resolve + apply placement.
        let placement_outcome = if outcome.surface_was_preexisting {
            if placement.map(|p| !p.is_effectively_empty()).unwrap_or(false) {
                PlacementOutcome::SkippedPreexisting
            } else {
                PlacementOutcome::NotRequested
            }
        } else {
            self.apply_placement(&outcome.surface, placement).await
        };

        Ok(LaunchPipelineOutcome { outcome, placement: placement_outcome })
    }

    async fn apply_placement(
        &self,
        surface: &SurfaceInfo,
        placement: Option<&PlacementSpec>,
    ) -> PlacementOutcome {
        let Some(p) = placement else { return PlacementOutcome::NotRequested; };
        if p.is_effectively_empty() {
            return PlacementOutcome::NotRequested;
        }

        match resolve_placement_rect(p, &self.adapter).await {
            Ok(rect) => match self.adapter.place_surface(surface, rect).await {
                Ok(()) => PlacementOutcome::Applied,
                Err(e) => PlacementOutcome::Failed { reason: e.message },
            },
            Err(reason) => PlacementOutcome::Failed { reason },
        }
    }
}

/// Resolve a PlacementSpec to a global screen rectangle. Uses the adapter's
/// displays() and attention() to find target display; applies anchor/geometry
/// semantics per spec §5.
async fn resolve_placement_rect(
    spec: &PlacementSpec,
    adapter: &Arc<dyn Adapter>,
) -> Result<Rect, String> {
    let displays = adapter.displays().await.map_err(|e| e.message)?;
    if displays.is_empty() {
        return Err("no displays enumerated".into());
    }

    // 1. Determine target display.
    let target = match &spec.on_display {
        Some(DisplayTarget::Id(id)) => displays
            .iter()
            .find(|d| &d.id == id)
            .cloned()
            .ok_or_else(|| format!("unknown display id '{}'", id.as_str()))?,
        Some(DisplayTarget::Primary) => displays
            .iter()
            .find(|d| d.primary)
            .cloned()
            .unwrap_or_else(|| displays[0].clone()),
        Some(DisplayTarget::Focused) => {
            let attn = adapter.attention().await.map_err(|e| e.message)?;
            match attn.focused_display_id {
                Some(id) => displays.iter().find(|d| d.id == id).cloned().unwrap_or_else(|| displays[0].clone()),
                None => displays.iter().find(|d| d.primary).cloned().unwrap_or_else(|| displays[0].clone()),
            }
        }
        None => match spec.anchor {
            Some(Anchor::Cursor) => {
                let attn = adapter.attention().await.map_err(|e| e.message)?;
                displays
                    .iter()
                    .find(|d| {
                        attn.cursor.x >= d.bounds.x
                            && attn.cursor.x < d.bounds.x + d.bounds.w
                            && attn.cursor.y >= d.bounds.y
                            && attn.cursor.y < d.bounds.y + d.bounds.h
                    })
                    .cloned()
                    .unwrap_or_else(|| displays[0].clone())
            }
            Some(Anchor::FocusedDisplay) => {
                let attn = adapter.attention().await.map_err(|e| e.message)?;
                match attn.focused_display_id {
                    Some(id) => displays.iter().find(|d| d.id == id).cloned().unwrap_or_else(|| displays[0].clone()),
                    None => displays.iter().find(|d| d.primary).cloned().unwrap_or_else(|| displays[0].clone()),
                }
            }
            None => {
                // Geometry supplied without on_display or anchor — applies to primary.
                displays.iter().find(|d| d.primary).cloned().unwrap_or_else(|| displays[0].clone())
            }
        },
    };

    // 2. Compute geometry on that display.
    let global = if let Some(local) = &spec.geometry {
        Rect {
            x: target.bounds.x + local.x,
            y: target.bounds.y + local.y,
            w: local.w,
            h: local.h,
        }
    } else {
        // No explicit geometry — use a conservative centered default based on display size.
        // Anchor: center of display, with some default size (70% of display width/height, capped).
        let w = (target.bounds.w * 0.7).min(1400.0);
        let h = (target.bounds.h * 0.7).min(1000.0);
        let x = target.bounds.x + (target.bounds.w - w) / 2.0;
        let y = target.bounds.y + (target.bounds.h - h) / 2.0;
        Rect { x, y, w, h }
    };

    Ok(global)
}

#[derive(Debug)]
pub enum LaunchPipelineError {
    Porthole(PortholeError),
    ReturnedExisting(ExistingSurfaceInfo),
}

impl From<PortholeError> for LaunchPipelineError {
    fn from(e: PortholeError) -> Self {
        Self::Porthole(e)
    }
}
```

Keep the existing `launch_process` method as a thin wrapper for backward compatibility:

```rust
    /// Backward-compat: legacy launch_process entry used by routes that
    /// haven't migrated to the unified `launch()` API yet. Calls the new
    /// unified path with no placement.
    pub async fn launch_process(&self, spec: &ProcessLaunchSpec) -> Result<LaunchOutcome, PortholeError> {
        match self.launch(&LaunchSpec::Process(spec.clone()), None).await {
            Ok(out) => Ok(out.outcome),
            Err(LaunchPipelineError::Porthole(e)) => Err(e),
            Err(LaunchPipelineError::ReturnedExisting(_)) => Err(PortholeError::new(
                ErrorCode::LaunchReturnedExisting,
                "process launch returned existing (should be unreachable for process kind)",
            )),
        }
    }
```

- [ ] **Step 3: Update the existing tests to match**

Edit `crates/porthole-core/src/launch.rs` tests. The existing `launch_process` pipeline tests continue to use the old entry point; they should still pass. Add new tests for the unified path:

```rust
    #[tokio::test]
    async fn artifact_launch_via_unified_entry_point() {
        let adapter = Arc::new(InMemoryAdapter::new());
        let handles = HandleStore::new();
        let pipeline = LaunchPipeline::new(adapter.clone(), handles.clone());
        let spec = LaunchSpec::Artifact(ArtifactLaunchSpec {
            path: "/tmp/x.pdf".into(),
            require_confidence: RequireConfidence::Strong,
            require_fresh_surface: false,
            timeout: std::time::Duration::from_secs(5),
        });
        let result = pipeline.launch(&spec, None).await.unwrap();
        assert_eq!(result.placement, PlacementOutcome::NotRequested);
        assert_eq!(adapter.launch_artifact_calls().await.len(), 1);
    }

    #[tokio::test]
    async fn require_fresh_surface_errors_on_preexisting() {
        let adapter = Arc::new(InMemoryAdapter::new());
        let handles = HandleStore::new();
        let mut outcome = InMemoryAdapter::make_default_launch_outcome(77);
        outcome.surface_was_preexisting = true;
        outcome.surface.cg_window_id = Some(42);
        adapter.set_next_launch_artifact_outcome(Ok(outcome)).await;

        let pipeline = LaunchPipeline::new(adapter.clone(), handles);
        let spec = LaunchSpec::Artifact(ArtifactLaunchSpec {
            path: "/tmp/x.pdf".into(),
            require_confidence: RequireConfidence::Strong,
            require_fresh_surface: true,
            timeout: std::time::Duration::from_secs(5),
        });
        match pipeline.launch(&spec, None).await {
            Err(LaunchPipelineError::ReturnedExisting(info)) => {
                assert_eq!(info.cg_window_id, 42);
                assert!(info.ref_.starts_with("ref_"));
            }
            other => panic!("expected ReturnedExisting, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn placement_applied_on_fresh_launch() {
        let adapter = Arc::new(InMemoryAdapter::new());
        let handles = HandleStore::new();
        let pipeline = LaunchPipeline::new(adapter.clone(), handles);
        let spec = LaunchSpec::Process(spec_minimal(RequireConfidence::Strong));
        let placement = PlacementSpec {
            on_display: Some(DisplayTarget::Primary),
            geometry: Some(Rect { x: 10.0, y: 20.0, w: 800.0, h: 600.0 }),
            anchor: None,
        };
        let result = pipeline.launch(&spec, Some(&placement)).await.unwrap();
        assert_eq!(result.placement, PlacementOutcome::Applied);
        let place_calls = adapter.place_surface_calls().await;
        assert_eq!(place_calls.len(), 1);
    }

    #[tokio::test]
    async fn placement_skipped_on_preexisting() {
        let adapter = Arc::new(InMemoryAdapter::new());
        let handles = HandleStore::new();
        let mut outcome = InMemoryAdapter::make_default_launch_outcome(77);
        outcome.surface_was_preexisting = true;
        adapter.set_next_launch_outcome(Ok(outcome)).await;
        let pipeline = LaunchPipeline::new(adapter.clone(), handles);

        let placement = PlacementSpec {
            on_display: Some(DisplayTarget::Primary),
            geometry: Some(Rect { x: 0.0, y: 0.0, w: 500.0, h: 500.0 }),
            anchor: None,
        };
        let spec = LaunchSpec::Process(spec_minimal(RequireConfidence::Strong));
        let result = pipeline.launch(&spec, Some(&placement)).await.unwrap();
        assert_eq!(result.placement, PlacementOutcome::SkippedPreexisting);
        assert!(adapter.place_surface_calls().await.is_empty());
    }

    #[tokio::test]
    async fn placement_failure_reported_as_outcome_not_error() {
        let adapter = Arc::new(InMemoryAdapter::new());
        adapter
            .set_next_place_surface_result(Err(PortholeError::new(
                ErrorCode::CapabilityMissing,
                "window refused resize",
            )))
            .await;
        let handles = HandleStore::new();
        let pipeline = LaunchPipeline::new(adapter.clone(), handles);
        let placement = PlacementSpec {
            on_display: Some(DisplayTarget::Primary),
            geometry: Some(Rect { x: 0.0, y: 0.0, w: 200.0, h: 200.0 }),
            anchor: None,
        };
        let spec = LaunchSpec::Process(spec_minimal(RequireConfidence::Strong));
        let result = pipeline.launch(&spec, Some(&placement)).await.unwrap();
        match result.placement {
            PlacementOutcome::Failed { reason } => assert!(reason.contains("refused")),
            _ => panic!("expected Failed"),
        }
    }

    fn spec_minimal(rc: RequireConfidence) -> ProcessLaunchSpec {
        ProcessLaunchSpec {
            app: "X".into(),
            args: vec![],
            cwd: None,
            env: vec![],
            timeout: std::time::Duration::from_secs(5),
            require_confidence: rc,
            require_fresh_surface: false,
        }
    }
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p porthole-core --lib launch`
Expected: all existing tests still pass + 5 new. Confirm overall workspace test count doesn't regress.

- [ ] **Step 5: Commit**

```bash
git add crates/porthole-core/src/launch.rs
git commit -m "feat(core): LaunchPipeline dispatches by kind, resolves placement, enforces require_fresh_surface"
```

---

## Task 6: Auto-dismiss timer in `HandleStore`

**Files:**
- Modify: `crates/porthole-core/src/handle.rs`

Context: when a launch specifies `auto_dismiss_after_ms`, the daemon spawns a tokio task that closes the surface after the delay. The timer is not cancellable via the handle store in this slice — if the surface dies earlier for any reason (explicit close, external death detection, replace), the timer's close call becomes a no-op (the adapter's close returns `SurfaceDead` which the timer swallows). This is acceptable v0 behaviour; richer cancellation lands if a future slice needs "extend timer" or "cancel timer explicitly."

No new type or state in HandleStore. The timer task closes over an `Arc` of the adapter and the surface id and calls `adapter.close(surface)` + `handles.mark_dead(surface_id)` after sleeping. This keeps HandleStore simple and reuses existing close paths.

- [ ] **Step 1: Add a helper for scheduling auto-dismiss**

Edit `crates/porthole-core/src/handle.rs`. Add a free function (not a method on HandleStore — this couples to the adapter, and HandleStore doesn't know about adapters):

Actually, cleaner: add this helper to `crates/porthole-core/src/launch.rs` instead, since it's close-related logic tied to the launch result:

Edit `crates/porthole-core/src/launch.rs`:

```rust
use std::time::Duration;

/// Schedule an auto-dismiss of the surface after `delay`. Returns a JoinHandle
/// that the caller can abort to cancel early. Fire-and-forget is also fine —
/// the timer swallows dead-surface errors.
pub fn schedule_auto_dismiss(
    adapter: Arc<dyn Adapter>,
    handles: HandleStore,
    surface_id: SurfaceId,
    delay: Duration,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        tokio::time::sleep(delay).await;
        // Best-effort close. Errors are expected if the surface already died.
        if let Ok(info) = handles.require_alive(&surface_id).await {
            if adapter.close(&info).await.is_ok() {
                let _ = handles.mark_dead(&surface_id).await;
            }
        }
    })
}
```

- [ ] **Step 2: Add test**

Append to the `tests` module in `launch.rs`:

```rust
    #[tokio::test]
    async fn auto_dismiss_closes_surface_after_delay() {
        let adapter = Arc::new(InMemoryAdapter::new());
        let handles = HandleStore::new();
        let info = SurfaceInfo::window(SurfaceId::new(), 1);
        let id = info.id.clone();
        handles.insert(info).await;

        let _handle = schedule_auto_dismiss(adapter.clone(), handles.clone(), id.clone(), Duration::from_millis(20));
        tokio::time::sleep(Duration::from_millis(60)).await;

        // Expect adapter.close was called once and handle is dead.
        assert_eq!(adapter.close_calls().await.len(), 1);
        let err = handles.require_alive(&id).await.unwrap_err();
        assert_eq!(err.code, ErrorCode::SurfaceDead);
    }

    #[tokio::test]
    async fn auto_dismiss_is_noop_when_surface_already_dead() {
        let adapter = Arc::new(InMemoryAdapter::new());
        let handles = HandleStore::new();
        let info = SurfaceInfo::window(SurfaceId::new(), 1);
        let id = info.id.clone();
        handles.insert(info).await;
        handles.mark_dead(&id).await.unwrap();

        schedule_auto_dismiss(adapter.clone(), handles.clone(), id.clone(), Duration::from_millis(10));
        tokio::time::sleep(Duration::from_millis(40)).await;

        assert_eq!(adapter.close_calls().await.len(), 0);
    }
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p porthole-core --lib launch::tests::auto_dismiss`
Expected: 2 new passes.

- [ ] **Step 4: Commit**

```bash
git add crates/porthole-core/src/launch.rs
git commit -m "feat(core): schedule_auto_dismiss helper for auto-dismiss timers"
```

---

## Task 7: `ReplacePipeline`

**Files:**
- Create: `crates/porthole-core/src/replace_pipeline.rs`
- Modify: `crates/porthole-core/src/lib.rs`

Context: the replace pipeline coordinates the snapshot → close → launch → placement-inherit sequence. See spec §6.2 for semantics. The key invariant: `placement: {}` (or any supplied value) is used verbatim; only the completely-absent `placement` key (represented as `Option::None` in core) triggers inheritance.

- [ ] **Step 1: Write `replace_pipeline.rs`**

Create `crates/porthole-core/src/replace_pipeline.rs`:

```rust
use std::sync::Arc;

use crate::adapter::Adapter;
use crate::display::Rect;
use crate::handle::HandleStore;
use crate::launch::{ExistingSurfaceInfo, LaunchPipeline, LaunchPipelineError, LaunchPipelineOutcome};
use crate::placement::{DisplayTarget, PlacementSpec};
use crate::surface::SurfaceId;
use crate::{ErrorCode, PortholeError};

pub struct ReplacePipeline {
    adapter: Arc<dyn Adapter>,
    handles: HandleStore,
    launch: Arc<LaunchPipeline>,
}

pub struct ReplaceOutcome {
    pub new: LaunchPipelineOutcome,
    pub old_surface_id: SurfaceId,
}

#[derive(Debug)]
pub enum ReplacePipelineError {
    Porthole(PortholeError),
    ReturnedExisting { info: ExistingSurfaceInfo, old_handle_alive: bool },
    CloseFailed { old_handle_alive: bool, reason: String },
}

impl From<PortholeError> for ReplacePipelineError {
    fn from(e: PortholeError) -> Self {
        Self::Porthole(e)
    }
}

impl ReplacePipeline {
    pub fn new(adapter: Arc<dyn Adapter>, handles: HandleStore, launch: Arc<LaunchPipeline>) -> Self {
        Self { adapter, handles, launch }
    }

    pub async fn replace(
        &self,
        old_id: &SurfaceId,
        new_spec: &crate::adapter::LaunchSpec,
        caller_placement: Option<&PlacementSpec>,
    ) -> Result<ReplaceOutcome, ReplacePipelineError> {
        // 1. Snapshot (best-effort — snapshot failure doesn't abort).
        let old_info = self
            .handles
            .require_alive(old_id)
            .await
            .map_err(ReplacePipelineError::Porthole)?;
        let snapshot = self.adapter.snapshot_geometry(&old_info).await.ok();

        // 2. Close old.
        if let Err(e) = self.adapter.close(&old_info).await {
            // Old handle stays alive — don't mark dead.
            return Err(ReplacePipelineError::CloseFailed {
                old_handle_alive: true,
                reason: e.message,
            });
        }
        self.handles
            .mark_dead(old_id)
            .await
            .map_err(ReplacePipelineError::Porthole)?;

        // 3. Inheritance: inject snapshot only if caller_placement is None AND we have a snapshot.
        let inherited = match (caller_placement, snapshot) {
            (None, Some(snap)) => Some(PlacementSpec {
                on_display: Some(DisplayTarget::Id(snap.display_id)),
                geometry: Some(snap.display_local),
                anchor: None,
            }),
            _ => None,
        };
        let effective = inherited.as_ref().or(caller_placement);

        // 4. Launch the replacement.
        match self.launch.launch(new_spec, effective).await {
            Ok(out) => Ok(ReplaceOutcome { new: out, old_surface_id: old_id.clone() }),
            Err(LaunchPipelineError::Porthole(e)) => Err(ReplacePipelineError::Porthole(e)),
            Err(LaunchPipelineError::ReturnedExisting(info)) => Err(ReplacePipelineError::ReturnedExisting {
                info,
                old_handle_alive: false, // old was already closed by step 2
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::{ArtifactLaunchSpec, LaunchSpec, RequireConfidence};
    use crate::display::DisplayId;
    use crate::in_memory::InMemoryAdapter;
    use crate::placement::{GeometrySnapshot, PlacementOutcome};
    use crate::surface::SurfaceInfo;

    async fn tracked_surface(handles: &HandleStore, pid: u32, cg: u32) -> SurfaceId {
        let mut info = SurfaceInfo::window(SurfaceId::new(), pid);
        info.cg_window_id = Some(cg);
        let id = info.id.clone();
        handles.insert(info).await;
        id
    }

    fn artifact_spec(path: &str, fresh: bool) -> LaunchSpec {
        LaunchSpec::Artifact(ArtifactLaunchSpec {
            path: path.into(),
            require_confidence: RequireConfidence::Strong,
            require_fresh_surface: fresh,
            timeout: std::time::Duration::from_secs(5),
        })
    }

    #[tokio::test]
    async fn replace_inherits_snapshot_when_placement_absent() {
        let adapter = Arc::new(InMemoryAdapter::new());
        let handles = HandleStore::new();
        let launch = Arc::new(LaunchPipeline::new(adapter.clone(), handles.clone()));
        let replace = ReplacePipeline::new(adapter.clone(), handles.clone(), launch);

        let old_id = tracked_surface(&handles, 100, 50).await;
        adapter
            .set_next_snapshot_geometry(Ok(GeometrySnapshot {
                display_id: DisplayId::new("disp_1"),
                display_local: Rect { x: 100.0, y: 50.0, w: 800.0, h: 600.0 },
            }))
            .await;

        let out = replace.replace(&old_id, &artifact_spec("/tmp/new.pdf", false), None).await.unwrap();
        assert_eq!(out.old_surface_id, old_id);
        assert_eq!(out.new.placement, PlacementOutcome::Applied);
        // Old handle is dead now.
        let err = handles.require_alive(&old_id).await.unwrap_err();
        assert_eq!(err.code, ErrorCode::SurfaceDead);
    }

    #[tokio::test]
    async fn replace_with_empty_placement_does_not_inherit() {
        let adapter = Arc::new(InMemoryAdapter::new());
        let handles = HandleStore::new();
        let launch = Arc::new(LaunchPipeline::new(adapter.clone(), handles.clone()));
        let replace = ReplacePipeline::new(adapter.clone(), handles.clone(), launch);

        let old_id = tracked_surface(&handles, 100, 50).await;
        adapter
            .set_next_snapshot_geometry(Ok(GeometrySnapshot {
                display_id: DisplayId::new("disp_1"),
                display_local: Rect { x: 100.0, y: 50.0, w: 800.0, h: 600.0 },
            }))
            .await;

        // Caller passes Some(PlacementSpec::default()) — empty but present.
        let empty = PlacementSpec::default();
        let out = replace.replace(&old_id, &artifact_spec("/tmp/new.pdf", false), Some(&empty)).await.unwrap();
        assert_eq!(out.new.placement, PlacementOutcome::NotRequested);
        // place_surface should NOT have been called since placement was effectively empty.
        assert!(adapter.place_surface_calls().await.is_empty());
    }

    #[tokio::test]
    async fn replace_close_failure_keeps_old_handle_alive() {
        let adapter = Arc::new(InMemoryAdapter::new());
        let handles = HandleStore::new();
        let launch = Arc::new(LaunchPipeline::new(adapter.clone(), handles.clone()));
        let replace = ReplacePipeline::new(adapter.clone(), handles.clone(), launch);

        let old_id = tracked_surface(&handles, 100, 50).await;
        adapter
            .set_next_close_result(Err(PortholeError::new(
                ErrorCode::CloseFailed,
                "save dialog blocking close",
            )))
            .await;

        match replace.replace(&old_id, &artifact_spec("/tmp/new.pdf", false), None).await {
            Err(ReplacePipelineError::CloseFailed { old_handle_alive, .. }) => {
                assert!(old_handle_alive);
                // Old handle still alive.
                assert!(handles.require_alive(&old_id).await.is_ok());
            }
            other => panic!("expected CloseFailed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn replace_returned_existing_kills_old_handle() {
        let adapter = Arc::new(InMemoryAdapter::new());
        let handles = HandleStore::new();
        let launch = Arc::new(LaunchPipeline::new(adapter.clone(), handles.clone()));
        let replace = ReplacePipeline::new(adapter.clone(), handles.clone(), launch);

        let old_id = tracked_surface(&handles, 100, 50).await;

        // Script a fresh_surface violation on the replacement launch.
        let mut outcome = InMemoryAdapter::make_default_launch_outcome(77);
        outcome.surface_was_preexisting = true;
        outcome.surface.cg_window_id = Some(99);
        adapter.set_next_launch_artifact_outcome(Ok(outcome)).await;

        match replace.replace(&old_id, &artifact_spec("/tmp/new.pdf", true), None).await {
            Err(ReplacePipelineError::ReturnedExisting { old_handle_alive, .. }) => {
                assert!(!old_handle_alive, "old should have been closed in step 2");
                let err = handles.require_alive(&old_id).await.unwrap_err();
                assert_eq!(err.code, ErrorCode::SurfaceDead);
            }
            other => panic!("expected ReturnedExisting, got {other:?}"),
        }
    }
}
```

- [ ] **Step 2: Register module**

Edit `crates/porthole-core/src/lib.rs`. Add:

```rust
pub mod replace_pipeline;
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p porthole-core --lib replace_pipeline`
Expected: 4 passes.

- [ ] **Step 4: Commit**

```bash
git add crates/porthole-core/src/replace_pipeline.rs crates/porthole-core/src/lib.rs
git commit -m "feat(core): add ReplacePipeline with snapshot/close/launch/inherit sequencing"
```

---

## Task 8: Protocol — extend launches + add replace + rich error bodies

**Files:**
- Modify: `crates/porthole-protocol/src/launches.rs`
- Modify: `crates/porthole-protocol/src/error.rs`

- [ ] **Step 1: Extend `LaunchKind` with `Artifact` variant**

Edit `crates/porthole-protocol/src/launches.rs`. Current enum has just `Process(ProcessLaunch)`. Extend:

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LaunchKind {
    Process(ProcessLaunch),
    Artifact(ArtifactLaunch),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ArtifactLaunch {
    pub path: String,
}
```

Add new fields to `LaunchRequest`:

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LaunchRequest {
    pub kind: LaunchKind,
    #[serde(default)]
    pub session: Option<String>,
    #[serde(default = "default_require_confidence")]
    pub require_confidence: WireConfidence,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
    // NEW:
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placement: Option<PlacementSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_dismiss_after_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub require_fresh_surface: bool,
}
```

Add `PlacementSpec` wire type (re-export from core):

```rust
pub use porthole_core::placement::{Anchor, DisplayTarget, PlacementOutcome, PlacementSpec};
```

And also re-export the Rect type used within PlacementSpec:

```rust
pub use porthole_core::display::Rect as PlacementRect;
```

Extend `LaunchResponse`:

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LaunchResponse {
    pub launch_id: String,
    pub surface_id: SurfaceId,
    pub surface_was_preexisting: bool,
    pub confidence: WireConfidence,
    pub correlation: WireCorrelation,
    pub placement: PlacementOutcome,
}
```

- [ ] **Step 2: Extend error body**

Edit `crates/porthole-protocol/src/error.rs`. The current `WireError` has `code`, `message`, `details`. Make `details` a tagged enum to carry slice-C–specific structured bodies:

Actually — the existing `details: Option<serde_json::Value>` from the quality round is flexible enough. New bodies are passed as JSON:

```rust
// For LaunchReturnedExisting:
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LaunchReturnedExistingBody {
    pub ref_: String,
    pub app_name: Option<String>,
    pub title: Option<String>,
    pub pid: u32,
    pub cg_window_id: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CloseFailedBody {
    pub old_handle_alive: bool,
}
```

Serde rename `ref_` to `ref` on the wire:

```rust
pub struct LaunchReturnedExistingBody {
    #[serde(rename = "ref")]
    pub ref_: String,
    pub app_name: Option<String>,
    pub title: Option<String>,
    pub pid: u32,
    pub cg_window_id: u32,
}
```

- [ ] **Step 3: Add a `ReplaceRequest` alias (just a LaunchRequest)**

Replace requests are semantically a LaunchRequest plus the path-level old surface id. Add:

```rust
// ReplaceRequest is structurally identical to LaunchRequest — the old
// surface id comes from the URL path parameter. Keep the alias for clarity
// in route handlers and OpenAPI generation later.
pub type ReplaceRequest = LaunchRequest;
```

- [ ] **Step 4: Add tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn artifact_launch_kind_serializes_with_path() {
        let json = serde_json::json!({
            "type": "artifact",
            "path": "/tmp/x.pdf"
        });
        let k: LaunchKind = serde_json::from_value(json).unwrap();
        match k {
            LaunchKind::Artifact(a) => assert_eq!(a.path, "/tmp/x.pdf"),
            _ => panic!("expected artifact"),
        }
    }

    #[test]
    fn launch_request_placement_key_absent_deserializes_as_none() {
        let json = r#"{"kind":{"type":"process","app":"x"}}"#;
        let req: LaunchRequest = serde_json::from_str(json).unwrap();
        assert!(req.placement.is_none());
    }

    #[test]
    fn launch_request_placement_empty_object_deserializes_as_some_default() {
        let json = r#"{"kind":{"type":"process","app":"x"},"placement":{}}"#;
        let req: LaunchRequest = serde_json::from_str(json).unwrap();
        assert!(req.placement.is_some());
        assert!(req.placement.unwrap().is_effectively_empty());
    }

    #[test]
    fn placement_outcome_applied_serializes() {
        let o = PlacementOutcome::Applied;
        assert_eq!(serde_json::to_string(&o).unwrap(), r#"{"type":"applied"}"#);
    }

    #[test]
    fn close_failed_body_carries_old_handle_alive() {
        let body = CloseFailedBody { old_handle_alive: true };
        let json = serde_json::to_string(&body).unwrap();
        assert_eq!(json, r#"{"old_handle_alive":true}"#);
    }
}
```

- [ ] **Step 5: Run tests**

Run: `cargo test -p porthole-protocol --lib`
Expected: all existing tests still pass + 5 new.

- [ ] **Step 6: Commit**

```bash
git add crates/porthole-protocol/src/launches.rs crates/porthole-protocol/src/error.rs
git commit -m "feat(protocol): ArtifactLaunch variant, placement/auto_dismiss/require_fresh fields, error body types"
```

---

## Task 9: Daemon — extend `/launches` + state + error mappers

**Files:**
- Modify: `crates/portholed/src/routes/launches.rs`
- Modify: `crates/portholed/src/routes/errors.rs`
- Modify: `crates/portholed/src/state.rs`

- [ ] **Step 1: Extend AppState with ReplacePipeline**

Edit `crates/portholed/src/state.rs`:

```rust
use porthole_core::replace_pipeline::ReplacePipeline;
```

In the struct:

```rust
    pub replace: Arc<ReplacePipeline>,
```

In `AppState::new`:

```rust
        let replace = Arc::new(ReplacePipeline::new(adapter.clone(), handles.clone(), pipeline.clone()));
```

And add `replace,` to the returned struct literal.

- [ ] **Step 2: Extend `errors.rs` to map the new errors**

Edit `crates/portholed/src/routes/errors.rs`. Add conversions:

```rust
use porthole_core::launch::{ExistingSurfaceInfo, LaunchPipelineError};
use porthole_core::replace_pipeline::ReplacePipelineError;
use porthole_protocol::error::{CloseFailedBody, LaunchReturnedExistingBody};

impl From<LaunchPipelineError> for ApiError {
    fn from(err: LaunchPipelineError) -> Self {
        match err {
            LaunchPipelineError::Porthole(e) => Self(e.into()),
            LaunchPipelineError::ReturnedExisting(info) => Self(existing_to_wire(info)),
        }
    }
}

impl From<ReplacePipelineError> for ApiError {
    fn from(err: ReplacePipelineError) -> Self {
        match err {
            ReplacePipelineError::Porthole(e) => Self(e.into()),
            ReplacePipelineError::ReturnedExisting { info, old_handle_alive: _ } => {
                // old_handle_alive is always false here (replace closed the old surface
                // before the launch step, by construction). No need to distinguish in
                // the wire body — only close_failed carries that flag.
                Self(existing_to_wire(info))
            }
            ReplacePipelineError::CloseFailed { old_handle_alive, reason } => {
                let body = CloseFailedBody { old_handle_alive };
                Self(WireError {
                    code: ErrorCode::CloseFailed,
                    message: reason,
                    details: serde_json::to_value(body).ok(),
                })
            }
        }
    }
}

fn existing_to_wire(info: ExistingSurfaceInfo) -> WireError {
    let body = LaunchReturnedExistingBody {
        ref_: info.ref_,
        app_name: info.app_name,
        title: info.title,
        pid: info.pid,
        cg_window_id: info.cg_window_id,
    };
    WireError {
        code: ErrorCode::LaunchReturnedExisting,
        message: "launch correlated to a preexisting surface (require_fresh_surface: true)".into(),
        details: serde_json::to_value(body).ok(),
    }
}
```

Add `LaunchReturnedExisting` to the HTTP status match:

```rust
            ErrorCode::LaunchReturnedExisting => StatusCode::CONFLICT,
```

- [ ] **Step 3: Update `launches.rs` route handler**

Edit `crates/portholed/src/routes/launches.rs`. Replace the existing handler with one that uses the unified pipeline:

```rust
use std::collections::BTreeMap;
use std::time::Duration;

use axum::extract::State;
use axum::Json;
use porthole_core::adapter::{ArtifactLaunchSpec, LaunchSpec, ProcessLaunchSpec, RequireConfidence};
use porthole_core::placement::PlacementOutcome;
use porthole_protocol::launches::{
    ArtifactLaunch, LaunchKind, LaunchRequest, LaunchResponse, WireConfidence, WireCorrelation,
};
use uuid::Uuid;

use crate::routes::errors::ApiError;
use crate::state::AppState;

pub async fn post_launches(
    State(state): State<AppState>,
    Json(req): Json<LaunchRequest>,
) -> Result<Json<LaunchResponse>, ApiError> {
    let spec = request_to_launch_spec(&req)?;
    let placement = req.placement.as_ref();
    let result = state.pipeline.launch(&spec, placement).await?;

    // Schedule auto-dismiss if requested.
    if let Some(ms) = req.auto_dismiss_after_ms {
        if ms > 0 {
            porthole_core::launch::schedule_auto_dismiss(
                state.adapter.clone(),
                state.handles.clone(),
                result.outcome.surface.id.clone(),
                Duration::from_millis(ms),
            );
        }
    }

    let launch_id = format!("launch_{}", Uuid::new_v4().simple());
    Ok(Json(LaunchResponse {
        launch_id,
        surface_id: result.outcome.surface.id.clone(),
        surface_was_preexisting: result.outcome.surface_was_preexisting,
        confidence: confidence_to_wire(result.outcome.confidence),
        correlation: correlation_to_wire(result.outcome.correlation),
        placement: result.placement,
    }))
}

fn request_to_launch_spec(req: &LaunchRequest) -> Result<LaunchSpec, ApiError> {
    let timeout = Duration::from_millis(req.timeout_ms);
    let require_confidence = wire_to_require(req.require_confidence);
    let require_fresh = req.require_fresh_surface;
    match &req.kind {
        LaunchKind::Process(p) => Ok(LaunchSpec::Process(ProcessLaunchSpec {
            app: p.app.clone(),
            args: p.args.clone(),
            cwd: p.cwd.clone(),
            env: to_env_vec(&p.env),
            timeout,
            require_confidence,
            require_fresh_surface: require_fresh,
        })),
        LaunchKind::Artifact(a) => {
            if a.path.starts_with("http://") || a.path.starts_with("https://") || a.path.starts_with("file://") {
                return Err(ApiError::from(porthole_core::PortholeError::new(
                    porthole_core::ErrorCode::InvalidArgument,
                    "URL paths are not supported in this slice (defer to browser-CDP)",
                )));
            }
            Ok(LaunchSpec::Artifact(ArtifactLaunchSpec {
                path: std::path::PathBuf::from(&a.path),
                require_confidence,
                require_fresh_surface: require_fresh,
                timeout,
            }))
        }
    }
}

// Reject auto_dismiss_after_ms = 0 at the route level. Non-zero is fine.
// This was added for clarity but isn't strictly required — callers passing 0
// just get a no-op (the if ms > 0 guard above). Better to reject explicitly:
// edit request_to_launch_spec to also return Err(InvalidArgument) when
// auto_dismiss_after_ms is Some(0). Left as an exercise for the implementer
// since the integration tests will exercise the invariant.

fn to_env_vec(map: &BTreeMap<String, String>) -> Vec<(String, String)> {
    map.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
}

fn wire_to_require(c: WireConfidence) -> RequireConfidence {
    match c {
        WireConfidence::Strong => RequireConfidence::Strong,
        WireConfidence::Plausible => RequireConfidence::Plausible,
        WireConfidence::Weak => RequireConfidence::Weak,
    }
}

fn confidence_to_wire(c: porthole_core::adapter::Confidence) -> WireConfidence {
    use porthole_core::adapter::Confidence::*;
    match c {
        Strong => WireConfidence::Strong,
        Plausible => WireConfidence::Plausible,
        Weak => WireConfidence::Weak,
    }
}

fn correlation_to_wire(c: porthole_core::adapter::Correlation) -> WireCorrelation {
    use porthole_core::adapter::Correlation::*;
    match c {
        Tag => WireCorrelation::Tag,
        PidTree => WireCorrelation::PidTree,
        Temporal => WireCorrelation::Temporal,
        DocumentMatch => WireCorrelation::DocumentMatch,
        FrontmostChanged => WireCorrelation::FrontmostChanged,
    }
}
```

Actually — the "edit request_to_launch_spec to reject zero" note is sloppy. Just add the check:

```rust
    if req.auto_dismiss_after_ms == Some(0) {
        return Err(ApiError::from(porthole_core::PortholeError::new(
            porthole_core::ErrorCode::InvalidArgument,
            "auto_dismiss_after_ms must be > 0",
        )));
    }
```

Put it at the top of `request_to_launch_spec` or inline in `post_launches` before calling the spec builder.

- [ ] **Step 4: Build + test**

Run: `cargo build -p portholed --lib`
Expected: clean.

Run: `cargo test -p portholed --lib`
Expected: existing tests still pass. Some may need minor updates if they construct `LaunchResponse` literals (now has `placement` field). Update test fixtures as needed.

- [ ] **Step 5: Commit**

```bash
git add crates/portholed/src/routes/launches.rs crates/portholed/src/routes/errors.rs crates/portholed/src/state.rs
git commit -m "feat(daemon): launches route handles artifact, placement, auto_dismiss; error mappers for new types"
```

---

## Task 10: Daemon — `POST /surfaces/{id}/replace` route

**Files:**
- Create: `crates/portholed/src/routes/replace.rs`
- Modify: `crates/portholed/src/routes/mod.rs`
- Modify: `crates/portholed/src/server.rs`

- [ ] **Step 1: Write `routes/replace.rs`**

```rust
use std::time::Duration;

use axum::extract::{Path, State};
use axum::Json;
use porthole_core::surface::SurfaceId;
use porthole_protocol::launches::{LaunchResponse, ReplaceRequest, WireConfidence, WireCorrelation};

use crate::routes::errors::ApiError;
use crate::routes::launches::{request_to_launch_spec, confidence_to_wire, correlation_to_wire};
use crate::state::AppState;

pub async fn post_replace(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<ReplaceRequest>,
) -> Result<Json<LaunchResponse>, ApiError> {
    // Same auto_dismiss validation as launches.
    if req.auto_dismiss_after_ms == Some(0) {
        return Err(ApiError::from(porthole_core::PortholeError::new(
            porthole_core::ErrorCode::InvalidArgument,
            "auto_dismiss_after_ms must be > 0",
        )));
    }

    let old_id = SurfaceId::from(id);
    let spec = request_to_launch_spec(&req)?;
    let placement = req.placement.as_ref();

    let out = state.replace.replace(&old_id, &spec, placement).await?;

    if let Some(ms) = req.auto_dismiss_after_ms {
        if ms > 0 {
            porthole_core::launch::schedule_auto_dismiss(
                state.adapter.clone(),
                state.handles.clone(),
                out.new.outcome.surface.id.clone(),
                Duration::from_millis(ms),
            );
        }
    }

    let launch_id = format!("launch_{}", uuid::Uuid::new_v4().simple());
    Ok(Json(LaunchResponse {
        launch_id,
        surface_id: out.new.outcome.surface.id.clone(),
        surface_was_preexisting: out.new.outcome.surface_was_preexisting,
        confidence: confidence_to_wire(out.new.outcome.confidence),
        correlation: correlation_to_wire(out.new.outcome.correlation),
        placement: out.new.placement,
    }))
}
```

Note: `request_to_launch_spec`, `confidence_to_wire`, `correlation_to_wire` need to be `pub(crate)` or `pub` in `routes/launches.rs`. Update them accordingly.

- [ ] **Step 2: Export helpers from `launches.rs`**

Edit `crates/portholed/src/routes/launches.rs` and change `fn request_to_launch_spec` / `fn confidence_to_wire` / `fn correlation_to_wire` to `pub(crate) fn`.

- [ ] **Step 3: Register module**

Edit `crates/portholed/src/routes/mod.rs`. Add:

```rust
pub mod replace;
```

- [ ] **Step 4: Wire route into server**

Edit `crates/portholed/src/server.rs`. Add the import:

```rust
use crate::routes::replace as replace_route;
```

In `build_router`:

```rust
        .route("/surfaces/{id}/replace", post(replace_route::post_replace))
```

- [ ] **Step 5: Add router tests**

Append to the `tests` module in `server.rs`:

```rust
    #[tokio::test]
    async fn post_replace_inherits_snapshot_when_no_placement() {
        use porthole_core::display::DisplayId;
        use porthole_core::display::Rect;
        use porthole_core::placement::GeometrySnapshot;
        use porthole_core::surface::{SurfaceId, SurfaceInfo};

        let adapter = Arc::new(InMemoryAdapter::new());
        // Seed an alive handle with cg_window_id.
        let mut old = SurfaceInfo::window(SurfaceId::new(), 1);
        old.cg_window_id = Some(50);
        let old_id = old.id.clone();
        let state = AppState::new(adapter.clone());
        state.handles.insert(old).await;

        adapter
            .set_next_snapshot_geometry(Ok(GeometrySnapshot {
                display_id: DisplayId::new("in-mem-display-0"),
                display_local: Rect { x: 10.0, y: 20.0, w: 500.0, h: 400.0 },
            }))
            .await;

        let router = build_router(state);
        let res = post(
            router,
            &format!("/surfaces/{old_id}/replace"),
            serde_json::json!({
                "kind": { "type": "artifact", "path": "/tmp/x.pdf" }
            }),
        )
        .await;
        assert_eq!(res.status(), StatusCode::OK);
        let body = to_bytes(res.into_body(), 1024 * 1024).await.unwrap();
        let resp: porthole_protocol::launches::LaunchResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(resp.placement, porthole_core::placement::PlacementOutcome::Applied);
    }

    #[tokio::test]
    async fn post_replace_with_empty_placement_does_not_inherit() {
        use porthole_core::surface::{SurfaceId, SurfaceInfo};

        let adapter = Arc::new(InMemoryAdapter::new());
        let mut old = SurfaceInfo::window(SurfaceId::new(), 1);
        old.cg_window_id = Some(51);
        let old_id = old.id.clone();
        let state = AppState::new(adapter.clone());
        state.handles.insert(old).await;

        let router = build_router(state);
        let res = post(
            router,
            &format!("/surfaces/{old_id}/replace"),
            serde_json::json!({
                "kind": { "type": "artifact", "path": "/tmp/x.pdf" },
                "placement": {}
            }),
        )
        .await;
        assert_eq!(res.status(), StatusCode::OK);
        let body = to_bytes(res.into_body(), 1024 * 1024).await.unwrap();
        let resp: porthole_protocol::launches::LaunchResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(resp.placement, porthole_core::placement::PlacementOutcome::NotRequested);
    }

    #[tokio::test]
    async fn post_launches_rejects_url_artifact() {
        let adapter = Arc::new(InMemoryAdapter::new());
        let router = build_router(AppState::new(adapter));
        let res = post(
            router,
            "/launches",
            serde_json::json!({
                "kind": { "type": "artifact", "path": "https://example.com" }
            }),
        )
        .await;
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn post_launches_require_fresh_returns_409_with_ref_in_body() {
        use porthole_core::in_memory::InMemoryAdapter;

        let adapter = Arc::new(InMemoryAdapter::new());
        let mut outcome = InMemoryAdapter::make_default_launch_outcome(100);
        outcome.surface_was_preexisting = true;
        outcome.surface.cg_window_id = Some(321);
        adapter.set_next_launch_artifact_outcome(Ok(outcome)).await;

        let router = build_router(AppState::new(adapter));
        let res = post(
            router,
            "/launches",
            serde_json::json!({
                "kind": { "type": "artifact", "path": "/tmp/x.pdf" },
                "require_fresh_surface": true
            }),
        )
        .await;
        assert_eq!(res.status(), StatusCode::CONFLICT);
        let body = to_bytes(res.into_body(), 1024 * 1024).await.unwrap();
        let err: porthole_protocol::error::WireError = serde_json::from_slice(&body).unwrap();
        assert_eq!(err.code, porthole_core::ErrorCode::LaunchReturnedExisting);
        let details = err.details.expect("details populated");
        assert!(details.get("ref").is_some());
        assert_eq!(details.get("cg_window_id").and_then(|v| v.as_u64()), Some(321));
    }
```

- [ ] **Step 6: Run tests**

Run: `cargo test -p portholed --lib`
Expected: existing tests pass + 4 new.

- [ ] **Step 7: Commit**

```bash
git add crates/portholed/src/routes/replace.rs crates/portholed/src/routes/mod.rs crates/portholed/src/server.rs crates/portholed/src/routes/launches.rs
git commit -m "feat(daemon): POST /surfaces/{id}/replace route + tests for artifact/placement/require_fresh"
```

---

## Task 11: Daemon — `/info` capability additions (via adapters)

**Files:**
- Modify: `crates/porthole-core/src/in_memory.rs`
- Modify: `crates/porthole-adapter-macos/src/lib.rs`

- [ ] **Step 1: Extend InMemoryAdapter capabilities**

Edit `crates/porthole-core/src/in_memory.rs`. In `capabilities()`, append:

```rust
        "launch_artifact",
        "placement",
        "replace",
        "auto_dismiss",
```

(The existing test `capabilities_non_empty_and_excludes_attention_focused_surface` keeps passing.)

- [ ] **Step 2: Extend macOS adapter capabilities**

Edit `crates/porthole-adapter-macos/src/lib.rs`. In `capabilities()`, append the same four entries.

- [ ] **Step 3: Tests**

Run: `cargo test --workspace --locked`
Expected: still pass. Consider adding one test to the `server::tests` module that checks `/info` includes the new capabilities:

```rust
    #[tokio::test]
    async fn info_lists_slice_c_capabilities() {
        let adapter = Arc::new(InMemoryAdapter::new());
        let router = build_router(AppState::new(adapter));
        let req = Request::builder().method(Method::GET).uri("/info").body(Body::empty()).unwrap();
        let res = router.oneshot(req).await.unwrap();
        let body = to_bytes(res.into_body(), 1024 * 1024).await.unwrap();
        let info: porthole_protocol::info::InfoResponse = serde_json::from_slice(&body).unwrap();
        let caps = &info.adapters[0].capabilities;
        for expected in &["launch_artifact", "placement", "replace", "auto_dismiss"] {
            assert!(caps.contains(&expected.to_string()), "missing capability: {expected}");
        }
    }
```

- [ ] **Step 4: Commit**

```bash
git add crates/porthole-core/src/in_memory.rs crates/porthole-adapter-macos/src/lib.rs crates/portholed/src/server.rs
git commit -m "feat(core,adapter-macos): advertise launch_artifact, placement, replace, auto_dismiss capabilities"
```

---

## Task 12: CLI — extend `launch` subcommand

**Files:**
- Modify: `crates/porthole/src/commands/launch.rs`
- Modify: `crates/porthole/src/main.rs`

- [ ] **Step 1: Extend `LaunchArgs` and handler**

Edit `crates/porthole/src/commands/launch.rs`. Update `LaunchArgs` and `run`:

```rust
use std::collections::BTreeMap;
use std::path::PathBuf;

use porthole_core::placement::{Anchor, DisplayTarget, PlacementSpec};
use porthole_core::display::Rect;
use porthole_protocol::launches::{
    ArtifactLaunch, LaunchKind, LaunchRequest, LaunchResponse, ProcessLaunch, WireConfidence,
};

use crate::client::{ClientError, DaemonClient};

pub enum LaunchKindArg {
    Process { app: String, args: Vec<String>, env: Vec<(String, String)>, cwd: Option<String> },
    Artifact { path: PathBuf },
}

pub struct LaunchArgs {
    pub kind: LaunchKindArg,
    pub session: Option<String>,
    pub timeout_ms: u64,
    pub require_confidence: WireConfidence,
    pub require_fresh_surface: bool,
    pub placement: Option<PlacementSpec>,
    pub auto_dismiss_after_ms: Option<u64>,
    pub json: bool,
}

pub async fn run(client: &DaemonClient, args: LaunchArgs) -> Result<(), ClientError> {
    let kind = match args.kind {
        LaunchKindArg::Process { app, args: a, env, cwd } => {
            let mut env_map = BTreeMap::new();
            for (k, v) in env {
                env_map.insert(k, v);
            }
            LaunchKind::Process(ProcessLaunch { app, args: a, cwd, env: env_map })
        }
        LaunchKindArg::Artifact { path } => LaunchKind::Artifact(ArtifactLaunch {
            path: path.to_string_lossy().to_string(),
        }),
    };
    let req = LaunchRequest {
        kind,
        session: args.session,
        require_confidence: args.require_confidence,
        timeout_ms: args.timeout_ms,
        placement: args.placement,
        auto_dismiss_after_ms: args.auto_dismiss_after_ms,
        require_fresh_surface: args.require_fresh_surface,
    };
    let res: LaunchResponse = client.post_json("/launches", &req).await?;
    if args.json {
        let text = serde_json::to_string_pretty(&res)
            .map_err(|e| ClientError::Local(format!("json encode: {e}")))?;
        println!("{text}");
    } else {
        println!("launch_id: {}", res.launch_id);
        println!("surface_id: {}", res.surface_id);
        println!("confidence: {:?}", res.confidence);
        println!("correlation: {:?}", res.correlation);
        println!("surface_was_preexisting: {}", res.surface_was_preexisting);
        println!("placement: {:?}", res.placement);
    }
    Ok(())
}
```

- [ ] **Step 2: Update `main.rs` Launch subcommand**

Edit `crates/porthole/src/main.rs`. The current `Launch` variant is process-only. Extend:

```rust
    /// Launch a process or an artifact.
    Launch {
        /// "process" or "artifact". Default "process".
        #[arg(long, value_enum, default_value_t = LaunchKindArg::Process)]
        kind: LaunchKindArg,
        /// Process: app bundle path or executable. Artifact: file path.
        #[arg(long)]
        app_or_path: String,
        /// Process: extra args (repeatable).
        #[arg(long = "arg", value_name = "ARG")]
        args: Vec<String>,
        /// Process: KEY=VALUE env vars.
        #[arg(long = "env", value_name = "KEY=VALUE")]
        env: Vec<String>,
        /// Process: working directory.
        #[arg(long)]
        cwd: Option<String>,
        /// Session tag.
        #[arg(long)]
        session: Option<String>,
        #[arg(long, default_value_t = 10_000)]
        timeout_ms: u64,
        #[arg(long, value_enum, default_value_t = ConfidenceArg::Strong)]
        require_confidence: ConfidenceArg,
        #[arg(long)]
        require_fresh_surface: bool,
        /// Placement: which display.
        #[arg(long, value_parser = parse_display_target)]
        on_display: Option<DisplayTarget>,
        /// Placement: x position (display-local logical points).
        #[arg(long, requires_all = ["geom_y", "geom_w", "geom_h"])]
        geom_x: Option<f64>,
        #[arg(long)]
        geom_y: Option<f64>,
        #[arg(long)]
        geom_w: Option<f64>,
        #[arg(long)]
        geom_h: Option<f64>,
        /// Placement: anchor strategy when no explicit geometry.
        #[arg(long, value_enum)]
        anchor: Option<AnchorArg>,
        /// Auto-dismiss delay in milliseconds.
        #[arg(long)]
        auto_dismiss_ms: Option<u64>,
        #[arg(long)]
        json: bool,
    },
```

Add supporting enums and parser:

```rust
#[derive(clap::ValueEnum, Clone, Copy, Debug)]
enum LaunchKindArg { Process, Artifact }

#[derive(clap::ValueEnum, Clone, Copy, Debug)]
enum AnchorArg { FocusedDisplay, Cursor }

impl From<AnchorArg> for Anchor {
    fn from(a: AnchorArg) -> Self {
        match a {
            AnchorArg::FocusedDisplay => Anchor::FocusedDisplay,
            AnchorArg::Cursor => Anchor::Cursor,
        }
    }
}

fn parse_display_target(s: &str) -> Result<DisplayTarget, String> {
    Ok(match s {
        "focused" => DisplayTarget::Focused,
        "primary" => DisplayTarget::Primary,
        _ => DisplayTarget::Id(porthole_core::display::DisplayId::new(s)),
    })
}
```

In the match arm:

```rust
        Command::Launch {
            kind, app_or_path, args, env, cwd, session, timeout_ms, require_confidence,
            require_fresh_surface, on_display, geom_x, geom_y, geom_w, geom_h, anchor,
            auto_dismiss_ms, json,
        } => {
            let kind_arg = match kind {
                LaunchKindArg::Process => {
                    let parsed_env: Vec<(String, String)> = env
                        .into_iter()
                        .filter_map(|s| s.split_once('=').map(|(k, v)| (k.to_string(), v.to_string())))
                        .collect();
                    launch_cmd::LaunchKindArg::Process {
                        app: app_or_path, args, env: parsed_env, cwd,
                    }
                }
                LaunchKindArg::Artifact => {
                    launch_cmd::LaunchKindArg::Artifact { path: std::path::PathBuf::from(app_or_path) }
                }
            };

            // Build placement if any related flag was given.
            let geometry = match (geom_x, geom_y, geom_w, geom_h) {
                (Some(x), Some(y), Some(w), Some(h)) => Some(Rect { x, y, w, h }),
                _ => None,
            };
            let placement = if on_display.is_some() || geometry.is_some() || anchor.is_some() {
                Some(PlacementSpec {
                    on_display,
                    geometry,
                    anchor: anchor.map(Anchor::from),
                })
            } else {
                None
            };

            launch_cmd::run(&client, launch_cmd::LaunchArgs {
                kind: kind_arg,
                session,
                timeout_ms,
                require_confidence: require_confidence.into(),
                require_fresh_surface,
                placement,
                auto_dismiss_after_ms: auto_dismiss_ms,
                json,
            }).await
        }
```

Add imports:

```rust
use porthole_core::placement::{Anchor, DisplayTarget, PlacementSpec};
use porthole_core::display::Rect;
use porthole::commands::launch as launch_cmd;
```

(Adjust `use` ordering as existing file requires.)

- [ ] **Step 3: Build**

Run: `cargo build -p porthole`
Expected: clean.

- [ ] **Step 4: Commit**

```bash
git add crates/porthole/src/commands/launch.rs crates/porthole/src/main.rs
git commit -m "feat(cli): launch subcommand supports artifact kind, placement, auto-dismiss, require-fresh"
```

---

## Task 13: CLI — `replace` subcommand

**Files:**
- Create: `crates/porthole/src/commands/replace.rs`
- Modify: `crates/porthole/src/commands/mod.rs`
- Modify: `crates/porthole/src/main.rs`

- [ ] **Step 1: Write `commands/replace.rs`**

```rust
use porthole_protocol::launches::{LaunchRequest, LaunchResponse};

use crate::client::{ClientError, DaemonClient};

pub async fn run(
    client: &DaemonClient,
    surface_id: String,
    req: LaunchRequest,
    json: bool,
) -> Result<(), ClientError> {
    let res: LaunchResponse = client.post_json(&format!("/surfaces/{surface_id}/replace"), &req).await?;
    if json {
        let text = serde_json::to_string_pretty(&res)
            .map_err(|e| ClientError::Local(format!("json encode: {e}")))?;
        println!("{text}");
    } else {
        println!("new surface_id: {}", res.surface_id);
        println!("old surface closed.");
    }
    Ok(())
}
```

- [ ] **Step 2: Register module**

Edit `crates/porthole/src/commands/mod.rs`. Add:

```rust
pub mod replace;
```

- [ ] **Step 3: Add `Replace` variant to main.rs**

The clap surface for `replace` mirrors `launch` (same body), plus a positional surface_id. To avoid duplicating the whole flag set, factor the shared flags into a separate struct. Simpler for v0: copy the flags. Edit `crates/porthole/src/main.rs`:

```rust
    /// Replace a tracked surface — close the old, launch the new in its slot.
    Replace {
        surface_id: String,
        #[arg(long, value_enum, default_value_t = LaunchKindArg::Process)]
        kind: LaunchKindArg,
        #[arg(long)]
        app_or_path: String,
        #[arg(long = "arg", value_name = "ARG")]
        args: Vec<String>,
        #[arg(long = "env", value_name = "KEY=VALUE")]
        env: Vec<String>,
        #[arg(long)]
        cwd: Option<String>,
        #[arg(long)]
        session: Option<String>,
        #[arg(long, default_value_t = 10_000)]
        timeout_ms: u64,
        #[arg(long, value_enum, default_value_t = ConfidenceArg::Strong)]
        require_confidence: ConfidenceArg,
        #[arg(long)]
        require_fresh_surface: bool,
        #[arg(long, value_parser = parse_display_target)]
        on_display: Option<DisplayTarget>,
        #[arg(long)]
        geom_x: Option<f64>,
        #[arg(long)]
        geom_y: Option<f64>,
        #[arg(long)]
        geom_w: Option<f64>,
        #[arg(long)]
        geom_h: Option<f64>,
        #[arg(long, value_enum)]
        anchor: Option<AnchorArg>,
        #[arg(long)]
        auto_dismiss_ms: Option<u64>,
        /// Omit placement block entirely (inherit from old surface).
        #[arg(long, conflicts_with_all = ["on_display", "geom_x", "anchor"])]
        inherit_placement: bool,
        #[arg(long)]
        json: bool,
    },
```

In the match arm:

```rust
        Command::Replace {
            surface_id, kind, app_or_path, args, env, cwd, session, timeout_ms,
            require_confidence, require_fresh_surface,
            on_display, geom_x, geom_y, geom_w, geom_h, anchor,
            auto_dismiss_ms, inherit_placement, json,
        } => {
            let wire_kind = match kind {
                LaunchKindArg::Process => {
                    let parsed_env: std::collections::BTreeMap<String, String> = env
                        .into_iter()
                        .filter_map(|s| s.split_once('=').map(|(k, v)| (k.to_string(), v.to_string())))
                        .collect();
                    porthole_protocol::launches::LaunchKind::Process(
                        porthole_protocol::launches::ProcessLaunch {
                            app: app_or_path, args, cwd, env: parsed_env,
                        }
                    )
                }
                LaunchKindArg::Artifact => {
                    porthole_protocol::launches::LaunchKind::Artifact(
                        porthole_protocol::launches::ArtifactLaunch { path: app_or_path }
                    )
                }
            };

            let geometry = match (geom_x, geom_y, geom_w, geom_h) {
                (Some(x), Some(y), Some(w), Some(h)) => Some(Rect { x, y, w, h }),
                _ => None,
            };
            let placement = if inherit_placement {
                None
            } else if on_display.is_some() || geometry.is_some() || anchor.is_some() {
                Some(PlacementSpec {
                    on_display,
                    geometry,
                    anchor: anchor.map(Anchor::from),
                })
            } else {
                // No placement flags and no --inherit-placement: send empty placement.
                // This is the "don't inherit, use OS default" path. If the caller
                // wants inheritance, they pass --inherit-placement.
                Some(PlacementSpec::default())
            };

            let req = LaunchRequest {
                kind: wire_kind,
                session,
                require_confidence: require_confidence.into(),
                timeout_ms,
                placement,
                auto_dismiss_after_ms: auto_dismiss_ms,
                require_fresh_surface,
            };
            porthole::commands::replace::run(&client, surface_id, req, json).await
        }
```

Note: the CLI treats "no flags and no `--inherit-placement`" as "send empty placement" (OS default) rather than "absent" (inherit), so that CLI behaviour is explicit. Callers must pass `--inherit-placement` to trigger wire-level absence. This makes the CLI's default "new launch at OS-default" consistent with `porthole launch` default behaviour.

- [ ] **Step 4: Build**

Run: `cargo build -p porthole`
Expected: clean.

- [ ] **Step 5: Commit**

```bash
git add crates/porthole/src/commands/replace.rs crates/porthole/src/commands/mod.rs crates/porthole/src/main.rs
git commit -m "feat(cli): add replace subcommand with --inherit-placement"
```

---

## Task 14: macOS adapter — `AxElement::set_attribute_value` + AX value builders

**Files:**
- Modify: `crates/porthole-adapter-macos/src/ax.rs`

Context: the quality round's `ax.rs` provides `copy_attribute_raw` for reading. This slice needs to *write* `AXPosition` and `AXSize` to move/resize windows. The values are `AXValue` wrappers around `CGPoint` / `CGSize`. FFI: `AXUIElementSetAttributeValue` + `AXValueCreate`.

- [ ] **Step 1: Extend `ax.rs` with write + value helpers**

Edit `crates/porthole-adapter-macos/src/ax.rs`. Add to the existing `unsafe extern "C"` block:

```rust
unsafe extern "C" {
    // ... existing ...
    fn AXUIElementSetAttributeValue(
        element: AxElementRef,
        attribute: CFStringRef,
        value: *const std::ffi::c_void,
    ) -> AxError;
    fn AXValueCreate(the_type: i32, value_ptr: *const std::ffi::c_void) -> *const std::ffi::c_void;
}
```

Constants for AXValue types (matches macOS `ApplicationServices/AXValue.h`):

```rust
pub const AX_VALUE_CG_POINT: i32 = 1;
pub const AX_VALUE_CG_SIZE: i32 = 2;
pub const AX_VALUE_CG_RECT: i32 = 3;
```

Methods on `AxElement`:

```rust
impl AxElement {
    /// Write an AXValue-wrapped CGPoint to an attribute (typically "AXPosition").
    pub fn set_position(&self, x: f64, y: f64) -> AxError {
        use core_graphics::geometry::CGPoint;
        let pt = CGPoint::new(x, y);
        unsafe {
            let value = AXValueCreate(AX_VALUE_CG_POINT, &pt as *const _ as *const std::ffi::c_void);
            if value.is_null() {
                return -1;
            }
            let attr = CFString::new("AXPosition");
            let err = AXUIElementSetAttributeValue(
                self.ptr,
                attr.as_concrete_TypeRef() as CFStringRef,
                value,
            );
            CFRelease(value);
            err
        }
    }

    /// Write an AXValue-wrapped CGSize to an attribute (typically "AXSize").
    pub fn set_size(&self, w: f64, h: f64) -> AxError {
        use core_graphics::geometry::CGSize;
        let sz = CGSize::new(w, h);
        unsafe {
            let value = AXValueCreate(AX_VALUE_CG_SIZE, &sz as *const _ as *const std::ffi::c_void);
            if value.is_null() {
                return -1;
            }
            let attr = CFString::new("AXSize");
            let err = AXUIElementSetAttributeValue(
                self.ptr,
                attr.as_concrete_TypeRef() as CFStringRef,
                value,
            );
            CFRelease(value);
            err
        }
    }

    /// Read an AXValue-wrapped CGPoint from an attribute.
    pub fn get_position(&self) -> Option<(f64, f64)> {
        use core_graphics::geometry::CGPoint;
        let raw = self.copy_attribute_raw("AXPosition")?;
        let mut pt = CGPoint::new(0.0, 0.0);
        unsafe {
            let ok = crate::ax::AXValueGetValue(raw, AX_VALUE_CG_POINT, &mut pt as *mut _ as *mut std::ffi::c_void);
            super::cf_release(raw);
            if ok != 0 { Some((pt.x, pt.y)) } else { None }
        }
    }

    pub fn get_size(&self) -> Option<(f64, f64)> {
        use core_graphics::geometry::CGSize;
        let raw = self.copy_attribute_raw("AXSize")?;
        let mut sz = CGSize::new(0.0, 0.0);
        unsafe {
            let ok = crate::ax::AXValueGetValue(raw, AX_VALUE_CG_SIZE, &mut sz as *mut _ as *mut std::ffi::c_void);
            super::cf_release(raw);
            if ok != 0 { Some((sz.width, sz.height)) } else { None }
        }
    }
}
```

Add `AXValueGetValue` to the extern block if not already there (it was added in the quality round for close_focus.rs):

```rust
    fn AXValueGetValue(value: *const std::ffi::c_void, the_type: i32, value_ptr: *mut std::ffi::c_void) -> u8;
```

- [ ] **Step 2: Build + test**

Run: `cargo test -p porthole-adapter-macos --lib ax`
Expected: existing tests pass, no new ones in this task (the helpers are driven by Tasks 15-17).

- [ ] **Step 3: Commit**

```bash
git add crates/porthole-adapter-macos/src/ax.rs
git commit -m "feat(adapter-macos): AxElement::set_position, set_size, get_position, get_size"
```

---

## Task 15: macOS — `place_surface`

**Files:**
- Create: `crates/porthole-adapter-macos/src/placement.rs`
- Modify: `crates/porthole-adapter-macos/src/lib.rs`

- [ ] **Step 1: Write `placement.rs`**

```rust
#![cfg(target_os = "macos")]

use porthole_core::display::Rect;
use porthole_core::surface::SurfaceInfo;
use porthole_core::{ErrorCode, PortholeError};

use crate::ax::AxElement;

/// Apply a global screen-coordinate rectangle to the tracked surface.
pub async fn place_surface(surface: &SurfaceInfo, rect: Rect) -> Result<(), PortholeError> {
    let pid = surface
        .pid
        .ok_or_else(|| PortholeError::new(ErrorCode::CapabilityMissing, "place_surface: no pid on surface"))?
        as i32;
    let cg = surface
        .cg_window_id
        .ok_or_else(|| PortholeError::new(ErrorCode::CapabilityMissing, "place_surface: no cg_window_id on surface"))?;

    // Walk AXWindows for this pid, find the one whose _AXUIElementGetWindow
    // matches cg, then write AXPosition + AXSize.
    crate::close_focus::with_ax_window_by_cg_id(pid, cg, |raw| {
        let elem = unsafe { AxElement::from_retained_borrowed(raw) };
        let err_pos = elem.set_position(rect.x, rect.y);
        let err_size = elem.set_size(rect.w, rect.h);
        std::mem::forget(elem); // borrowed, don't drop

        if err_pos != 0 || err_size != 0 {
            return Err(PortholeError::new(
                ErrorCode::CapabilityMissing,
                format!("AX refused position/size write: pos={err_pos} size={err_size}"),
            ));
        }
        Ok(())
    })
}
```

Note: `from_retained_borrowed` is a new helper on `AxElement` — it wraps a borrowed raw pointer without taking ownership. Drop must not fire. Using `std::mem::forget` after use is the pattern.

Add to `ax.rs`:

```rust
impl AxElement {
    /// Wrap a borrowed raw pointer without taking ownership. The returned
    /// AxElement must NOT be dropped normally — call `std::mem::forget` on
    /// it when done, or wrap use in a scope that ensures forget is called.
    /// Typical pattern is one-shot method invocations followed by forget.
    ///
    /// # Safety
    /// Caller must guarantee the pointer outlives the returned value.
    pub unsafe fn from_retained_borrowed(ptr: AxElementRef) -> Self {
        Self { ptr }
    }
}
```

This is a sharp tool — consider a `with_borrowed` closure-style helper instead to avoid the forget footgun:

```rust
impl AxElement {
    /// Run an operation against a borrowed AX pointer. The element is NOT
    /// dropped at the end (it's borrowed). Prefer this over from_retained_borrowed
    /// to avoid accidental double-release.
    ///
    /// # Safety
    /// Caller must guarantee the pointer outlives the closure.
    pub unsafe fn with_borrowed<F, R>(ptr: AxElementRef, op: F) -> R
    where
        F: FnOnce(&Self) -> R,
    {
        let tmp = Self { ptr };
        let r = op(&tmp);
        std::mem::forget(tmp);
        r
    }
}
```

Use `AxElement::with_borrowed` in `placement.rs`:

```rust
    crate::close_focus::with_ax_window_by_cg_id(pid, cg, |raw| {
        let (err_pos, err_size) = unsafe {
            AxElement::with_borrowed(raw, |elem| {
                (elem.set_position(rect.x, rect.y), elem.set_size(rect.w, rect.h))
            })
        };
        if err_pos != 0 || err_size != 0 {
            return Err(PortholeError::new(
                ErrorCode::CapabilityMissing,
                format!("AX refused position/size write: pos={err_pos} size={err_size}"),
            ));
        }
        Ok(())
    })
```

(The `from_retained_borrowed` helper is optional — use it only if `with_borrowed` feels awkward in other call sites. For this task, `with_borrowed` is sufficient.)

- [ ] **Step 2: Replace the `todo!()` in `lib.rs`**

Edit `crates/porthole-adapter-macos/src/lib.rs`:

```rust
    async fn place_surface(
        &self,
        surface: &porthole_core::surface::SurfaceInfo,
        rect: porthole_core::display::Rect,
    ) -> Result<(), porthole_core::PortholeError> {
        placement::place_surface(surface, rect).await
    }
```

Add `pub mod placement;` to the module declarations.

- [ ] **Step 3: Build**

Run: `cargo build -p porthole-adapter-macos`
Expected: clean.

- [ ] **Step 4: Commit**

```bash
git add crates/porthole-adapter-macos/src/ax.rs crates/porthole-adapter-macos/src/placement.rs crates/porthole-adapter-macos/src/lib.rs
git commit -m "feat(adapter-macos): place_surface via AX position/size writes"
```

---

## Task 16: macOS — `snapshot_geometry`

**Files:**
- Create: `crates/porthole-adapter-macos/src/snapshot.rs`
- Modify: `crates/porthole-adapter-macos/src/lib.rs`

- [ ] **Step 1: Write `snapshot.rs`**

```rust
#![cfg(target_os = "macos")]

use core_graphics::display::CGDisplay;
use porthole_core::display::{DisplayId, Rect};
use porthole_core::placement::GeometrySnapshot;
use porthole_core::surface::SurfaceInfo;
use porthole_core::{ErrorCode, PortholeError};

use crate::ax::AxElement;

/// Read the current global position + size of the tracked surface and
/// resolve which display it's on, returning display-local coordinates.
pub async fn snapshot_geometry(surface: &SurfaceInfo) -> Result<GeometrySnapshot, PortholeError> {
    let pid = surface
        .pid
        .ok_or_else(|| PortholeError::new(ErrorCode::CapabilityMissing, "snapshot_geometry: no pid"))?
        as i32;
    let cg = surface
        .cg_window_id
        .ok_or_else(|| PortholeError::new(ErrorCode::CapabilityMissing, "snapshot_geometry: no cg_window_id"))?;

    let (global_x, global_y, w, h) = crate::close_focus::with_ax_window_by_cg_id(pid, cg, |raw| {
        let (pos, size) = unsafe {
            AxElement::with_borrowed(raw, |elem| (elem.get_position(), elem.get_size()))
        };
        let (px, py) = pos.ok_or_else(|| {
            PortholeError::new(ErrorCode::CapabilityMissing, "AXPosition read failed")
        })?;
        let (sw, sh) = size.ok_or_else(|| {
            PortholeError::new(ErrorCode::CapabilityMissing, "AXSize read failed")
        })?;
        Ok((px, py, sw, sh))
    })?;

    // Resolve which display the window's center is on.
    let center_x = global_x + w / 2.0;
    let center_y = global_y + h / 2.0;
    let display_ids = CGDisplay::active_displays().unwrap_or_default();
    let (display_id, display_origin_x, display_origin_y) = display_ids
        .iter()
        .find_map(|id| {
            let display = CGDisplay::new(*id);
            let b = display.bounds();
            if center_x >= b.origin.x
                && center_x < b.origin.x + b.size.width
                && center_y >= b.origin.y
                && center_y < b.origin.y + b.size.height
            {
                Some((DisplayId::new(format!("disp_{id}")), b.origin.x, b.origin.y))
            } else {
                None
            }
        })
        .ok_or_else(|| {
            PortholeError::new(ErrorCode::CapabilityMissing, "window center not on any active display")
        })?;

    Ok(GeometrySnapshot {
        display_id,
        display_local: Rect {
            x: global_x - display_origin_x,
            y: global_y - display_origin_y,
            w,
            h,
        },
    })
}
```

- [ ] **Step 2: Replace the `todo!()` in `lib.rs`**

```rust
    async fn snapshot_geometry(
        &self,
        surface: &porthole_core::surface::SurfaceInfo,
    ) -> Result<porthole_core::placement::GeometrySnapshot, porthole_core::PortholeError> {
        snapshot::snapshot_geometry(surface).await
    }
```

Add `pub mod snapshot;`.

- [ ] **Step 3: Build**

Run: `cargo build -p porthole-adapter-macos`
Expected: clean.

- [ ] **Step 4: Commit**

```bash
git add crates/porthole-adapter-macos/src/snapshot.rs crates/porthole-adapter-macos/src/lib.rs
git commit -m "feat(adapter-macos): snapshot_geometry reads AX position/size + resolves display"
```

---

## Task 17: macOS — `launch_artifact` (dispatch + correlation)

**Files:**
- Create: `crates/porthole-adapter-macos/src/artifact.rs`
- Modify: `crates/porthole-adapter-macos/src/lib.rs`

Context: `launch_artifact` is the tricky one. We call `/usr/bin/open <path>`, then correlate the resulting window using AXDocument match (strong) / frontmost-changed (plausible) / temporal (weak).

For v0, a simple approach:
1. Snapshot the set of on-screen windows + their AXDocument attributes before the call.
2. Run `open <path>`.
3. Poll after a short delay: enumerate windows again, look for one whose AXDocument matches `file://<path>`.
4. If found, classify as `DocumentMatch` + compute `surface_was_preexisting` from pre-snapshot.
5. If not found, fall back to frontmost-changed or temporal.

For the initial implementation, the document-match path is the most valuable; getting it working for a few core apps (Preview, Safari, BBEdit) covers most use cases. Apps that don't expose AXDocument fall through to weaker correlation.

- [ ] **Step 1: Write `artifact.rs`**

This is a non-trivial module. Keep the first iteration focused on DocumentMatch correlation, with the Temporal fallback for apps that don't expose AXDocument. FrontmostChanged can be a small addition later.

```rust
#![cfg(target_os = "macos")]

use std::collections::HashSet;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use porthole_core::adapter::{ArtifactLaunchSpec, Confidence, Correlation, LaunchOutcome};
use porthole_core::surface::{SurfaceId, SurfaceInfo, SurfaceKind, SurfaceState};
use porthole_core::{ErrorCode, PortholeError};
use tokio::process::Command;
use tokio::time::sleep;

use crate::close_focus::with_ax_window_by_cg_id;
use crate::enumerate::{list_windows, WindowRecord};

const SAMPLE_INTERVAL: Duration = Duration::from_millis(150);

pub async fn launch_artifact(spec: &ArtifactLaunchSpec) -> Result<LaunchOutcome, PortholeError> {
    let path_str = spec
        .path
        .to_str()
        .ok_or_else(|| PortholeError::new(ErrorCode::InvalidArgument, "path is not valid UTF-8"))?
        .to_string();
    let file_url = format!("file://{path_str}");

    // Snapshot the set of existing window IDs (for surface_was_preexisting).
    let before = list_windows()?;
    let before_ids: HashSet<u32> = before.iter().map(|w| w.cg_window_id).collect();

    // Invoke `open <path>`.
    let status = Command::new("/usr/bin/open")
        .arg(&path_str)
        .status()
        .await
        .map_err(|e| PortholeError::new(ErrorCode::CapabilityMissing, format!("failed to spawn `open`: {e}")))?;
    if !status.success() {
        return Err(PortholeError::new(
            ErrorCode::LaunchCorrelationFailed,
            format!("`open` exited with status {status}"),
        ));
    }

    // Poll until the deadline for a window whose AXDocument matches the file url.
    let deadline = Instant::now() + spec.timeout;
    loop {
        // DocumentMatch attempt.
        if let Some((record, preexisting)) = find_window_by_document(&file_url, &before_ids).await? {
            return Ok(LaunchOutcome {
                surface: make_surface(&record),
                confidence: Confidence::Strong,
                correlation: Correlation::DocumentMatch,
                surface_was_preexisting: preexisting,
            });
        }

        if Instant::now() >= deadline {
            break;
        }
        sleep(SAMPLE_INTERVAL).await;
    }

    // Fallback: temporal — first new window across all apps within the timeout window.
    let after = list_windows()?;
    let new_windows: Vec<_> = after.iter().filter(|w| !before_ids.contains(&w.cg_window_id)).collect();
    if let Some(w) = new_windows.first() {
        return Ok(LaunchOutcome {
            surface: make_surface(w),
            confidence: Confidence::Weak,
            correlation: Correlation::Temporal,
            surface_was_preexisting: false,
        });
    }

    Err(PortholeError::new(
        ErrorCode::LaunchCorrelationFailed,
        format!("no window with matching document and no new windows after open"),
    ))
}

async fn find_window_by_document(
    target_url: &str,
    before_ids: &HashSet<u32>,
) -> Result<Option<(WindowRecord, bool)>, PortholeError> {
    let windows = list_windows()?;
    for w in windows {
        let pid = w.owner_pid;
        let cg = w.cg_window_id;
        // Query AXDocument for this window.
        let doc = match with_ax_window_by_cg_id(pid, cg, |raw| Ok(ax_document_for(raw))) {
            Ok(Some(s)) => s,
            _ => continue,
        };
        if doc == target_url {
            let preexisting = before_ids.contains(&cg);
            return Ok(Some((w, preexisting)));
        }
    }
    Ok(None)
}

fn ax_document_for(raw: crate::ax::AxElementRef) -> Option<String> {
    use crate::ax::{AxElement, cf_release};
    unsafe {
        AxElement::with_borrowed(raw, |elem| {
            let ptr = elem.copy_attribute_raw("AXDocument")?;
            // AXDocument returns a CFStringRef.
            let cfs = core_foundation::string::CFString::wrap_under_create_rule(
                ptr as core_foundation::string::CFStringRef,
            );
            let s = cfs.to_string();
            // cfs owns ptr now; when cfs drops, it releases. Prevent double-release
            // by letting cfs's drop do the work (we used wrap_under_create_rule which
            // takes ownership).
            drop(cfs);
            // Don't call cf_release on ptr since cfs already released it.
            let _ = cf_release;
            Some(s)
        })
    }
}

fn make_surface(w: &WindowRecord) -> SurfaceInfo {
    SurfaceInfo {
        id: SurfaceId::new(),
        kind: SurfaceKind::Window,
        state: SurfaceState::Alive,
        title: w.title.clone(),
        app_name: w.app_name.clone(),
        pid: Some(w.owner_pid as u32),
        parent_surface_id: None,
        cg_window_id: Some(w.cg_window_id),
    }
}

#[allow(dead_code)]
fn now_unix_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}
```

Note on `ax_document_for`: the `with_borrowed` + `wrap_under_create_rule` dance is finicky. The concrete behaviour an implementer needs to verify:
- `copy_attribute_raw` returns a retained pointer.
- `wrap_under_create_rule` on a `CFStringRef` takes that retain.
- When the wrapper drops, it calls `CFRelease` exactly once.
- So don't call `cf_release` additionally.

If this double-release dance feels dangerous, an alternative is: call `copy_attribute_raw`, check non-null, convert to Rust String via raw pointer -> CFStringGetCString-style FFI, then `cf_release(ptr)` explicitly. Whichever the implementer finds cleaner; both end with a valid single release.

- [ ] **Step 2: Replace `todo!()` in `lib.rs`**

```rust
    async fn launch_artifact(
        &self,
        spec: &porthole_core::adapter::ArtifactLaunchSpec,
    ) -> Result<porthole_core::adapter::LaunchOutcome, porthole_core::PortholeError> {
        artifact::launch_artifact(spec).await
    }
```

Add `pub mod artifact;`.

- [ ] **Step 3: Build**

Run: `cargo build --workspace --locked`
Expected: clean. All `todo!()`s gone.

- [ ] **Step 4: Commit**

```bash
git add crates/porthole-adapter-macos/src/artifact.rs crates/porthole-adapter-macos/src/lib.rs
git commit -m "feat(adapter-macos): launch_artifact via open + AXDocument correlation"
```

---

## Task 18: Full workspace sanity

- [ ] **Step 1: Build**

```
cargo build --workspace --locked
```

Expected: clean.

- [ ] **Step 2: Tests**

```
cargo test --workspace --locked
```

Expected: all non-ignored pass. Count should rise notably — lots of new unit tests.

- [ ] **Step 3: Clippy**

```
cargo clippy --workspace --all-targets --locked -- -D warnings
```

Expected: clean. Fix minor warnings if surfaced.

- [ ] **Step 4: Commit any cleanup**

Skip if nothing needed.

---

## Task 19: macOS integration tests (ignored)

**Files:**
- Create: `crates/porthole-adapter-macos/tests/slice_c_integration.rs`

- [ ] **Step 1: Write the tests**

```rust
#![cfg(target_os = "macos")]

use std::path::PathBuf;
use std::time::Duration;

use porthole_adapter_macos::MacOsAdapter;
use porthole_core::adapter::{Adapter, ArtifactLaunchSpec, RequireConfidence};
use porthole_core::display::{DisplayId, Rect};
use porthole_core::placement::GeometrySnapshot;

fn pdf_spec(path: &str) -> ArtifactLaunchSpec {
    ArtifactLaunchSpec {
        path: PathBuf::from(path),
        require_confidence: RequireConfidence::Plausible,
        require_fresh_surface: false,
        timeout: Duration::from_secs(10),
    }
}

#[tokio::test]
#[ignore = "requires a real macOS desktop session + permissions + a PDF file"]
async fn artifact_launch_pdf_and_screenshot() {
    // Requires /tmp/slice-c-test.pdf to exist. Build your own minimal PDF:
    //   printf "%%PDF-1.0\n%%EOF" > /tmp/slice-c-test.pdf
    let adapter = MacOsAdapter::new();
    let outcome = adapter.launch_artifact(&pdf_spec("/tmp/slice-c-test.pdf")).await.expect("launch");
    let shot = adapter.screenshot(&outcome.surface).await.expect("screenshot");
    assert!(shot.png_bytes.starts_with(&[0x89, 0x50, 0x4e, 0x47]));
    adapter.close(&outcome.surface).await.ok();
}

#[tokio::test]
#[ignore = "requires a real macOS desktop session + permissions"]
async fn place_surface_moves_textedit() {
    use porthole_core::adapter::{Adapter, ProcessLaunchSpec};
    let adapter = MacOsAdapter::new();
    let spec = ProcessLaunchSpec {
        app: "/System/Applications/TextEdit.app".to_string(),
        args: vec![],
        cwd: None,
        env: vec![],
        timeout: Duration::from_secs(10),
        require_confidence: RequireConfidence::Strong,
        require_fresh_surface: false,
    };
    let outcome = adapter.launch_process(&spec).await.expect("launch");
    adapter
        .place_surface(&outcome.surface, Rect { x: 200.0, y: 100.0, w: 800.0, h: 600.0 })
        .await
        .expect("place");
    let snap = adapter.snapshot_geometry(&outcome.surface).await.expect("snapshot");
    assert!((snap.display_local.w - 800.0).abs() < 5.0);
    adapter.close(&outcome.surface).await.ok();
}

#[tokio::test]
#[ignore = "requires a real macOS desktop session + permissions"]
async fn snapshot_geometry_returns_display_id() {
    use porthole_core::adapter::{Adapter, ProcessLaunchSpec};
    let adapter = MacOsAdapter::new();
    let spec = ProcessLaunchSpec {
        app: "/System/Applications/TextEdit.app".to_string(),
        args: vec![],
        cwd: None,
        env: vec![],
        timeout: Duration::from_secs(10),
        require_confidence: RequireConfidence::Strong,
        require_fresh_surface: false,
    };
    let outcome = adapter.launch_process(&spec).await.expect("launch");
    let snap = adapter.snapshot_geometry(&outcome.surface).await.expect("snapshot");
    assert!(snap.display_id.as_str().starts_with("disp_"));
    adapter.close(&outcome.surface).await.ok();
}
```

- [ ] **Step 2: Build the tests**

Run: `cargo build --tests -p porthole-adapter-macos`
Expected: clean.

- [ ] **Step 3: Commit**

```bash
git add crates/porthole-adapter-macos/tests/slice_c_integration.rs
git commit -m "test(adapter-macos): ignored artifact launch + place + snapshot integration tests"
```

---

## Task 20: E2E CLI-through-UDS test

**Files:**
- Create: `crates/portholed/tests/slice_c_e2e.rs`

- [ ] **Step 1: Write the test**

```rust
use std::sync::Arc;
use std::time::Duration;

use porthole_core::display::{DisplayId, Rect};
use porthole_core::in_memory::InMemoryAdapter;
use porthole_core::placement::GeometrySnapshot;
use porthole_core::surface::{SurfaceId, SurfaceInfo};
use portholed::server::serve;

#[tokio::test]
async fn artifact_launch_place_replace_autodismiss_over_uds() {
    let tmp = tempfile::tempdir().unwrap();
    let socket = tmp.path().join("porthole.sock");

    let adapter = Arc::new(InMemoryAdapter::new());
    // Seed for an artifact launch — default outcome is fine; we inspect calls.
    adapter
        .set_next_snapshot_geometry(Ok(GeometrySnapshot {
            display_id: DisplayId::new("in-mem-display-0"),
            display_local: Rect { x: 30.0, y: 40.0, w: 500.0, h: 400.0 },
        }))
        .await;

    let socket_for_serve = socket.clone();
    let adapter_for_serve: Arc<dyn porthole_core::adapter::Adapter> = adapter.clone();
    let server_task = tokio::spawn(async move { serve(adapter_for_serve, socket_for_serve).await });

    for _ in 0..200 {
        if socket.exists() { break; }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(socket.exists(), "socket did not appear");

    let client = porthole::client::DaemonClient::new(&socket);

    // 1. Artifact launch with full placement.
    let launch: porthole_protocol::launches::LaunchResponse = client
        .post_json(
            "/launches",
            &serde_json::json!({
                "kind": { "type": "artifact", "path": "/tmp/x.pdf" },
                "placement": {
                    "on_display": "primary",
                    "geometry": { "x": 10.0, "y": 20.0, "w": 400.0, "h": 300.0 }
                }
            }),
        )
        .await
        .expect("launch");
    assert_eq!(
        launch.placement,
        porthole_core::placement::PlacementOutcome::Applied
    );

    // 2. Replace with omitted placement → inheritance.
    let replace: porthole_protocol::launches::LaunchResponse = client
        .post_json(
            &format!("/surfaces/{}/replace", launch.surface_id),
            &serde_json::json!({
                "kind": { "type": "artifact", "path": "/tmp/y.pdf" }
            }),
        )
        .await
        .expect("replace");
    assert_eq!(
        replace.placement,
        porthole_core::placement::PlacementOutcome::Applied
    );
    assert_ne!(replace.surface_id, launch.surface_id, "replace should mint a fresh id");

    // 3. URL artifact rejected.
    let url_res: Result<porthole_protocol::launches::LaunchResponse, _> = client
        .post_json(
            "/launches",
            &serde_json::json!({
                "kind": { "type": "artifact", "path": "https://example.com" }
            }),
        )
        .await;
    assert!(url_res.is_err(), "URL artifact should be rejected");

    server_task.abort();
}
```

- [ ] **Step 2: Run the test**

Run: `cargo test -p portholed --test slice_c_e2e`
Expected: 1 pass.

- [ ] **Step 3: Final workspace sanity**

```
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
```

Expected: all pass, clippy clean.

- [ ] **Step 4: Commit**

```bash
git add crates/portholed/tests/slice_c_e2e.rs
git commit -m "test(daemon): end-to-end slice-C artifact launch + placement + replace over UDS"
```

---

## What slice C delivers

- `POST /launches` with `kind: artifact`, file paths only (URLs rejected)
- Placement on launches: `on_display` (focused/primary/disp_N), `geometry` (display-local), `anchor` (focused_display/cursor)
- `LaunchResponse.placement: PlacementOutcome` — not an error on failure; surface_id always returned
- `require_fresh_surface: true` → `launch_returned_existing` with a slice-B `ref` in the error body for attach-as-fallback
- `POST /surfaces/{id}/replace` — snapshot → close → launch, inheritance on absent-placement, handle-atomic (not visually atomic)
- `close_failed` during replace carries `old_handle_alive: true` body field
- `auto_dismiss_after_ms` tokio timer per surface (in-memory; not persistent across daemon restart)
- macOS adapter implements launch_artifact (AXDocument-match correlation with temporal fallback), place_surface (AX position/size writes), snapshot_geometry (AX reads + display resolution)
- CLI `launch --kind artifact`, new placement flags, `--auto-dismiss-ms`, `--require-fresh-surface`, new `porthole replace` subcommand with `--inherit-placement`
- `/info` capabilities: `launch_artifact`, `placement`, `replace`, `auto_dismiss`

## What slice C intentionally does not deliver

- URL artifacts (browser-CDP slice)
- QuickLook opener / porthole-viewer
- `POST /surfaces/{id}/place` for post-launch repositioning
- Tab surfaces
- Events SSE, real `recently_active_surface_ids`
- Recording
- `FrontmostChanged` correlation (Temporal fallback covers the non-DocumentMatch case; FrontmostChanged is a future refinement)
- Persistent auto-dismiss timers
- `force_place: true` for preexisting surfaces

