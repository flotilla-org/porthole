# Platform surface refs are explicit

Porthole surfaces expose stable `SurfaceId`s to clients, but adapters need a
native identity for the underlying OS-level window. That native identity is now
modeled as a platform surface ref rather than leaking macOS `CGWindowID` through
core and protocol types, because KWin and later adapters do not share macOS's
window id shape.

We prefer an explicit typed ref over a single generic string so core code cannot
accidentally compare identities from different adapters. Adapter-internal code
may still use native names such as `CGWindowID`; shared types should name the
concept, not the platform-specific representation.
