# macOS Onboarding UI Design

Date: 2026-05-17
Status: approved

## 1. Purpose

Add the first native onboarding surface to `Porthole.app`. The user should be
able to open Porthole from the menu bar, see which macOS system permissions are
missing, trigger the same OS prompt path as `porthole onboard`, jump to the
right System Settings pane, restart the daemon when needed, and verify the final
state without leaving the helper UI.

This is the native equivalent of the existing CLI onboarding flow, not a new
permission model. The daemon remains the source of truth for permission state and
prompt behavior through `/info` and `/system-permissions/request`.

## 2. Current State

`Porthole.app` now contains:

```text
Contents/MacOS/PortholeHelper
Contents/MacOS/portholed
Contents/MacOS/porthole
```

`PortholeHelper` is an `LSUIElement` AppKit menu-bar app. It owns daemon
supervision, status rendering, restart, and quit. The daemon owns HTTP-over-UDS,
adapter state, permission truth, and prompt dispatch.

The CLI onboarding flow already handles important macOS TCC behavior:

1. Read missing permissions from `/info`.
2. Request one permission prompt at a time with
   `POST /system-permissions/request`.
3. Wait for the user to grant in System Settings.
4. Restart the daemon between grants so cached Accessibility and Screen
   Recording state is refreshed.
5. Re-read `/info` and verify before moving to the next missing permission.

The native UI must preserve these semantics. A static checklist that fires all
prompts at once would regress the TCC coalescing and stale-trust behavior the
CLI already solved.

## 3. Goals

- Add an `Open Onboarding...` action to the menu-bar menu.
- Show daemon readiness and loaded adapter permission state from `/info`.
- Show every advertised `system_permissions` item with granted/missing state and
  purpose text.
- Drive one active missing permission at a time.
- Trigger OS prompts by calling `POST /system-permissions/request`.
- Offer best-effort deep links to System Settings with a clear text fallback.
- Restart the helper-owned daemon and re-check `/info` after the user grants.
- Handle request errors and daemon-unavailable states without silently
  succeeding.
- Keep the UI thin: it presents daemon-owned state and calls daemon APIs; it
  does not duplicate permission policy.

## 4. Non-Goals

- No agent-permission approval UI.
- No notification surface.
- No `SMAppService` registration or login-item migration.
- No production notarization or hardened-runtime entitlements.
- No new daemon permission endpoints unless the existing API is insufficient.
- No mock permission bypasses or permission-free verification path.
- No live AX, input, screenshot, or capture smoke in automated tests. If live
  verification hits a missing Accessibility or Screen Recording grant, stop
  `BLOCKED` per `AGENTS.md`.

## 5. Approach Options

### Option A: Invoke the CLI from Swift

The helper can call the bundled `porthole` binary with `Process`.

This reuses Rust client code, but the existing CLI output is human text. A UI
would either parse fragile text or require new JSON CLI flags. Polling through a
subprocess is also a poor fit for UI state.

### Option B: Native Swift UDS Client

Add a small Swift client in `PortholeHelper` that speaks HTTP over the existing
Unix socket using `Network.framework`. It implements only what the helper needs:

- `GET /info`
- `POST /system-permissions/request`

This is the recommended path. It keeps the daemon protocol as the contract,
does not expand CLI surface area, and makes future helper UI slices reuse the
same local client.

### Option C: New Daemon Onboarding State Machine Endpoint

Add a daemon endpoint that owns the full onboarding state machine and lets the
helper subscribe or poll.

This might become useful once SSE/events exist, but it is too much policy for
this slice. The current daemon API already exposes the state and primitive prompt
action the UI needs.

## 6. Helper Client Design

Create a focused Swift client under `apps/macos/PortholeHelper`:

```text
Sources/PortholeHelper/
  PortholeClient.swift
  PortholeModels.swift
  PortholeSocketPath.swift
```

`PortholeSocketPath` mirrors the Rust runtime path resolution:

1. `$PORTHOLE_RUNTIME_DIR/porthole.sock`
2. `$XDG_RUNTIME_DIR/porthole/porthole.sock`
3. `$TMPDIR/porthole-<uid>/porthole.sock`
4. `/tmp/porthole-<uid>/porthole.sock`

The Swift implementation should normalize the final file URL with
`URL(fileURLWithPath:).standardized` so macOS `$TMPDIR` values with trailing
slashes do not produce surprising display strings or test snapshots.

`PortholeClient` uses `Network.framework` with a Unix-domain endpoint and writes
simple HTTP/1.1 requests. It should keep the implementation deliberately small:
one request at a time, a 1 MiB maximum response body, JSON body decode, and
clear errors for connection failure, non-2xx status, invalid HTTP, and invalid
JSON.

`PortholeModels` mirrors only the protocol shapes this UI needs:

```swift
struct InfoResponse: Decodable {
    let daemonVersion: String
    let uptimeSeconds: UInt64
    let adapters: [AdapterInfo]
}

struct AdapterInfo: Decodable {
    let name: String
    let loaded: Bool
    let capabilities: [String]
    let systemPermissions: [SystemPermissionStatus]
}

struct SystemPermissionStatus: Decodable, Identifiable {
    let name: String
    let granted: Bool
    let purpose: String
    var id: String { name }
}

struct SystemPermissionPromptOutcome: Decodable {
    let permission: String
    let grantedBefore: Bool
    let grantedAfter: Bool
    let requiresDaemonRestart: Bool
    let notes: String
}
```

Use `JSONDecoder.keyDecodingStrategy = .convertFromSnakeCase` rather than
duplicating serde field names in Swift.

## 7. Onboarding UI Design

Add an `OnboardingWindowController` and a small AppKit view hierarchy. This can
be AppKit-first for consistency with the status item. SwiftUI is not required
for this slice.

The window opens from a new menu item:

```text
Open Onboarding...
```

The first screen is the real tool, not an explanatory landing page:

- header: `Porthole Onboarding`
- daemon status: connected, disconnected, restarting, or error
- permission list: one row per advertised permission
- active action area for the first missing permission
- footer actions: Refresh, Restart Daemon, Close

Each permission row shows:

- display name (`Accessibility`, `Screen Recording`, fallback from raw name)
- purpose from `/info`
- state: Granted, Missing, Checking, Error
- last request/error note when available

For the active missing permission, show:

- `Request Permission` - calls `POST /system-permissions/request`
- `Open Settings` - opens the best-known Settings URL and leaves the readable
  Settings path visible
- `I Granted It` - restarts/verifies when a restart is needed, otherwise just
  re-reads `/info`

The UI should not hide raw failure reasons. If the daemon returns
`system_permission_request_failed`, show the wire message and any `reason`,
`settings_path`, and `binary_path` details.

## 8. Settings Deep Links

Deep links are best-effort convenience, not the source of truth. The UI should
always show the human-readable path from daemon notes or local fallback text.

Initial mapping:

```text
accessibility     -> x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility
screen_recording -> x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture
```

If `NSWorkspace.shared.open(url)` fails, the UI keeps the Settings path visible
and lets the user navigate manually. The design must not depend on the URL
remaining stable across macOS releases.

## 9. State Machine

Model onboarding as a helper-owned state machine over daemon-owned facts:

```text
loadingInfo
ready(info)
requesting(permission)
waitingForUser(permission, outcome)
restarting(permission)
verifying(permission)
complete(info)
blocked(error)
```

Flow:

1. On open, fetch `/info`.
2. If no adapters or no permissions are advertised, show the daemon response and
   no-op completion.
3. If all permissions are granted, show complete.
4. Pick the first ungranted permission in daemon order.
5. `Request Permission` posts `{ "name": permission }`.
6. Show prompt outcome notes and open Settings.
7. User clicks `I Granted It`.
8. If the outcome requires daemon restart, restart the helper-owned daemon and
   wait up to 10 seconds for `/info` to respond again. Use bounded polling with
   short exponential backoff, matching the CLI's default restart timeout shape.
9. Re-fetch `/info`.
10. If the permission is now granted, move to the next missing permission.
11. If still missing, keep that permission active and show a clear "still
    missing" state.

If the daemon does not respond before the restart timeout, transition to
`blocked(error)` with a "Daemon failed to restart" message and keep Restart
Daemon plus Refresh available.

The UI must not fire request calls for multiple missing permissions
back-to-back.

## 10. Daemon Restart Semantics

Normal installed-helper mode should have a helper-owned `portholed` child.
Restart means terminate that child, let `DaemonSupervisor` launch the next one,
then wait up to 10 seconds for `/info`. Timeout is not success; surface it as a
blocked restart failure and leave manual controls available.

If the helper is in `.runningExternal`, it does not own the daemon process and
must not pretend it can restart it. For this slice:

- show `External daemon detected`
- allow refresh and prompt requests
- when a restart is required, show a manual-restart instruction instead of
  silently succeeding

Later `SMAppService` and migration slices can make external-daemon ownership
rarer and improve this path.

## 11. Error Handling

The UI has three failure classes:

- **Daemon unavailable:** cannot connect to socket or `/info` fails. Show
  disconnected state and keep Restart Daemon available.
- **Prompt request failed:** daemon returned a wire error. Show message and
  structured details if present.
- **Still missing after verify:** user likely dismissed or did not enable the
  permission. Keep the same permission active and show Settings path.

The helper must never convert these to green success. Missing OS permissions are
expected setup blockers, not soft warnings.

## 12. Testing

Automated tests should cover logic and packaging, not real TCC prompts:

- Swift model decoding for `/info` and prompt outcomes.
- Swift socket-path resolution with injected environment values where possible.
- Swift onboarding reducer/state-machine tests using fake client responses.
- `swift build --package-path apps/macos/PortholeHelper --product PortholeHelper
  --scratch-path target/swift/PortholeHelper -c debug`.
- Existing `cargo xtask bundle --platform macos` smoke should still pass.
- Existing repo gates from `AGENTS.md` must pass.

The Swift build and bundle smoke are mandatory validation for the onboarding UI
implementation PR, but they are not added to the global `AGENTS.md` gate list in
this design-only slice.

Manual verification can open the helper and the onboarding window. It may click
through non-permission UI. If the flow needs real Accessibility or Screen
Recording grants and they are missing, stop `BLOCKED` and ask the user to grant
them. Do not add code-level workarounds.

## 13. Roadmap Update

After implementation and verification, tick only this Phase 3 item:

```text
Onboard UI flow - native equivalent of `porthole onboard`.
```

Do not tick notification approvals, `SMAppService`, LaunchAgent migration, or
external-daemon passive recovery in this slice.
