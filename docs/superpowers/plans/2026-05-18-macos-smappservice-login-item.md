# macOS SMAppService Login Item Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Register `Porthole.app` itself as the macOS per-user login item through `SMAppService.mainApp`, replacing the legacy CLI-installed LaunchAgent path for users running the helper app.

**Architecture:** Add a small Swift `LoginItemRegistrar` wrapper around `SMAppService.mainApp` with injectable dependencies for deterministic tests. Extend `HelperStartup` so startup runs legacy LaunchAgent migration, reports it, registers the app login item, reports it, then starts the daemon supervisor. Registration failures are surfaced through logs/status UI but do not block daemon startup.

**Tech Stack:** Swift 6, AppKit helper app, ServiceManagement `SMAppService`, XCTest, existing Rust CI gates.

---

## Scope Notes

Apple's ServiceManagement docs describe `SMAppService.mainApp` as the API for launching the main app at login. `SMAppService.agent(plistName:)` is for an embedded LaunchAgent plist, and `SMAppService.daemon(plistName:)` is for a system LaunchDaemon that can run before user login. Porthole's helper is a per-user menu-bar app that needs the logged-in user's TCC identity, so this slice should use `SMAppService.mainApp` and update the roadmap item text accordingly.

## Files

- Create: `apps/macos/PortholeHelper/Sources/PortholeHelper/LoginItemRegistrar.swift`
- Create: `apps/macos/PortholeHelper/Tests/PortholeHelperTests/LoginItemRegistrarTests.swift`
- Modify: `apps/macos/PortholeHelper/Sources/PortholeHelper/AppDelegate.swift`
- Modify: `apps/macos/PortholeHelper/Sources/PortholeHelper/LaunchAgentMigrator.swift`
- Modify: `apps/macos/PortholeHelper/Tests/PortholeHelperTests/LaunchAgentMigratorTests.swift`
- Modify: `docs/roadmap.md`

## Chunk 1: Pure Login Item Registration Logic

### Task 1: Registrar test-first implementation

**Files:**
- Create: `apps/macos/PortholeHelper/Tests/PortholeHelperTests/LoginItemRegistrarTests.swift`
- Create: `apps/macos/PortholeHelper/Sources/PortholeHelper/LoginItemRegistrar.swift`

- [x] **Step 1: Write failing tests**

Cover:
- enabled status returns `alreadyEnabled` without calling `register`
- requires-approval status returns `requiresApproval` without calling `register`
- not-registered status calls `register` and returns `registered`
- registration throw reports `failed`
- registration throw followed by requires-approval status reports `requiresApproval`

- [x] **Step 2: Verify red**

Run:

```bash
swift test --package-path apps/macos/PortholeHelper --filter LoginItemRegistrarTests
```

Expected: FAIL because `LoginItemRegistrar` does not exist.

- [x] **Step 3: Implement minimal registrar**

Add:
- `LoginItemRegistrar.ServiceStatus`
- `LoginItemRegistrar.RegistrationResult`
- injectable `Dependencies`
- `registerIfNeeded()`
- production dependency using `SMAppService.mainApp`

- [x] **Step 4: Verify green**

Run:

```bash
swift test --package-path apps/macos/PortholeHelper --filter LoginItemRegistrarTests
```

Expected: PASS.

## Chunk 2: Helper Startup Integration

### Task 2: Startup order and UI reporting

**Files:**
- Modify: `apps/macos/PortholeHelper/Sources/PortholeHelper/AppDelegate.swift`
- Modify: `apps/macos/PortholeHelper/Sources/PortholeHelper/LaunchAgentMigrator.swift`
- Modify: `apps/macos/PortholeHelper/Tests/PortholeHelperTests/LaunchAgentMigratorTests.swift`

- [x] **Step 1: Update startup test first**

Change `testStartupRunsMigrationBeforeSupervisorStart` to assert this order:

```swift
["migrate", "reportMigration", "registerLoginItem", "reportLoginItem", "start"]
```

- [x] **Step 2: Verify red**

Run:

```bash
swift test --package-path apps/macos/PortholeHelper --filter LaunchAgentMigratorTests/testStartupRunsMigrationBeforeSupervisorStart
```

Expected: FAIL because `HelperStartup` has no login-item hooks.

- [x] **Step 3: Extend startup and app delegate**

Add `registerLoginItem` and `reportLoginItem` closures to `HelperStartup`. In `AppDelegate.applicationDidFinishLaunching`, call `LoginItemRegistrar.mainApp().registerIfNeeded()` through that closure. Add a disabled status-menu row for login item state and report:
- enabled/already enabled
- registered
- needs approval in System Settings
- failed

- [x] **Step 4: Verify green**

Run:

```bash
swift test --package-path apps/macos/PortholeHelper --filter LaunchAgentMigratorTests/testStartupRunsMigrationBeforeSupervisorStart
swift test --package-path apps/macos/PortholeHelper
```

Expected: PASS.

## Chunk 3: Roadmap and Full Verification

### Task 3: Documentation and gates

**Files:**
- Modify: `docs/roadmap.md`

- [x] **Step 1: Update roadmap**

Mark the Phase 3 login-item row complete and correct the API wording from `SMAppService.daemon(plistName:)` to `SMAppService.mainApp`, noting that Porthole is a per-user helper app.

- [x] **Step 2: Run required gates**

Run:

```bash
swift test --package-path apps/macos/PortholeHelper --filter LoginItemRegistrarTests
swift test --package-path apps/macos/PortholeHelper
cargo build --workspace --locked
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo +nightly-2026-03-12 fmt --check
git diff --check
```

Expected: all commands exit 0.

- [x] **Step 3: Commit**

```bash
git add apps/macos/PortholeHelper docs/roadmap.md docs/superpowers/plans/2026-05-18-macos-smappservice-login-item.md
git commit -m "Add macOS SMAppService login item registration"
```
