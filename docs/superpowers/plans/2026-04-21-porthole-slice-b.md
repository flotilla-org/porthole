# Porthole Slice B — Attach Mode Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let a caller promote an already-running OS window to a tracked porthole surface, search for candidate windows by query, and find a script's own containing terminal window via PID ancestry.

**Architecture:** Extends the slice-A workspace. New `porthole-core` types (`SearchQuery`, `Candidate`, `AttachPipeline`). New atomic `HandleStore::track_or_get` for concurrency-safe idempotent tracking. Two new `Adapter` trait methods (`search` filters on-screen windows; `window_alive` checks existence against all windows including off-screen). New `POST /surfaces/search` + `POST /surfaces/track` routes. New CLI subcommands (`search`, `track`, `attach`) plus a `porthole::ancestry` library helper. Also renames the misnomer `app_bundle` → `app_name` on `SurfaceInfo` and `AttentionInfo` to reflect that the field holds a human display name, not a bundle identifier.

**Tech Stack:** Same as slice A — Rust 2024, tokio, axum 0.8, hyper 1, hyperlocal 0.9, serde, regex, base64, objc2 + objc2-app-kit + objc2-foundation, core-graphics, core-foundation.

---

## Out of Scope

Per the slice-B spec (`docs/superpowers/specs/2026-04-21-porthole-slice-b-design.md`, §10):

- Advanced search filters: `on_display`, `bounds_in`, AX-path predicate (`frontmost` **is** in this slice)
- `include_hidden` search flag (search is on-screen only; tracked handles stay valid for hidden windows)
- True bundle-id field (`app_bundle_id`) via `NSRunningApplication.bundleIdentifier`
- Tab candidates
- Cache-backed refs
- Cross-search correlation, "watch for new matches" notifications
- Dead-handle garbage collection

---

## File Structure

```
crates/porthole-core/
  src/
    adapter.rs                   # modify: add search + window_alive trait methods
    attention.rs                 # modify: focused_app_bundle → focused_app_name
    handle.rs                    # modify: add track_or_get
    in_memory.rs                 # modify: rename + script search/window_alive
    search.rs                    # NEW: SearchQuery, Candidate, ref encoding
    attach_pipeline.rs           # NEW: AttachPipeline (validation, idempotent track)
    surface.rs                   # modify: app_bundle → app_name
    lib.rs                       # modify: declare new modules, update re-exports

crates/porthole-protocol/
  src/
    search.rs                    # NEW: wire types for search + track
    lib.rs                       # modify: declare new module

crates/portholed/
  src/
    routes/
      attach.rs                  # NEW: post_search, post_track
      attention.rs               # modify: focused_app_name rename
      info.rs                    # modify: capability additions
      mod.rs                     # modify: declare new module
    state.rs                     # modify: add AttachPipeline
    server.rs                    # modify: wire new routes + tests
  tests/
    slice_b_e2e.rs               # NEW: CLI-through-UDS attach flow

crates/porthole/
  src/
    ancestry.rs                  # NEW: containing_ancestors helper
    commands/
      attach.rs                  # NEW
      attention.rs               # modify: focused_app_name field
      mod.rs                     # modify
      search.rs                  # NEW
      track.rs                   # NEW
    lib.rs                       # modify: declare ancestry module
    main.rs                      # modify: add subcommands

crates/porthole-adapter-macos/
  src/
    attention.rs                 # modify: localizedName + field rename
    capture.rs                   # modify: field rename in screenshot metadata path
    enumerate.rs                 # modify: field rename
    launch.rs                    # modify: field rename in SurfaceInfo construction
    search.rs                    # NEW: Adapter::search impl
    window_alive.rs              # NEW: Adapter::window_alive impl (broad enumeration)
    lib.rs                       # modify: new trait method impls, module decls
  tests/
    attach_integration.rs        # NEW: ignored real-macOS tests
```

---

## Task 1: Rename `app_bundle` → `app_name`

**Files:**
- Modify: `crates/porthole-core/src/surface.rs`
- Modify: `crates/porthole-core/src/attention.rs`
- Modify: `crates/porthole-core/src/in_memory.rs`
- Modify: `crates/porthole-adapter-macos/src/enumerate.rs`
- Modify: `crates/porthole-adapter-macos/src/attention.rs`
- Modify: `crates/porthole-adapter-macos/src/launch.rs`
- Modify: `crates/porthole-adapter-macos/src/capture.rs` (if it references the field)
- Modify: `crates/portholed/src/routes/info.rs` (if it references the field)
- Modify: `crates/portholed/src/routes/attention.rs` (if it references the field)
- Modify: `crates/porthole/src/commands/attention.rs`
- Modify: any tests referencing the old name

Context: the existing field holds `kCGWindowOwnerName` (display name like "Ghostty"), not a bundle identifier. Attention code additionally calls `bundleIdentifier`, which *is* a true bundle ID — this change also switches that to `localizedName` so both fields are display names consistently. A future slice may add `app_bundle_id`.

- [ ] **Step 1: Rename `SurfaceInfo.app_bundle` → `app_name`**

Edit `crates/porthole-core/src/surface.rs`. Find `pub app_bundle: Option<String>` in the `SurfaceInfo` struct; rename to `pub app_name: Option<String>`. Update the `SurfaceInfo::window` helper if it references the old name (it currently doesn't — `app_bundle: None` via struct-update syntax is the only reference).

- [ ] **Step 2: Rename `AttentionInfo.focused_app_bundle` → `focused_app_name`**

Edit `crates/porthole-core/src/attention.rs`. Find `pub focused_app_bundle: Option<String>`; rename to `pub focused_app_name: Option<String>`. Update the roundtrip test in that file to use the new name.

- [ ] **Step 3: Update `InMemoryAdapter`**

Edit `crates/porthole-core/src/in_memory.rs`. Find two references to `app_bundle`:
- In `make_default_launch_outcome`: `app_bundle: Some("com.example.test".to_string())` → `app_name: Some("test-app".to_string())`.
- In `default_attention`: `focused_app_bundle: None` → `focused_app_name: None`.

- [ ] **Step 4: Update macOS enumerate**

Edit `crates/porthole-adapter-macos/src/enumerate.rs`. Find `app_bundle: Option<String>` in `WindowRecord`; rename to `app_name`. Find the two sites where it's read (`kCGWindowOwnerName` → field write) and referenced (`.app_bundle` access); rename consistently.

- [ ] **Step 5: Update macOS attention**

Edit `crates/porthole-adapter-macos/src/attention.rs`. Two changes:
- Rename the field `focused_app_bundle` → `focused_app_name` in the returned `AttentionInfo`.
- Switch from `bundleIdentifier` to `localizedName` on the `NSRunningApplication` call so the field holds a display name consistent with `SurfaceInfo.app_name`. The call pattern is:
  ```rust
  let name: Option<Retained<NSString>> = msg_send![&*a, localizedName];
  ```

- [ ] **Step 6: Update macOS launch**

Edit `crates/porthole-adapter-macos/src/launch.rs`. Find `app_bundle: window.app_bundle` in the `SurfaceInfo` construction; rename to `app_name: window.app_name`.

- [ ] **Step 7: Update macOS capture**

Edit `crates/porthole-adapter-macos/src/capture.rs`. If any references to `app_bundle` exist in the screenshot metadata path (the foundation's response plumbing may not use it), rename. `grep`-check: `rg 'app_bundle' crates/porthole-adapter-macos/` should come back empty.

- [ ] **Step 8: Update daemon routes**

Edit `crates/portholed/src/routes/info.rs` and `crates/portholed/src/routes/attention.rs`. Any references to `focused_app_bundle` → `focused_app_name`. If any field routing touches `app_bundle` (unlikely; the foundation's routes deal in surface_ids, not SurfaceInfo), rename.

- [ ] **Step 9: Update CLI attention command**

Edit `crates/porthole/src/commands/attention.rs`. Find `println!("focused_app_bundle: {:?}", info.focused_app_bundle)` → `println!("focused_app_name: {:?}", info.focused_app_name)`.

- [ ] **Step 10: Check remaining tests**

Run: `rg 'app_bundle|focused_app_bundle' crates/` — should be empty or only in docs/comments. Update any straggler test files.

- [ ] **Step 11: Build + test**

```
cargo build --workspace --locked
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
```

Expected: all pass, no warnings. Test count unchanged from slice-A's final state (77 non-ignored + 5 ignored).

- [ ] **Step 12: Commit**

```bash
git add -A
git commit -m "refactor: rename app_bundle → app_name, focused_app_bundle → focused_app_name"
```

---

## Task 2: porthole-core — SearchQuery, Candidate, ref encoding

**Files:**
- Create: `crates/porthole-core/src/search.rs`
- Modify: `crates/porthole-core/src/lib.rs`
- Modify: `crates/porthole-core/Cargo.toml` (add base64 dep)

- [ ] **Step 1: Add base64 dep**

Edit `crates/porthole-core/Cargo.toml` — under `[dependencies]`:

```toml
base64 = "0.22"
```

If the workspace `[workspace.dependencies]` already has `base64` (slice A added it to `porthole` CLI — check), use `{ workspace = true }` and add the workspace entry if missing.

Run: `rg '^base64' crates/*/Cargo.toml Cargo.toml`. If only present in `porthole/Cargo.toml`, promote to workspace: add `base64 = "0.22"` to `[workspace.dependencies]` in root `Cargo.toml`, change `porthole/Cargo.toml`'s entry to `{ workspace = true }`, and use `{ workspace = true }` here.

- [ ] **Step 2: Write `search.rs`**

Create `crates/porthole-core/src/search.rs`:

```rust
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde::{Deserialize, Serialize};

use crate::{ErrorCode, PortholeError};

/// Query passed to `Adapter::search`. Every field is optional; matching is
/// AND across fields, OR within a list.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SearchQuery {
    #[serde(default)]
    pub app_name: Option<String>,
    #[serde(default)]
    pub title_pattern: Option<String>,
    #[serde(default)]
    pub pids: Vec<u32>,
    #[serde(default)]
    pub cg_window_ids: Vec<u32>,
    #[serde(default)]
    pub frontmost: Option<bool>,
}

/// A window that matched a search. Opaque `ref` carries enough state to
/// re-identify the window in a later `track` call.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Candidate {
    #[serde(rename = "ref")]
    pub ref_: String,
    pub app_name: Option<String>,
    pub title: Option<String>,
    pub pid: u32,
    pub cg_window_id: u32,
}

const REF_PREFIX: &str = "ref_";
const REF_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct RefPayload {
    pid: u32,
    cg_window_id: u32,
    v: u32,
}

/// Encode (pid, cg_window_id) into a self-describing opaque ref.
pub fn encode_ref(pid: u32, cg_window_id: u32) -> String {
    let payload = RefPayload { pid, cg_window_id, v: REF_SCHEMA_VERSION };
    let json = serde_json::to_vec(&payload).expect("RefPayload is JSON-serialisable");
    format!("{REF_PREFIX}{}", URL_SAFE_NO_PAD.encode(json))
}

/// Decode a ref back to (pid, cg_window_id). Returns `candidate_ref_unknown`
/// on any structural failure (wrong prefix, bad base64, bad JSON, unknown
/// schema version).
pub fn decode_ref(r: &str) -> Result<(u32, u32), PortholeError> {
    let body = r.strip_prefix(REF_PREFIX).ok_or_else(|| {
        PortholeError::new(ErrorCode::CandidateRefUnknown, format!("ref missing '{REF_PREFIX}' prefix"))
    })?;
    let bytes = URL_SAFE_NO_PAD
        .decode(body)
        .map_err(|e| PortholeError::new(ErrorCode::CandidateRefUnknown, format!("ref base64 decode failed: {e}")))?;
    let payload: RefPayload = serde_json::from_slice(&bytes).map_err(|e| {
        PortholeError::new(ErrorCode::CandidateRefUnknown, format!("ref JSON decode failed: {e}"))
    })?;
    if payload.v != REF_SCHEMA_VERSION {
        return Err(PortholeError::new(
            ErrorCode::CandidateRefUnknown,
            format!("ref schema version {} is not supported (expected {REF_SCHEMA_VERSION})", payload.v),
        ));
    }
    Ok((payload.pid, payload.cg_window_id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_decode_roundtrip() {
        let r = encode_ref(9876, 42);
        let (pid, cg) = decode_ref(&r).unwrap();
        assert_eq!(pid, 9876);
        assert_eq!(cg, 42);
    }

    #[test]
    fn encoded_ref_has_prefix() {
        assert!(encode_ref(1, 1).starts_with("ref_"));
    }

    #[test]
    fn decode_missing_prefix_errors() {
        let err = decode_ref("abc").unwrap_err();
        assert_eq!(err.code, ErrorCode::CandidateRefUnknown);
    }

    #[test]
    fn decode_bad_base64_errors() {
        let err = decode_ref("ref_!!!not-base64!!!").unwrap_err();
        assert_eq!(err.code, ErrorCode::CandidateRefUnknown);
    }

    #[test]
    fn decode_bad_json_errors() {
        let payload = URL_SAFE_NO_PAD.encode(b"not-json");
        let err = decode_ref(&format!("ref_{payload}")).unwrap_err();
        assert_eq!(err.code, ErrorCode::CandidateRefUnknown);
    }

    #[test]
    fn decode_wrong_schema_version_errors() {
        let payload = serde_json::to_vec(&serde_json::json!({ "pid": 1, "cg_window_id": 1, "v": 99 })).unwrap();
        let encoded = URL_SAFE_NO_PAD.encode(payload);
        let err = decode_ref(&format!("ref_{encoded}")).unwrap_err();
        assert_eq!(err.code, ErrorCode::CandidateRefUnknown);
    }
}
```

- [ ] **Step 3: Register module**

Edit `crates/porthole-core/src/lib.rs` — add `pub mod search;` and extend re-exports:

```rust
pub use search::{Candidate, SearchQuery};
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p porthole-core --lib search`
Expected: 6 passes.

- [ ] **Step 5: Commit**

```bash
git add crates/porthole-core/Cargo.toml crates/porthole-core/src/search.rs crates/porthole-core/src/lib.rs Cargo.toml
git commit -m "feat(core): add SearchQuery, Candidate, and self-encoded ref helpers"
```

---

## Task 3: porthole-core — `HandleStore::track_or_get`

**Files:**
- Modify: `crates/porthole-core/src/handle.rs`

Context: slice B's idempotent-track rule (one alive tracked handle per `cg_window_id`) must be enforceable under concurrent HTTP requests. The existing `find_by_cg_window_id` + `insert` composition does a read-then-write without holding the lock, so two concurrent track calls can both "miss" and both insert. We need an atomic get-or-insert.

- [ ] **Step 1: Write the failing concurrency test**

Append to the existing test module in `crates/porthole-core/src/handle.rs`:

```rust
    #[tokio::test]
    async fn track_or_get_is_atomic_under_concurrency() {
        use std::sync::Arc;
        use crate::surface::SurfaceInfo;

        let store = HandleStore::new();
        let cg_id: u32 = 4242;

        // N tasks all racing to track_or_get the same cg_window_id with fresh
        // SurfaceInfo each time. Exactly one should see reused=false; the rest
        // reused=true. All should return the same surface_id.
        let n = 20;
        let mut tasks = Vec::with_capacity(n);
        let store = Arc::new(store);
        for _ in 0..n {
            let s = Arc::clone(&store);
            tasks.push(tokio::spawn(async move {
                let mut info = SurfaceInfo::window(SurfaceId::new(), 1);
                info.cg_window_id = Some(cg_id);
                s.track_or_get(info).await
            }));
        }

        let mut observed_ids = std::collections::HashSet::new();
        let mut newly_minted = 0usize;
        for t in tasks {
            let (info, reused) = t.await.unwrap();
            observed_ids.insert(info.id);
            if !reused {
                newly_minted += 1;
            }
        }

        assert_eq!(newly_minted, 1, "exactly one task should have minted the handle");
        assert_eq!(observed_ids.len(), 1, "all tasks should see the same surface_id");
    }

    #[tokio::test]
    async fn track_or_get_inserts_when_absent() {
        use crate::surface::SurfaceInfo;

        let store = HandleStore::new();
        let mut info = SurfaceInfo::window(SurfaceId::new(), 1);
        info.cg_window_id = Some(99);
        let (returned, reused) = store.track_or_get(info.clone()).await;
        assert!(!reused);
        assert_eq!(returned.id, info.id);
    }

    #[tokio::test]
    async fn track_or_get_reuses_alive_handle() {
        use crate::surface::SurfaceInfo;

        let store = HandleStore::new();
        let mut first = SurfaceInfo::window(SurfaceId::new(), 1);
        first.cg_window_id = Some(7);
        store.track_or_get(first.clone()).await;

        let mut second = SurfaceInfo::window(SurfaceId::new(), 1);
        second.cg_window_id = Some(7);
        let (returned, reused) = store.track_or_get(second).await;
        assert!(reused);
        assert_eq!(returned.id, first.id);
    }

    #[tokio::test]
    async fn track_or_get_skips_dead_handle() {
        use crate::surface::SurfaceInfo;

        let store = HandleStore::new();
        let mut dead = SurfaceInfo::window(SurfaceId::new(), 1);
        dead.cg_window_id = Some(5);
        let old_id = dead.id.clone();
        store.track_or_get(dead).await;
        store.mark_dead(&old_id).await.unwrap();

        let mut fresh = SurfaceInfo::window(SurfaceId::new(), 1);
        fresh.cg_window_id = Some(5);
        let (returned, reused) = store.track_or_get(fresh.clone()).await;
        assert!(!reused, "dead handle should not be reused");
        assert_eq!(returned.id, fresh.id);
    }
```

- [ ] **Step 2: Run the test (expected failure)**

Run: `cargo test -p porthole-core --lib handle::tests::track_or_get`
Expected: FAIL — `track_or_get` method doesn't exist yet.

- [ ] **Step 3: Implement `track_or_get`**

Modify `crates/porthole-core/src/handle.rs` — add a new method on `HandleStore` inside the existing `impl HandleStore` block:

```rust
    /// Atomic get-or-insert keyed by `cg_window_id`. Holds the write lock
    /// across both the lookup and the insert so concurrent callers cannot
    /// both mint a new handle for the same window.
    ///
    /// Returns `(SurfaceInfo, reused)`:
    /// - If an alive tracked surface with this `cg_window_id` exists,
    ///   returns that surface with `reused = true`.
    /// - Otherwise inserts `candidate` and returns it with `reused = false`.
    ///
    /// Dead handles for the same `cg_window_id` are skipped — a fresh
    /// insert happens anyway, so re-tracking a window whose previous handle
    /// died returns a new surface id.
    pub async fn track_or_get(&self, candidate: SurfaceInfo) -> (SurfaceInfo, bool) {
        let mut guard = self.inner.write().await;
        if let Some(cg) = candidate.cg_window_id {
            for info in guard.values() {
                if info.cg_window_id == Some(cg) && info.state == SurfaceState::Alive {
                    return (info.clone(), true);
                }
            }
        }
        let key = candidate.id.clone();
        guard.insert(key, candidate.clone());
        (candidate, false)
    }
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -p porthole-core --lib handle`
Expected: all existing handle tests still pass, plus the 4 new ones. Should be 7 total in the handle module.

- [ ] **Step 5: Commit**

```bash
git add crates/porthole-core/src/handle.rs
git commit -m "feat(core): add HandleStore::track_or_get for atomic idempotent tracking"
```

---

## Task 4: porthole-core — Extend Adapter trait + InMemoryAdapter

**Files:**
- Modify: `crates/porthole-core/src/adapter.rs`
- Modify: `crates/porthole-core/src/in_memory.rs`

Context: this task breaks the macOS adapter build until Task 5 implements the new methods. The pattern used in slice A is to add `todo!()` stubs temporarily.

- [ ] **Step 1: Add trait methods**

Edit `crates/porthole-core/src/adapter.rs`. Add imports:

```rust
use crate::search::{Candidate, SearchQuery};
use crate::surface::SurfaceInfo;
```

Extend the `Adapter` trait inside the `#[async_trait]` impl:

```rust
    /// Enumerate candidate surfaces matching the query. Empty matches
    /// return `Ok(vec![])`, not an error.
    async fn search(&self, query: &SearchQuery) -> Result<Vec<Candidate>, PortholeError>;

    /// Return a live `SurfaceInfo` for the window identified by
    /// `(pid, cg_window_id)` if it still exists. The liveness check
    /// encompasses *all* windows, including hidden / minimized /
    /// other-Space windows — not just on-screen enumeration.
    async fn window_alive(
        &self,
        pid: u32,
        cg_window_id: u32,
    ) -> Result<Option<SurfaceInfo>, PortholeError>;
```

- [ ] **Step 2: Extend `InMemoryAdapter`**

Edit `crates/porthole-core/src/in_memory.rs`. Add to the `Script` struct:

```rust
    next_search_result: Option<Result<Vec<Candidate>, PortholeError>>,
    next_window_alive_result: Option<Result<Option<SurfaceInfo>, PortholeError>>,
    search_calls: Vec<SearchQuery>,
    window_alive_calls: Vec<(u32, u32)>,
```

Add scripting setters + recorders:

```rust
    pub async fn set_next_search_result(&self, v: Result<Vec<Candidate>, PortholeError>) {
        self.script.lock().await.next_search_result = Some(v);
    }
    pub async fn set_next_window_alive_result(&self, v: Result<Option<SurfaceInfo>, PortholeError>) {
        self.script.lock().await.next_window_alive_result = Some(v);
    }
    pub async fn search_calls(&self) -> Vec<SearchQuery> {
        self.script.lock().await.search_calls.clone()
    }
    pub async fn window_alive_calls(&self) -> Vec<(u32, u32)> {
        self.script.lock().await.window_alive_calls.clone()
    }
```

Add trait method implementations (inside the `impl Adapter for InMemoryAdapter` block):

```rust
    async fn search(&self, query: &SearchQuery) -> Result<Vec<Candidate>, PortholeError> {
        let mut s = self.script.lock().await;
        s.search_calls.push(query.clone());
        s.next_search_result.take().unwrap_or_else(|| Ok(vec![]))
    }

    async fn window_alive(
        &self,
        pid: u32,
        cg_window_id: u32,
    ) -> Result<Option<SurfaceInfo>, PortholeError> {
        let mut s = self.script.lock().await;
        s.window_alive_calls.push((pid, cg_window_id));
        s.next_window_alive_result.take().unwrap_or(Ok(None))
    }
```

Add imports at the top of `in_memory.rs`:

```rust
use crate::search::{Candidate, SearchQuery};
```

- [ ] **Step 3: Add an in-memory test**

Append to `in_memory.rs`'s test module:

```rust
    #[tokio::test]
    async fn search_records_query_and_returns_scripted_candidates() {
        let adapter = InMemoryAdapter::new();
        let candidate = Candidate {
            ref_: "ref_abc".to_string(),
            app_name: Some("TestApp".into()),
            title: Some("t".into()),
            pid: 42,
            cg_window_id: 7,
        };
        adapter.set_next_search_result(Ok(vec![candidate.clone()])).await;
        let result = adapter
            .search(&SearchQuery { app_name: Some("TestApp".into()), ..Default::default() })
            .await
            .unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].cg_window_id, 7);
        let calls = adapter.search_calls().await;
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].app_name, Some("TestApp".into()));
    }

    #[tokio::test]
    async fn window_alive_returns_scripted_outcome() {
        let adapter = InMemoryAdapter::new();
        let mut info = SurfaceInfo::window(SurfaceId::new(), 42);
        info.cg_window_id = Some(7);
        adapter.set_next_window_alive_result(Ok(Some(info))).await;
        let got = adapter.window_alive(42, 7).await.unwrap();
        assert!(got.is_some());
        let calls = adapter.window_alive_calls().await;
        assert_eq!(calls, vec![(42, 7)]);
    }
```

- [ ] **Step 4: Add `todo!()` stubs to macOS adapter**

Edit `crates/porthole-adapter-macos/src/lib.rs`. In the `impl Adapter for MacOsAdapter` block, add:

```rust
    async fn search(
        &self,
        _query: &porthole_core::SearchQuery,
    ) -> Result<Vec<porthole_core::Candidate>, porthole_core::PortholeError> {
        todo!("implemented in Task 5")
    }

    async fn window_alive(
        &self,
        _pid: u32,
        _cg_window_id: u32,
    ) -> Result<Option<porthole_core::SurfaceInfo>, porthole_core::PortholeError> {
        todo!("implemented in Task 5")
    }
```

- [ ] **Step 5: Run tests**

Run:
```
cargo test -p porthole-core --lib
cargo build --workspace --locked
```

Expected: porthole-core tests pass, workspace builds (macOS adapter has `todo!()` but compiles).

- [ ] **Step 6: Commit**

```bash
git add crates/porthole-core/src/adapter.rs crates/porthole-core/src/in_memory.rs crates/porthole-adapter-macos/src/lib.rs
git commit -m "feat(core): extend Adapter trait with search and window_alive"
```

---

## Task 5: porthole-adapter-macos — Implement search + window_alive

**Files:**
- Create: `crates/porthole-adapter-macos/src/search.rs`
- Create: `crates/porthole-adapter-macos/src/window_alive.rs`
- Modify: `crates/porthole-adapter-macos/src/lib.rs`

- [ ] **Step 1: Write `search.rs`**

Create `crates/porthole-adapter-macos/src/search.rs`:

```rust
#![cfg(target_os = "macos")]

use porthole_core::search::{encode_ref, Candidate, SearchQuery};
use porthole_core::{ErrorCode, PortholeError};
use regex::Regex;

use crate::enumerate::{list_windows, WindowRecord};

pub async fn search(query: &SearchQuery) -> Result<Vec<Candidate>, PortholeError> {
    let title_regex = match &query.title_pattern {
        Some(p) => Some(Regex::new(p).map_err(|e| {
            PortholeError::new(ErrorCode::InvalidArgument, format!("invalid title_pattern regex: {e}"))
        })?),
        None => None,
    };

    let windows = list_windows()?;
    let mut matches: Vec<WindowRecord> =
        windows.into_iter().filter(|w| matches_query(w, query, title_regex.as_ref())).collect();

    if matches!(query.frontmost, Some(true)) && !matches.is_empty() {
        // list_windows returns on-screen windows in roughly Z-order
        // (front-to-back) because CGWindowListCopyWindowInfo is called with
        // kCGWindowListOptionOnScreenOnly and no explicit reordering. Take
        // the first match, which is the frontmost.
        matches.truncate(1);
    }

    Ok(matches
        .into_iter()
        .map(|w| Candidate {
            ref_: encode_ref(w.owner_pid as u32, w.cg_window_id),
            app_name: w.app_name,
            title: w.title,
            pid: w.owner_pid as u32,
            cg_window_id: w.cg_window_id,
        })
        .collect())
}

fn matches_query(w: &WindowRecord, q: &SearchQuery, title_re: Option<&Regex>) -> bool {
    if let Some(name) = &q.app_name {
        if w.app_name.as_deref() != Some(name) {
            return false;
        }
    }
    if let Some(re) = title_re {
        let title = w.title.as_deref().unwrap_or("");
        if !re.is_match(title) {
            return false;
        }
    }
    if !q.pids.is_empty() {
        let pid_u32 = w.owner_pid as u32;
        if !q.pids.contains(&pid_u32) {
            return false;
        }
    }
    if !q.cg_window_ids.is_empty() && !q.cg_window_ids.contains(&w.cg_window_id) {
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(pid: i32, cg: u32, app: Option<&str>, title: Option<&str>) -> WindowRecord {
        WindowRecord {
            cg_window_id: cg,
            owner_pid: pid,
            title: title.map(str::to_string),
            app_name: app.map(str::to_string),
        }
    }

    #[test]
    fn matches_with_empty_query() {
        let w = rec(1, 42, Some("X"), Some("t"));
        assert!(matches_query(&w, &SearchQuery::default(), None));
    }

    #[test]
    fn app_name_filter_is_exact() {
        let w = rec(1, 42, Some("Ghostty"), None);
        let q = SearchQuery { app_name: Some("Ghostty".into()), ..Default::default() };
        assert!(matches_query(&w, &q, None));
        let q = SearchQuery { app_name: Some("ghostty".into()), ..Default::default() };
        assert!(!matches_query(&w, &q, None));
    }

    #[test]
    fn pids_filter_is_or_within_list() {
        let w = rec(77, 42, None, None);
        let q = SearchQuery { pids: vec![10, 77, 99], ..Default::default() };
        assert!(matches_query(&w, &q, None));
    }

    #[test]
    fn title_pattern_compiles_and_matches() {
        let re = Regex::new("^demo-").unwrap();
        let w = rec(1, 1, None, Some("demo-terminal"));
        assert!(matches_query(&w, &SearchQuery::default(), Some(&re)));
        let w = rec(1, 1, None, Some("other"));
        assert!(!matches_query(&w, &SearchQuery::default(), Some(&re)));
    }
}
```

- [ ] **Step 2: Write `window_alive.rs`**

Create `crates/porthole-adapter-macos/src/window_alive.rs`. This uses a broader enumeration than `list_windows()` so hidden/minimized windows are still "alive":

```rust
#![cfg(target_os = "macos")]

use porthole_core::surface::{SurfaceId, SurfaceInfo, SurfaceKind, SurfaceState};
use porthole_core::PortholeError;

/// Enumerates all windows (including off-screen, minimized, and other-Space)
/// and returns a fresh SurfaceInfo if a window with the given
/// (pid, cg_window_id) exists.
///
/// Unlike `list_windows()` which uses `kCGWindowListOptionOnScreenOnly`, this
/// uses a broader option set so tracked handles remain valid through hide /
/// minimize / Space-switch cycles.
pub async fn window_alive(pid: u32, cg_window_id: u32) -> Result<Option<SurfaceInfo>, PortholeError> {
    use core_foundation::array::CFArray;
    use core_foundation::base::{CFType, TCFType};
    use core_foundation::dictionary::CFDictionary;
    use core_foundation::number::CFNumber;
    use core_foundation::string::CFString;
    use core_graphics::window::{
        kCGNullWindowID, kCGWindowListExcludeDesktopElements, kCGWindowListOptionAll,
        kCGWindowName, kCGWindowNumber, kCGWindowOwnerName, kCGWindowOwnerPID,
    };

    let opts = kCGWindowListOptionAll | kCGWindowListExcludeDesktopElements;
    let raw = unsafe { core_graphics::window::copy_window_info(opts, kCGNullWindowID) };
    let array: CFArray<CFDictionary<CFString, CFType>> = match raw {
        Some(a) => a,
        None => return Ok(None),
    };

    for item in array.iter() {
        let dict: &CFDictionary<CFString, CFType> = &*item;

        let owner_pid = dict
            .find(unsafe { CFString::wrap_under_get_rule(kCGWindowOwnerPID) })
            .and_then(|v| v.downcast::<CFNumber>().and_then(|n| n.to_i32()))
            .unwrap_or(0);
        if owner_pid as u32 != pid {
            continue;
        }
        let this_cg = dict
            .find(unsafe { CFString::wrap_under_get_rule(kCGWindowNumber) })
            .and_then(|v| v.downcast::<CFNumber>().and_then(|n| n.to_i32()))
            .map(|n| n as u32)
            .unwrap_or(0);
        if this_cg != cg_window_id {
            continue;
        }
        let title = dict
            .find(unsafe { CFString::wrap_under_get_rule(kCGWindowName) })
            .and_then(|v| v.downcast::<CFString>())
            .map(|s| s.to_string());
        let app_name = dict
            .find(unsafe { CFString::wrap_under_get_rule(kCGWindowOwnerName) })
            .and_then(|v| v.downcast::<CFString>())
            .map(|s| s.to_string());

        let mut info = SurfaceInfo {
            id: SurfaceId::new(),
            kind: SurfaceKind::Window,
            state: SurfaceState::Alive,
            title,
            app_name,
            pid: Some(pid),
            parent_surface_id: None,
            cg_window_id: Some(cg_window_id),
        };
        let _ = &mut info;
        return Ok(Some(info));
    }
    Ok(None)
}
```

Note: the exact function name and signature for "copy window info without the on-screen-only flag" depends on the `core-graphics` 0.24 crate shape. If `core_graphics::window::copy_window_info` isn't exposed publicly, fall back to `CGWindowListCopyWindowInfo` raw FFI as used in `enumerate.rs`. The two constants `kCGWindowListOptionAll` and `kCGWindowListExcludeDesktopElements` should both be in `core_graphics::window`.

If `kCGWindowListOptionAll` isn't exposed in core-graphics 0.24, its numeric value is `0` (absence of `kCGWindowListOptionOnScreenOnly` = all windows) — you can pass `kCGWindowListExcludeDesktopElements` on its own.

- [ ] **Step 3: Wire into `lib.rs`**

Edit `crates/porthole-adapter-macos/src/lib.rs`:
- Add `pub mod search;` and `pub mod window_alive;` module declarations.
- Replace the `todo!()` stubs in the `impl Adapter for MacOsAdapter` block:

```rust
    async fn search(
        &self,
        query: &porthole_core::SearchQuery,
    ) -> Result<Vec<porthole_core::Candidate>, porthole_core::PortholeError> {
        search::search(query).await
    }

    async fn window_alive(
        &self,
        pid: u32,
        cg_window_id: u32,
    ) -> Result<Option<porthole_core::SurfaceInfo>, porthole_core::PortholeError> {
        window_alive::window_alive(pid, cg_window_id).await
    }
```

- [ ] **Step 4: Build and run unit tests**

Run:
```
cargo build -p porthole-adapter-macos
cargo test -p porthole-adapter-macos --lib search
```

Expected: clean build, 4 passes in the search module.

- [ ] **Step 5: Run whole workspace test**

```
cargo test --workspace --locked
```

Expected: all tests pass.

- [ ] **Step 6: Commit**

```bash
git add crates/porthole-adapter-macos/src/search.rs crates/porthole-adapter-macos/src/window_alive.rs crates/porthole-adapter-macos/src/lib.rs
git commit -m "feat(adapter-macos): implement search (on-screen filter) and window_alive (broad enumeration)"
```

---

## Task 6: porthole-core — AttachPipeline

**Files:**
- Create: `crates/porthole-core/src/attach_pipeline.rs`
- Modify: `crates/porthole-core/src/lib.rs`

- [ ] **Step 1: Write `attach_pipeline.rs`**

```rust
use std::sync::Arc;

use crate::adapter::Adapter;
use crate::handle::HandleStore;
use crate::search::{decode_ref, Candidate, SearchQuery};
use crate::surface::SurfaceInfo;
use crate::{ErrorCode, PortholeError};

pub struct AttachPipeline {
    adapter: Arc<dyn Adapter>,
    handles: HandleStore,
}

pub struct TrackedOutcome {
    pub surface: SurfaceInfo,
    pub reused_existing_handle: bool,
}

impl AttachPipeline {
    pub fn new(adapter: Arc<dyn Adapter>, handles: HandleStore) -> Self {
        Self { adapter, handles }
    }

    pub async fn search(&self, query: &SearchQuery) -> Result<Vec<Candidate>, PortholeError> {
        if let Some(p) = &query.title_pattern {
            if let Err(e) = regex::Regex::new(p) {
                return Err(PortholeError::new(
                    ErrorCode::InvalidArgument,
                    format!("invalid title_pattern regex: {e}"),
                ));
            }
        }
        self.adapter.search(query).await
    }

    pub async fn track(&self, r: &str) -> Result<TrackedOutcome, PortholeError> {
        let (pid, cg) = decode_ref(r)?;
        let info = self
            .adapter
            .window_alive(pid, cg)
            .await?
            .ok_or_else(|| {
                PortholeError::new(
                    ErrorCode::SurfaceDead,
                    format!("window with cg_window_id {cg} (pid {pid}) is no longer alive"),
                )
            })?;
        let (surface, reused) = self.handles.track_or_get(info).await;
        Ok(TrackedOutcome { surface, reused_existing_handle: reused })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::in_memory::InMemoryAdapter;
    use crate::search::encode_ref;
    use crate::surface::SurfaceId;

    fn surface_with_cg(pid: u32, cg: u32) -> SurfaceInfo {
        let mut info = SurfaceInfo::window(SurfaceId::new(), pid);
        info.cg_window_id = Some(cg);
        info
    }

    #[tokio::test]
    async fn track_decodes_ref_and_dispatches_to_adapter() {
        let adapter = Arc::new(InMemoryAdapter::new());
        adapter.set_next_window_alive_result(Ok(Some(surface_with_cg(9876, 42)))).await;
        let pipeline = AttachPipeline::new(adapter.clone(), HandleStore::new());
        let r = encode_ref(9876, 42);
        let out = pipeline.track(&r).await.unwrap();
        assert!(!out.reused_existing_handle);
        assert_eq!(out.surface.cg_window_id, Some(42));
        let calls = adapter.window_alive_calls().await;
        assert_eq!(calls, vec![(9876, 42)]);
    }

    #[tokio::test]
    async fn track_returns_surface_dead_when_window_gone() {
        let adapter = Arc::new(InMemoryAdapter::new());
        adapter.set_next_window_alive_result(Ok(None)).await;
        let pipeline = AttachPipeline::new(adapter, HandleStore::new());
        let r = encode_ref(1, 1);
        let err = pipeline.track(&r).await.unwrap_err();
        assert_eq!(err.code, ErrorCode::SurfaceDead);
    }

    #[tokio::test]
    async fn track_errors_on_malformed_ref() {
        let pipeline = AttachPipeline::new(Arc::new(InMemoryAdapter::new()), HandleStore::new());
        let err = pipeline.track("not-a-ref").await.unwrap_err();
        assert_eq!(err.code, ErrorCode::CandidateRefUnknown);
    }

    #[tokio::test]
    async fn track_reuses_existing_alive_handle() {
        let adapter = Arc::new(InMemoryAdapter::new());
        adapter
            .set_next_window_alive_result(Ok(Some(surface_with_cg(1, 7))))
            .await;
        let handles = HandleStore::new();
        let pipeline = AttachPipeline::new(adapter.clone(), handles.clone());
        let r = encode_ref(1, 7);
        let first = pipeline.track(&r).await.unwrap();

        // Second call — adapter returns a different SurfaceInfo (fresh id),
        // but track_or_get should return the first one.
        adapter
            .set_next_window_alive_result(Ok(Some(surface_with_cg(1, 7))))
            .await;
        let second = pipeline.track(&r).await.unwrap();
        assert!(second.reused_existing_handle);
        assert_eq!(second.surface.id, first.surface.id);
    }

    #[tokio::test]
    async fn search_rejects_invalid_regex() {
        let pipeline = AttachPipeline::new(Arc::new(InMemoryAdapter::new()), HandleStore::new());
        let err = pipeline
            .search(&SearchQuery { title_pattern: Some("[invalid".into()), ..Default::default() })
            .await
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidArgument);
    }
}
```

- [ ] **Step 2: Register module**

Edit `crates/porthole-core/src/lib.rs`: add `pub mod attach_pipeline;`.

- [ ] **Step 3: Run tests**

Run: `cargo test -p porthole-core --lib attach_pipeline`
Expected: 5 passes.

- [ ] **Step 4: Commit**

```bash
git add crates/porthole-core/src/attach_pipeline.rs crates/porthole-core/src/lib.rs
git commit -m "feat(core): add AttachPipeline with validation, ref decode, idempotent track"
```

---

## Task 7: porthole-protocol — Wire types

**Files:**
- Create: `crates/porthole-protocol/src/search.rs`
- Modify: `crates/porthole-protocol/src/lib.rs`

- [ ] **Step 1: Write `search.rs`**

Create `crates/porthole-protocol/src/search.rs`:

```rust
use serde::{Deserialize, Serialize};

pub use porthole_core::search::{Candidate, SearchQuery};

#[derive(Clone, Debug, Deserialize)]
pub struct SearchRequest {
    #[serde(flatten)]
    pub query: SearchQuery,
    #[serde(default)]
    pub session: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct SearchResponse {
    pub candidates: Vec<Candidate>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct TrackRequest {
    #[serde(rename = "ref")]
    pub ref_: String,
    #[serde(default)]
    pub session: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct TrackResponse {
    pub surface_id: String,
    pub cg_window_id: u32,
    pub pid: u32,
    pub app_name: Option<String>,
    pub title: Option<String>,
    pub reused_existing_handle: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_request_flattens_query_fields() {
        let json = r#"{"app_name":"Ghostty","pids":[1,2],"frontmost":true}"#;
        let req: SearchRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.query.app_name, Some("Ghostty".into()));
        assert_eq!(req.query.pids, vec![1, 2]);
        assert_eq!(req.query.frontmost, Some(true));
    }

    #[test]
    fn track_request_uses_ref_wire_field() {
        let json = r#"{"ref":"ref_abc"}"#;
        let req: TrackRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.ref_, "ref_abc");
    }

    #[test]
    fn track_response_roundtrip() {
        let r = TrackResponse {
            surface_id: "surf_1".into(),
            cg_window_id: 42,
            pid: 9876,
            app_name: Some("Ghostty".into()),
            title: Some("t".into()),
            reused_existing_handle: true,
        };
        let json = serde_json::to_string(&r).unwrap();
        assert!(json.contains("\"reused_existing_handle\":true"));
    }
}
```

- [ ] **Step 2: Register module**

Edit `crates/porthole-protocol/src/lib.rs`: add `pub mod search;`.

- [ ] **Step 3: Run tests**

Run: `cargo test -p porthole-protocol --lib search`
Expected: 3 passes.

- [ ] **Step 4: Commit**

```bash
git add crates/porthole-protocol/src/search.rs crates/porthole-protocol/src/lib.rs
git commit -m "feat(protocol): add wire types for search and track"
```

---

## Task 8: portholed — Routes + state + /info capabilities

**Files:**
- Modify: `crates/portholed/src/state.rs`
- Create: `crates/portholed/src/routes/attach.rs`
- Modify: `crates/portholed/src/routes/mod.rs`
- Modify: `crates/portholed/src/routes/info.rs`
- Modify: `crates/portholed/src/server.rs`

- [ ] **Step 1: Extend AppState**

Edit `crates/portholed/src/state.rs` — add `AttachPipeline`:

```rust
use porthole_core::attach_pipeline::AttachPipeline;
```

In the struct, after `wait: Arc<WaitPipeline>`:

```rust
    pub attach: Arc<AttachPipeline>,
```

In `AppState::new`, after constructing `wait`:

```rust
        let attach = Arc::new(AttachPipeline::new(adapter.clone(), handles.clone()));
```

And in the returned struct literal, add `attach,`.

- [ ] **Step 2: Write `routes/attach.rs`**

Create `crates/portholed/src/routes/attach.rs`:

```rust
use axum::extract::State;
use axum::Json;
use porthole_protocol::search::{
    SearchRequest, SearchResponse, TrackRequest, TrackResponse,
};

use crate::routes::errors::ApiError;
use crate::state::AppState;

pub async fn post_search(
    State(state): State<AppState>,
    Json(req): Json<SearchRequest>,
) -> Result<Json<SearchResponse>, ApiError> {
    // session intentionally dropped until SSE events slice
    let candidates = state.attach.search(&req.query).await?;
    Ok(Json(SearchResponse { candidates }))
}

pub async fn post_track(
    State(state): State<AppState>,
    Json(req): Json<TrackRequest>,
) -> Result<Json<TrackResponse>, ApiError> {
    // session intentionally dropped until SSE events slice
    let outcome = state.attach.track(&req.ref_).await?;
    let info = &outcome.surface;
    Ok(Json(TrackResponse {
        surface_id: info.id.to_string(),
        cg_window_id: info.cg_window_id.unwrap_or(0),
        pid: info.pid.unwrap_or(0),
        app_name: info.app_name.clone(),
        title: info.title.clone(),
        reused_existing_handle: outcome.reused_existing_handle,
    }))
}
```

- [ ] **Step 3: Register route module**

Edit `crates/portholed/src/routes/mod.rs`: add `pub mod attach;`.

- [ ] **Step 4: Wire routes into server**

Edit `crates/portholed/src/server.rs` — inside `build_router`:

```rust
        .route("/surfaces/search", post(attach_route::post_search))
        .route("/surfaces/track", post(attach_route::post_track))
```

Add the import:

```rust
use crate::routes::attach as attach_route;
```

- [ ] **Step 5: Update `/info` capabilities via adapter**

Edit `crates/porthole-core/src/in_memory.rs`. Add `"search"` and `"track"` to the `capabilities()` list, after `"displays"`.

Edit `crates/porthole-adapter-macos/src/lib.rs`. Add `"search"` and `"track"` to the `capabilities()` list, after `"displays"`.

- [ ] **Step 6: Add router tests**

Append to the `tests` module in `crates/portholed/src/server.rs`:

```rust
    #[tokio::test]
    async fn post_search_returns_candidates_from_adapter_script() {
        use porthole_core::search::Candidate;
        let adapter = Arc::new(InMemoryAdapter::new());
        let candidate = Candidate {
            ref_: "ref_abc".into(),
            app_name: Some("X".into()),
            title: Some("t".into()),
            pid: 1,
            cg_window_id: 7,
        };
        adapter.set_next_search_result(Ok(vec![candidate])).await;
        let router = build_router(AppState::new(adapter));
        let res = post(router, "/surfaces/search", serde_json::json!({})).await;
        assert_eq!(res.status(), StatusCode::OK);
        let body = to_bytes(res.into_body(), 1024 * 1024).await.unwrap();
        let resp: porthole_protocol::search::SearchResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(resp.candidates.len(), 1);
    }

    #[tokio::test]
    async fn post_track_mints_handle_and_idempotent_reuse() {
        use porthole_core::search::encode_ref;
        use porthole_core::surface::{SurfaceId, SurfaceInfo};

        let adapter = Arc::new(InMemoryAdapter::new());
        // Two back-to-back window_alive calls, both returning the same (pid,cg).
        for _ in 0..2 {
            let mut info = SurfaceInfo::window(SurfaceId::new(), 1);
            info.cg_window_id = Some(7);
            info.app_name = Some("X".into());
            adapter.set_next_window_alive_result(Ok(Some(info))).await;
        }
        let router = build_router(AppState::new(adapter));

        let r = encode_ref(1, 7);
        let body = serde_json::json!({ "ref": r });

        let res = post(router.clone(), "/surfaces/track", body.clone()).await;
        assert_eq!(res.status(), StatusCode::OK);
        let first_body = to_bytes(res.into_body(), 1024 * 1024).await.unwrap();
        let first: porthole_protocol::search::TrackResponse = serde_json::from_slice(&first_body).unwrap();
        assert!(!first.reused_existing_handle);

        let res = post(router, "/surfaces/track", body).await;
        assert_eq!(res.status(), StatusCode::OK);
        let second_body = to_bytes(res.into_body(), 1024 * 1024).await.unwrap();
        let second: porthole_protocol::search::TrackResponse = serde_json::from_slice(&second_body).unwrap();
        assert!(second.reused_existing_handle);
        assert_eq!(second.surface_id, first.surface_id);
    }

    #[tokio::test]
    async fn post_track_with_malformed_ref_returns_not_found() {
        let adapter = Arc::new(InMemoryAdapter::new());
        let router = build_router(AppState::new(adapter));
        let res = post(router, "/surfaces/track", serde_json::json!({ "ref": "junk" })).await;
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
    }
```

Note: `ApiError` maps `CandidateRefUnknown` → 404 NOT_FOUND; confirm against slice A's errors.rs. If it's mapped elsewhere, adjust the test.

- [ ] **Step 7: Run tests**

Run: `cargo test -p portholed --lib server`
Expected: slice A's tests still pass + 3 new = 12 total (adjust if slice A's count has drifted).

- [ ] **Step 8: Commit**

```bash
git add -A
git commit -m "feat(daemon): add POST /surfaces/search and /surfaces/track routes"
```

---

## Task 9: porthole CLI — Ancestry helper

**Files:**
- Create: `crates/porthole/src/ancestry.rs`
- Modify: `crates/porthole/src/lib.rs`

- [ ] **Step 1: Write `ancestry.rs`**

Create `crates/porthole/src/ancestry.rs`:

```rust
//! Walks up a process ancestry chain via `/bin/ps -o ppid= -p <pid>`.
//!
//! Best-effort: returns whatever ancestors could be walked. Failures mid-walk
//! log via `tracing::warn!` and the partial chain is returned.

use std::process::Command;

const MAX_DEPTH: usize = 128;

/// Returns the ancestry chain starting from `pid`. Includes `pid` itself as
/// the first element, followed by parent, grandparent, etc. Stops at PID 1,
/// at `MAX_DEPTH`, or at the first `ps` failure.
pub fn containing_ancestors(pid: u32) -> Vec<u32> {
    let mut out = vec![pid];
    let mut current = pid;
    for _ in 0..MAX_DEPTH {
        if current <= 1 {
            break;
        }
        match parent_of(current) {
            Some(parent) if parent != current => {
                out.push(parent);
                current = parent;
            }
            Some(_) => break, // self-loop guard
            None => break,
        }
    }
    out
}

fn parent_of(pid: u32) -> Option<u32> {
    let output = Command::new("/bin/ps")
        .args(["-o", "ppid=", "-p", &pid.to_string()])
        .output();
    let output = match output {
        Ok(o) => o,
        Err(e) => {
            tracing::warn!(pid, error = %e, "ancestry: ps invocation failed");
            return None;
        }
    };
    if !output.status.success() {
        tracing::warn!(pid, status = ?output.status, "ancestry: ps exited non-zero");
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let trimmed = text.trim();
    match trimmed.parse::<u32>() {
        Ok(ppid) => Some(ppid),
        Err(_) => {
            tracing::warn!(pid, output = %trimmed, "ancestry: could not parse ppid");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn walk_from_current_process_returns_self_plus_ancestors() {
        let me = std::process::id();
        let chain = containing_ancestors(me);
        assert!(!chain.is_empty());
        assert_eq!(chain[0], me);
        // Most likely there is at least one ancestor (the test runner / shell).
        // Don't assert exact depth — depends on runner.
    }

    #[test]
    fn walk_stops_at_pid_1() {
        let chain = containing_ancestors(1);
        assert_eq!(chain, vec![1]);
    }

    #[test]
    fn walk_on_nonexistent_pid_returns_just_the_pid() {
        // Pick a PID that's very unlikely to exist. If ps fails, we still
        // return the seed pid.
        let chain = containing_ancestors(999_999_999);
        assert_eq!(chain[0], 999_999_999);
    }
}
```

- [ ] **Step 2: Register module**

Edit `crates/porthole/src/lib.rs`: add `pub mod ancestry;`.

Also add `tracing` as a dep of the CLI crate if not already present. Check `crates/porthole/Cargo.toml` — if `tracing` isn't there, add it:

```toml
tracing = { workspace = true }
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p porthole --lib ancestry`
Expected: 3 passes.

- [ ] **Step 4: Commit**

```bash
git add crates/porthole/src/ancestry.rs crates/porthole/src/lib.rs crates/porthole/Cargo.toml
git commit -m "feat(cli): add ancestry::containing_ancestors helper"
```

---

## Task 10: porthole CLI — search, track, attach subcommands

**Files:**
- Create: `crates/porthole/src/commands/search.rs`
- Create: `crates/porthole/src/commands/track.rs`
- Create: `crates/porthole/src/commands/attach.rs`
- Modify: `crates/porthole/src/commands/mod.rs`
- Modify: `crates/porthole/src/main.rs`

- [ ] **Step 1: Write `commands/search.rs`**

```rust
use porthole_core::search::SearchQuery;
use porthole_protocol::search::{SearchRequest, SearchResponse};

use crate::client::{ClientError, DaemonClient};

pub struct SearchArgs {
    pub app_name: Option<String>,
    pub title_pattern: Option<String>,
    pub pids: Vec<u32>,
    pub cg_window_ids: Vec<u32>,
    pub frontmost: Option<bool>,
    pub session: Option<String>,
    pub json: bool,
}

pub async fn run(client: &DaemonClient, args: SearchArgs) -> Result<(), ClientError> {
    let query = SearchQuery {
        app_name: args.app_name,
        title_pattern: args.title_pattern,
        pids: args.pids,
        cg_window_ids: args.cg_window_ids,
        frontmost: args.frontmost,
    };
    let req = SearchRequest { query, session: args.session };
    let res: SearchResponse = client.post_json("/surfaces/search", &req).await?;
    if args.json {
        let text = serde_json::to_string_pretty(&res.candidates)
            .map_err(|e| ClientError::Local(format!("json encode: {e}")))?;
        println!("{text}");
    } else {
        for c in &res.candidates {
            println!(
                "{}  pid={}  cg={}  app={}  title={}",
                c.ref_,
                c.pid,
                c.cg_window_id,
                c.app_name.as_deref().unwrap_or("-"),
                c.title.as_deref().unwrap_or("-"),
            );
        }
    }
    Ok(())
}
```

- [ ] **Step 2: Write `commands/track.rs`**

```rust
use porthole_protocol::search::{TrackRequest, TrackResponse};

use crate::client::{ClientError, DaemonClient};

pub struct TrackArgs {
    pub ref_: String,
    pub session: Option<String>,
    pub json: bool,
}

pub async fn run(client: &DaemonClient, args: TrackArgs) -> Result<(), ClientError> {
    let req = TrackRequest { ref_: args.ref_, session: args.session };
    let res: TrackResponse = client.post_json("/surfaces/track", &req).await?;
    if args.json {
        let text = serde_json::to_string_pretty(&res)
            .map_err(|e| ClientError::Local(format!("json encode: {e}")))?;
        println!("{text}");
    } else {
        println!("surface_id: {}", res.surface_id);
        println!("pid: {}, cg_window_id: {}", res.pid, res.cg_window_id);
        println!("app_name: {}", res.app_name.as_deref().unwrap_or("-"));
        println!("title: {}", res.title.as_deref().unwrap_or("-"));
        println!("reused_existing_handle: {}", res.reused_existing_handle);
    }
    Ok(())
}
```

- [ ] **Step 3: Write `commands/attach.rs`**

```rust
use porthole_core::search::SearchQuery;
use porthole_protocol::search::{SearchRequest, SearchResponse, TrackRequest, TrackResponse};

use crate::ancestry::containing_ancestors;
use crate::client::{ClientError, DaemonClient};

pub struct AttachArgs {
    pub app_name: Option<String>,
    pub title_pattern: Option<String>,
    pub pids: Vec<u32>,
    pub containing_pids: Vec<u32>,
    pub cg_window_ids: Vec<u32>,
    pub frontmost: Option<bool>,
    pub session: Option<String>,
    pub json: bool,
}

pub async fn run(client: &DaemonClient, args: AttachArgs) -> Result<(), ClientError> {
    // Union --pid and --containing-pid into a single list.
    let mut pids = args.pids.clone();
    for root in &args.containing_pids {
        pids.extend(containing_ancestors(*root));
    }
    pids.sort_unstable();
    pids.dedup();

    let query = SearchQuery {
        app_name: args.app_name,
        title_pattern: args.title_pattern,
        pids,
        cg_window_ids: args.cg_window_ids,
        frontmost: args.frontmost,
    };

    // Search, pick if unique, track.
    let search: SearchResponse =
        client.post_json("/surfaces/search", &SearchRequest { query, session: args.session.clone() }).await?;
    if search.candidates.is_empty() {
        return Err(ClientError::Local("attach: no matching windows".to_string()));
    }
    if search.candidates.len() > 1 {
        let list = search
            .candidates
            .iter()
            .map(|c| {
                format!(
                    "  {}  pid={}  cg={}  app={:?}  title={:?}",
                    c.ref_, c.pid, c.cg_window_id, c.app_name, c.title,
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        return Err(ClientError::Local(format!(
            "attach: {} candidates matched — use `porthole search` with stricter filters or pass --frontmost. Matches:\n{list}",
            search.candidates.len()
        )));
    }
    let chosen = search.candidates.into_iter().next().unwrap();
    let res: TrackResponse = client
        .post_json(
            "/surfaces/track",
            &TrackRequest { ref_: chosen.ref_, session: args.session },
        )
        .await?;
    if args.json {
        let text = serde_json::to_string_pretty(&res)
            .map_err(|e| ClientError::Local(format!("json encode: {e}")))?;
        println!("{text}");
    } else {
        println!("{}", res.surface_id);
    }
    Ok(())
}
```

- [ ] **Step 4: Update `commands/mod.rs`**

Add:
```rust
pub mod attach;
pub mod search;
pub mod track;
```

- [ ] **Step 5: Update `main.rs`**

Add three new `Command` variants:

```rust
    /// Search for candidate windows.
    Search {
        #[arg(long)]
        app_name: Option<String>,
        #[arg(long)]
        title_pattern: Option<String>,
        #[arg(long = "pid")]
        pids: Vec<u32>,
        #[arg(long = "cg-window-id")]
        cg_window_ids: Vec<u32>,
        #[arg(long)]
        frontmost: bool,
        #[arg(long)]
        session: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Track a candidate ref, minting a surface handle.
    Track {
        #[arg(value_name = "REF")]
        ref_: String,
        #[arg(long)]
        session: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Search + pick-if-unique + track. Exits non-zero on 0 or >1 matches.
    Attach {
        #[arg(long)]
        app_name: Option<String>,
        #[arg(long)]
        title_pattern: Option<String>,
        #[arg(long = "pid")]
        pids: Vec<u32>,
        #[arg(long = "containing-pid")]
        containing_pids: Vec<u32>,
        #[arg(long = "cg-window-id")]
        cg_window_ids: Vec<u32>,
        #[arg(long)]
        frontmost: bool,
        #[arg(long)]
        session: Option<String>,
        #[arg(long)]
        json: bool,
    },
```

In the `match cli.command` block:

```rust
        Command::Search { app_name, title_pattern, pids, cg_window_ids, frontmost, session, json } => {
            use porthole::commands::search as search_cmd;
            search_cmd::run(
                &client,
                search_cmd::SearchArgs {
                    app_name,
                    title_pattern,
                    pids,
                    cg_window_ids,
                    frontmost: if frontmost { Some(true) } else { None },
                    session,
                    json,
                },
            )
            .await
        }
        Command::Track { ref_, session, json } => {
            use porthole::commands::track as track_cmd;
            track_cmd::run(&client, track_cmd::TrackArgs { ref_, session, json }).await
        }
        Command::Attach { app_name, title_pattern, pids, containing_pids, cg_window_ids, frontmost, session, json } => {
            use porthole::commands::attach as attach_cmd;
            attach_cmd::run(
                &client,
                attach_cmd::AttachArgs {
                    app_name,
                    title_pattern,
                    pids,
                    containing_pids,
                    cg_window_ids,
                    frontmost: if frontmost { Some(true) } else { None },
                    session,
                    json,
                },
            )
            .await
        }
```

- [ ] **Step 6: Build**

Run: `cargo build -p porthole`
Expected: clean.

- [ ] **Step 7: Commit**

```bash
git add crates/porthole
git commit -m "feat(cli): add search, track, attach subcommands"
```

---

## Task 11: Full workspace sanity

- [ ] **Step 1: Build**

```
cargo build --workspace --locked
```

Expected: clean.

- [ ] **Step 2: Tests**

```
cargo test --workspace --locked
```

Expected: all non-ignored pass. Count rises from slice-A's 77 to roughly 95–100 (search+ancestry+attach+handle unit tests plus new server tests).

- [ ] **Step 3: Clippy gate**

```
cargo clippy --workspace --all-targets --locked -- -D warnings
```

Expected: clean. Fix any warnings that surface — unused imports, redundant clones, etc.

- [ ] **Step 4: Commit any cleanup**

If clippy required fixes:

```bash
git add -A
git commit -m "chore: workspace clippy cleanup after slice-B"
```

Skip if nothing was needed.

---

## Task 12: macOS integration tests (ignored)

**Files:**
- Create: `crates/porthole-adapter-macos/tests/attach_integration.rs`

- [ ] **Step 1: Write the test suite**

```rust
#![cfg(target_os = "macos")]

use std::time::Duration;

use porthole_adapter_macos::MacOsAdapter;
use porthole_core::adapter::{Adapter, ProcessLaunchSpec, RequireConfidence};
use porthole_core::search::{encode_ref, SearchQuery};

fn textedit_spec() -> ProcessLaunchSpec {
    ProcessLaunchSpec {
        app: "/System/Applications/TextEdit.app".to_string(),
        args: vec![],
        cwd: None,
        env: vec![],
        timeout: Duration::from_secs(10),
        require_confidence: RequireConfidence::Strong,
    }
}

#[tokio::test]
#[ignore = "requires a real macOS desktop session + permissions"]
async fn search_finds_launched_textedit_by_app_name() {
    let adapter = MacOsAdapter::new();
    let outcome = adapter.launch_process(&textedit_spec()).await.expect("launch");
    let query = SearchQuery { app_name: Some("TextEdit".into()), ..Default::default() };
    let candidates = adapter.search(&query).await.expect("search");
    assert!(
        candidates.iter().any(|c| c.pid == outcome.surface.pid.unwrap()),
        "search did not find launched TextEdit"
    );
    adapter.close(&outcome.surface).await.ok();
}

#[tokio::test]
#[ignore = "requires a real macOS desktop session + permissions"]
async fn window_alive_survives_app_hide() {
    use std::process::Command;
    let adapter = MacOsAdapter::new();
    let outcome = adapter.launch_process(&textedit_spec()).await.expect("launch");
    let pid = outcome.surface.pid.unwrap();
    let cg = outcome.surface.cg_window_id.unwrap();
    // Issue Cmd+H via osascript to hide the app.
    Command::new("/usr/bin/osascript")
        .args([
            "-e",
            r#"tell application "System Events" to tell process "TextEdit" to set visible to false"#,
        ])
        .output()
        .ok();
    tokio::time::sleep(Duration::from_millis(500)).await;
    // Hidden windows should still resolve as alive under window_alive's broad enum.
    let alive = adapter.window_alive(pid, cg).await.expect("window_alive");
    assert!(alive.is_some(), "hidden window should still be alive");
    adapter.close(&outcome.surface).await.ok();
}

#[tokio::test]
#[ignore = "requires a real macOS desktop session + permissions"]
async fn track_via_ref_roundtrip() {
    let adapter = MacOsAdapter::new();
    let outcome = adapter.launch_process(&textedit_spec()).await.expect("launch");
    let pid = outcome.surface.pid.unwrap();
    let cg = outcome.surface.cg_window_id.unwrap();
    let r = encode_ref(pid, cg);
    // Decode via window_alive (what the track path does).
    let info = adapter.window_alive(pid, cg).await.expect("alive").expect("some");
    assert_eq!(info.cg_window_id, Some(cg));
    adapter.close(&outcome.surface).await.ok();
    let _ = r;
}
```

- [ ] **Step 2: Build the test**

Run: `cargo build --tests -p porthole-adapter-macos`
Expected: clean.

- [ ] **Step 3: Commit**

```bash
git add crates/porthole-adapter-macos/tests/attach_integration.rs
git commit -m "test(adapter-macos): ignored attach + window_alive integration tests"
```

Optionally run the suite on a real macOS desktop with:
```
cargo test -p porthole-adapter-macos --test attach_integration -- --ignored --nocapture
```

---

## Task 13: End-to-end CLI-through-UDS test

**Files:**
- Create: `crates/portholed/tests/slice_b_e2e.rs`

- [ ] **Step 1: Write the test**

```rust
use std::sync::Arc;
use std::time::Duration;

use porthole_core::in_memory::InMemoryAdapter;
use porthole_core::search::{encode_ref, Candidate};
use porthole_core::surface::{SurfaceId, SurfaceInfo};
use portholed::server::serve;

#[tokio::test]
async fn search_track_roundtrip_over_uds() {
    let tmp = tempfile::tempdir().unwrap();
    let socket = tmp.path().join("porthole.sock");

    let adapter = Arc::new(InMemoryAdapter::new());
    // Seed one candidate + one window_alive result.
    let r = encode_ref(77, 123);
    adapter
        .set_next_search_result(Ok(vec![Candidate {
            ref_: r.clone(),
            app_name: Some("ScriptedApp".into()),
            title: Some("one".into()),
            pid: 77,
            cg_window_id: 123,
        }]))
        .await;
    let mut info = SurfaceInfo::window(SurfaceId::new(), 77);
    info.cg_window_id = Some(123);
    info.app_name = Some("ScriptedApp".into());
    adapter.set_next_window_alive_result(Ok(Some(info))).await;

    let socket_for_serve = socket.clone();
    let adapter_for_serve: Arc<dyn porthole_core::adapter::Adapter> = adapter.clone();
    let server_task = tokio::spawn(async move { serve(adapter_for_serve, socket_for_serve).await });

    for _ in 0..200 {
        if socket.exists() { break; }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(socket.exists(), "socket did not appear");

    let client = porthole::client::DaemonClient::new(&socket);

    let search: porthole_protocol::search::SearchResponse = client
        .post_json(
            "/surfaces/search",
            &serde_json::json!({ "app_name": "ScriptedApp" }),
        )
        .await
        .expect("search");
    assert_eq!(search.candidates.len(), 1);
    assert_eq!(search.candidates[0].ref_, r);

    let track: porthole_protocol::search::TrackResponse = client
        .post_json(
            "/surfaces/track",
            &serde_json::json!({ "ref": r }),
        )
        .await
        .expect("track");
    assert!(!track.reused_existing_handle);
    assert_eq!(track.cg_window_id, 123);

    server_task.abort();
}
```

- [ ] **Step 2: Run the test**

Run: `cargo test -p portholed --test slice_b_e2e`
Expected: 1 pass.

- [ ] **Step 3: Full workspace test**

```
cargo test --workspace --locked
```

Expected: all non-ignored pass. Integration tests from slice A + this one + the ignored attach tests.

- [ ] **Step 4: Clippy gate**

```
cargo clippy --workspace --all-targets --locked -- -D warnings
```

Expected: clean.

- [ ] **Step 5: Commit**

```bash
git add crates/portholed/tests/slice_b_e2e.rs
git commit -m "test(daemon): end-to-end slice-B search + track over UDS"
```

---

## What Slice B Delivers

- `POST /surfaces/search` with `app_name` / `title_pattern` / `pids` / `cg_window_ids` / `frontmost` filters.
- `POST /surfaces/track` with self-encoded refs, idempotent (one alive handle per `cg_window_id`).
- `HandleStore::track_or_get` atomic under concurrency (race-tested).
- `Adapter::search` (on-screen filter on macOS) and `Adapter::window_alive` (broad enumeration, survives hide/minimize/Space-switch).
- CLI `search`, `track`, and `attach` subcommands; `attach --containing-pid <PID>` walks PID ancestry via `/bin/ps`.
- `porthole::ancestry::containing_ancestors` library helper.
- Rename of `app_bundle` → `app_name` on `SurfaceInfo` and `AttentionInfo` (and `bundleIdentifier` → `localizedName` in the macOS attention path) to consistently hold display names.

## What Slice B Intentionally Does Not Deliver

Revisit in subsequent plans:

- True bundle-id fields (`app_bundle_id`) — future additive field
- `on_display` / `bounds_in` / AX-path search filters
- `include_hidden` search flag
- Tab candidates in search
- Events SSE — still the unblocker for reactive search / MRU
- Cache-backed refs (not needed without a security boundary)
- Dead-handle garbage collection
