# Porthole Polish Round Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Five targeted fixes surfaced by manual testing after slice C merged: filter menu-bar noise from window enumeration, rename the CLI flag, expose permissions in `info`, clean up attention output, and fix display scale + screenshot metadata to be accurate on multi-monitor / Retina setups.

**Architecture:** No architectural change. Small localised edits to existing files; no new modules or wire contracts. The one wire-adjacent change is populating `LaunchResponse.placement_outcome`'s sibling `scale` field with real backing-scale values — this is a semantic fix, not a contract change (the field exists; values were wrong).

**Tech Stack:** Same as slice C. Adds one new objc2 call (`NSScreen.backingScaleFactor`) via the already-imported `objc2-app-kit`.

---

## Out of Scope

- **Process-launch correlation overhaul** (separate correlation-slice brainstorm, coming next)
- **Artifact correlation when handler is already frontmost** (same correlation slice)
- **Tab surfaces, events SSE, recording, browser-CDP** (ongoing deferrals)

The polish round is strictly mechanical cleanup + one correctness fix on display geometry.

---

## File Structure

```
crates/porthole-adapter-macos/
  src/
    enumerate.rs                  # modify: filter layer 0; expose layer in WindowRecord
    window_alive.rs               # modify: filter layer 0 in the broad enumeration
    display.rs                    # modify: compute scale via NSScreen.backingScaleFactor
    capture.rs                    # modify: emit real logical-points bounds + real scale
  src/
    nsscreen.rs                   # NEW: CGDirectDisplayID → NSScreen.backingScaleFactor lookup

crates/porthole/
  src/
    main.rs                       # modify: rename --app-or-path → --app
    commands/
      info.rs                     # modify: print permissions
      attention.rs                # modify: clean up output formatting

README.md                          # modify: update --app flag examples
```

---

## Task 1: Filter `CGWindowList` to layer 0

**Files:**
- Modify: `crates/porthole-adapter-macos/src/enumerate.rs`
- Modify: `crates/porthole-adapter-macos/src/window_alive.rs`

Context: `CGWindowList` returns all windows including menu-bar items (layer 25), dock (layer 20), Control Centre extras, notification center, etc. Real app windows are on layer 0. Manual testing showed 92 enumerated candidates of which only ~30 were real.

- [ ] **Step 1: Read the `kCGWindowLayer` field in `enumerate.rs`**

Edit `crates/porthole-adapter-macos/src/enumerate.rs`. In the per-window parse loop, add a layer read alongside the existing owner_pid/cg_window_id reads. The constant is `kCGWindowLayer` in `core_graphics::window`. Skip any record where `layer != 0`.

Concrete change inside the CFArray iteration:

```rust
let layer = dict
    .find(unsafe { CFString::wrap_under_get_rule(kCGWindowLayer) })
    .and_then(|v| v.downcast::<CFNumber>().and_then(|n| n.to_i32()))
    .unwrap_or(0);
if layer != 0 {
    continue;
}
```

Add `kCGWindowLayer` to the existing `core_graphics::window::{...}` import list.

- [ ] **Step 2: Apply the same filter in `window_alive.rs`**

Edit `crates/porthole-adapter-macos/src/window_alive.rs`. Same pattern — read layer and skip non-zero. (`window_alive` uses the broader enumeration including off-screen windows; a menu-bar item that's logically off-screen shouldn't be eligible as a tracked surface either.)

- [ ] **Step 3: Manual smoke**

Start the daemon and run `porthole search | wc -l` before and after. Expect ~30 real windows instead of 92. Real test:

```sh
./target/release/porthole search | awk -F'app=' '{print $2}' | awk -F'  title=' '{print $1}' | sort -u
```

Should list real app names only — no `Control Centre`, no `Dock`, no `WindowManager`, no `Window Server`.

- [ ] **Step 4: Run existing tests**

```
cargo test --workspace --locked
```

Existing tests should still pass. The in-memory adapter tests don't exercise the layer filter; macOS integration tests are `#[ignore]`'d.

- [ ] **Step 5: Commit**

```bash
git add crates/porthole-adapter-macos/src/enumerate.rs crates/porthole-adapter-macos/src/window_alive.rs
git commit -m "fix(adapter-macos): filter CGWindowList to layer 0 (exclude menu bar, dock, etc.)"
```

---

## Task 2: Rename CLI `--app-or-path` → `--app`

**Files:**
- Modify: `crates/porthole/src/main.rs`
- Modify: `README.md`

Context: the `--app-or-path` flag is clunky and doesn't help the user — the semantic "either a bundle path, an executable, or a file path" is better conveyed in the help text than crammed into the flag name. The README and recipes all reach for `--app` anyway.

- [ ] **Step 1: Rename in `main.rs`**

Edit `crates/porthole/src/main.rs`. In both the `Launch` and `Replace` command variants, change `#[arg(long)] pub app_or_path: String` → `#[arg(long = "app")] pub app: String`. Update the field name in the match arm destructuring (`app_or_path` → `app`) and downstream uses in the same arms.

Update the doc comment on the field to: `/// For process launches: an app bundle path (.app) or executable path. For artifact launches: a file path.`

- [ ] **Step 2: Update README**

Find every `--app-or-path` in `README.md` and replace with `--app`. `rg` to confirm:

```sh
rg 'app-or-path' README.md
```

Should come back empty after the fix.

- [ ] **Step 3: Build + manual smoke**

```
cargo build -p porthole
./target/release/porthole launch --help | grep -A2 app
```

Expect `--app <APP>` in the help, with the updated doc comment.

- [ ] **Step 4: Run tests**

```
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
```

- [ ] **Step 5: Commit**

```bash
git add crates/porthole/src/main.rs README.md
git commit -m "chore(cli): rename --app-or-path to --app; update README"
```

---

## Task 3: `porthole info` prints permissions

**Files:**
- Modify: `crates/porthole/src/commands/info.rs`

Context: the wire `/info` response carries `permissions: [{name, granted, purpose}]` per adapter (added in slice A). The CLI's info subcommand skips this field, so users have to curl the raw JSON to know whether Accessibility / Screen Recording are granted.

- [ ] **Step 1: Extend the info output**

Edit `crates/porthole/src/commands/info.rs`. After the adapter capabilities line, add a permissions block:

```rust
        for adapter in info.adapters {
            println!(
                "adapter: {} (loaded={}) capabilities={}",
                adapter.name,
                adapter.loaded,
                adapter.capabilities.join(","),
            );
            if !adapter.permissions.is_empty() {
                for perm in &adapter.permissions {
                    println!(
                        "  permission {}: {} ({})",
                        perm.name,
                        if perm.granted { "granted" } else { "MISSING" },
                        perm.purpose,
                    );
                }
            }
        }
```

Use `MISSING` in uppercase so a quick visual scan on an ungranted state stands out.

- [ ] **Step 2: Manual smoke**

```
./target/release/portholed &
./target/release/porthole info
```

Expect output like:
```
daemon_version: 0.0.0
uptime_seconds: 3
adapter: macos (loaded=true) capabilities=...
  permission accessibility: granted (input injection and some wait conditions)
  permission screen_recording: granted (window screenshot capture and frame-diff waits)
```

Kill the daemon.

- [ ] **Step 3: Commit**

```bash
git add crates/porthole/src/commands/info.rs
git commit -m "feat(cli): porthole info prints permission grant state"
```

---

## Task 4: Clean up `porthole attention` output

**Files:**
- Modify: `crates/porthole/src/commands/attention.rs`

Context: current output prints Rust Debug format — `DisplayId("disp_3")`, `Some("cmux")`, `None`. Should be human-readable: `disp_3`, `cmux`, `(none)`.

- [ ] **Step 1: Rewrite the display block**

Edit `crates/porthole/src/commands/attention.rs`. Current shape is roughly:

```rust
    println!("focused_surface_id: {:?}", info.focused_surface_id);
    println!("focused_app_name: {:?}", info.focused_app_name);
    println!("focused_display_id: {:?}", info.focused_display_id);
    println!("cursor: ({}, {}) display_id={:?}", info.cursor.x, info.cursor.y, info.cursor.display_id);
    println!("recently_active: {:?}", info.recently_active_surface_ids);
```

Replace with helpers that strip wrappers and Option-None prettily:

```rust
fn fmt_surface_id(v: &Option<porthole_core::surface::SurfaceId>) -> String {
    match v {
        Some(id) => id.as_str().to_string(),
        None => "(none)".to_string(),
    }
}

fn fmt_display_id(v: &Option<porthole_core::display::DisplayId>) -> String {
    match v {
        Some(id) => id.as_str().to_string(),
        None => "(none)".to_string(),
    }
}

fn fmt_opt_str(v: &Option<String>) -> String {
    v.as_deref().unwrap_or("(none)").to_string()
}
```

Use in:

```rust
    println!("focused_surface_id: {}", fmt_surface_id(&info.focused_surface_id));
    println!("focused_app_name: {}", fmt_opt_str(&info.focused_app_name));
    println!("focused_display_id: {}", fmt_display_id(&info.focused_display_id));
    println!(
        "cursor: ({:.1}, {:.1}) display_id={}",
        info.cursor.x, info.cursor.y, fmt_display_id(&info.cursor.display_id),
    );
    if info.recently_active_surface_ids.is_empty() {
        println!("recently_active: (none)");
    } else {
        let ids: Vec<String> = info.recently_active_surface_ids.iter().map(|s| s.as_str().to_string()).collect();
        println!("recently_active: {}", ids.join(", "));
    }
```

- [ ] **Step 2: Manual smoke**

```
./target/release/portholed &
./target/release/porthole attention
```

Expect:
```
focused_surface_id: (none)
focused_app_name: cmux
focused_display_id: disp_3
cursor: (149.2, 436.3) display_id=disp_3
recently_active: (none)
```

Kill the daemon.

- [ ] **Step 3: Commit**

```bash
git add crates/porthole/src/commands/attention.rs
git commit -m "chore(cli): human-readable attention output"
```

---

## Task 5: Fix display scale + screenshot metadata

**Files:**
- Create: `crates/porthole-adapter-macos/src/nsscreen.rs`
- Modify: `crates/porthole-adapter-macos/src/lib.rs`
- Modify: `crates/porthole-adapter-macos/src/display.rs`
- Modify: `crates/porthole-adapter-macos/src/capture.rs`

Context: **this is the substantive fix in the polish round.** Today on a mixed display setup (Retina + 4K + ultrawide):
- `displays::displays()` computes `scale = pixels_wide / bounds.width`, but `CGDisplayPixelsWide` returns logical points on recent macOS, not backing pixels. Scale comes out ≈1 for every display.
- `capture::screenshot()` sets `window_bounds_points` to the PNG's pixel dimensions and `scale: 1.0` hardcoded. The "points" value is actually pixels, and the scale is always wrong.

Together this means a caller doing multi-monitor coordinate work gets wrong conversions between pixel and logical coordinates.

Fix:

1. Read the actual backing scale from `NSScreen.backingScaleFactor` — directly returns the 2.0 / 1.0 / etc. factor we want.
2. Make `window_bounds_points` actually logical points (from AX position/size on the window, same as `snapshot_geometry` already does).
3. Report the correct per-window scale in the screenshot response (the scale of the display the window is on).

- [ ] **Step 1: Add `nsscreen.rs` helper**

Create `crates/porthole-adapter-macos/src/nsscreen.rs`:

```rust
//! CGDirectDisplayID → NSScreen.backingScaleFactor lookup.
//!
//! `CGDisplayPixelsWide` on modern macOS returns the logical width of the
//! active display mode (i.e., points), not the backing pixel count, so it
//! cannot be used to compute the backing scale factor. `NSScreen.back-
//! ingScaleFactor` is the authoritative source — this module bridges the
//! CGDirectDisplayID we have to the matching `NSScreen` and reads it.

#![cfg(target_os = "macos")]

use objc2::msg_send;
use objc2::rc::Retained;
use objc2_app_kit::NSScreen;
use objc2_foundation::{NSNumber, NSString};

/// Look up the backing scale factor for a display. Returns 1.0 if the
/// screen can't be found (e.g., just disconnected between our display
/// enumeration and this call).
pub fn backing_scale_factor_for(display_id: u32) -> f64 {
    unsafe {
        let screens: Retained<objc2_foundation::NSArray<NSScreen>> = NSScreen::screens();
        let count = screens.count();
        for i in 0..count {
            let screen = screens.objectAtIndex(i);
            let device_description: Retained<objc2_foundation::NSDictionary<NSString, objc2::runtime::AnyObject>> =
                msg_send![&*screen, deviceDescription];
            let screen_number_key = NSString::from_str("NSScreenNumber");
            let value: Option<Retained<objc2::runtime::AnyObject>> =
                msg_send![&*device_description, objectForKey: &*screen_number_key];
            let Some(value) = value else { continue };
            let number: Retained<NSNumber> = Retained::cast(value);
            let this_id: u32 = number.unsignedIntValue();
            if this_id == display_id {
                let factor: f64 = msg_send![&*screen, backingScaleFactor];
                return factor;
            }
        }
        1.0
    }
}
```

Note: the exact message-send / retained-cast patterns depend on the objc2 version in use. The above targets objc2 0.5 + objc2-app-kit. If the `Retained::cast` call doesn't compile cleanly, use `objc2::ffi::NSObject` casts or the `downcast_ref` variants. The test harness does `cargo build -p porthole-adapter-macos` after this step and iterates.

- [ ] **Step 2: Register module**

Edit `crates/porthole-adapter-macos/src/lib.rs`. Add:

```rust
pub mod nsscreen;
```

- [ ] **Step 3: Use it in `display.rs`**

Edit `crates/porthole-adapter-macos/src/display.rs`. Replace the scale computation:

```rust
use crate::nsscreen::backing_scale_factor_for;

// ... inside the per-display loop ...

let scale = backing_scale_factor_for(id);  // real backing scale
```

Remove the old `pixels_wide / bounds.size.width` computation and the now-unused `pixels_wide` / `pixels_high` reads.

- [ ] **Step 4: Fix `capture.rs`**

Edit `crates/porthole-adapter-macos/src/capture.rs`. Two semantic changes:

a. Compute `window_bounds_points` from the window's AX position/size, not from the PNG dimensions. Use the existing `snapshot_geometry` pathway or read AX directly (it's used in several places already).

b. Compute `scale` from the display the window is on. Use `snapshot_geometry` which already returns `{display_id, display_local}`, combined with `displays()` to find the matching display, and read `scale` from it. Alternatively call `backing_scale_factor_for(raw_id)` directly with the CGDirectDisplayID you resolved — slightly less code.

Concrete shape inside `screenshot()`:

```rust
// ... after decoding the CGImage bytes into PNG ...

let snap = crate::snapshot::snapshot_geometry(surface).await;
let (window_bounds_points, scale) = match snap {
    Ok(snap) => {
        // Look up the backing scale for the display the window is on.
        let display_id_str = snap.display_id.as_str();
        // The macOS id encoding is "disp_<cgid>".
        let cg_id: u32 = display_id_str
            .strip_prefix("disp_")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        let scale = if cg_id != 0 {
            crate::nsscreen::backing_scale_factor_for(cg_id)
        } else {
            1.0
        };
        (snap.display_local, scale)
    }
    Err(_) => {
        // Fall back: report pixel dimensions with scale 1 and make a note.
        // Better than failing the whole screenshot — the PNG is still useful.
        tracing::warn!("snapshot_geometry failed during screenshot; reporting pixel bounds with scale 1");
        (
            Rect { x: 0.0, y: 0.0, w: width as f64, h: height as f64 },
            1.0,
        )
    }
};
```

Return `window_bounds_points` (now genuine logical points) and `scale` (now real) in the Screenshot struct.

- [ ] **Step 5: Update the `Screenshot` type if necessary**

The existing `Screenshot` type in `crates/porthole-core/src/adapter.rs` has a `scale: f64` field — used. `window_bounds_points` already has the correct name. No core/protocol change; the fix is entirely in the adapter's computation.

- [ ] **Step 6: Manual verification**

The real test for this fix is running it against your three-display setup. After the build:

```
./target/release/portholed &
./target/release/porthole displays
```

Expect three lines with appropriate scale factors. The Retina should be `scale=2`, the 4K likely `scale=1` (or possibly `2` if configured for HiDPI), the ultrawide `scale=1`.

```
# Attach to a window on the Retina display, screenshot it.
SURFACE=$(./target/release/porthole attach --containing-pid $$ --frontmost --json | jq -r .surface_id)
./target/release/porthole screenshot $SURFACE --out /tmp/screen.png
```

Check the output: `window_bounds` should be the AX bounds (e.g. `1200x800 at 400,100`) and `scale` should be `2` on a Retina display. `file /tmp/screen.png` should show pixel dimensions ≈ 2× the bounds.

Repeat the attach + screenshot on a window on the ultrawide: `scale: 1`, PNG dimensions match bounds.

- [ ] **Step 7: Run tests**

```
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
```

Existing tests continue to pass (in-memory adapter paths; macOS integration stays ignored).

- [ ] **Step 8: Commit**

```bash
git add crates/porthole-adapter-macos/src/nsscreen.rs crates/porthole-adapter-macos/src/lib.rs crates/porthole-adapter-macos/src/display.rs crates/porthole-adapter-macos/src/capture.rs
git commit -m "fix(adapter-macos): real per-display backing scale; screenshot reports logical bounds"
```

---

## Task 6: Workspace sanity + docs note

- [ ] **Step 1: Final checks**

```
cargo build --workspace --locked
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
git diff --check main...HEAD
```

All clean. No new warnings, no trailing whitespace.

- [ ] **Step 2: Update README with any wording affected by the fixes**

Check:
- `rg '--app-or-path' README.md` → empty after Task 2.
- `README.md`'s permissions section references grant states; mention that `porthole info` now surfaces them.
- If the README mentions `scale: 1` or similar in any example, update to reflect that it's now accurate.

- [ ] **Step 3: Commit any doc-only fixups**

```bash
git add README.md
git commit -m "docs: README tweaks for polish-round changes"
```

Skip this commit if nothing changed.

---

## What this round delivers

- `porthole search` returns only real app windows (layer 0) — from ~92 noisy candidates down to ~30 useful ones
- CLI flag is the natural `--app` across `launch` and `replace`
- `porthole info` tells you whether Accessibility / Screen Recording are granted
- `porthole attention` prints clean, human-readable output
- Per-display backing scale is now accurate; screenshot metadata reports real logical bounds + real scale, so mixed multi-monitor (Retina + 4K + ultrawide) coordinate math works

## What this round intentionally does not deliver

- Correlation overhaul for `launch --kind process` (tag correlation, broken on modern macOS because `ps eww` hides env vars). Separate brainstorm + slice.
- Correlation for artifact launches when the handler app is already frontmost with the same doc open. Same separate slice.
- Tab surfaces, events SSE, recording, browser-CDP — still deferred.
