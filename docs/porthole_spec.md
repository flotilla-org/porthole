# porthole — Presentation Orchestration for Agents

## Purpose
A cross-platform tool to discover, create, arrange, and focus presentation surfaces (windows, tabs, panes, artifacts) without becoming an “uber app.” Intended to be useful standalone and as a substrate for systems like flotilla.

## Non-Goals
- Not a full desktop environment
- Not a general GUI testing framework
- Not universal accessibility automation
- Not a single unified windowing model

---

## Core Concepts

### Target
A desired thing to present.
Examples:
- URL (http://localhost:3000)
- File (video, logs, artifacts)
- Terminal pane/session (zellij/tmux)
- App-specific resource (Slack channel, editor file)

### Surface
A live presentation instance of a target.
- Window
- Tab
- Pane
- Viewer

### Slot
A place where surfaces are shown.
- Monitor region
- Window container
- Pane position
- Remote display

### Binding
Desired mapping of target → slot.

### Adapter
Backend for interacting with a system:
- macOS (AX + window APIs)
- Windows (UIA + Win32)
- Linux:
  - Hyprland (IPC)
  - KWin (DBus)
  - X11 (wmctrl/xdotool)
  - Generic Wayland (degraded)
- Browser (DevTools)
- Terminal mux (zellij/tmux)
- Remote (SSH/WebSocket)

---

## System Architecture

CLI / API / MCP
↓
Core planner (no OS calls)
↓
Adapters (per platform / tool)
↓
Native APIs

### Core Responsibilities
- Resolve targets into surfaces
- Decide reuse vs creation
- Bind surfaces to slots
- Reconcile actual vs desired state

### Adapter Responsibilities
- Discover surfaces
- Activate targets
- Move/resize/focus
- Report capabilities

---

## Capability Model

Capabilities are discovered per adapter:
- move
- focus
- inspect
- click (optional)
- type (optional)
- overlay (optional)

Commands must degrade gracefully.

---

## Operations Model

### Discover
What surfaces exist?

### Activate
Bring target into view (open, focus, switch tab)

### Enforce
Ensure correct placement/geometry

---

## API Shape (Examples)

- ensure target is visible in slot
- focus target
- stage artifact
- reveal logs without stealing focus

Return data:
- surface id
- match confidence
- location
- focus status
- stability (guaranteed vs best-effort)

---

## Surface Types

- Top-level windows
- Browser tabs (first-class)
- Terminal panes
- Artifact viewers
- Remote surfaces

No universal “tab” abstraction:
- use subsurface selectors instead

---

## Subsurface Selectors

- URL match
- Title match
- Accessibility chain
- DevTools selector
- Pane id
- File path
- Custom adapter-specific selectors

---

## Attention Model

- Manual (user focus)
- Derived (active pane)
- Requested (agent)
- Policy (no focus stealing, etc.)

---

## Event / State Tracking

Event-sourced style:
- intent
- resolution
- activation result
- reconciliation
- failures

Useful for:
- debugging
- replay
- agent reasoning

---

## Overlay System (Future Layer)

Separate subsystem.

### Concepts
- overlay
- anchor
- lifecycle

### Anchor Types
- screen coords
- window-relative
- semantic (selectors, panes)

### Rendering
- Global overlay window (primary)
- In-surface overlays (browser, etc.)

### Commands
- highlight element
- point to region
- annotate surface
- guided flows

Constraints:
- declarative overlays
- persistent IDs
- auto-expiry
- re-resolution of anchors

---

## Platform Reality

### macOS
- Strong AX + window APIs
- Good for discovery + control

### Windows
- UIA + Win32
- Capable but complex

### Linux
Focus on compositors:

#### Hyprland
- IPC (hyprctl)
- strong control

#### KWin
- DBus + scripting
- structured control

#### X11
- wmctrl / xdotool
- broad but legacy

#### Wayland (generic)
- restricted
- minimal support

---

## Strategy

- Do not “support Linux”
- Support specific compositors
- Accept degraded capabilities elsewhere

---

## Integration Model

Flotilla or agents provide:
- targets
- priorities
- policies

porthole provides:
- realization of presentation

---

## MVP Scope

Platforms:
- macOS
- Hyprland
- Browser (DevTools)

Features:
- discover windows
- open targets
- wait for surfaces
- move/focus windows
- switch browser tabs
- basic slot mapping

---

## Key Design Constraints

- Do not unify semantics across platforms
- Unify transport, shape, capabilities
- Keep API goal-oriented, not imperative
- Surfaces > windows
- Anchors > coordinates

---

## Long-Term Extensions

- overlay/annotation system
- remote multi-machine presentation
- record/replay integration
- richer semantic anchors
- MCP server interface
