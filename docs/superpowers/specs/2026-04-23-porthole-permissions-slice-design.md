# Porthole System-Permissions Slice — Design

Date: 2026-04-23
Status: Draft for review (revised 2026-04-23 after first-round review)
Supersedes: nothing; extends v0 + slices A/B/C + polish round.

## 1. Purpose

Make porthole's dependency on macOS privacy permissions a first-class, robust part of the system. Today those permissions are granted-or-not by the user outside porthole; the system silently returns empty / null / wrong results on permission gaps in several places, the cold-start experience is "nothing works and no one knows why," and there is no documented development workflow for keeping grants alive across rebuilds.

The slice uses the term **system permissions** throughout — meaning permissions granted by the operating system (macOS TCC, future Linux/Windows equivalents). This is deliberately distinct from a future **agent permissions** layer that governs what subordinate agents are allowed to do (see §13 Out of scope). The two are different axes: system permissions are "what does the OS let this process do?" and agent permissions are "what does this agent let its workers do?" Don't conflate them in naming, wire fields, or error codes.

This slice ships:

- A single **onboarding CLI** (`porthole onboard`) that's the setup entrypoint for a new developer: triggers the OS permission prompts for everything porthole needs, prints status, flags restart requirements, and exits with a code that tells setup scripts whether a restart is pending. The verb is deliberately general so future slices can hang additional onboarding steps off it (agent-policy choices, recommended-app install, external-service auth) without a rename.
- A `remediation` block on every `system_permission_needed` error so agents can propose the exact fix rather than guess. Implemented via an extension to `PortholeError` so details actually propagate through the wire layer (see §6).
- A silent-degrade audit — one pass through the macOS adapter, plus preflight helpers at the top of every method that touches a guarded API. Preflight both triggers the OS prompt as a side effect *and* returns `system_permission_needed` with remediation, so a caller that skipped `onboard` still gets a dialog on the first blocked call.
- A development workflow: a `Portholed.app` bundle for grant stability across rebuilds, ad-hoc code signing, and a short playbook for first-time setup / re-grant / TCC reset.
- A guard against architectural retreat — a written rule (also in `AGENTS.md`) that agents encountering permission walls must stop and wait, not invent workarounds.

## 2. Agent behaviour when blocked

**Read this before implementing anything in this slice.**

When any task in this slice (or any future slice) hits a permission-dependent operation whose permission is not granted:

- **Do not** build a mock layer, a feature flag to skip the call, a "degrade to empty" path, or any other code-level workaround.
- **Do not** refactor the operation to "not need the permission" unless the user has explicitly approved that direction first.
- **Do** stop with `BLOCKED`, state the missing permission, provide the remediation command (`porthole onboard`), and wait.
- **Do** resume where you stopped after the user confirms the grant.

Waiting is cheaper than invention. Every workaround produces dead code that gets removed once permissions are right, while burning context and often papering over real issues downstream. The permission dependency is deliberate; porthole's job is desktop orchestration and macOS reserves these permissions precisely for that job.

This is codified in `AGENTS.md` and will be repeated in implementation plan task prompts for anything permission-adjacent.

## 3. Relationship to existing design

Additive, with a clarification pass over existing behaviour and one structural extension to the error type.

- **New endpoint**: `POST /system-permissions/request` — internal primitive that `porthole onboard` calls; public for symmetry with the rest of the wire but not the recommended agent interface.
- **Renamed `/info` field**: `permissions` → `system_permissions`. Pre-v1 wire change; applied consistently so the system vs agent distinction lands.
- **New CLI verb**: `porthole onboard`. Replaces the old plan's `ensure-permissions`; there is no separate `request-permission` verb.
- **Error code rename**: wire `permission_needed` → `system_permission_needed` (Rust variant `ErrorCode::PermissionNeeded` → `ErrorCode::SystemPermissionNeeded`). Pre-v1; clearer scope.
- **New error code**: `system_permission_request_failed`, returned when the adapter tries to trigger a prompt but the OS rejects the call (see §11).
- **`PortholeError` extension**: gains a `details: Option<serde_json::Value>` field, and `From<PortholeError> for WireError` carries it through. Route-level errors that wrap a `PortholeError` while adding their own details merge rather than overwrite (§6). This is the structural change that lets remediation actually reach the client.
- **Adapter trait**: gains a `request_system_permission_prompt` method, and the existing `permissions()` method renames to `system_permissions()` for internal consistency with the wire-side `system_permissions` field. The `PermissionStatus` Rust type renames to `SystemPermissionStatus` for the same reason. No other trait methods change.
- **Silent-degrade audit**: changes internal behaviour of some existing methods so they return `system_permission_needed` where they previously returned degraded results, and the preflight helpers additionally trigger the OS prompt as a side effect. Callers that previously got empty / null silently will now see a typed error and a dialog — strictly more informative.
- **Dev tooling**: adds `scripts/dev-bundle.sh` and `docs/development.md`. No code changes.

Items not in this slice (see §13 Out of scope):

- Agent permissions — the policy model for subordinate agents (crew scoping, guest-machine free-roam, default file-open handlers, etc.). The `onboard` verb is shaped to accommodate future steps, but none ship here.
- Linux / Windows permission models.
- Entitlements / signed-bundle-distribution for production releases.

## 4. New resources and endpoints

```
POST /system-permissions/request   — trigger the OS permission prompt for a named system permission
```

That's the only new endpoint. Reading state stays on `/info`. The wire shape is unchanged except for a field rename inside each adapter block (`permissions` → `system_permissions`):

```json
{
  "daemon_version": "...",
  "uptime_seconds": 0,
  "adapters": [
    {
      "name": "macos",
      "loaded": true,
      "capabilities": ["...", "system_permission_prompt"],
      "system_permissions": [
        { "name": "accessibility",    "granted": false, "purpose": "..." },
        { "name": "screen_recording", "granted": true,  "purpose": "..." }
      ]
    }
  ]
}
```

Capability additions: `"system_permission_prompt"` — declared by adapters that can trigger a prompt. macOS declares it; the in-memory adapter does not. The daemon checks this capability *before* dispatching a request to the adapter (see §5.1).

## 5. Prompt-triggering primitive and the `onboard` flow

### 5.1 The endpoint

`POST /system-permissions/request` body:

```json
{ "name": "accessibility" }
```

Request handling order is fixed:

1. **Capability check.** If the active adapter does not advertise `"system_permission_prompt"` in `capabilities()`, the route returns `capability_missing` (501) without touching the adapter. The in-memory adapter falls here.
2. **Name validation.** Otherwise, the adapter is called. The adapter validates `name` against its supported set (the names it returns from `system_permissions()`). Unknown names return `invalid_argument` with the supported-name list in `details`.
3. **Trigger.** On a valid name, the adapter calls the underlying OS API to open the prompt and returns the outcome.

This means a caller interacting with an adapter that can't prompt gets a single, specific error (`capability_missing`) rather than reaching name validation with an empty supported set. The name is a free-form string, matching the `SystemPermissionStatus` model already used on `/info`; there is no closed Rust enum.

### 5.2 Response

```json
{
  "permission": "accessibility",
  "granted_before": false,
  "granted_after": false,
  "prompt_triggered": true,
  "requires_daemon_restart": true,
  "notes": "Open System Settings → Privacy & Security → Accessibility and enable porthole. After granting, restart the daemon so the AX runtime initialises with the new trust state."
}
```

- `granted_before` / `granted_after` are read from the adapter's liveness check (`AXIsProcessTrusted()` / `CGPreflightScreenCaptureAccess()`) before and immediately after the prompt call. Because the user usually hasn't granted in that window, `granted_after == false` is the common case on first request.
- `prompt_triggered: true` means porthole asked the OS to show the prompt (via `AXIsProcessTrustedWithOptions({kAXTrustedCheckOptionPrompt: true})` or `CGRequestScreenCaptureAccess()`). On recent macOS the OS opens Settings directly; on older versions it shows a modal.
- `requires_daemon_restart`: true for Accessibility, false for Screen Recording.
  - **Why Accessibility is different.** `AXIsProcessTrusted()` itself is a live TCC query — after the user grants, the daemon's next call returns `true`, and `/info` reflects that immediately. What *doesn't* automatically recover is the internal AX runtime state: event taps (`CGEventTapCreate`), observers, and UI-element connections initialised when the daemon came up untrusted. Those stay in their old state until the process restarts. So the permission can show "granted" while AX-dependent operations still misbehave. `requires_daemon_restart` signals "trust is live, but restart the process for the AX runtime to pick it up."
  - Screen Recording has no equivalent init-time state; grants take effect on the next call.
- `notes` includes the restart instruction when `requires_daemon_restart` is true.

### 5.3 Polling vs. event

This slice does not block the connection waiting for the user to grant. The endpoint returns immediately after triggering the prompt. Callers check `/info` to observe the new grant state. Once SSE events ship, a `system_permission_changed` event will let callers subscribe rather than poll; not in this slice.

### 5.4 `porthole onboard`

This is the primary CLI surface and the intended user entrypoint.

Behaviour (default, blocking):

1. Read `system_permissions` from the daemon's `/info` to capture `granted_before` per advertised permission.
2. If all are already granted, print status and exit **0**.
3. For each ungranted permission, call `POST /system-permissions/request`. Interpret the response:
   - **Success:** print "dialog opened for X" if `prompt_triggered` is true; otherwise "prompt already fired earlier this daemon session — grant via Settings".
   - **`system_permission_request_failed`:** print the OS reason and the Settings path from the error's `details` (see §6.4). This permission cannot be prompted by the daemon in its current state; the user grants manually in Settings. Onboard's final exit code will be non-zero.
   - **Any other error:** print the code and message and treat the permission as un-promptable for this run.
4. Poll `/info` every 500 ms, printing live state changes, until either all permissions are granted or a 60-second timeout elapses (`--wait Ns` overrides).
5. Read `/info` one final time to capture `granted_after`.
6. Print a per-permission summary. For any permission that transitioned `ungranted → granted` this session *and* whose adapter reports `requires_daemon_restart = true`, flag "restart required" prominently.
7. Exit with:
   - **0** — all permissions were already granted at step 1 (no prompts fired, nothing to restart).
   - **2** — all permissions are granted as of step 5, and at least one permission that requires a daemon restart transitioned during this invocation. Setup scripts should restart the daemon before continuing.
   - **1** — at least one permission remains ungranted at step 5, or any `system_permission_request_failed` was observed at step 3. The user should grant in Settings (using the path printed in the summary) and re-run `porthole onboard`.

**Re-running after dismissal.** If the user dismissed the OS dialog, re-running `porthole onboard` will *not* re-open the dialog — macOS fires a prompt at most once per daemon process (§14). It will, however, still report current grant state and print the Settings path. Users can grant directly in Settings without a fresh dialog; the liveness check and `/info` will reflect the change. Re-arming the dialog is optional and requires restarting the daemon — onboard prints this as a hint when it detects a prompt was already fired.

Flags:

- `--wait Ns` — override the 60-second polling window. `--wait 0` disables polling (behaves like `--no-wait`).
- `--no-wait` — skip step 4 entirely; exit immediately after firing prompts with code **3** ("prompts triggered, awaiting grants"). For setup scripts that want to do other work in parallel with the user clicking through Settings.

Exit-code summary:

| Code | Meaning |
|---|---|
| 0 | All granted, no action pending |
| 1 | Some permissions still ungranted after waiting; user needs to grant and re-run |
| 2 | All granted, but daemon restart pending before AX-dependent calls work |
| 3 | `--no-wait`: prompts fired, grant state unknown |

The exit-code distinction matters because granting Accessibility while the daemon is running leaves the daemon in a state where `AXIsProcessTrusted()` reports true but some AX-dependent features may still misbehave until a restart; `onboard` is the surface that captures the transition and tells the user. A future launchctl-based lifecycle makes the restart automatic; exit 2 remains a useful signal either way.

Future slices will extend `onboard` with additional steps (agent-policy choices, recommended apps, external-service auth). For this slice, the only step is system-permission prompting.

## 6. Remediation and the error-details extension

### 6.1 Wire contract

Every `system_permission_needed` error returned from any endpoint includes a structured `remediation` block in `details`:

```json
{
  "code": "system_permission_needed",
  "message": "accessibility permission required for window inspection",
  "details": {
    "permission": "accessibility",
    "remediation": {
      "cli_command": "porthole onboard",
      "requires_daemon_restart": true,
      "settings_path": "System Settings → Privacy & Security → Accessibility",
      "binary_path": "/Users/.../target/debug/Portholed.app/Contents/MacOS/portholed"
    }
  }
}
```

- `binary_path` is the path the user must grant in Settings. On dev builds this is typically `target/debug/Portholed.app` (via the dev bundle). Resolved at runtime via `SecCodeCopyPath` or `CFBundleURLForApplication`.
- `cli_command` is always `porthole onboard` in this slice — the single setup entrypoint. Agents can surface it to users or run it themselves.
- `settings_path` is the human-navigable location in Settings for users not using the CLI.

### 6.2 Structural change to `PortholeError`

Today `PortholeError` has only `code` and `message`; `From<PortholeError> for WireError` sets `details: None`. That means anywhere preflight builds a `PortholeError` with remediation, the wire layer drops it.

Fix: extend `PortholeError`:

```rust
pub struct PortholeError {
    pub code: ErrorCode,
    pub message: String,
    pub details: Option<serde_json::Value>,
}
```

With a constructor that preserves the ergonomics of the existing call sites (`PortholeError::new(code, msg)` leaves `details: None`, and a new `with_details(json)` chainer adds them).

`From<PortholeError> for WireError` then carries `details` through directly. All existing error construction sites remain unchanged behaviourally (they produce `details: None`, same as today).

### 6.3 Merge rule for wrapping route errors

Some route-level errors wrap a `PortholeError` and add their own keys. The current `ReplacePipelineError::Porthole` conversion overwrites `details` with an object containing only `{"old_handle_alive": ...}` — which would clobber remediation if the wrapped error had it.

The rule, applied uniformly when a route-level error attaches additional details to a wrapped `PortholeError`:

1. If the wrapped `PortholeError.details` is `None`, the route's details object is the final `details`.
2. If it's `Some(Value::Object)`, merge at the top level: the route's keys are inserted into the wrapped object. On key collision, the route layer wins (it has broader context).
3. If it's `Some(non-object)`, that's a construction bug — none of our preflight helpers build non-object details — but log and fall through to rule (1) rather than crash.

`ApiError::from(ReplacePipelineError::Porthole { error, old_handle_alive })` will be rewritten to apply this rule (see the existing `ReturnedExisting` arm — same pattern, generalised).

### 6.4 Typed bodies for permission errors

To keep `WireError::details` JSON well-formed, define typed bodies at the protocol layer (same pattern as `LaunchReturnedExistingBody`, `CloseFailedBody`, `WaitTimeoutBody`):

```rust
// crates/porthole-protocol/src/error.rs

/// Body for `system_permission_needed`. The user can fix this by running
/// the CLI command or granting in Settings.
pub struct SystemPermissionNeededBody {
    pub permission: String,
    pub remediation: Remediation,
}

pub struct Remediation {
    pub cli_command: String,
    pub requires_daemon_restart: bool,
    pub settings_path: String,
    pub binary_path: String,
}

/// Body for `system_permission_request_failed`. The daemon cannot open the
/// prompt; the user must grant in Settings manually. No `cli_command` field
/// because `onboard` can't help either.
pub struct SystemPermissionRequestFailedBody {
    pub permission: String,
    pub reason: String,        // OS-level error text
    pub settings_path: String,
    pub binary_path: String,
}
```

The preflight helpers and the explicit endpoint both build one of these, serialize it into `PortholeError.details`, and every downstream conversion carries it through.

## 7. Silent-degrade audit

Today the macOS adapter has places where missing permissions produce degraded or empty results rather than `system_permission_needed`. The audit has two parts.

### 7.1 Light audit (one pass through the macOS adapter)

Find every call site that can return `Ok(empty)` / `Ok(None)` / `Ok(null_ptr_handled)` when a permission is missing. Confirmed candidates from manual inspection:

- `enumerate::list_windows()` — `CGWindowListCopyWindowInfo` returns records with empty titles when Screen Recording is missing. Enumeration still works; titles silently empty.
- `window_alive::window_alive()` — same, broader enumeration.
- `attention::attention()` — can return reduced info without Accessibility (no AX focus).
- `snapshot::snapshot_geometry()` — AX reads silently fail to null.
- `artifact::launch_artifact()` — AXDocument lookups fail silently.

All of these get preflight guards (§7.2) and return `system_permission_needed` when the required permission is missing.

### 7.2 Preflight everywhere, with prompt on miss

Add `ensure_accessibility_granted()` / `ensure_screen_recording_granted()` helpers in `permissions.rs`. Each one:

1. Checks the live liveness API (`AXIsProcessTrusted()` / `CGPreflightScreenCaptureAccess()`).
2. If granted: returns `Ok(())`.
3. If not granted: **triggers the OS prompt as a side effect** (`AXIsProcessTrustedWithOptions({prompt: true})` / `CGRequestScreenCaptureAccess()`). Two sub-cases for the return:
   - Prompt call succeeded (or was a no-op because a prompt already fired this process): return `Err(PortholeError)` with `code = system_permission_needed` and `details` populated via `SystemPermissionNeededBody` (§6.4).
   - Prompt call failed because the OS rejected it (rare; process not in a bundle / missing context): return `Err(PortholeError)` with `code = system_permission_request_failed` and `details.reason` set to the OS-level error. This surfaces a daemon-structural problem rather than asking the user to "just grant" something they can't be prompted for. The error is route-agnostic — the same code is returned by the explicit endpoint in §5 when it hits the same failure (§11).

This is the "safety-net" behaviour: if a caller skipped `porthole onboard` and hits a guarded method, the first blocked call pops the dialog and returns a remediation-carrying error. The OS only surfaces the prompt once per process lifetime — subsequent calls are prompt-silent but still return the typed error.

Each live check is sub-microsecond, so the overhead is negligible. The explicitness is worth the ~nanoseconds.

Call preflight at the top of every adapter method that depends on the corresponding permission. Trait method → permission mapping (drive from a table in `permissions.rs` to keep it honest):

| Adapter method | Requires |
|---|---|
| `launch_process`, `launch_artifact` | accessibility (for correlation) |
| `screenshot` | screen_recording |
| `key`, `text`, `click`, `scroll` | accessibility |
| `close`, `focus` | accessibility |
| `wait` (stable/dirty) | screen_recording + accessibility |
| `wait` (exists/gone/title_matches) | accessibility |
| `attention`, `displays` | (neither — CG basics work unprivileged; AX-based focus resolution degrades without accessibility and preflights accordingly) |
| `search`, `window_alive` | screen_recording |
| `place_surface`, `snapshot_geometry` | accessibility |

### 7.3 Fail loudly, uniformly

Some calls — `list_windows`, `search`, `window_alive` — could in principle return partial data (enumeration without titles) when Screen Recording is missing. We don't do that. Every guarded method returns `system_permission_needed` when its permission is missing.

Reasons: simpler contract for agents (no "did this return empty because there are no windows, or because I lack a permission?" ambiguity), matches the anti-workaround rule, aligns with the preflight-triggers-prompt UX (one clear failure mode, one clear dialog). A future slice can relax specific endpoints to partial-success mode with an explicit `warnings` field if a concrete use case emerges; no such use case drives this slice.

## 8. Development workflow

### 8.1 Dev bundle

Ship a `scripts/dev-bundle.sh` that:

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
4. Prints the path so the user can drag it into Settings or run `porthole onboard`.

The Info.plist uses a fixed bundle identifier like `org.flotilla.porthole.dev` so TCC tracks it stably. Real production releases would use a real identifier and real signing; this is just for dev.

Rebuild workflow: `cargo build` replaces `target/debug/portholed`; a separate `scripts/dev-bundle.sh --refresh` (or an equivalent cargo-xtask task) re-copies it into the bundle and re-signs. The bundle identity stays the same so TCC grant persists.

### 8.2 Playbook doc

Create `docs/development.md` with:

- First-time setup (build, bundle, start daemon, run `porthole onboard`, grant both permissions in Settings, restart daemon when prompted, re-run `onboard` to confirm).
- Running the daemon from the bundle (`open target/debug/Portholed.app` or direct `./target/debug/Portholed.app/Contents/MacOS/portholed`).
- What to do if grants get stuck: `tccutil reset Accessibility org.flotilla.porthole.dev`; same for `ScreenCapture`; rebundle and regrant.
- Debug vs. release bundle — they're separate TCC identities; grant both if switching frequently.
- How `cargo test -p porthole-adapter-macos -- --ignored` interacts with grants (tests use the same bundled daemon if you start it first, or spawn their own from `CARGO_BIN_EXE_portholed` which is a different path → needs its own grant, typically you test against the bundled one).

### 8.3 No production-signing work

Production-ready code signing (Apple Developer ID, notarization, entitlements) is a separate concern and out of scope. This slice ensures dev works smoothly; shipping a real release is a later project-level decision.

## 9. Startup warnings + info integration

When the daemon starts:

1. Check both permissions via the adapter's `system_permissions()` method (renamed from `permissions()` in this slice; same logic).
2. If either is missing, log at `WARN` level: `"accessibility system permission missing; calls that need it will return system_permission_needed. Run `porthole onboard` or see docs/development.md."`
3. Include the same info in every `/info` response (already shipped — grant state and purpose, under the renamed `system_permissions` field).
4. Do not block startup. The daemon runs fine without permissions; most endpoints simply error consistently.

`porthole info` already prints the permission grant state as of the polish round. This slice extends the printed output to include the remediation hint when a permission is missing:

```
adapter: macos (loaded=true) capabilities=...
  system permission accessibility: MISSING (input injection and some wait conditions)
    fix: porthole onboard  (will trigger the OS prompt; daemon restart required after grant)
  system permission screen_recording: granted (window screenshot capture and frame-diff waits)
```

## 10. Adapter trait additions

One new method on `Adapter`:

```rust
/// Trigger the OS prompt for the named system permission. Returns a structured
/// result with the grant state before/after and any restart requirement.
/// Calling this for a permission that's already granted is a no-op that
/// still returns the current state.
///
/// `name` is a string matching one of the names the adapter advertises via
/// `system_permissions()`. Unknown names return an `InvalidArgument` error
/// with the supported names in details.
async fn request_system_permission_prompt(
    &self,
    name: &str,
) -> Result<SystemPermissionPromptOutcome, PortholeError>;
```

Where `SystemPermissionPromptOutcome` is a struct matching the wire response shape in §5.2. No closed Rust enum for the permission name — we use `&str` to stay aligned with the `SystemPermissionStatus.name: String` model and to let each adapter declare its own supported set at runtime. Future Linux / Windows adapters add new permission names by returning them from `system_permissions()`; nothing in core needs to change.

The macOS adapter implements `request_system_permission_prompt` via `AXIsProcessTrustedWithOptions` / `CGRequestScreenCaptureAccess`. The in-memory adapter returns `AdapterUnsupported` — there is no OS prompt to trigger in a scripted context, and it does not advertise `"system_permission_prompt"` in its capability list.

The preflight helpers in §7.2 live alongside (not on the trait) — they're adapter-internal:

```rust
// In crates/porthole-adapter-macos/src/permissions.rs
pub fn ensure_accessibility_granted() -> Result<(), PortholeError>;
pub fn ensure_screen_recording_granted() -> Result<(), PortholeError>;
```

Both build the `PortholeError` with `code = system_permission_needed` and the full `SystemPermissionNeededBody` serialized into `details`. They trigger the OS prompt as a side effect when the permission is missing (§7.2).

## 11. Error model additions

One rename, one new code, and one extension:

- **Rename**: wire `permission_needed` → `system_permission_needed`, Rust `ErrorCode::PermissionNeeded` → `ErrorCode::SystemPermissionNeeded`. Same semantics; clearer scope.
- **New**: `system_permission_request_failed`. Returned whenever porthole tries to trigger a prompt and the OS rejects the call (rare; usually when the process isn't in a bundle or lacks the right context). Produced by both paths — the explicit `POST /system-permissions/request` endpoint and the preflight helpers that auto-prompt on miss. `details` includes `reason` with the OS-level error so callers can tell a structural daemon problem apart from an ungranted-but-promptable state.
- **Extension**: `PortholeError` gains `details: Option<serde_json::Value>` (§6.2), and `From<PortholeError> for WireError` carries it through. Route-level wrappers merge rather than overwrite (§6.3).

`invalid_argument` continues to cover unknown permission names passed to `request_system_permission_prompt`; its `details` include the adapter's supported-name list.

## 12. Testing strategy

### 12.1 Core / protocol

- `SystemPermissionPromptOutcome` serde roundtrip (snake_case wire).
- `SystemPermissionNeededBody` serde roundtrip.
- `SystemPermissionRequestFailedBody` serde roundtrip.
- `PortholeError::with_details` builder and `From<PortholeError> for WireError` carrying details through — table-tested.
- Route-wrapper merge rule (`ReplacePipelineError::Porthole`) — test that wrapped `system_permission_needed` details survive and gain the `old_handle_alive` key.

### 12.2 In-memory adapter

- `InMemoryAdapter::request_system_permission_prompt` returns `AdapterUnsupported` — one test. No scripted happy path.
- `InMemoryAdapter::capabilities()` does **not** include `"system_permission_prompt"` — one test.

### 12.3 Daemon route

- `POST /system-permissions/request` against the in-memory adapter returns `capability_missing` (501). In-memory doesn't advertise `"system_permission_prompt"`, so the route short-circuits before reaching name validation. This is the daemon-side happy path of the wiring — it proves the capability check is hooked up and ordered correctly.
- Unknown-name validation is tested against a cross-adapter harness or the macOS integration path (since in-memory never reaches step 2 of §5.1). The assertion: an adapter that *does* advertise the capability, given an unsupported name, returns `invalid_argument` with a supported-names list in `details`.
- `system_permission_needed` errors emitted from existing routes carry populated `details.remediation` — assert on two representative routes using scripted `ensure_*` helper stubs. Specifically: a screenshot call without Screen Recording, a key call without Accessibility.
- Route-wrapper merge: a `ReplacePipelineError::Porthole` wrapping a `system_permission_needed` produces a wire body containing both the remediation object and `old_handle_alive`.

### 12.4 macOS adapter (ignored, real desktop)

- `request_system_permission_prompt("accessibility")` — verifies the call returns without panicking. Cannot assert on the prompt appearing (no test-automatable way to see it), but can assert the `granted_before` / `granted_after` booleans match `AXIsProcessTrusted()` directly.
- `ensure_*_granted` preflight — gated real-desktop test confirming that a known-granted preflight returns `Ok`, and a known-missing preflight returns the expected `PortholeError` shape (code, message, populated `details`).

### 12.5 `porthole onboard` CLI

Drive these tests with an HTTP test double for `/info` and `/system-permissions/request` so the CLI's logic is exercised without a real daemon. Use a controllable clock / polling hook so timeout paths are fast.

- All-granted path: `/info` reports both granted on the first read. Exit code 0. No `request` calls made.
- Transition path (restart required): initial `/info` reports Accessibility ungranted; test double flips it to granted during the poll loop; `requires_daemon_restart = true` for Accessibility. Exit code 2 with restart message.
- Transition path (no restart required): same as above but only Screen Recording transitions. Exit code 0 (no AX transition → no restart needed).
- Still-ungranted path: test double keeps at least one permission ungranted through the full poll window. Exit code 1 with "grant in Settings and re-run" message.
- `system_permission_request_failed` path: test double returns this error from `/system-permissions/request`; onboard prints the OS reason and Settings path, exits 1.
- Prompt-already-fired path: test double returns success with `prompt_triggered: false`; onboard prints the "grant via Settings" hint and proceeds to polling.
- `--no-wait`: prompts fire, poll loop is skipped, exit code 3 regardless of `/info` state.
- Output formatting golden tests for each exit-code path.

### 12.6 Dev tooling

- `scripts/dev-bundle.sh` — a tiny shell test that runs the script in a tempdir, then `codesign -v` to verify the signature, then `./target/debug/Portholed.app/Contents/MacOS/portholed --help` to confirm the bundled binary is executable.
- `docs/development.md` — no programmatic test; it's prose for humans. Correctness is validated by following the playbook on a clean machine (see §15 success criterion).

## 13. Out of scope

- **Agent permissions.** The policy model for subordinate agents — crew-scoped process access, guest-machine free-roam, default file-open handlers, recommended-app install, external-service auth — is a future direction. The `porthole onboard` verb is deliberately general so these can hang off the same entrypoint later; none of them ship here.
- **Linux / Windows system permissions.** The adapter trait uses `&str` for permission names and each adapter declares its own supported set, so a Linux / Windows adapter can add permissions without touching core. No Linux / Windows adapter code in this slice.
- **Production signing / notarization / Developer ID.** This slice makes dev work; shipping a release-signed binary is a separate project decision.
- **`launchctl`-based daemon lifecycle management.** Starting the daemon as a user agent that restarts itself on grant changes is future tooling. Today the user manually restarts after an Accessibility grant; `onboard` exit code 2 signals when.
- **Permission state change events on SSE.** The `/events` slice adds a `system_permission_changed` event so callers can react without polling. Today callers poll `/info`.
- **Fine-grained authorization per caller.** No caller-identity model in this slice. The UDS is already per-user and trusts any connected caller.

## 14. Known limitations

- **Accessibility grants need a daemon restart for full effect.** `AXIsProcessTrusted()` itself is a live TCC query — the daemon sees the grant immediately, and `/info` reflects it without restart. What doesn't automatically recover is the internal AX runtime: event taps, observers, and UI-element connections initialised at process start when the daemon was untrusted. Those stay stale until the process restarts. `porthole onboard` captures the transition and exits with code 2 to signal the restart. A future launchctl-based lifecycle makes the restart automatic.
- **The CLI's `porthole onboard` triggers prompts for the daemon's binary, not the CLI's.** The CLI is just a client; the binary whose permission is being granted is `portholed`. The response and `info` output expose the binary path so it's clear.
- **TCC state can get stale.** Rarely, macOS's TCC database reports stale grants (usually after crashes or force-quits). The dev playbook (§8.2) covers `tccutil reset` as the recovery. No automatic detection in this slice.
- **Ad-hoc signing is not a real code signature.** The dev bundle is signed but not notarized and has no Developer ID. Fine for dev on the user's own machine; not distributable.
- **Prompt fires once per daemon process.** macOS shows the grant dialog only on the first `prompt: true` call for a given permission within a process. If the user dismisses it, subsequent prompt-triggering calls (from preflight or from `POST /system-permissions/request`) are silent no-ops for the dialog but still return current state. This is a UX limitation, not a blocker: users can grant directly in Settings (the path is in every remediation block), and the liveness check picks it up without a dialog. Re-arming the dialog requires restarting the daemon; `onboard` prints this as a hint when it observes a prompt has already fired this session.
- **Fail-loudly is a behaviour change for callers of `list_windows` / `search` / `window_alive`.** Previously these returned enumeration with empty titles when Screen Recording was missing; they now return `system_permission_needed`. Callers that depended on the empty-titles quirk must request the permission. There is no partial-success mode in this slice (§7.3).

## 15. Success criterion

A first-time developer on a clean Mac can run this and end up with a fully functioning porthole in under five minutes:

```sh
git clone https://github.com/flotilla-org/porthole
cd porthole
cargo build --workspace --release
./scripts/dev-bundle.sh --release
open -R target/release/Portholed.app    # finder reveals the bundle
./target/release/Portholed.app/Contents/MacOS/portholed &
./target/release/porthole onboard
# (daemon prints remediation; user grants both permissions in Settings)
# onboard exits with code 2: "Accessibility granted; restart the daemon before using AX-dependent features."
kill %1 && ./target/release/Portholed.app/Contents/MacOS/portholed &
./target/release/porthole onboard
# exits 0; everything granted and daemon is fresh
./target/release/porthole info
# shows both system_permissions granted
./target/release/porthole attach --containing-pid $$ --frontmost
# returns a tracked surface handle
```

No manual Info.plist editing. No mysterious silent failures. No agent inventing workarounds because something failed with an opaque error. Every wall has a remediation sign in front of it, and the setup path is one CLI verb.
