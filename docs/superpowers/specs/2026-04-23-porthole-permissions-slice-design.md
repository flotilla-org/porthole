# Porthole Permissions Slice — Design

Date: 2026-04-23
Status: Draft for review
Supersedes: nothing; extends v0 + slices A/B/C + polish round.

## 1. Purpose

Make porthole's permission dependency a first-class, robust part of the system instead of a fragile implicit. Today permissions are granted-or-not by the user outside porthole; the system silently returns empty / null / wrong results on permission gaps in several places, the cold-start experience is "nothing works and no one knows why," and there is no documented development workflow for keeping grants alive across rebuilds.

This slice ships:

- An explicit request flow (`POST /permissions/request` + CLI verbs) that triggers the macOS permission prompt and opens the right Settings panel.
- A `remediation` field on every `permission_needed` error so agents can propose the exact fix rather than guess.
- A silent-degrade audit — light pass plus preflight checks at the top of every adapter method that touches a guarded API, so missing permissions fail loudly with a clear code rather than degrade to empty results.
- A development workflow: a `Portholed.app` bundle for grant stability across rebuilds, ad-hoc code signing, and a short playbook for first-time setup / re-grant / TCC reset.
- A guard against architectural retreat — a written rule (also in `AGENTS.md`) that agents encountering permission walls must stop and wait, not invent workarounds.

## 2. Agent behaviour when blocked

**Read this before implementing anything in this slice.**

When any task in this slice (or any future slice) hits a permission-dependent operation whose permission is not granted:

- **Do not** build a mock layer, a feature flag to skip the call, a "degrade to empty" path, or any other code-level workaround.
- **Do not** refactor the operation to "not need the permission" unless the user has explicitly approved that direction first.
- **Do** stop with `BLOCKED`, state the missing permission, provide the remediation command (`porthole request-permission <name>`), and wait.
- **Do** resume where you stopped after the user confirms the grant.

Waiting is cheaper than invention. Every workaround produces dead code that gets removed once permissions are right, while burning context and often papering over real issues downstream. The permission dependency is deliberate; porthole's job is desktop orchestration and macOS reserves these permissions precisely for that job.

This is codified in `AGENTS.md` and will be repeated in implementation plan task prompts for anything permission-adjacent.

## 3. Relationship to existing design

Additive, with one clarification pass over existing behaviour.

- **New endpoint**: `POST /permissions/request`.
- **New CLI verbs**: `porthole request-permission <name>`, `porthole ensure-permissions`.
- **Error body change**: every `permission_needed` response gains a `details.remediation` block. Existing callers who ignored `details` are unaffected; callers using `details` see the new field.
- **Adapter trait**: gains a `request_permission` method. Existing methods unchanged.
- **Silent-degrade audit**: changes internal behaviour of some existing methods so they return `permission_needed` where they previously returned degraded results. Callers that previously got empty / null silently will now see a typed error they can respond to — strictly more informative.
- **Dev tooling**: adds `scripts/dev-bundle.sh` and `docs/development.md`. No code changes.

Items not in this slice (see §13 Out of scope):

- Fine-grained permission policy for subordinate agents (future "machine-side model" concept the user referenced)
- Linux / Windows permission models
- Entitlements / signed-bundle-distribution for production releases

## 4. New resources and endpoints

```
POST /permissions/request   — trigger the OS permission prompt for a named permission
```

That's the only new endpoint. Reading state stays on `/info` (already shipped — returns `permissions: [{name, granted, purpose}]` per adapter).

`/info` capability additions: `"request_permission"` — declared by adapters that can trigger a prompt. macOS gets it; the in-memory adapter doesn't (it's scripted, not backed by an OS).

## 5. Request-permission flow

### 5.1 Request

`POST /permissions/request` body:

```json
{ "name": "accessibility" | "screen_recording" }
```

Unknown names return `invalid_argument` with the list of known names in the error details.

### 5.2 Response

```json
{
  "permission": "accessibility",
  "granted_before": false,
  "granted_after": false,
  "prompt_triggered": true,
  "requires_daemon_restart": true,
  "notes": "Open System Settings → Privacy & Security → Accessibility and enable porthole. After granting, run `launchctl kickstart` on the daemon or restart it manually for Accessibility changes to take effect."
}
```

- `granted_before` / `granted_after` are read from the adapter's liveness check (`AXIsProcessTrusted()` / `CGPreflightScreenCaptureAccess()`) before and immediately after the prompt call. Because the user may not have granted in that window, `granted_after == false` is the common case on first request.
- `prompt_triggered: true` means porthole asked the OS to show the prompt (via `AXIsProcessTrustedWithOptions({kAXTrustedCheckOptionPrompt: true})` or `CGRequestScreenCaptureAccess()`). On recent macOS the OS opens Settings directly; on older versions it shows a modal.
- `requires_daemon_restart`: true for Accessibility (macOS caches the trust state at process start), false for Screen Recording (takes effect on next call). `notes` includes the restart instruction when true.

### 5.3 Polling vs. event

This slice does not block the connection waiting for the user to grant. The endpoint returns immediately after triggering the prompt. Callers check `/info` to observe the new grant state. Once SSE events ship, a `permission_changed` event will let callers subscribe rather than poll; not in this slice.

### 5.4 CLI surface

- `porthole request-permission accessibility` — triggers the prompt, prints the response.
- `porthole request-permission screen_recording` — same.
- `porthole ensure-permissions` — checks both, triggers prompts for any ungranted, prints per-permission status and restart hints. Exits non-zero if any remain ungranted.

## 6. Remediation on error bodies

Every `permission_needed` error returned from any endpoint gains a structured `remediation` field in `details`:

```json
{
  "code": "permission_needed",
  "message": "accessibility permission required for window inspection",
  "details": {
    "permission": "accessibility",
    "remediation": {
      "cli_command": "porthole request-permission accessibility",
      "requires_daemon_restart": true,
      "settings_path": "System Settings → Privacy & Security → Accessibility",
      "binary_path": "/Users/.../target/debug/Portholed.app/Contents/MacOS/portholed"
    }
  }
}
```

- `binary_path` is the path the user must grant in Settings. On dev builds this is typically `target/debug/Portholed.app` (via the dev bundle). On production it would be the installed location. Resolved at runtime via `SecCodeCopyPath` or `CFBundleURLForApplication`.
- Agents can surface `cli_command` directly to users or run it themselves.
- `settings_path` is the human-navigable location in Settings for users not using the CLI.

Agents reading `details.remediation` can remediate without guessing. This is the wire-level contract that enforces "don't invent workarounds, propose the fix" at the agent layer.

## 7. Silent-degrade audit

Today the macOS adapter has places where missing permissions produce degraded or empty results rather than `permission_needed`. The audit has two parts.

### 7.1 Light audit (one pass through the macOS adapter)

Find every call site that can return `Ok(empty)` / `Ok(None)` / `Ok(null_ptr_handled)` when a permission is missing. Confirmed candidates from manual inspection:

- `enumerate::list_windows()` — `CGWindowListCopyWindowInfo` returns records with empty titles when Screen Recording is missing. Enumeration still works; titles silently empty. Should return `permission_needed` if screen recording is not granted (or, arguably, a warning in the response since enumeration still partially works — see §7.3).
- `window_alive::window_alive()` — same, broader enumeration.
- `attention::attention()` — can return reduced info without Accessibility (no AX focus).
- `snapshot::snapshot_geometry()` — AX reads silently fail to null.
- `artifact::launch_artifact()` — AXDocument lookups fail silently.

### 7.2 Preflight everywhere

Add `ensure_accessibility_granted()` / `ensure_screen_recording_granted()` helpers in `permissions.rs`. Each returns `Result<(), PortholeError>` with the full `permission_needed` error (including remediation) ready to propagate. Call at the top of every adapter method that depends on the corresponding permission.

Each call is sub-microsecond (`AXIsProcessTrusted()` / `CGPreflightScreenCaptureAccess()` are cheap) so the overhead is negligible. The explicitness is worth the ~nanoseconds.

Trait method → permission mapping (drive from a table in `permissions.rs` to keep it honest):

| Adapter method | Requires |
|---|---|
| `launch_process`, `launch_artifact` | accessibility (for correlation) |
| `screenshot` | screen_recording |
| `key`, `text`, `click`, `scroll` | accessibility |
| `close`, `focus` | accessibility |
| `wait` (stable/dirty) | screen_recording + accessibility |
| `wait` (exists/gone/title_matches) | accessibility |
| `attention`, `displays` | (neither — CG basics work unprivileged; but AX-based focus resolution degrades without accessibility) |
| `search`, `window_alive` | screen_recording for titles; the call otherwise succeeds |
| `place_surface`, `snapshot_geometry` | accessibility |

### 7.3 Partial-success cases

Some calls degrade partially rather than fail entirely. For example, `list_windows` returns useful enumeration without Screen Recording — just with empty titles. Two choices:

- **Fail loudly**: require the permission up front, return `permission_needed` even for the partial-success paths. Forces the caller to grant before getting any data.
- **Succeed with warning**: return the partial data with a `warnings` field in the response noting that titles are missing because of the permission gap.

I propose **fail loudly** for this slice. It's the simpler contract, matches the anti-workaround rule, and avoids a "did my search return empty because there are no windows or because I lack a permission?" ambiguity for agents. A future slice can relax specific endpoints to partial-success mode if use cases emerge.

## 8. Development workflow

### 8.1 Dev bundle

Shipping a `scripts/dev-bundle.sh` that:

1. Builds the workspace (debug by default, release with `--release`).
2. Creates `target/<profile>/Portholed.app/` with the standard bundle skeleton:
   ```
   Portholed.app/
     Contents/
       Info.plist       # bundle id, version, executable name
       MacOS/
         portholed      # the actual binary, copied from target/<profile>/portholed
   ```
3. Ad-hoc code-signs it: `codesign -s - --force --deep target/<profile>/Portholed.app`.
4. Prints the path so the user can drag it into Settings.

The Info.plist uses a fixed bundle identifier like `org.flotilla.porthole.dev` so TCC tracks it stably. Real production releases would use a real identifier and real signing; this is just for dev.

Rebuild workflow: `cargo build` replaces `target/debug/portholed`; a separate `scripts/dev-bundle.sh --refresh` (or an equivalent cargo-xtask task) re-copies it into the bundle and re-signs. The bundle identity stays the same so TCC grant persists.

### 8.2 Playbook doc

Create `docs/development.md` with:

- First-time setup (build, bundle, grant both permissions, restart daemon).
- Running the daemon from the bundle (`open target/debug/Portholed.app` or direct `./target/debug/Portholed.app/Contents/MacOS/portholed`).
- What to do if grants get stuck: `tccutil reset Accessibility org.flotilla.porthole.dev`; same for `ScreenCapture`; rebundle and regrant.
- Debug vs. release bundle — they're separate TCC identities; grant both if switching frequently.
- How `cargo test -p porthole-adapter-macos -- --ignored` interacts with grants (tests use the same bundled daemon if you start it first, or spawn their own from `CARGO_BIN_EXE_portholed` which is a different path → needs its own grant, typically you test against the bundled one).

### 8.3 No production-signing work

Production-ready code signing (Apple Developer ID, notarization, entitlements) is a separate concern and out of scope. This slice ensures dev works smoothly; shipping a real release is a later project-level decision.

## 9. Startup warnings + info integration

When the daemon starts:

1. Check both permissions via the adapter's `permissions()` method (already shipped).
2. If either is missing, log at `WARN` level: `"accessibility permission missing; calls that need it will return permission_needed. Run `porthole request-permission accessibility` or see docs/development.md."`
3. Include the same info in every `/info` response (already shipped — grant state and purpose).
4. Do not block startup. The daemon runs fine without permissions; most endpoints simply error consistently.

`porthole info` already prints the permission grant state as of the polish round. This slice extends the printed output to include the remediation hint when a permission is missing:

```
adapter: macos (loaded=true) capabilities=...
  permission accessibility: MISSING (input injection and some wait conditions)
    fix: porthole request-permission accessibility  (requires daemon restart after grant)
  permission screen_recording: granted (window screenshot capture and frame-diff waits)
```

## 10. Adapter trait additions

One new method on `Adapter`:

```rust
/// Trigger the OS prompt for the named permission. Returns a structured
/// result with the grant state before/after and any restart requirement.
/// Calling this for a permission that's already granted is a no-op that
/// still returns the current state.
async fn request_permission(&self, name: &PermissionName) -> Result<RequestPermissionOutcome, PortholeError>;
```

Where `PermissionName` is an enum (`Accessibility`, `ScreenRecording` for this slice; future adapters can extend) and `RequestPermissionOutcome` is a struct matching the wire response shape in §5.2.

The macOS adapter implements it via `AXIsProcessTrustedWithOptions` / `CGRequestScreenCaptureAccess`. The in-memory adapter returns `AdapterUnsupported` — there's no prompt to trigger in a scripted context, and agents depending on the in-memory adapter's behaviour shouldn't be calling request-permission anyway.

The preflight helpers in §7.2 live alongside (not on the trait) — they're adapter-internal:

```rust
// In crates/porthole-adapter-macos/src/permissions.rs
pub fn ensure_accessibility_granted() -> Result<(), PortholeError>;
pub fn ensure_screen_recording_granted() -> Result<(), PortholeError>;
```

Both build the `PortholeError` with the full `details.remediation` block populated (permission name, CLI command, restart requirement, settings path, binary path).

## 11. Error model additions

One new error code and one extended body:

- **`permission_request_failed`** — the adapter tried to trigger the prompt but the OS rejected the call (rare; usually happens when the process isn't in a bundle / doesn't have the right context). The body includes `details.reason` with the OS-level error.
- **`permission_needed`** (existing) — body gains `details.remediation` per §6. Existing callers reading only `code` / `message` are unaffected.

`invalid_argument` continues to cover unknown permission names passed to `request_permission`.

## 12. Testing strategy

### 12.1 Core / protocol

- `PermissionName` serde roundtrip (snake_case strings).
- `RequestPermissionOutcome` serde roundtrip.
- `ensure_accessibility_granted` / `ensure_screen_recording_granted` returning correctly-formed `PortholeError`s with populated remediation details — table-tested with scripted grant states via a shim.

### 12.2 In-memory adapter

- `InMemoryAdapter::request_permission` returns `AdapterUnsupported` — one test.
- Scripting hooks so core tests can simulate grant states. New field: `next_permission_state: HashMap<PermissionName, bool>`. Preflight helpers check this.

### 12.3 Daemon route

- `POST /permissions/request` happy path (via in-memory adapter's scripted state).
- Unknown permission name returns `invalid_argument` with known-names list.
- Every `permission_needed` error emitted from existing routes now has `details.remediation` populated — assert on at least two representative routes (screenshot without screen_recording, key without accessibility).

### 12.4 macOS adapter (ignored)

- `request_permission_accessibility` — verifies the call returns without panicking. Cannot assert on the prompt appearing (no test-automatable way to see it), but can assert the `granted_before` / `granted_after` booleans match `AXIsProcessTrusted()` directly.
- `ensure_*_granted` preflight — gated real-desktop test confirming that a known-granted permission preflight returns Ok and a known-missing one returns the expected error shape.

### 12.5 Dev tooling

- `scripts/dev-bundle.sh` — a tiny shell test that runs the script in a tempdir, then `codesign -v` to verify the signature, then `./target/debug/Portholed.app/Contents/MacOS/portholed --help` to confirm the bundled binary is executable.
- `docs/development.md` — no programmatic test; it's prose for humans. Correctness is validated by following the playbook on a clean machine (see §15 success criterion).

## 13. Out of scope

- **Policy model for subordinate agents** — the "machine-side curator that gives constrained agents blurred screenshots" concept is a future direction, not this slice.
- **Linux / Windows permission models** — the adapter trait accepts a `PermissionName` enum that future platform adapters can extend. No Linux / Windows adapter code here.
- **Production signing / notarization / Developer ID** — this slice makes dev work; shipping a release-signed binary is a separate project decision.
- **`launchctl`-based daemon lifecycle management** — starting the daemon as a user agent that restarts on grant changes is future tooling. Today the user manually restarts after an Accessibility grant.
- **Permission state change events on SSE** — the `/events` slice adds a `permission_changed` event so callers can react without polling. Today callers poll `/info`.
- **Fine-grained permissions per session tag or caller** — no authorization model in this slice. The UDS is already per-user and trusts any connected caller.

## 14. Known limitations

- **Accessibility grants require daemon restart.** macOS caches AX trust state at process-start time. After granting, the user must restart the daemon; the response and CLI output both call this out, but the restart is manual. A future launchctl-based lifecycle makes this automatic.
- **The CLI's `porthole request-permission` can only prompt for the daemon's binary, not the CLI's.** The CLI is just a client; the binary whose permission is being granted is `portholed`. The response and `info` output expose the binary path so it's clear.
- **TCC state can get stale.** Rarely, macOS's TCC database reports stale grants (usually after crashes or force-quits). The dev playbook (§8.2) covers `tccutil reset` as the recovery. No automatic detection in this slice.
- **Ad-hoc signing is not a real code signature.** The dev bundle is signed but not notarized and has no Developer ID. Fine for dev on the user's own machine; not distributable.
- **Partial-success paths are removed.** Calls that previously returned empty titles without Screen Recording now return `permission_needed`. This is intentional (§7.3) but represents a behaviour change — callers depending on the empty-titles quirk need to either request the permission or use `search` / `attach` which are permission-tolerant for their own enumeration paths.

## 15. Success criterion

A first-time developer on a clean Mac can run this and end up with a fully functioning porthole in under five minutes:

```sh
git clone https://github.com/flotilla-org/porthole
cd porthole
cargo build --workspace --release
./scripts/dev-bundle.sh --release
open -R target/release/Portholed.app    # finder reveals the bundle
./target/release/Portholed.app/Contents/MacOS/portholed &
./target/release/porthole ensure-permissions
# (daemon prints remediation, the user grants in Settings, restarts when prompted)
./target/release/porthole info
# shows both permissions granted
./target/release/porthole attach --containing-pid $$ --frontmost
# returns a tracked surface handle
```

No manual Info.plist editing. No mysterious silent failures. No agent inventing workarounds because something failed with an opaque error. Every wall has a remediation sign in front of it.
