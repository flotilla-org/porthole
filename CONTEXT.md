# Porthole

Porthole exposes a small HTTP API for inspecting and driving desktop windows from
test harnesses and orchestration tools. It is intended to be cross-platform:
**macOS first** (today's implementation), with **Linux compositor adapters**
(KWin first, then other compositors such as Hyprland as separate adapters) and
**Windows** to follow. The primary near-term consumers are terminal-emulator
test harnesses (kitty-image-tests and similar); the eventual consumer is
**flotilla**, an agent fleet controller.

The vocabulary below is intentionally platform-neutral; macOS-specific
realisations are flagged inline. New adapters should land on the same
vocabulary, even when the underlying OS surface differs.

## Language

### Product identity

**Product identity domain**:
The canonical reverse-DNS root for Porthole product identifiers is
`work.flotilla`, derived from `flotilla.work`.
_Avoid_: org.flotilla, flotilla.org.

### Identity

**Surface**:
Porthole's handle for a tracked window. Carries a stable `SurfaceId` and
remembers the OS-level identifiers needed to operate on the window after it
moves or refocuses. The OS-level identifier set is platform-dependent (macOS:
pid + CGWindowID; X11: window XID; Wayland: handle from the compositor; Windows:
HWND). Clients never see these directly.
_Avoid_: window-handle, target, window-id.

**SurfaceId**:
Opaque string identifier minted by porthole when a surface is attached. Used in
every command path.
_Avoid_: handle, ref, token.

**Platform surface ref**:
The adapter-owned identity for the OS-level window behind a Surface. It is
platform-specific and kept below the public SurfaceId contract.
_Avoid_: CGWindowID, window id, native id.

**Platform adapter**:
The crate that implements porthole's surface operations against a specific OS.
Today only `porthole-adapter-macos` exists; future siblings will cover KWin,
Hyprland, and Windows. The porthole-core API is what every adapter must satisfy.
_Avoid_: backend, driver.

**Linux compositor adapter**:
A platform adapter for one Linux compositor family, not for Linux as a whole.
The first target is KWin on Plasma Wayland; other compositors are separate
adapter targets with their own capability maps.
_Avoid_: Linux adapter, generic Linux support.

**KWin control script**:
The compositor-hosted companion script used by the KWin adapter to discover and
manage KWin windows through KWin's own scripting environment.
_Avoid_: helper script, window script, Linux daemon.

**KWin bridge service**:
The daemon-owned session D-Bus service that the KWin control script calls to
publish compositor state and receive queued compositor commands.
_Avoid_: public transport, script service, Linux bus.

**Compositor plane**:
The part of a Linux compositor adapter that owns compositor-native window
identity, focus, placement, close, geometry, and attention state.
_Avoid_: window plane, KWin API.

**Portal capture plane**:
The part of a Linux compositor adapter that captures screen or window imagery
through the desktop portal and PipeWire permission model.
_Avoid_: screenshot plane, PipeWire adapter.

**Input plane**:
The part of a Linux compositor adapter that injects keyboard, pointer, and text
input through the desktop portal or EIS permission model.
_Avoid_: drive plane, uinput layer.

**Portal consent**:
User approval brokered by a Linux desktop portal for a specific capability or
session, such as remote desktop input or screen capture. It is not assumed to
be a durable system permission.
_Avoid_: Linux TCC, portal permission, permanent grant.

### Capture and transfer

**Jackstay**:
The single-producer, multi-consumer broadcast transfer ring — shared memory for
the hot path, a setup socket for one-time handle/fd passing — that streams
captured surfaces (and, later, structured events) to heterogeneous consumers
(native panes, browsers, terminals). Today it is the in-repo `capture-transfer`
crate; it is to be extracted as a standalone, language-neutral library. Porthole
is a **producer/consumer integration** on top of jackstay, not the owner of its
protocol semantics.
_Avoid_: capture-transfer (as the long-term name), the ring, transfer channel.

**Native handle path**:
The primary jackstay transport: the producer passes an OS-native GPU surface
handle (macOS `IOSurface`, Linux dmabuf, Windows D3D-shared) plus an explicit
sync primitive, so a capable local consumer presents the frame zero-copy. This
is the target model, not an optimisation — "lowest possible overhead" is the
point of jackstay.
_Avoid_: zero-copy mode, GPU path, fast path.

**Pixel-streaming fallback**:
The degraded jackstay transport for consumers that cannot receive a native
handle (ssh, tmux, terminals without the handle-passing extension, the
conformance harness). Same producer code; only consumer behaviour differs. It is
a fallback, not the baseline.
_Avoid_: CPU path, shm baseline, software path.

**Producer class**:
A category of jackstay producer defined by where in the pipeline it taps the
imagery. **At-source** capture (injecting into an app and grabbing its own GPU
output before composite) is the lowest-overhead path but only reaches injectable
apps; **post-composite** capture (through the OS compositor — ScreenCaptureKit,
PipeWire) reaches any window but pays the compositor tax. Both publish into the
same ring.
_Avoid_: capture mode, source type.

**Consumer class**:
A category of jackstay consumer defined by its latency and handle-receiving
capability. A native pane and a pty-bound terminal are different classes and must
not be treated alike; a slow terminal consumer must be lappable without affecting
a fast native one.
_Avoid_: subscriber type, sink kind.

### Coordinate units

**Logical point**:
A scale-independent unit (Cocoa native on macOS; Wayland's "logical pixel"; CSS
px on Windows). The default coordinate unit across the porthole API. On a 2×
HiDPI display, one logical point typically spans two physical pixels.
_Avoid_: pt, screen-pt, native-pt.

**Physical pixel**:
A pixel on the actual display surface. Used as the input unit when the caller
sourced coordinates from APIs that report physical pixels (terminal capability
queries, screenshot dimensions). Opt-in via `--units physical` on input
commands.
_Avoid_: device-px, hardware-px, raw-px.

### Coordinate frames

**Window-local coords**:
Points measured from the window's outer origin. Used as the input frame for any
command that targets a point *inside* a specific window.
_Avoid_: surface-local, window-relative, inner coords.

**Screen-global coords**:
Points measured from the primary display's origin. Used as the input frame for
commands that place a window in space, since the caller's mental model is
already "where does this window go on the desktop".
_Avoid_: desktop coords, global, world coords.

### Rectangles inside a window

**Content rect**:
The OS-reported "what counts as the content of this window" rectangle,
expressed in window-local coordinates. Excludes window chrome the OS owns
(title bar, borders). **Includes** any internal padding the app itself draws
(e.g. a terminal's `window_padding_width`) because that padding lives below the
accessibility tree's resolution.

On macOS this is sourced from the AX content child (try `AXContents` attribute
first, fall back to largest non-zero `AXChildren` element by area). On Linux
and Windows the source will differ (AT-SPI tree on Linux, UIAutomation tree on
Windows); the abstract concept is the same.
_Avoid_: inner rect, viewport, client area.

**Cell grid** *(not a porthole concept; defined here to forestall conflation)*:
The terminal-specific grid of character cells rendered inside the content rect.
Porthole has no view into the cell grid; clients that care about it own the
math, typically using `cell_size_px` from terminal capability queries and
subtracting the terminal's known internal padding from the content rect.
_Avoid_: text grid, char grid, terminal rect.

**Chrome padding** *(client-side concept; not in any porthole response)*:
The gap between a window's outer frame and the area that matters to the client.
Different callers compute it differently. Porthole does not expose a single
"chrome padding" number — `content-rect` gives the inner rect and the client
subtracts.
_Avoid_: window-decoration, border, frame-pad.

## Relationships

- A **Surface** wraps exactly one OS-level window, identified by
  platform-specific identifiers held inside the **Platform adapter**.
- A **Surface**'s outer rect is in **Screen-global coords**; its **Content rect** is in **Window-local coords**.
- A **Display** has a scale factor that converts **Logical points** ↔
  **Physical pixels** for any window currently on that display. A window's
  scale can change mid-flight if it moves to a different display.

## Per-command frame convention

Different commands use different coordinate frames *intentionally*. The frame matches what the command is about:

- **Input that targets a point inside a window** — `click`, `scroll`, `text` (and future `pointer move`) — takes **window-local** coords. The adapter converts to screen-global before posting input events.
- **Commands that move a window** — `place` — takes **screen-global** coords.
- **Commands that report inner geometry** — `content-rect` — returns **window-local** coords, matching the frame the inner-targeting commands consume.

See [ADR-0001](./docs/adr/0001-per-command-coordinate-frame.md) for the rationale.

## Example dialogue

> **Dev:** "If I want to move the kitty window to (100, 100) and then click in its top-left cell, what frames am I in?"
>
> **Domain expert:** "Two different ones. `porthole place --rect 100,100,1400,900` puts the **outer frame** at **screen-global** (100, 100). Then `porthole click --x 4 --y 4` clicks at **window-local** (4, 4) — porthole adds the window's current outer origin internally. You never write `1404, 104`."

> **Dev:** "What about `content-rect`?"
>
> **Domain expert:** "Window-local — same frame as `click`/`scroll`. So `content_rect.x + col * cell_w_logical` is a valid window-local x you can feed straight into `click`. You don't add the window origin."

> **Dev:** "On Linux, will `content-rect` still mean the same thing?"
>
> **Domain expert:** "Same abstract concept, different source. macOS reads it from the AX tree; the Linux adapter will read it from AT-SPI or the compositor protocol depending on the WM. The vocabulary stays the same so callers don't have to relearn anything when porting tests."

## Flagged ambiguities

- "content rect" vs "cell grid" — initially used interchangeably in tickets. Resolved: **Content rect** is porthole's accessibility-derived rect; **Cell grid** is the terminal's renderer-internal grid, not part of porthole's API.
- "global coords" — used loosely to mean both screen-global and "not surface-specific". Resolved: prefer **Screen-global coords**.
- "AX" / "accessibility" — used in implementation context on macOS but does not belong in the public vocabulary because the Linux/Windows source isn't AX. Talk about the **Content rect** without saying how it was sourced.
