# Porthole Quality Round Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close three specific quality gaps in the v0 foundation + slice A + slice B work before starting the presentation slice: (1) end-to-end test that a script finds its own terminal window via `attach --containing-pid $$ --frontmost`, (2) RAII wrapper for raw AX refs in the macOS adapter, (3) populate `last_observed` diagnostics on `wait` timeouts instead of returning placeholder zeros.

**Architecture:** Maintenance-level; no new public API surface. Adds one integration test, one new module (`porthole-adapter-macos::ax`), refactors existing AX helpers to use it, and threads a deadline through `Adapter::wait` so the adapter can return real diagnostics on timeout instead of the pipeline discarding the wait future via `tokio::time::timeout`.

**Tech Stack:** Same as slice B — Rust 2024, existing crate deps.

---

## Out of Scope for This Plan

- Presentation slice (placement, artifact launches, replace)
- Events SSE
- TTY-based multi-window disambiguation
- True `app_bundle_id` field
- Dead-handle GC
- Recording
- AX observer-based wait (stable/dirty stays polling; the diagnostics fix does not change the sampling mechanism)

---

## File Structure

```
crates/porthole-adapter-macos/
  src/
    ax.rs                         # NEW: AxElement RAII wrapper
    close_focus.rs                # modify: use AxElement instead of raw pointers
    wait.rs                       # modify: deadline-aware wait with real last_observed
    lib.rs                        # modify: declare ax module
  tests/
    self_find_e2e.rs              # NEW: spawns porthole CLI against test daemon

crates/porthole-core/
  src/
    adapter.rs                    # modify: deadline on wait; drop wait_last_observed
    in_memory.rs                  # modify: match new signature
    wait_pipeline.rs              # modify: pass deadline to adapter instead of wrapping in timeout
    wait.rs                       # unchanged (types stay)
```

---

## Task 1: End-to-end self-find test

**Files:**
- Create: `crates/porthole-adapter-macos/tests/self_find_e2e.rs`

Context: the entire "run TUI / find my own window / screenshot" story hinges on `attach --containing-pid $$ --frontmost` working end-to-end inside a real terminal. The current tests exercise each component (ancestry walk, search, track) but no test spawns the real CLI against a real daemon with a real ancestry chain. This gap is flagged in the slice-B review.

The test spawns the daemon on a tempfile UDS with a scripted in-memory adapter that returns a candidate matching the test process's own PID, then invokes the `porthole` CLI binary (via `CARGO_BIN_EXE_porthole`) with `attach --containing-pid $$ --frontmost`, and asserts the CLI exits zero and prints a surface id.

- [ ] **Step 1: Write the test**

Create `crates/porthole-adapter-macos/tests/self_find_e2e.rs`:

```rust
#![cfg(target_os = "macos")]

use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use porthole_core::in_memory::InMemoryAdapter;
use porthole_core::search::Candidate;
use porthole_core::surface::{SurfaceId, SurfaceInfo};
use portholed::server::serve;

/// Exercises the full path from `porthole attach --containing-pid $$
/// --frontmost` back to a scripted candidate that matches the test
/// process's own PID. Uses the in-memory adapter so no real macOS
/// desktop is required — this asserts the ancestry walk + CLI flag
/// wiring + daemon round-trip, not the real AX integration.
#[tokio::test]
async fn attach_containing_pid_self_finds_scripted_candidate() {
    let test_pid = std::process::id();

    let tmp = tempfile::tempdir().unwrap();
    let socket = tmp.path().join("porthole.sock");

    let adapter = Arc::new(InMemoryAdapter::new());
    // Script a single candidate whose PID equals this test's own PID.
    // The CLI will include this PID in its ancestry-derived pids list,
    // so the in-memory adapter returning any candidate is sufficient:
    // search doesn't actually filter inside the in-memory adapter
    // (it just returns whatever is scripted).
    let candidate = Candidate {
        ref_: porthole_core::search::encode_ref(test_pid, 42),
        app_name: Some("TestHarness".into()),
        title: Some("self-find".into()),
        pid: test_pid,
        cg_window_id: 42,
    };
    adapter.set_next_search_result(Ok(vec![candidate])).await;

    let mut info = SurfaceInfo::window(SurfaceId::new(), test_pid);
    info.cg_window_id = Some(42);
    info.app_name = Some("TestHarness".into());
    adapter.set_next_window_alive_result(Ok(Some(info))).await;

    let socket_for_serve = socket.clone();
    let adapter_for_serve: Arc<dyn porthole_core::adapter::Adapter> = adapter.clone();
    let server_task = tokio::spawn(async move { serve(adapter_for_serve, socket_for_serve).await });

    for _ in 0..200 {
        if socket.exists() { break; }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(socket.exists(), "socket did not appear");

    let cli = PathBuf::from(env!("CARGO_BIN_EXE_porthole"));
    let output = tokio::task::spawn_blocking(move || {
        std::process::Command::new(cli)
            .env("PORTHOLE_RUNTIME_DIR", tmp.path())
            .args([
                "attach",
                "--containing-pid",
                &test_pid.to_string(),
                "--frontmost",
                "--json",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
    })
    .await
    .expect("join")
    .expect("spawn");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "CLI exit {:?}\nstdout: {stdout}\nstderr: {stderr}",
        output.status.code()
    );
    let parsed: serde_json::Value =
        serde_json::from_str(stdout.trim()).expect("CLI stdout is not JSON");
    assert!(parsed.get("surface_id").is_some(), "response missing surface_id");
    assert_eq!(parsed.get("cg_window_id").and_then(|v| v.as_u64()), Some(42));

    server_task.abort();

    // keep tmp alive through the command run
    drop(tmp);
}
```

Note the test's lifecycle: `tmp` (a `TempDir`) must outlive the CLI subprocess so the socket path stays valid. The `drop(tmp)` at the end is explicit documentation that we're holding it intentionally.

- [ ] **Step 2: Dev-deps check**

Verify `porthole-adapter-macos/Cargo.toml` already has `porthole`, `portholed`, `porthole-core`, `porthole-protocol`, `tempfile`, `tokio`, `serde_json` as `[dev-dependencies]`. If `porthole` (the CLI crate) isn't there, add:

```toml
porthole = { path = "../porthole" }
```

If `portholed` isn't there, add similarly.

Note on `CARGO_BIN_EXE_porthole`: this env var is set by cargo for integration tests. It requires the current crate to have a dev-dep on the `porthole` CLI crate — cargo then compiles the binary before running the test. If the dev-dep is missing, the env var is empty and `env!` fails at compile time with a clear message.

- [ ] **Step 3: Build + run the test**

Run: `cargo test -p porthole-adapter-macos --test self_find_e2e -- --nocapture`
Expected: 1 pass.

- [ ] **Step 4: Full workspace test**

Run: `cargo test --workspace --locked`
Expected: 110 non-ignored tests (slice B's 109 + this one), 8 ignored.

- [ ] **Step 5: Commit**

```bash
git add crates/porthole-adapter-macos/Cargo.toml crates/porthole-adapter-macos/tests/self_find_e2e.rs
git commit -m "test(adapter-macos): end-to-end self-find via attach --containing-pid"
```

---

## Task 2: Introduce `AxElement` RAII wrapper

**Files:**
- Create: `crates/porthole-adapter-macos/src/ax.rs`
- Modify: `crates/porthole-adapter-macos/src/lib.rs`

Context: the current AX FFI in `close_focus.rs` uses raw `AXUIElementRef` pointers with explicit `CFRelease` calls. The closure-scoped helpers (`with_first_window_for_pid`, `with_ax_window_by_cg_id`) constrain lifetimes correctly today, but any new AX code must re-discover the pattern. A proper RAII wrapper kills the whole class of use-after-free and double-free bugs forever, and makes future AX-heavy work (events slice, tabs slice, presentation's move/resize) much safer.

This task introduces the wrapper without yet migrating existing code.

- [ ] **Step 1: Write `ax.rs`**

Create `crates/porthole-adapter-macos/src/ax.rs`:

```rust
//! RAII wrapper for raw Accessibility (AX) element references.
//!
//! Raw `AXUIElementRef` values are `CFType`-shaped pointers that must be
//! `CFRelease`d by the creator/copier. `AxElement` owns one such pointer
//! and releases it on drop. All FFI calls that produce or consume
//! `AXUIElementRef` should go through this module.

#![cfg(target_os = "macos")]

use core_foundation::base::TCFType;
use core_foundation::string::{CFString, CFStringRef};

pub type AxError = i32;
pub const AX_ERROR_SUCCESS: AxError = 0;

/// Opaque AX element pointer. Implementation detail — callers should
/// operate through `AxElement` methods.
pub type AxElementRef = *const std::ffi::c_void;

unsafe extern "C" {
    fn AXUIElementCreateApplication(pid: i32) -> AxElementRef;
    fn AXUIElementCopyAttributeValue(
        element: AxElementRef,
        attribute: CFStringRef,
        value: *mut *const std::ffi::c_void,
    ) -> AxError;
    fn AXUIElementPerformAction(element: AxElementRef, action: CFStringRef) -> AxError;
    fn _AXUIElementGetWindow(element: AxElementRef, out: *mut u32) -> AxError;
    fn CFRelease(ptr: *const std::ffi::c_void);
}

/// Owned AX element pointer. Drops via `CFRelease`.
///
/// Construct via `AxElement::for_application(pid)` or by wrapping a
/// retained pointer from an AX copy/create call. Never wrap an
/// unretained (get-rule) pointer — it will double-free when the
/// wrapper drops.
pub struct AxElement {
    ptr: AxElementRef,
}

impl AxElement {
    /// Create a top-level application AX element for the given PID.
    /// Returns `None` if the underlying FFI returns null.
    pub fn for_application(pid: i32) -> Option<Self> {
        let ptr = unsafe { AXUIElementCreateApplication(pid) };
        if ptr.is_null() {
            None
        } else {
            Some(Self { ptr })
        }
    }

    /// Wrap a raw retained AXElement pointer. The caller guarantees the
    /// pointer follows the create/copy retain rule (i.e., needs to be
    /// CFRelease'd exactly once by the owner).
    ///
    /// # Safety
    /// Caller must hand over ownership: after this call, do not call
    /// CFRelease on the pointer yourself.
    pub unsafe fn from_retained(ptr: AxElementRef) -> Option<Self> {
        if ptr.is_null() { None } else { Some(Self { ptr }) }
    }

    /// Borrow the raw pointer for FFI calls that need it (e.g.
    /// AXUIElementPerformAction). Must not be used to CFRelease.
    pub fn as_ptr(&self) -> AxElementRef {
        self.ptr
    }

    /// Perform an AX action by name (e.g. "AXPress", "AXRaise").
    pub fn perform_action(&self, action: &str) -> AxError {
        let action_str = CFString::new(action);
        unsafe { AXUIElementPerformAction(self.ptr, action_str.as_concrete_TypeRef() as CFStringRef) }
    }

    /// Copy an attribute value by name. Returns the raw retained pointer
    /// on success — callers wrap it in an appropriate owned type. Returns
    /// None on any error or null value.
    pub fn copy_attribute_raw(&self, attribute: &str) -> Option<*const std::ffi::c_void> {
        let attr_str = CFString::new(attribute);
        let mut out: *const std::ffi::c_void = std::ptr::null();
        let err = unsafe {
            AXUIElementCopyAttributeValue(
                self.ptr,
                attr_str.as_concrete_TypeRef() as CFStringRef,
                &mut out,
            )
        };
        if err == AX_ERROR_SUCCESS && !out.is_null() { Some(out) } else { None }
    }

    /// Look up the CGWindowID for this AX element via the private
    /// `_AXUIElementGetWindow` API. Stable across macOS versions in
    /// widespread use. Returns `None` on any failure.
    pub fn cg_window_id(&self) -> Option<u32> {
        let mut id: u32 = 0;
        let err = unsafe { _AXUIElementGetWindow(self.ptr, &mut id) };
        if err == AX_ERROR_SUCCESS { Some(id) } else { None }
    }
}

impl Drop for AxElement {
    fn drop(&mut self) {
        if !self.ptr.is_null() {
            unsafe { CFRelease(self.ptr) };
        }
    }
}

// AXUIElementRef is not Sync by nature (the AX API is main-thread-ish)
// but we wrap the pointer with reasonable care; don't implement Send/Sync.
// Leave the default non-Send/non-Sync behaviour from the raw pointer.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_from_retained_returns_none() {
        // SAFETY: passing a null pointer; from_retained handles null.
        let e = unsafe { AxElement::from_retained(std::ptr::null()) };
        assert!(e.is_none());
    }

    #[test]
    fn for_application_with_nonexistent_pid_returns_none_or_some() {
        // AXUIElementCreateApplication may return a non-null ref even for
        // a nonexistent PID (it doesn't validate immediately). The value
        // is test-environment dependent, so we only assert it doesn't
        // panic and respects RAII: the returned option drops cleanly.
        let _ = AxElement::for_application(999_999_999);
    }
}
```

- [ ] **Step 2: Register the module**

Edit `crates/porthole-adapter-macos/src/lib.rs`. Add near the other module declarations:

```rust
pub mod ax;
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p porthole-adapter-macos --lib ax`
Expected: 2 passes.

- [ ] **Step 4: Build whole workspace**

Run: `cargo build --workspace --locked`
Expected: clean.

- [ ] **Step 5: Commit**

```bash
git add crates/porthole-adapter-macos/src/ax.rs crates/porthole-adapter-macos/src/lib.rs
git commit -m "feat(adapter-macos): add AxElement RAII wrapper for AX refs"
```

---

## Task 3: Migrate `close_focus.rs` to use `AxElement`

**Files:**
- Modify: `crates/porthole-adapter-macos/src/close_focus.rs`

Context: the existing module uses raw `AXUIElementRef` pointers threaded through closure helpers. Migrate to the new `AxElement` wrapper. Behaviour should be unchanged; the test suite (including the ignored real-macOS integration tests) should continue to pass.

The migration reduces the footprint of `unsafe` and the raw `unsafe extern "C"` block in this file — most of it moves to `ax.rs`.

- [ ] **Step 1: Read the existing file**

Use `Read` to examine `crates/porthole-adapter-macos/src/close_focus.rs` fully. Key landmarks:
- `unsafe extern "C"` block declaring `AXUIElementCreateApplication`, `AXUIElementCopyAttributeValue`, `AXUIElementPerformAction`, `AXUIElementSetAttributeValue`, `CFRelease`, `CFGetTypeID`, `_AXUIElementGetWindow` (some of these may have been dropped in earlier cleanups — verify current state).
- `with_first_window_for_pid` closure helper.
- `with_ax_window_by_cg_id` closure helper (added in slice A round-2 fixes).
- `focus`, `close`, `window_bounds` public functions.
- `bounds_from_ax` helper.

- [ ] **Step 2: Replace the raw FFI block**

Remove the `unsafe extern "C"` block from `close_focus.rs` (all of its function declarations are now in `ax.rs`). Remove local `AxElementRef` type alias. Update imports at the top:

```rust
use core_foundation::array::{CFArrayGetCount, CFArrayGetValueAtIndex, CFArrayRef};
use core_foundation::base::TCFType;
use core_foundation::string::{CFString, CFStringRef};
use porthole_core::adapter::Rect;
use porthole_core::surface::SurfaceInfo;
use porthole_core::{ErrorCode, PortholeError};

use crate::ax::{AxElement, AX_ERROR_SUCCESS};
```

(Adjust CF import names to match what the crate provides — the existing file already imports CFArray helpers, just keep those.)

- [ ] **Step 3: Rewrite `with_first_window_for_pid`**

Replace the existing helper with an `AxElement`-based version:

```rust
fn with_first_window_for_pid<F, R>(pid: i32, op: F) -> Result<R, PortholeError>
where
    F: FnOnce(AxElementRef) -> Result<R, PortholeError>,
{
    let app = AxElement::for_application(pid).ok_or_else(|| {
        PortholeError::new(ErrorCode::PermissionNeeded, "AXUIElementCreateApplication returned null")
    })?;
    let windows_ptr = app.copy_attribute_raw("AXWindows").ok_or_else(|| {
        PortholeError::new(ErrorCode::PermissionNeeded, "AXWindows read failed")
    })?;
    // windows_ptr is a CFArrayRef; we hold its retain.
    let arr = windows_ptr as CFArrayRef;
    let count = unsafe { CFArrayGetCount(arr) };
    let result = if count == 0 {
        Err(PortholeError::new(ErrorCode::SurfaceDead, "no AX windows found"))
    } else {
        let raw = unsafe { CFArrayGetValueAtIndex(arr, 0) } as AxElementRef;
        op(raw)
    };
    // Release the array we copied.
    unsafe { crate::ax::cf_release(windows_ptr) };
    result
}
```

This requires a small addition to `ax.rs` — either expose `CFRelease` wrapped as `pub(crate) fn cf_release(ptr: *const std::ffi::c_void)` or keep the raw symbol accessible via another path. Add that helper now:

Edit `crates/porthole-adapter-macos/src/ax.rs`, add below the AxElement impl:

```rust
/// Release a raw retained CF pointer. For use with copy-rule pointers
/// (e.g., attribute values copied via `AxElement::copy_attribute_raw`)
/// when they aren't wrapped in an owned type.
pub(crate) unsafe fn cf_release(ptr: *const std::ffi::c_void) {
    if !ptr.is_null() {
        unsafe { CFRelease(ptr) }
    }
}
```

(And ensure `CFRelease` is still in the extern block in ax.rs — it is, per Task 2.)

Also note: the signature takes `AxElementRef` (the type alias from ax.rs, which is the raw pointer type). The closure receives a *borrowed* raw pointer (still owned by `windows_ptr` until we release it). This is analogous to the current closure pattern.

- [ ] **Step 4: Rewrite `with_ax_window_by_cg_id`**

Similar pattern, using `AxElement` + iterating AXWindows with `_AXUIElementGetWindow` per element:

```rust
fn with_ax_window_by_cg_id<F, R>(pid: i32, target: u32, op: F) -> Result<R, PortholeError>
where
    F: FnOnce(AxElementRef) -> Result<R, PortholeError>,
{
    let app = AxElement::for_application(pid).ok_or_else(|| {
        PortholeError::new(ErrorCode::PermissionNeeded, "AXUIElementCreateApplication returned null")
    })?;
    let windows_ptr = app.copy_attribute_raw("AXWindows").ok_or_else(|| {
        PortholeError::new(ErrorCode::PermissionNeeded, "AXWindows read failed")
    })?;
    let arr = windows_ptr as CFArrayRef;
    let count = unsafe { CFArrayGetCount(arr) };

    let mut matched: Option<AxElementRef> = None;
    for i in 0..count {
        let raw = unsafe { CFArrayGetValueAtIndex(arr, i) } as AxElementRef;
        // Borrow the raw ptr into a temporary AxElement without taking
        // ownership by bumping the retain count first, or just call the
        // private API directly. Simpler: call _AXUIElementGetWindow
        // via AxElement-ish API, but it needs a &self. We cheat: use
        // a temporary that we don't let drop.
        let cg = unsafe { ax_get_window_id_borrowed(raw) };
        if cg == Some(target) {
            matched = Some(raw);
            break;
        }
    }

    let result = match matched {
        Some(raw) => op(raw),
        None => Err(PortholeError::new(
            ErrorCode::SurfaceDead,
            format!("window with cg_window_id {target} no longer exists for pid {pid}"),
        )),
    };
    unsafe { crate::ax::cf_release(windows_ptr) };
    result
}
```

This needs a small helper in `ax.rs`:

```rust
/// Call `_AXUIElementGetWindow` against a borrowed AX pointer without
/// taking ownership. Used when iterating AX arrays.
pub(crate) unsafe fn ax_get_window_id_borrowed(ptr: AxElementRef) -> Option<u32> {
    let mut id: u32 = 0;
    let err = unsafe { _AXUIElementGetWindow(ptr, &mut id) };
    if err == AX_ERROR_SUCCESS { Some(id) } else { None }
}
```

- [ ] **Step 5: Update `focus`, `close`, and `window_bounds`**

These already use the closure helpers — they should work unchanged as long as the helpers' contract is the same. Re-read them after the edits above and verify no raw extern calls remain in `close_focus.rs`.

One place that likely needs updating: `bounds_from_ax` or the inline `AXPosition`/`AXSize` reading. Check if it uses `AXUIElementCopyAttributeValue` directly; if so, route it through the new helpers.

- [ ] **Step 6: Verify the raw extern block is gone**

Run: `rg 'unsafe extern "C"' crates/porthole-adapter-macos/src/close_focus.rs`
Expected: no matches.

Run: `rg 'unsafe extern "C"' crates/porthole-adapter-macos/src/`
Expected: the remaining hits should be in `ax.rs` (the new home) and any other modules that legitimately need unique extern declarations (e.g., `permissions.rs` for `AXIsProcessTrusted`).

- [ ] **Step 7: Build + test**

Run:
```
cargo build --workspace --locked
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
```

Expected: all pass, no warnings.

- [ ] **Step 8: Commit**

```bash
git add crates/porthole-adapter-macos/src/ax.rs crates/porthole-adapter-macos/src/close_focus.rs
git commit -m "refactor(adapter-macos): close_focus uses AxElement RAII wrapper"
```

---

## Task 4: Deadline-aware wait with populated `last_observed`

**Files:**
- Modify: `crates/porthole-core/src/adapter.rs`
- Modify: `crates/porthole-core/src/in_memory.rs`
- Modify: `crates/porthole-core/src/wait_pipeline.rs`
- Modify: `crates/porthole-adapter-macos/src/wait.rs`

Context: today `Adapter::wait` has no deadline parameter. The pipeline wraps the call in `tokio::time::timeout`, which when it fires cancels the wait future. The pipeline then calls `adapter.wait_last_observed` separately — but that call has no state from the cancelled wait, so the macOS adapter returns placeholder `last_change_ms_ago: 0, last_change_pct: 0.0` for stable/dirty timeouts. The API contract promises real diagnostics and isn't delivering.

Fix: pass a deadline to `wait()`. The adapter is then responsible for returning before the deadline with a structured outcome — either the condition was satisfied (Ok), or it wasn't but here's what we observed (Err with diagnostics). The pipeline no longer wraps in `tokio::time::timeout`; the adapter owns the timing. The `wait_last_observed` separate trait method is removed.

- [ ] **Step 1: Change the trait signature**

Edit `crates/porthole-core/src/adapter.rs`. Find `async fn wait` and `async fn wait_last_observed`. Replace with a single method that takes a deadline:

```rust
    /// Wait until the condition is satisfied, or `deadline` passes.
    ///
    /// Returns:
    /// - `Ok(WaitOutcome)` if the condition was satisfied.
    /// - `Err(WaitTimeout { last_observed, elapsed_ms })` if the deadline
    ///   passed first. Adapter populates `last_observed` with whatever
    ///   state it tracked during polling.
    async fn wait(
        &self,
        surface: &SurfaceInfo,
        condition: &WaitCondition,
        deadline: std::time::Instant,
    ) -> Result<WaitOutcome, WaitTimeout>;
```

Define `WaitTimeout` in `crates/porthole-core/src/wait.rs` (this is a new type; add it):

```rust
use std::time::Instant;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WaitTimeout {
    pub last_observed: LastObserved,
    pub elapsed_ms: u64,
}
```

Import `WaitTimeout` in `adapter.rs` and use it.

Remove `async fn wait_last_observed` from the trait entirely.

- [ ] **Step 2: Update `InMemoryAdapter`**

Edit `crates/porthole-core/src/in_memory.rs`:

1. In the `Script` struct, remove `next_wait_last_observed`. Change `next_wait_result` type: it stays `Option<Result<WaitOutcome, PortholeError>>` since scripted success paths return WaitOutcome. For scripted timeout paths, add a new `next_wait_timeout: Option<WaitTimeout>` field.

Actually simpler: change `next_wait_result` type to `Option<Result<WaitOutcome, WaitTimeout>>` directly, matching the new signature. Adapt setters + recorders accordingly. Drop the `set_next_wait_last_observed` setter entirely.

2. Replace the `wait` impl:

```rust
    async fn wait(
        &self,
        surface: &SurfaceInfo,
        condition: &WaitCondition,
        _deadline: std::time::Instant,
    ) -> Result<WaitOutcome, WaitTimeout> {
        let mut s = self.script.lock().await;
        s.wait_calls.push((surface.id.clone(), condition.clone()));
        s.next_wait_result.take().unwrap_or_else(|| {
            Ok(WaitOutcome {
                condition: wait_condition_tag(condition).to_string(),
                elapsed_ms: 0,
            })
        })
    }
```

3. Drop the `wait_last_observed` method from the impl entirely.

4. Update `set_next_wait_result` setter signature to `Result<WaitOutcome, WaitTimeout>`.

5. For tests that previously called `set_next_wait_last_observed`, rewrite to script a `WaitTimeout` via the new `set_next_wait_result(Err(WaitTimeout {...}))` path.

- [ ] **Step 3: Update `WaitPipeline`**

Edit `crates/porthole-core/src/wait_pipeline.rs`. Remove `tokio::time::timeout` wrapping. Convert the single-method adapter return to the pipeline's existing `WaitPipelineError::Timeout` case:

```rust
    pub async fn wait(
        &self,
        surface: &SurfaceId,
        condition: &WaitCondition,
        timeout_duration: Duration,
    ) -> Result<WaitOutcome, WaitPipelineError> {
        validate_condition(condition)?;
        let info = self.handles.require_alive(surface).await.map_err(WaitPipelineError::Porthole)?;

        let deadline = std::time::Instant::now() + timeout_duration;
        match self.adapter.wait(&info, condition, deadline).await {
            Ok(outcome) => Ok(outcome),
            Err(wait_timeout) => Err(WaitPipelineError::Timeout(WaitTimeoutInfo {
                last_observed: wait_timeout.last_observed,
                elapsed_ms: wait_timeout.elapsed_ms,
            })),
        }
    }
```

Update any pipeline tests that previously relied on the `tokio::time::timeout` cancellation semantic. The `timeout_surfaces_last_observed` test (marked as a placeholder in slice A) can now be fixed to actually exercise the timeout — script the in-memory adapter to return `Err(WaitTimeout { ... })` and assert the pipeline converts correctly.

- [ ] **Step 4: Update macOS wait implementation**

Edit `crates/porthole-adapter-macos/src/wait.rs`. The new shape: each poll loop tracks its last-observed state and stops when `Instant::now() >= deadline`, returning `WaitTimeout` with real values.

Replace the existing signature:

```rust
pub async fn wait(
    surface: &SurfaceInfo,
    condition: &WaitCondition,
    deadline: Instant,
) -> Result<WaitOutcome, WaitTimeout> {
    let start = Instant::now();
    match condition {
        WaitCondition::Exists => {
            loop {
                if surface_is_alive(surface).map_err(|e| timeout_from_err(start, e))? {
                    return Ok(outcome("exists", start));
                }
                if Instant::now() >= deadline {
                    return Err(WaitTimeout {
                        last_observed: LastObserved::Presence {
                            alive: surface_is_alive(surface).unwrap_or(false),
                        },
                        elapsed_ms: start.elapsed().as_millis() as u64,
                    });
                }
                sleep(WAIT_SAMPLE_INTERVAL).await;
            }
        }
        // ... similar for Gone, TitleMatches ...
        WaitCondition::Stable { window_ms, threshold_pct } => {
            let mut last_fp = sample_fingerprint(surface).await.map_err(|e| timeout_from_err(start, e))?;
            let mut last_change_at = Instant::now();
            let mut last_change_pct: f64 = 0.0;
            loop {
                if Instant::now() >= deadline {
                    return Err(WaitTimeout {
                        last_observed: LastObserved::FrameChange {
                            last_change_ms_ago: last_change_at.elapsed().as_millis() as u64,
                            last_change_pct,
                        },
                        elapsed_ms: start.elapsed().as_millis() as u64,
                    });
                }
                sleep(WAIT_SAMPLE_INTERVAL).await;
                let fp = sample_fingerprint(surface).await.map_err(|e| timeout_from_err(start, e))?;
                let diff = fp.diff_pct(&last_fp);
                if diff > *threshold_pct {
                    last_change_at = Instant::now();
                    last_change_pct = diff;
                }
                last_fp = fp;
                if last_change_at.elapsed() >= Duration::from_millis(*window_ms) {
                    return Ok(outcome("stable", start));
                }
            }
        }
        WaitCondition::Dirty { threshold_pct } => {
            let initial = sample_fingerprint(surface).await.map_err(|e| timeout_from_err(start, e))?;
            let mut last_pct: f64 = 0.0;
            loop {
                if Instant::now() >= deadline {
                    return Err(WaitTimeout {
                        last_observed: LastObserved::FrameChange {
                            last_change_ms_ago: 0,
                            last_change_pct: last_pct,
                        },
                        elapsed_ms: start.elapsed().as_millis() as u64,
                    });
                }
                sleep(WAIT_SAMPLE_INTERVAL).await;
                let fp = sample_fingerprint(surface).await.map_err(|e| timeout_from_err(start, e))?;
                let diff = fp.diff_pct(&initial);
                last_pct = diff;
                if diff > *threshold_pct {
                    return Ok(outcome("dirty", start));
                }
            }
        }
        // Gone + TitleMatches: see below
    }
}
```

Apply the same deadline-check pattern to `Gone` (track `alive` at last sample) and `TitleMatches` (track the last observed title).

Helper:

```rust
fn timeout_from_err(start: Instant, err: PortholeError) -> WaitTimeout {
    // If our own sampling fails mid-wait, we surface the error as a
    // timeout with no useful diagnostics. In practice this happens only
    // on permission revocation or catastrophic OS errors.
    tracing::warn!(?err, "wait sampling failed; reporting as timeout");
    WaitTimeout {
        last_observed: LastObserved::FrameChange {
            last_change_ms_ago: 0,
            last_change_pct: 0.0,
        },
        elapsed_ms: start.elapsed().as_millis() as u64,
    }
}
```

Remove the `wait_last_observed` function from this module entirely.

- [ ] **Step 5: Update macOS adapter trait impl**

Edit `crates/porthole-adapter-macos/src/lib.rs`. Remove the `wait_last_observed` impl. Update the `wait` impl to match the new trait signature (pass through the deadline).

- [ ] **Step 6: Update any remaining consumers**

Run: `rg 'wait_last_observed' crates/` — expected to find only deletions. If any call sites remain, fix them. The pipeline is the only consumer; it was already updated in Step 3.

- [ ] **Step 7: Build + test**

Run:
```
cargo build --workspace --locked
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
```

Expected: all pass. The `timeout_surfaces_last_observed` test (previously a placeholder in slice A) should now actually exercise the timeout path; update its assertions to check `WaitTimeout::last_observed` values.

- [ ] **Step 8: Commit**

```bash
git add -A
git commit -m "fix(core,adapter-macos): deadline-aware wait with populated last_observed"
```

---

## Task 5: Workspace sanity + final gate

- [ ] **Step 1: Build**

```
cargo build --workspace --locked
```

Expected: clean.

- [ ] **Step 2: Tests**

```
cargo test --workspace --locked
```

Expected: 110+ non-ignored pass, 8 ignored.

- [ ] **Step 3: Clippy**

```
cargo clippy --workspace --all-targets --locked -- -D warnings
```

Expected: clean.

- [ ] **Step 4: Commit any straggler cleanup**

If clippy required minor touch-ups:

```bash
git add -A
git commit -m "chore: workspace cleanup after quality round"
```

Skip if nothing needed.

---

## What this round delivers

- End-to-end test that `porthole attach --containing-pid $$ --frontmost` actually works through the full CLI→daemon→adapter pipeline (using the in-memory adapter, so CI-safe).
- `AxElement` RAII wrapper now owns every AX reference in `close_focus.rs`. Future AX-heavy slices (presentation, events, tabs) have a safe foundation to build on.
- `wait` timeouts now carry real `last_observed` diagnostics. `stable`/`dirty` report the last observed change pct and time-since-change. `exists`/`gone` report the presence state at the deadline. `title_matches` reports the last seen title.
- `wait_last_observed` as a separate trait method is gone — the adapter's single `wait` call fully owns both success and timeout paths.
- Known-limitations doc updates: remove the "last_observed placeholder zeros" bullet.

## What this round intentionally does not deliver

- TTY-based multi-window disambiguation — still deferred.
- Command-at-launch for terminals — still deferred.
- Window size/geometry at launch — slice C.
- Events SSE, attach-based `recently_active_surface_ids` — separate slices.
