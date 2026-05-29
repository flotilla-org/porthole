# KWin Platform Parity Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development
> (if subagents available) or superpowers:executing-plans to implement this
> plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build KWin/Plasma Wayland support as Porthole's first Linux
compositor adapter, reaching staged parity with the existing macOS adapter where
the platform allows.

**Architecture:** KWin support is split into a compositor plane, input plane,
and portal capture plane. The compositor plane uses a KWin control script and a
daemon-owned KWin bridge service on the user session bus. Input and capture use
Linux portal/PipeWire/EIS mechanisms rather than root-owned input or capture
bypasses.

**Tech Stack:** Rust, KWin JavaScript scripting, session D-Bus (`zbus`
expected), xdg-desktop-portal RemoteDesktop/ScreenCast/Screenshot, PipeWire,
EIS/libei, existing cargo workspace gates.

---

## Files

Expected new files:

- Add: `docs/superpowers/specs/2026-05-29-kwin-platform-parity-design.md`
- Add: `docs/adr/0003-platform-surface-ref.md`
- Add: `apps/linux/kwin/porthole-control-script/`
- Add: `crates/porthole-adapter-kwin/`

Expected modified files across the branch chain:

- Modify: `CONTEXT.md`
- Modify: `docs/roadmap.md`
- Modify: `Cargo.toml`
- Modify: `crates/porthole-core/src/adapter.rs`
- Modify: `crates/porthole-core/src/surface.rs`
- Modify: `crates/porthole-core/src/search.rs`
- Modify: `crates/porthole-core/src/handle.rs`
- Modify: `crates/portholed/src/state.rs`
- Modify: `crates/portholed/src/routes/attention.rs`
- Modify: `crates/porthole/src/commands/mod.rs`
- Modify: `crates/porthole/src/main.rs`

## Branch 1: `identity-flotilla-work`

- [x] Rename macOS dev bundle id from `org.flotilla.porthole.dev` to
  `work.flotilla.porthole.dev`.
- [x] Rename macOS production identity examples from `org.flotilla.porthole` to
  `work.flotilla.porthole`.
- [x] Rename LaunchAgent labels, plist filenames, Swift constants, Rust
  constants, tests, and docs.
- [x] Update TCC reset instructions.
- [x] Document one-time local cleanup for pre-release installs using the old
  identity.
- [ ] Run relevant Swift helper tests and workspace cargo gates.

## Branch 2: `kwin-control-script-spike`

- [ ] Add `apps/linux/kwin/porthole-control-script/` as a KPackage
  `KWin/Script` package.
- [ ] Add a minimal `metadata.json` and `contents/code/main.js`.
- [ ] Add `porthole kwin install-script`.
- [ ] Add `porthole kwin uninstall-script`.
- [ ] Add `porthole kwin status`.
- [ ] Add a reload/load path using KWin's `org.kde.kwin.Scripting` D-Bus API.
- [ ] Add a dev harness that owns a temporary session-bus service and confirms
  the script can call out.
- [ ] Publish a minimal snapshot: active window caption plus window count is
  enough for the spike.
- [ ] Document that script install is per-user and does not require sudo.

## Branch 3: `kwin-dbus-bridge`

- [ ] Add a daemon-owned session D-Bus service named
  `work.flotilla.Porthole.KWin`.
- [ ] Implement a narrow bridge API:
  - `PublishSnapshot`
  - `NextCommand`
  - `CompleteCommand`
- [ ] Start with JSON payloads if typed D-Bus structs slow down the spike.
- [ ] Add command queue tests independent of KWin.
- [ ] Connect the KWin control script to the real daemon bridge instead of the
  dev harness.
- [ ] Keep public HTTP-over-UDS unchanged.

## Branch 4a: `platform-surface-ref`

- [ ] Introduce a platform-neutral `PlatformSurfaceRef` in core.
- [ ] Migrate `SurfaceInfo` away from public/core `cg_window_id`.
- [ ] Migrate search candidate refs and attach/track decoding to platform refs.
- [ ] Replace `Adapter::frontmost_window_id` with a platform-neutral focus
  identity path.
- [ ] Replace `Adapter::window_alive(pid, cg_window_id)` with a platform-ref
  liveness method.
- [ ] Update `HandleStore` to deduplicate and resolve focus by platform ref.
- [ ] Keep macOS adapter-internal code free to use `CGWindowID`.
- [ ] Update tests that currently assert `cg_window_id`.

## Branch 4: `kwin-compositor-adapter`

- [ ] Add `crates/porthole-adapter-kwin`.
- [ ] Gate adapter construction to KDE/Plasma Wayland, initially explicit if
  auto-detection is too risky.
- [ ] Consume KWin bridge snapshots as the source of compositor state.
- [ ] Implement:
  - `name`
  - `capabilities`
  - `search`
  - platform-ref liveness
  - `focus`
  - `close`
  - `place_surface`
  - `snapshot_geometry`
  - `attention`
- [ ] Return `AdapterUnsupported` for input, screenshot, and recording in this
  branch.
- [ ] Add in-memory or bridge-fake tests for compositor-plane behavior.

## Branch 5: `kwin-input-plane`

- [ ] Add minimal Portal consent protocol/core language for runtime consent and
  active sessions.
- [ ] Establish xdg-desktop-portal RemoteDesktop sessions.
- [ ] Store the active RemoteDesktop session in KWin adapter runtime state.
- [ ] Implement keyboard input first:
  - `key`
  - `text`
- [ ] Implement pointer input after keyboard:
  - `click`
  - `scroll`
  - `pointer_move`
- [ ] Use EIS/libei only if portal D-Bus methods are insufficient.
- [ ] Return honest consent-cancelled/permission-needed errors.
- [ ] Do not use `/dev/uinput`.

## Branch 5a: `kwin-launch-correlation`

- [ ] Implement `launch_process` for the KWin adapter.
- [ ] Capture launched pid and launch timestamp.
- [ ] Match new KWin windows from bridge snapshots/events.
- [ ] Strong match when exactly one new window belongs to the launched pid or a
  descendant pid within timeout.
- [ ] Plausible match when app id/resource class matches and the window becomes
  active shortly after launch.
- [ ] Keep artifact launch/document matching out of scope unless KWin exposes
  enough metadata cheaply.
- [ ] Add terminal-launch manual smoke cases once permission/session state is
  available.

## Branch 6: `kwin-screenshot-capture`

- [ ] Implement `screenshot(surface)` for KWin.
- [ ] Prefer KWin `org.kde.KWin.ScreenShot2.CaptureWindow` if the KWin platform
  ref can be mapped to the API's expected window identifier.
- [ ] Fall back to portal screenshot/area only when window capture is
  unavailable.
- [ ] Preserve Porthole screenshot response semantics: PNG bytes, bounds, scale,
  and capture timestamp.
- [ ] Keep `start_video_capture` unsupported in this branch.
- [ ] Add manual consent-cancelled and successful screenshot smoke notes.

## Branch 7: `kwin-pipewire-recording`

- [ ] Implement PipeWire ScreenCast session setup through xdg-desktop-portal.
- [ ] Map stream frames into Porthole's existing capture-transfer model.
- [ ] Decide CPU baseline first before dmabuf/native handle transfer.
- [ ] Preserve ordered recording cursor semantics.
- [ ] Extend Portal consent handling for long-lived capture sessions.
- [ ] Add recording CLI smoke analogous to macOS recording where possible.

## Deferred Work

- Durable Portal consent investigation.
- Linux platform UI app/tray/status surface.
- Linux notification action approvals for agent permissions.
- Hyprland adapter.
- X11 adapter.
- Browser tabs via CDP.

## Current Sudo State

No known runtime step for KWin support requires sudo. KWin script installation,
session D-Bus, portal consent, and PipeWire/EIS runtime interaction are
per-user/session operations.

The expected development headers have been installed on the dogfood Fedora KDE
machine:

- `dbus-devel`
- `wayland-devel`
- `pipewire-devel`
- `libei-devel`
- `at-spi2-core-devel`
- `libxkbcommon-devel`

Verified pkg-config versions:

- `dbus-1` 1.16.0
- `wayland-client` 1.24.0
- `libpipewire-0.3` 1.4.11
- `libei-1.0` 1.4.1
- `atspi-2` 2.56.8
- `xkbcommon` 1.8.1

The implementation branch chain also requires a Rust toolchain before any branch
can update dependencies, regenerate `Cargo.lock`, run workspace tests, or run
the pinned nightly fmt gate. On the dogfood Fedora KDE machine used for the
initial grill, `cargo`, `rustc`, and `rustup` were not available on `PATH`;
install Rust before starting `kwin-dbus-bridge`.

## Gates

Before claiming an implementation branch is done, run the repo gates from
`AGENTS.md`:

```sh
cargo build --workspace --locked
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo +nightly-2026-03-12 fmt --check
```

KWin/portal live smokes require a real KDE Plasma Wayland session and user
interaction for consent. Do not invent mocks or bypasses for missing desktop
permissions or portal consent.
