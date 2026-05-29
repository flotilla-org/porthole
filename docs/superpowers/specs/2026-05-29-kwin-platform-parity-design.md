# KWin Platform Parity Design

Date: 2026-05-29
Status: draft from grill-with-docs session

## Context

Porthole is macOS-first today. The next platform target is KWin on Plasma
Wayland, not generic Linux and not Hyprland. Existing docs previously named
Hyprland as the first Linux adapter candidate; this slice reorders that plan
around the desktop environment currently available for dogfooding and around
KWin's compositor scripting surface.

Linux support is framed as compositor-specific adapters. The KWin adapter is
the first Linux compositor adapter. Hyprland, X11, and other Linux desktops are
separate future adapter targets with their own capability maps.

## Goals

- Reach useful macOS parity on KDE Plasma Wayland to the extent the platform
  allows.
- Preserve the public Porthole vocabulary: clients still use `SurfaceId`,
  logical points, window-local coordinates, screen-global coordinates, and the
  same route families.
- Use a KWin-hosted control script for compositor-native window discovery and
  management.
- Use Linux desktop security surfaces directly: xdg-desktop-portal, PipeWire,
  and EIS/libei where needed.
- Keep root out of the runtime design. Setup should be per-user wherever
  possible.
- Split parity into staged capabilities so unsupported operations report
  honest adapter capabilities.

## Non-Goals

- No generic Linux adapter.
- No Hyprland, X11, or Windows implementation in this roadmap.
- No Linux tray/status app or native notification approval UI in the KWin
  adapter parity chain.
- No durable portal-consent model in the first pass.
- No `/dev/uinput` bypass for input injection.
- No attempt to make KWin scripting own capture or input if portals/EIS are the
  platform security model for those capabilities.

## Product Identity

Porthole product identifiers should use the `flotilla.work` domain:

- macOS bundle id: `work.flotilla.porthole.dev` for dev builds
- macOS production bundle id: `work.flotilla.porthole`
- macOS LaunchAgent label: `work.flotilla.porthole`
- KWin D-Bus bridge service: `work.flotilla.Porthole.KWin`

The current `org.flotilla.*` identifiers are a pre-release leak and should be
renamed before new Linux platform work copies the old namespace.

## Capability Planes

### Compositor Plane

The compositor plane owns KWin-native window identity and management:

- window discovery
- active/focused window state
- focus
- placement
- close
- geometry snapshots
- attention state

The compositor plane is implemented by a KWin control script plus a daemon-owned
KWin bridge service. The control script runs inside KWin and observes
`workspace.windowList()`, window lifecycle signals, active-window changes, and
geometry changes. The script calls out to the daemon-owned D-Bus service because
KWin scripts can call D-Bus but do not provide a clean script-hosted D-Bus
service model.

### Input Plane

The input plane owns keyboard, text, click, scroll, and pointer movement. It
should use xdg-desktop-portal RemoteDesktop first, then EIS/libei if the portal
methods are insufficient or too session-scoped for Porthole's workflows.

Portal consent is not the same as macOS TCC. The first pass should model it as
runtime consent/session establishment, not as a durable `granted: true` system
permission.

### Portal Capture Plane

The portal capture plane owns screenshots and video capture. Screenshot support
should land before recording/live capture. On the current dogfood system, KWin
exposes `org.kde.KWin.ScreenShot2` with window/screen/area capture methods, and
the desktop portal exposes screenshot and screencast interfaces.

Recording/live capture should be a later PipeWire ScreenCast branch after
surface identity, screenshot capture, and portal consent semantics are proven.

## Platform Surface Identity

Core currently leaks macOS `CGWindowID` through `SurfaceInfo`, search refs,
handle lookup, and attention focus resolution. KWin does not have a CGWindowID.
Before the KWin adapter grows real behavior, core should introduce an explicit
platform surface ref: an adapter-owned identity for the OS-level window behind a
Porthole `Surface`.

The preferred shape is typed rather than stringly so core cannot accidentally
compare identities from different adapters. See
`docs/adr/0003-platform-surface-ref.md`.

## KWin Control Script

The KWin control script is packaged under `apps/linux/kwin/` and installed
per-user with:

```sh
kpackagetool6 --type KWin/Script --install <package>
```

The script should be installable, upgradeable, removable, and status-checkable
through explicit `porthole kwin ...` commands before it is folded into generic
`porthole install` or `porthole onboard`.

The script publishes snapshots/events to the KWin bridge service and receives
commands by polling or long-polling daemon methods, for example:

- `PublishSnapshot(...)`
- `NextCommand(script_instance_id)`
- `CompleteCommand(command_id, result)`

The exact D-Bus payload shape can start as JSON if it keeps the spike cheap,
then tighten to typed D-Bus structs when the bridge stabilizes.

## Launch Correlation

KWin launch correlation should come after the compositor plane can publish
window snapshots and before the platform is considered usable. The first
strategy:

- spawn the requested process and record pid/start time
- watch KWin-published new-window events
- strong match when exactly one new KWin window belongs to the launched pid or
  descendant pid within the timeout
- plausible match when app id/resource class matches and the window becomes
  active shortly after launch

Artifact/document matching is out of scope for the first KWin pass unless KWin
or app-specific APIs expose enough metadata cheaply.

## Setup and Sudo

No known runtime setup for the KWin path requires sudo:

- KWin script installation is per-user.
- The KWin bridge service is on the user session bus.
- Portal consent is user-session interaction.
- PipeWire, xdg-desktop-portal-kde, KWin, and libei are already runtime
  services/libraries on the dogfood machine.

The one up-front sudo action was installing development headers for expected
Linux platform work:

```sh
sudo dnf install \
  pipewire-devel \
  wayland-devel \
  dbus-devel \
  libei-devel \
  at-spi2-core-devel \
  libxkbcommon-devel
```

Verified versions on the dogfood machine:

- `dbus-1` 1.16.0
- `wayland-client` 1.24.0
- `libpipewire-0.3` 1.4.11
- `libei-1.0` 1.4.1
- `atspi-2` 2.56.8
- `xkbcommon` 1.8.1

A Rust toolchain is also required before implementation branches can update
dependencies, regenerate `Cargo.lock`, run tests, or run the pinned nightly fmt
gate. On the dogfood machine used for the initial grill, `cargo`, `rustc`, and
`rustup` were not available on `PATH`; install them before starting the
`kwin-dbus-bridge` branch.

## Parity Boundary

Initial KWin parity includes:

- launch/search/track
- focus/place/geometry/attention/close
- key/text/click/scroll/pointer movement
- screenshot
- later recording/live capture

Initial KWin parity excludes:

- Linux tray/status app
- Linux notification action approvals
- durable portal consent
- Hyprland/X11 support
- browser tabs via CDP
