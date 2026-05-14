# Wire-type vocabulary is platform-neutral

Field names on porthole's HTTP wire surface (and in the platform-neutral
`porthole-core` types they project from) do not embed the host OS's
accessibility-surface name. The `ContentRectResponse.role` field was renamed
from `ax_role` once the leak was noticed: AX is a macOS-specific name, but
the abstract concept (the host accessibility tree's role string for a UI
element) recurs across every adapter porthole will eventually grow —
AT-SPI on Linux, UIAutomation on Windows, ARIA in webview shims.
Every one of them calls it "role"; only macOS calls it "AX role".

The rule going forward: when a public-API name comes from one specific
OS's vocabulary, ask whether the underlying *concept* is general. If it
is, name the wire field after the concept, and let each adapter populate
it with that host's flavor. Adapter-internal code is free to use the
OS-native terms.

## Consequences

- The macOS adapter still reads `AXRole` and stores the resulting string
  in `role`; readers of `role` see `"AXScrollArea"`, `"AXGroup"` etc.
  today. A future Linux adapter would store AT-SPI role names there.
  Clients that scripted around macOS-only values will need to broaden
  their parsing once a second adapter ships — but the *field name* won't
  have to change.
- This is also why `ContentRectInfo` lives in `porthole-core` rather than
  `porthole-adapter-macos`. Core types must survive being implemented by
  every adapter.
- Doc-comments on cross-cutting wire types should call out the adapter
  → concrete-value mapping explicitly (see `ContentRectInfo` doc-comment
  for the canonical phrasing).
