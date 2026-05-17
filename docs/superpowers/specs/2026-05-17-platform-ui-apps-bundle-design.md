# Platform UI Apps And Bundle Design

Date: 2026-05-17
Status: approved

## 1. Purpose

Porthole needs native UI for setup, daemon status, login integration, and
agent-permission approval. On macOS that UI is `Porthole.app`, a menu-bar app
that contains and manages the Rust daemon. Other platforms will need their own
native shells later: a Windows tray app, a Linux desktop/status app, or other
platform-specific launchers.

This spec separates the shared product contract from the macOS implementation.
The first Phase 3 slice should build a more sensible `Porthole.app` assembly
path before adding helper UI behavior.

## 2. Current State

Today `scripts/dev-bundle.sh` assembles `target/<profile>/Porthole.app` with a
shell heredoc `Info.plist`, copies `portholed` and `porthole` into
`Contents/MacOS`, signs the bundle, and sets:

```text
CFBundleExecutable = portholed
LSBackgroundOnly = true
```

That is acceptable for daemon-only TCC stability, but it is not the right
center of gravity for a product app. Once a helper exists, the app executable
must be the native UI process. The daemon should be a bundled child executable,
not the bundle's main executable.

## 3. Goals

- Keep one macOS product bundle: `Porthole.app`.
- Make the macOS bundle a checked-in app layout rather than a shell heredoc.
- Put macOS-specific UI and bundle assets under `apps/macos/`.
- Add a repo-native bundle command that can later support non-macOS platforms.
- Keep the Rust daemon and CLI as daemon-owned product logic, not UI-app logic.
- Preserve the bundle identifier so existing TCC grants survive the transition.
- Make the transition from LaunchAgent-running-`portholed` to
  helper-running-`portholed` explicit and testable.

## 4. Non-Goals

- No Windows or Linux UI app implementation in this slice.
- No agent-permission enforcement implementation; the approved agent-permission
  spec defines the daemon contract.
- No polished helper UI in the first bundle-build slice.
- No notarized production release pipeline.
- No new system-permission workaround. Missing Accessibility or Screen
  Recording remains `BLOCKED` for live verification.

## 5. Platform UI App Contract

A platform UI app is a native shell over daemon-owned state. It owns OS
integration and user presentation, not porthole domain policy.

Shared responsibilities:

- start, stop, restart, and monitor the daemon
- integrate with platform startup/login mechanisms
- display daemon health and version
- display system-permission state and remediation actions
- display agent-permission requests and decisions
- expose local controls such as restart daemon, open logs, quit

Shared non-responsibilities:

- surface registry ownership
- capture-transfer ownership
- agent policy storage
- system-permission truth
- action authorization
- event history

Those remain daemon responsibilities exposed through HTTP-over-UDS and future
platform-equivalent local transports if needed.

## 6. Repository Layout

Use a platform-oriented `apps/` tree:

```text
apps/
  macos/
    PortholeHelper/
      Sources/
      Resources/
    bundle/
      Info.plist
      Resources/
        icon.png
  windows/        # future
  linux/          # future

crates/
  xtask/          # repo-native build/bundle tasks
  porthole/
  portholed/
  porthole-core/
```

`apps/macos/` is the platform app home. The Swift helper source is one part of
that app; bundle metadata, assets, signing inputs, and packaging scripts belong
beside it rather than under a helper-specific top-level directory.

Do not add an `entitlements.plist` in the transitional bundle-foundation slice.
The flat development bundle does not need one. Add entitlements only in the
later hardened-runtime/notarization slice or when a concrete helper capability
requires them.

The existing root `assets/icon.png` can move into `apps/macos/bundle/Resources`
or be copied from there. There should be one authoritative icon input for the
macOS bundle.

## 7. Bundle Command

Add a repo-native command:

```sh
cargo xtask bundle --platform macos
cargo xtask bundle --platform macos --release
cargo xtask bundle --platform macos --refresh
cargo xtask bundle --platform macos --sign "Apple Development: ..."
```

The command should:

1. choose the Rust profile (`debug` or `release`)
2. build Rust workspace binaries unless `--refresh`
3. build the Swift helper when helper source exists
4. assemble `target/<profile>/Porthole.app`
5. copy `porthole`, `portholed`, and eventually `PortholeHelper`
6. render or copy checked-in plist metadata
7. copy resources
8. sign the bundle with an Apple Development identity
9. reject ad-hoc signing for development bundles

`scripts/dev-bundle.sh` should become a compatibility wrapper around this task
or be retired after README and tests move to `cargo xtask bundle`.

## 8. macOS Bundle Shape

Final Phase 3 layout:

```text
Porthole.app/
  Contents/
    Info.plist
    MacOS/
      PortholeHelper
      portholed
      porthole
    Resources/
      icon.png
```

Final plist posture:

```text
CFBundleIdentifier = org.flotilla.porthole.dev   # dev builds
CFBundleName = Porthole
CFBundleExecutable = PortholeHelper
LSUIElement = true
LSBackgroundOnly = absent
NSAccessibilityUsageDescription = ...
NSScreenCaptureUsageDescription = ...
```

`LSUIElement=true` gives a menu-bar app without a Dock icon. `LSBackgroundOnly`
must go away because a background-only app is the wrong model for status-item
UI and user-driven prompts.

The bundle id must not change during the helper transition. TCC, notifications,
and login items key off bundle identity.

## 9. Transitional Bundle

Before Swift helper source exists, the bundle command may still produce the
current daemon-executable bundle:

```text
CFBundleExecutable = portholed
LSBackgroundOnly = true
```

That transitional path exists only to preserve today's development workflow
while moving bundle assembly out of shell. Once `PortholeHelper` lands, the
main executable flips to the helper and `LSBackgroundOnly` is removed.

The bundle command should make this state explicit in logs, for example:

```text
bundle mode: daemon-only transitional
```

or:

```text
bundle mode: helper app
```

## 10. Daemon Lifecycle

Final helper behavior:

- On launch, the helper checks for an already-running daemon for the current
  runtime directory.
- If absent, it spawns `Contents/MacOS/portholed`.
- It monitors process exit and restarts on crash.
- Quit stops the child daemon unless the user chooses a future "leave daemon
  running" option.
- Restart daemon kills and respawns the child.

The daemon remains the owner of its UDS socket. The helper is only the process
supervisor and native status surface.

During transition, install may still register a LaunchAgent whose program is
`Contents/MacOS/portholed`. Once the helper is the bundle executable, install
should register the app/helper startup path instead, or defer to
`SMAppService` when that lands.

## 11. Install And Migration

Current install writes a LaunchAgent that runs:

```text
Porthole.app/Contents/MacOS/portholed
```

Phase 3 needs an explicit migration:

1. Install new `Porthole.app`.
2. Boot out any existing `org.flotilla.porthole` LaunchAgent. This is the
   current LaunchAgent label from `crates/porthole/src/launchd.rs`; it is not
   derived from the development bundle id `org.flotilla.porthole.dev`.
3. Remove the old per-user plist when the helper will own startup.
4. Register the helper's login item or helper LaunchAgent, depending on the
   current slice.
5. Start the helper, which starts the daemon.

The system must avoid two supervisors fighting over one daemon. Duplicate
startup mechanisms are a release blocker for the helper slice.

## 12. Build Options

### Option A: Keep Shell As The Bundle Builder

Keep expanding `scripts/dev-bundle.sh`.

This is too brittle. Shell heredocs for plist metadata, platform-specific
branching, Swift build integration, signing, and future platform packaging will
be hard to test and hard to extend.

### Option B: Xcode Owns The Whole Product

Make Xcode build Rust binaries and assemble the app.

This fits macOS conventions, but it makes Rust workspace development and CI
secondary. Porthole is mostly Rust, and future Windows/Linux packaging should
not be modeled around Xcode.

### Option C: Repo-Native `xtask` Owns Bundling

Use Rust `xtask` as the canonical product assembly path. It invokes Cargo for
Rust, invokes `xcodebuild` or `swift build` for macOS helper code, copies
checked-in app metadata, and signs the final app.

This is the recommended path. It keeps one command for CI and developers while
still allowing native platform build tools inside each platform app.

## 13. Testing

Bundle-foundation tests should cover:

- ad-hoc signing rejection
- missing Apple Development identity error shape
- bundle contains `Info.plist`, icon, `porthole`, and `portholed`
- `CFBundleIdentifier` stays stable
- transitional daemon-only plist has `CFBundleExecutable=portholed`
- installed LaunchAgent program path changes only in the intended migration
  slice
- `codesign -v target/<profile>/Porthole.app`

Swift helper tests can be lighter initially. The first UI slice should verify
that the helper-mode plist has `CFBundleExecutable=PortholeHelper` and
`LSUIElement=true`, the helper starts, creates an `NSStatusItem`, can launch a
child process in a controlled test mode, and exits cleanly.

## 14. First Implementation Slice

The first slice should build the foundation only:

1. add `crates/xtask`
2. add `apps/macos/bundle/Info.plist` and resources
3. implement `cargo xtask bundle --platform macos`
4. make `scripts/dev-bundle.sh` delegate to xtask
5. update bundle tests and README references
6. leave `CFBundleExecutable=portholed` until the helper binary exists

This keeps TCC behavior stable while replacing the fragile bundle assembly path.

## 15. Success Criterion

A developer can run one repo-native command to build `target/debug/Porthole.app`
with stable signing, checked-in metadata, bundled Rust binaries, and the same
TCC identity as today. The design leaves an obvious next step: add
`PortholeHelper`, flip the bundle executable, and let the helper supervise the
daemon without changing the product bundle identity.
