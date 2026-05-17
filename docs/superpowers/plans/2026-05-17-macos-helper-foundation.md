# macOS Helper Foundation Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add the first native macOS helper app inside `Porthole.app`, flip the bundle executable from `portholed` to `PortholeHelper`, and make the helper own daemon startup for installed helper-mode bundles.

**Architecture:** Add a small SwiftPM AppKit helper under `apps/macos/PortholeHelper/`, build it from `cargo xtask bundle --platform macos`, and copy `PortholeHelper`, `portholed`, and `porthole` into one signed `Porthole.app`. Keep daemon policy, permissions truth, onboarding, and agent approvals in Rust/daemon-owned APIs; the helper is only the native shell, status item, and process supervisor in this slice.

**Tech Stack:** SwiftPM executable target, AppKit `NSStatusItem`, `Foundation.Process`, Rust `xtask`, existing Rust install/launchd flow, existing Cargo and shell CI gates.

---

## Scope Notes

This plan implements the next Phase 3 vertical slice after the xtask bundle foundation:

- Swift macOS helper under `apps/macos/PortholeHelper/`; build output copies `PortholeHelper`, `portholed`, and `porthole` into `Contents/MacOS/`. The roadmap says "Swift / SwiftUI", but this slice should use AppKit directly because `NSStatusItem` and process lifecycle are AppKit/Foundation-first and no window UI exists yet. SwiftUI can enter with onboarding windows later.
- Minimal `NSStatusItem` with a menu that exposes daemon status, restart, and quit.
- Helper process supervision: launch bundled `portholed`, restart on crash, and terminate it on helper quit.
- Helper-mode install path: if `PortholeHelper` exists in the installed bundle, the LaunchAgent program points at `Contents/MacOS/PortholeHelper`; otherwise it keeps the transitional `portholed` path.

This plan deliberately does not implement:

- Native onboarding UI.
- Notification-based agent-permission approval.
- `SMAppService` registration.
- LaunchAgent-to-`SMAppService` migration.
- Notarization/hardened-runtime entitlements.

Those remain separate roadmap items. This slice must not create a second daemon supervisor or a second bundle identity.

## Concerns Up Front

- **Startup ownership:** flipping `CFBundleExecutable` without changing install-time launchd behavior would produce an app whose helper is never started by install. The plan updates LaunchAgent program selection in the same slice.
- **Permission-sensitive verification:** helper launch and `portholed` supervision do not require Accessibility or Screen Recording. Do not add live input/capture smoke tests to this slice; if a permission-dependent call reports missing permission, stop `BLOCKED` per `AGENTS.md`.
- **Swift build availability:** CI runs tests on `macos-latest`, so SwiftPM build tests are acceptable there. Keep Linux-independent checks as Rust format only; do not add a Linux CI Swift requirement.
- **Swift build artifacts:** xtask uses `--scratch-path target/swift/PortholeHelper`, which is covered by the repo's existing `/target` ignore rule. Verify this before implementation so SwiftPM artifacts do not become stageable files.
- **`codesign --deep`:** still acceptable only for this development bundle. Keep the existing comment and do not expand this into notarization work.
- **Status item polish:** the menu bar UI is intentionally minimal. It should be functional and native, not a polished onboarding surface.
- **LaunchAgent log name:** this slice can keep the existing `portholed.log` stdout/stderr path even when launchd starts `PortholeHelper`. The helper-spawned daemon inherits the same stream, so the file remains useful; a cleaner helper/daemon log split belongs with later product polish.

## File Structure

- Create `apps/macos/PortholeHelper/Package.swift` — SwiftPM package for the helper executable.
- Create `apps/macos/PortholeHelper/Sources/PortholeHelper/main.swift` — AppKit entrypoint.
- Create `apps/macos/PortholeHelper/Sources/PortholeHelper/AppDelegate.swift` — `NSApplicationDelegate`, status item setup, menu actions.
- Create `apps/macos/PortholeHelper/Sources/PortholeHelper/DaemonSupervisor.swift` — process start/restart/quit behavior for bundled `portholed`.
- Create `apps/macos/PortholeHelper/Sources/PortholeHelper/BundlePaths.swift` — resolves bundled `portholed`, CLI, logs, and app paths.
- Modify `apps/macos/bundle/Info.plist` — helper-mode plist: `CFBundleExecutable=PortholeHelper`, `LSUIElement=true`, remove `LSBackgroundOnly`.
- Modify `crates/xtask/src/lib.rs` — expose new helper build module.
- Create `crates/xtask/src/macos_helper.rs` — SwiftPM command construction and helper binary path calculation.
- Modify `crates/xtask/src/macos_bundle.rs` — build/copy helper, print helper mode, sign final app.
- Modify `crates/xtask/tests/macos_bundle.rs` — helper-mode plist and helper build command tests.
- Modify `scripts/tests/test-dev-bundle.sh` — assert helper binary and helper-mode plist.
- Modify `crates/porthole/src/commands/install.rs` — choose `PortholeHelper` as LaunchAgent program when present.
- Modify `README.md` and `docs/development.md` — installed app is helper-owned, daemon is bundled child.
- Modify `docs/roadmap.md` — tick only the helper, status item, helper-spawns-daemon, and quit/restart menu items after implementation passes gates.

## Chunk 1: Helper-Mode Bundle Contract

### Task 1: Update checked-in plist expectations

**Files:**
- Modify: `apps/macos/bundle/Info.plist`
- Modify: `crates/xtask/tests/macos_bundle.rs`

- [ ] **Step 1: Add failing helper-mode plist tests**

Extend `crates/xtask/tests/macos_bundle.rs`:

```rust
#[test]
fn helper_info_plist_uses_helper_executable() {
    let plist = fs::read_to_string(workspace_root().join("apps/macos/bundle/Info.plist")).unwrap();
    assert!(plist.contains("<key>CFBundleExecutable</key>"));
    assert!(plist.contains("<string>PortholeHelper</string>"));
    assert!(plist.contains("<key>LSUIElement</key>"));
    assert!(plist.contains("<true/>"));
    assert!(!plist.contains("<key>LSBackgroundOnly</key>"));
    assert!(plist.contains("<string>org.flotilla.porthole.dev</string>"));
}
```

Remove or replace the old `transitional_info_plist_keeps_daemon_executable` test.

- [ ] **Step 2: Run the focused test and verify failure**

Run:

```sh
cargo test -p xtask --test macos_bundle --locked helper_info_plist_uses_helper_executable
```

Expected: FAIL because the current plist still names `portholed` and `LSBackgroundOnly`.

- [ ] **Step 3: Flip the checked-in plist to helper mode**

Edit `apps/macos/bundle/Info.plist`:

```xml
<key>CFBundleExecutable</key>
<string>PortholeHelper</string>
...
<key>LSUIElement</key>
<true/>
```

Remove the `LSBackgroundOnly` key/value pair. Keep:

```xml
<key>CFBundleIdentifier</key>
<string>org.flotilla.porthole.dev</string>
```

- [ ] **Step 4: Verify the plist test passes**

Run:

```sh
cargo test -p xtask --test macos_bundle --locked helper_info_plist_uses_helper_executable
```

Expected: PASS.

- [ ] **Step 5: Commit the plist contract**

```sh
git add apps/macos/bundle/Info.plist crates/xtask/tests/macos_bundle.rs
git commit -m "build: switch macOS bundle plist to helper mode"
```

## Chunk 2: Swift Helper App

### Task 2: Add a minimal AppKit helper

**Files:**
- Create: `apps/macos/PortholeHelper/Package.swift`
- Create: `apps/macos/PortholeHelper/Sources/PortholeHelper/main.swift`
- Create: `apps/macos/PortholeHelper/Sources/PortholeHelper/AppDelegate.swift`
- Create: `apps/macos/PortholeHelper/Sources/PortholeHelper/DaemonSupervisor.swift`
- Create: `apps/macos/PortholeHelper/Sources/PortholeHelper/BundlePaths.swift`

- [ ] **Step 1: Add Swift package skeleton**

Create `Package.swift`:

```swift
// swift-tools-version: 5.9
import PackageDescription

let package = Package(
    name: "PortholeHelper",
    platforms: [.macOS(.v13)],
    products: [
        .executable(name: "PortholeHelper", targets: ["PortholeHelper"]),
    ],
    targets: [
        .executableTarget(name: "PortholeHelper"),
    ]
)
```

- [ ] **Step 2: Add path resolution**

Create `BundlePaths.swift`:

```swift
import Foundation

struct BundlePaths {
    let bundleURL: URL

    static func current() -> BundlePaths {
        BundlePaths(bundleURL: Bundle.main.bundleURL)
    }

    var contentsURL: URL {
        bundleURL.appendingPathComponent("Contents", isDirectory: true)
    }

    var macOSURL: URL {
        contentsURL.appendingPathComponent("MacOS", isDirectory: true)
    }

    var daemonURL: URL {
        macOSURL.appendingPathComponent("portholed")
    }

}
```

- [ ] **Step 3: Add daemon supervisor**

Create `DaemonSupervisor.swift`:

```swift
import Foundation

@MainActor
final class DaemonSupervisor {
    enum State: Equatable {
        case stopped
        case running(pid: Int32)
        case crashed(status: Int32)
    }

    private let daemonURL: URL
    private var process: Process?
    private var shouldRestart = true
    private let onStateChange: (State) -> Void

    init(daemonURL: URL, onStateChange: @escaping (State) -> Void) {
        self.daemonURL = daemonURL
        self.onStateChange = onStateChange
    }

    func start() {
        guard process == nil else { return }
        shouldRestart = true

        let next = Process()
        next.executableURL = daemonURL
        next.terminationHandler = { [weak self] terminated in
            Task { @MainActor in
                self?.handleTermination(terminated.terminationStatus)
            }
        }

        do {
            try next.run()
            process = next
            onStateChange(.running(pid: next.processIdentifier))
        } catch {
            NSLog("failed to launch portholed: \(error)")
            onStateChange(.crashed(status: -1))
        }
    }

    func restart() {
        shouldRestart = true
        if let process {
            process.terminate()
        } else {
            start()
        }
    }

    func stopForQuit() {
        shouldRestart = false
        if let process {
            process.terminate()
        } else {
            onStateChange(.stopped)
        }
    }

    private func handleTermination(_ status: Int32) {
        process = nil
        if shouldRestart {
            onStateChange(.crashed(status: status))
            start()
        } else {
            onStateChange(.stopped)
        }
    }
}
```

If Swift reports actor-capture or sendability issues, keep all supervisor mutation on `@MainActor` and use `Task { @MainActor in ... }` in the termination handler. Do not introduce background shared mutable state. `stopForQuit()` intentionally lets the termination handler publish the final stopped state when a process exists, avoiding duplicate state transitions for future badge/status observers. If the daemon binary is missing, `next.run()` reports `.crashed(status: -1)` and does not retry in a loop; the user can rebuild or use the restart menu item after fixing the bundle.

- [ ] **Step 4: Add status item app delegate**

Create `AppDelegate.swift`:

```swift
import AppKit

@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate {
    private var statusItem: NSStatusItem?
    private var statusMenuItem: NSMenuItem?
    private var supervisor: DaemonSupervisor?

    func applicationDidFinishLaunching(_ notification: Notification) {
        NSApp.setActivationPolicy(.accessory)
        installStatusItem()

        let paths = BundlePaths.current()
        supervisor = DaemonSupervisor(daemonURL: paths.daemonURL) { [weak self] state in
            self?.render(state)
        }
        supervisor?.start()
    }

    func applicationWillTerminate(_ notification: Notification) {
        supervisor?.stopForQuit()
    }

    private func installStatusItem() {
        let item = NSStatusBar.system.statusItem(withLength: NSStatusItem.variableLength)
        if let button = item.button {
            button.image = NSImage(systemSymbolName: "circle.grid.cross", accessibilityDescription: "Porthole")
            button.image?.isTemplate = true
        }

        let menu = NSMenu()
        let status = NSMenuItem(title: "Daemon: starting", action: nil, keyEquivalent: "")
        status.isEnabled = false
        statusMenuItem = status
        menu.addItem(status)
        menu.addItem(.separator())
        menu.addItem(NSMenuItem(title: "Restart Daemon", action: #selector(restartDaemon), keyEquivalent: "r"))
        menu.addItem(NSMenuItem(title: "Quit Porthole", action: #selector(quit), keyEquivalent: "q"))
        item.menu = menu
        statusItem = item
    }

    private func render(_ state: DaemonSupervisor.State) {
        switch state {
        case .stopped:
            statusMenuItem?.title = "Daemon: stopped"
        case .running(let pid):
            statusMenuItem?.title = "Daemon: running (\(pid))"
        case .crashed(let status):
            statusMenuItem?.title = "Daemon: restarting after exit \(status)"
        }
    }

    @objc private func restartDaemon() {
        supervisor?.restart()
    }

    @objc private func quit() {
        NSApp.terminate(nil)
    }
}
```

- [ ] **Step 5: Add entrypoint**

Create `main.swift`:

```swift
import AppKit

let app = NSApplication.shared
let delegate = AppDelegate()
app.delegate = delegate
app.run()
```

- [ ] **Step 6: Verify Swift package builds**

Run:

```sh
swift build --package-path apps/macos/PortholeHelper --scratch-path target/swift/PortholeHelper -c debug
```

Expected: PASS and `target/swift/PortholeHelper/debug/PortholeHelper` exists. This matches the xtask scratch path instead of leaving manual smoke artifacts under the Swift package's `.build/` directory.

- [ ] **Step 7: Commit helper source**

```sh
git add apps/macos/PortholeHelper
git commit -m "feat: add macOS helper app"
```

## Chunk 3: xtask Builds And Bundles Helper

### Task 3: Teach xtask to build and copy `PortholeHelper`

**Files:**
- Create: `crates/xtask/src/macos_helper.rs`
- Modify: `crates/xtask/src/lib.rs`
- Modify: `crates/xtask/src/macos_bundle.rs`
- Modify: `crates/xtask/tests/macos_bundle.rs`
- Modify: `scripts/tests/test-dev-bundle.sh`

- [ ] **Step 1: Verify Swift scratch path is ignored**

Run:

```sh
git check-ignore -q target/swift/PortholeHelper
```

Expected: PASS because the repo's `/target` ignore rule covers `target/swift/`.

If it fails, add this to `.gitignore` and commit it before building Swift artifacts:

```gitignore
target/swift/
```

- [ ] **Step 2: Add failing xtask helper tests**

In `crates/xtask/tests/macos_bundle.rs`, add:

```rust
use xtask::macos_helper::{swift_build_args, swift_build_configuration};

#[test]
fn swift_build_configuration_tracks_rust_profile() {
    assert_eq!(swift_build_configuration(false), "debug");
    assert_eq!(swift_build_configuration(true), "release");
}

#[test]
fn swift_build_uses_package_path_and_scratch_path() {
    assert_eq!(
        swift_build_args(false),
        vec![
            "build",
            "--package-path",
            "apps/macos/PortholeHelper",
            "--scratch-path",
            "target/swift/PortholeHelper",
            "-c",
            "debug",
        ]
    );
}

#[test]
fn swift_build_release_uses_release_configuration() {
    assert_eq!(
        swift_build_args(true),
        vec![
            "build",
            "--package-path",
            "apps/macos/PortholeHelper",
            "--scratch-path",
            "target/swift/PortholeHelper",
            "-c",
            "release",
        ]
    );
}
```

Run:

```sh
cargo test -p xtask --test macos_bundle --locked swift_build
```

Expected: FAIL because `macos_helper` does not exist.

- [ ] **Step 3: Add helper build module**

Create `crates/xtask/src/macos_helper.rs`:

```rust
use std::path::{Path, PathBuf};

pub const PACKAGE_PATH: &str = "apps/macos/PortholeHelper";

pub fn swift_build_configuration(release: bool) -> &'static str {
    if release { "release" } else { "debug" }
}

pub fn scratch_path() -> PathBuf {
    Path::new("target")
        .join("swift")
        .join("PortholeHelper")
}

pub fn swift_build_args(release: bool) -> Vec<String> {
    vec![
        "build".to_owned(),
        "--package-path".to_owned(),
        PACKAGE_PATH.to_owned(),
        "--scratch-path".to_owned(),
        scratch_path().to_string_lossy().into_owned(),
        "-c".to_owned(),
        swift_build_configuration(release).to_owned(),
    ]
}

pub fn built_helper_path(release: bool) -> PathBuf {
    scratch_path()
        .join(swift_build_configuration(release))
        .join("PortholeHelper")
}
```

Update `crates/xtask/src/lib.rs`:

```rust
pub mod macos_bundle;
pub mod macos_helper;
```

- [ ] **Step 4: Verify helper tests pass**

Run:

```sh
cargo test -p xtask --test macos_bundle --locked swift_build
```

Expected: PASS.

- [ ] **Step 5: Call SwiftPM from bundle assembly**

In `crates/xtask/src/macos_bundle.rs`:

- Import `crate::macos_helper`.
- Unless `--refresh`, run `swift` with `macos_helper::swift_build_args(options.release)` after the Rust workspace build.
- Check `macos_helper::built_helper_path(options.release)` exists.
- Copy it to `Porthole.app/Contents/MacOS/PortholeHelper`.
- `chmod +x` it using the existing `copy_executable`.
- Print `bundle mode: helper app`.

Use `std::process::Command` directly, not shell strings. Because `run_status` currently accepts `&[&str]`, either:

- add a `run_status_owned(command: &str, args: &[String])` helper, or
- change `run_status` to accept `impl IntoIterator<Item = impl AsRef<OsStr>>`.

Keep the code simple; this is an xtask crate.

- [ ] **Step 6: Update bundle smoke test expectations**

In `scripts/tests/test-dev-bundle.sh`, change the plist expectation:

```bash
exec_name="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleExecutable' target/debug/Porthole.app/Contents/Info.plist)"
test "$exec_name" = "PortholeHelper" || { echo "expected helper executable PortholeHelper, got $exec_name" >&2; exit 1; }

ui_element="$(/usr/libexec/PlistBuddy -c 'Print :LSUIElement' target/debug/Porthole.app/Contents/Info.plist)"
test "$ui_element" = "true" || { echo "expected LSUIElement=true, got $ui_element" >&2; exit 1; }

if /usr/libexec/PlistBuddy -c 'Print :LSBackgroundOnly' target/debug/Porthole.app/Contents/Info.plist >/tmp/porthole-lsbackground.out 2>&1; then
    cat /tmp/porthole-lsbackground.out >&2
    echo "expected LSBackgroundOnly to be absent in helper mode" >&2
    exit 1
fi

test -x target/debug/Porthole.app/Contents/MacOS/PortholeHelper || { echo "PortholeHelper missing from bundle" >&2; exit 1; }
```

Keep the `portholed --help` and `porthole --help` binary checks. Do not launch the helper from this shell test; `NSStatusItem` launch behavior is covered by manual/local smoke because CI may not have a user GUI session.

- [ ] **Step 7: Verify bundle command**

Run:

```sh
cargo xtask bundle --platform macos
```

Expected with signing identity:

- Rust workspace builds with `--locked`.
- Swift helper builds.
- `target/debug/Porthole.app/Contents/MacOS/PortholeHelper` exists and is executable.
- Final app signs successfully.

Expected without signing identity: same explicit Apple Development identity error as today before build work begins.

- [ ] **Step 8: Verify bundle contents**

Run:

```sh
codesign -v target/debug/Porthole.app
test -x target/debug/Porthole.app/Contents/MacOS/PortholeHelper
test -x target/debug/Porthole.app/Contents/MacOS/portholed
test -x target/debug/Porthole.app/Contents/MacOS/porthole
/usr/libexec/PlistBuddy -c 'Print :CFBundleExecutable' target/debug/Porthole.app/Contents/Info.plist
/usr/libexec/PlistBuddy -c 'Print :LSUIElement' target/debug/Porthole.app/Contents/Info.plist
```

Expected:

```text
PortholeHelper
true
```

- [ ] **Step 9: Commit xtask helper bundling**

```sh
git add crates/xtask scripts/tests/test-dev-bundle.sh
git commit -m "build: bundle macOS helper app"
```

## Chunk 4: Install Uses Helper As Startup Program

### Task 4: Point LaunchAgent at helper in helper-mode bundles

**Files:**
- Modify: `crates/porthole/src/commands/install.rs`
- Modify: `README.md`
- Modify: `docs/development.md`

- [ ] **Step 1: Add failing install-path tests**

In `crates/porthole/src/commands/install.rs` tests, add a helper for startup program selection:

```rust
#[test]
fn startup_program_prefers_helper_when_present() {
    let tmp = tempfile::tempdir().unwrap();
    let bundle = tmp.path().join("Porthole.app");
    let macos = bundle.join("Contents/MacOS");
    fs::create_dir_all(&macos).unwrap();
    fs::write(macos.join("PortholeHelper"), "").unwrap();
    fs::write(macos.join("portholed"), "").unwrap();

    assert_eq!(startup_program_for_bundle(&bundle), macos.join("PortholeHelper"));
}

#[test]
fn startup_program_falls_back_to_daemon_for_transitional_bundle() {
    let tmp = tempfile::tempdir().unwrap();
    let bundle = tmp.path().join("Porthole.app");
    let macos = bundle.join("Contents/MacOS");
    fs::create_dir_all(&macos).unwrap();
    fs::write(macos.join("portholed"), "").unwrap();

    assert_eq!(startup_program_for_bundle(&bundle), macos.join("portholed"));
}
```

Run:

```sh
cargo test -p porthole startup_program --locked
```

Expected: FAIL because `startup_program_for_bundle` does not exist.

- [ ] **Step 2: Implement startup program selection**

Add near the install helpers:

```rust
fn startup_program_for_bundle(bundle: &Path) -> PathBuf {
    let macos = bundle.join("Contents/MacOS");
    let helper = macos.join("PortholeHelper");
    if helper.is_file() {
        helper
    } else {
        macos.join("portholed")
    }
}
```

In `do_install`, replace:

```rust
let daemon_path = dst_bundle.join("Contents/MacOS/portholed");
let plist_xml = render_launch_agent_plist(&daemon_path, &log_dir.join("portholed.log"));
```

with:

```rust
let startup_program = startup_program_for_bundle(&dst_bundle);
let plist_xml = render_launch_agent_plist(&startup_program, &log_dir.join("portholed.log"));
```

Keep the LaunchAgent label `org.flotilla.porthole`. This slice changes the program path, not the launchd identity.

- [ ] **Step 3: Verify install tests**

Run:

```sh
cargo test -p porthole startup_program --locked
cargo test -p porthole render_plist_includes_program_path_and_label --locked
```

Expected: PASS.

- [ ] **Step 4: Update README development language**

Update `README.md` install section:

- `Porthole.app` holds helper, daemon, and CLI.
- Install registers the helper-mode app startup program when present; the helper starts the daemon as a bundled child.
- The daemon still owns HTTP-over-UDS and permissions truth.

Update `docs/development.md`:

- `cargo xtask bundle --platform macos` now builds Rust binaries and the Swift helper.
- `./target/debug/Porthole.app/Contents/MacOS/porthole install --user --force` installs a helper-owned bundle.
- Manual daemon terminal launch remains discouraged because it changes TCC attribution.

- [ ] **Step 5: Commit install/docs update**

```sh
git add crates/porthole/src/commands/install.rs README.md docs/development.md
git commit -m "install: point LaunchAgent at helper binary when present"
```

## Chunk 5: Local Helper Smoke And Roadmap

### Task 5: Verify the helper slice and update roadmap

**Files:**
- Modify: `docs/roadmap.md`

- [ ] **Step 1: Run focused helper/bundle tests**

```sh
cargo test -p xtask --test macos_bundle --locked
cargo test -p porthole startup_program --locked
./scripts/tests/test-dev-bundle.sh
```

Expected: all PASS. If no Apple Development identity is available, the bundle smoke should PASS by verifying the missing-identity error path.

- [ ] **Step 2: Run optional GUI smoke if a user GUI session is available**

Only run this on a real logged-in macOS desktop session:

```sh
open target/debug/Porthole.app
sleep 3
pgrep -fl PortholeHelper
pgrep -fl portholed
osascript -e 'tell application id "org.flotilla.porthole.dev" to quit'
```

Expected:

- `PortholeHelper` starts.
- A bundled `portholed` child starts.
- Quit exits helper and child.

If this cannot be verified because the session is headless or the app cannot be opened, say so explicitly. Do not substitute permission-dependent input/capture tests.

- [ ] **Step 3: Update roadmap**

In `docs/roadmap.md`, tick:

```markdown
- [x] Swift / SwiftUI macOS helper under `apps/macos/PortholeHelper/`; build output copies `PortholeHelper`, `portholed`, and `porthole` into `Contents/MacOS/`.
- [x] `NSStatusItem` with monochrome glyph + optional badge (surface count, "broken" state).
- [x] Helper spawns `portholed` on launch via `Process` if not already running; restarts on crash.
- [x] Quit / Restart daemon menu items.
```

Do not tick onboarding UI, notification approvals, `SMAppService`, or migration to `SMAppService`.

- [ ] **Step 4: Run full gates**

```sh
cargo build --workspace --locked
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo +nightly-2026-03-12 fmt --check
git diff --check
```

- [ ] **Step 5: Inspect signed bundle identity**

If signing ran:

```sh
codesign -dvv target/debug/Porthole.app 2>&1 | sed -n 's/^Identifier=//p; s/^Authority=//p'
/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' target/debug/Porthole.app/Contents/Info.plist
/usr/libexec/PlistBuddy -c 'Print :CFBundleExecutable' target/debug/Porthole.app/Contents/Info.plist
/usr/libexec/PlistBuddy -c 'Print :LSUIElement' target/debug/Porthole.app/Contents/Info.plist
```

Expected:

```text
org.flotilla.porthole.dev
Apple Development: ...
org.flotilla.porthole.dev
PortholeHelper
true
```

- [ ] **Step 6: Commit roadmap**

```sh
git add docs/roadmap.md
git commit -m "docs: mark macOS helper foundation complete"
```

- [ ] **Step 7: Open PR**

Open a draft PR with:

- Summary: Swift helper, helper-mode plist, xtask Swift build/copy, LaunchAgent helper startup selection.
- Validation: all focused and full gates, script smoke result, whether optional GUI smoke ran.
- Explicit non-claim: no native onboarding UI, notification approval UI, `SMAppService`, notarization, or live permission/capture smoke in this slice.

Do not claim Accessibility or Screen Recording behavior was verified unless you actually ran permission-dependent installed-bundle smoke with grants present.
