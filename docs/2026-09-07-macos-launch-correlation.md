# macOS launch correlation: #89

The macOS process-launch path now correlates windows with the PID returned by
`NSWorkspace` for app bundles, or by the spawned child for executable paths.
It no longer injects a correlation environment variable or reads `ps eww`.
On the test host, a marker supplied to a new `sleep` process was absent from
`ps eww`, reproducing the broken premise behind the previous implementation.

Bundle paths, bundle identifiers and application names use
`NSWorkspace.openApplication(at:configuration:completionHandler:)`. Arguments and
environment overrides are supplied through its launch configuration. Executable
paths launch directly and support a working directory. Bundle launches reject a
working directory with `adapter_unsupported`; the native configuration cannot set
one. Previously, the directory applied to the `open` helper rather than reliably
to the application.

Before launching, porthole snapshots window identities, including offscreen and
minimized windows. It selects a visible window owned by the returned PID and
prefers a new window. Multiple new windows return `launch_correlation_ambiguous`.
An existing window is eligible only at the deadline and is marked preexisting;
multiple existing windows are also ambiguous. The shared launch pipeline still
enforces `require_fresh_surface` and placement requirements.

A unique match reports strong confidence and the existing `pid_tree` correlation
value. This implementation matches only the root PID. Descendant traversal and
brokered terminals remain #10. A timeout stops waiting; it does not cancel an
application launch already submitted to macOS or terminate an application.

## Verification

Run the repeatable desktop check from the repository root:

```sh
python3 scripts/manual-macos-launch-smoke.py
```

It uses the installed signed bundle with Accessibility and Screen Recording
already granted. Missing permissions stop the check. The script builds a small
Cocoa fixture, creates a temporary agent using the existing local-trust operator
commands, approves only that agent's requests, and revokes it afterward. It closes
only the fixture surfaces and retains evidence in a printed temporary directory.

The live run on 2026-09-07 passed three launches: a bundle, a second instance of
the same bundle while the first remained open, and a direct executable with an
explicit working directory. Each returned a distinct PID and a fresh surface.
The fixture confirmed arguments and environment values containing spaces; the
executable also confirmed its working directory. Each window accepted text and
produced a screenshot. The second screenshot was visually checked and showed the
expected input. Evidence directory for this run:

```text
/var/folders/6z/8gmpf02s6gz_9zfgj292bpy00000gn/T/porthole-launch-smoke-q1s3fmsc
```

Deterministic tests cover PID matching, fresh-window preference, delayed reuse,
ambiguity and unrelated windows. Adapter tests also cover launch configuration,
working-directory validation and missing application resolution. The workspace
build, tests, Clippy and pinned format checks passed.

This verifies the launch dependency of #115. The terminal/cleat/agent workflow,
including attachment from a separate client terminal, still needs its own proof.
