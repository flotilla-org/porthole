# macOS Helper LaunchAgent Migration Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `PortholeHelper` retire the legacy per-user LaunchAgent before it starts supervising `portholed`, so helper-owned daemon lifecycle cannot compete with an old launchd-owned daemon.

**Architecture:** Add a small Swift `LaunchAgentMigrator` unit owned by the helper. It detects `~/Library/LaunchAgents/work.flotilla.porthole.plist`, best-effort runs `launchctl bootout gui/$UID <plist>`, removes the plist, and reports a typed result. `AppDelegate` invokes it before constructing `DaemonSupervisor`; failures are logged and surfaced in the status menu without inventing daemon workarounds.

**Tech Stack:** Swift 6 package under `apps/macos/PortholeHelper`, XCTest, macOS `Process`, Foundation file APIs.

---

## Chunk 1: Helper LaunchAgent Migrator

### Task 1: Add the migrator unit with tests

**Files:**
- Create: `apps/macos/PortholeHelper/Sources/PortholeHelper/LaunchAgentMigrator.swift`
- Create: `apps/macos/PortholeHelper/Tests/PortholeHelperTests/LaunchAgentMigratorTests.swift`

- [ ] **Step 1: Write failing tests for no-op, removal, and bootout failure**

Add tests covering:
- missing plist returns `.notNeeded` and does not call bootout
- existing plist calls bootout then removes the plist
- bootout failure leaves the plist in place and returns `.failed`

Use injected closures for `fileExists`, `removeFile`, `bootout`, and `homeDirectory` so tests do not touch the real LaunchAgents directory.

- [ ] **Step 2: Run tests to verify RED**

Run:

```sh
swift test --package-path apps/macos/PortholeHelper --filter LaunchAgentMigratorTests
```

Expected: FAIL because `LaunchAgentMigrator` does not exist.

- [ ] **Step 3: Implement minimal migrator**

Create:

```swift
struct LaunchAgentMigrator {
    enum Result: Equatable {
        case notNeeded
        case migrated(URL)
        case failed(URL, String)
    }

    static let launchAgentLabel = "work.flotilla.porthole"
    static let plistName = "work.flotilla.porthole.plist"

    func migrate() -> Result
}
```

Use default live dependencies:
- plist path: `FileManager.default.homeDirectoryForCurrentUser/Library/LaunchAgents/work.flotilla.porthole.plist`
- bootout command: `/bin/launchctl bootout gui/<uid> <plist-path>`
- remove command: `FileManager.default.removeItem(at:)`

- [ ] **Step 4: Run focused tests to verify GREEN**

Run:

```sh
swift test --package-path apps/macos/PortholeHelper --filter LaunchAgentMigratorTests
```

Expected: PASS.

### Task 2: Invoke migration before daemon supervision

**Files:**
- Modify: `apps/macos/PortholeHelper/Sources/PortholeHelper/AppDelegate.swift`
- Test: `apps/macos/PortholeHelper/Tests/PortholeHelperTests/LaunchAgentMigratorTests.swift`

- [ ] **Step 1: Add a small startup integration seam**

Add an injected `launchAgentMigrator` closure to `AppDelegate` or a helper method that can be unit-tested without launching an app.

- [ ] **Step 2: Write a failing test proving migration happens before daemon start**

Use a tiny coordinator/helper function if direct `NSApplicationDelegate` testing is awkward. The test should record call order:

```text
migrate -> startSupervisor
```

- [ ] **Step 3: Run the test to verify RED**

Run:

```sh
swift test --package-path apps/macos/PortholeHelper --filter LaunchAgentMigratorTests
```

Expected: FAIL until startup invokes the migrator before supervisor start.

- [ ] **Step 4: Implement startup invocation**

In `applicationDidFinishLaunching`, run migration after `installStatusItem()` and before `DaemonSupervisor(...)`.

Handle results:
- `.notNeeded`: no menu noise
- `.migrated`: log with `NSLog`
- `.failed`: log and set an informational status menu title such as `LaunchAgent migration failed; daemon starting`

- [ ] **Step 5: Run focused Swift tests**

Run:

```sh
swift test --package-path apps/macos/PortholeHelper --filter LaunchAgentMigratorTests
```

Expected: PASS.

## Chunk 2: Docs, Roadmap, and Full Verification

### Task 3: Update roadmap and docs

**Files:**
- Modify: `docs/roadmap.md`

- [ ] **Step 1: Mark the migration checkbox complete**

Update Phase 3 item:

```markdown
- [x] Migration: helper's first launch detects and removes any phase-1 LaunchAgent plist...
```

- [ ] **Step 2: Run formatting and focused tests**

Run:

```sh
swift test --package-path apps/macos/PortholeHelper
cargo +nightly-2026-03-12 fmt --check
```

- [ ] **Step 3: Run repo gates before PR**

Run:

```sh
cargo build --workspace --locked
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo +nightly-2026-03-12 fmt --check
git diff --check
```

- [ ] **Step 4: Commit and shepherd PR**

Commit:

```sh
git add apps/macos/PortholeHelper docs/roadmap.md docs/superpowers/plans/2026-05-18-macos-helper-launchagent-migration.md
git commit -m "feat: migrate legacy launchagent from helper"
```

Then push, open a PR, wait for CI/review, address valid feedback, and merge only when clean.
