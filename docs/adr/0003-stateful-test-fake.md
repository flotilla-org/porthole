# Stateful test fake over per-method scripting

Porthole's test fake is a stateful in-memory model of the desktop, not a
per-method script of mocked return values. `MemoryAdapter` (in
`crates/porthole-core/src/memory_adapter/`) holds real windows, displays,
focus and cursor state; operations mutate that state. Tests assert against
observable state — "after `place_surface`, the window's `outer_rect` is X" —
rather than against "was the `place_surface` method called with X".

The original `InMemoryAdapter` accumulated a script-record-setter triple
per trait method as the trait grew. That shape made tests recite the
adapter's internal call shape rather than describe what the system did.
Migration is gradual: both fakes coexist, new tests use `MemoryAdapter`,
existing tests migrate when touched.

## Considered alternatives

- **Keep extending `InMemoryAdapter`** — every new trait method adds three
  pieces of code that all do the same thing. Tests grow recitation-heavy.
- **Per-op hook closures** (`set_op_hook(|op, args| -> Option<Result<...>>)`)
  — maximum flexibility, but creates an escape hatch back to scripting-per-call
  habits.
- **Drop-in replacement (single PR)** — would touch ~50 test sites and
  re-think every scripted scenario at once; the diff becomes unreviewable.
  Rejected in favour of coexistence.

## Consequences

- **Tests compose.** `builder.window(...).build()` → `place_surface(...)` →
  `snapshot_geometry(...)` reads back what was placed. No scripting needed
  to thread expected values through.
- **Per-call error injection is given up.** State-based simulation covers
  most error paths (no AX grant → permission error; dead window →
  SurfaceDead). Genuinely transient errors that don't surface from state
  (`LaunchTimeout`, `SystemPermissionRequestFailed`) keep using
  `InMemoryAdapter` until a future decorator wrapper lands. See issue #35
  for the planned decorator design.
- **Recorders dropped.** Tests assert against state, not call lists. The
  rare "was this called" wiring test can build a one-purpose spy inline.
- **`MemoryAdapter` advertises `attention_focused_surface`.** Unlike
  `InMemoryAdapter` which excludes it, the new fake can actually resolve
  focused surface from state, and the capability list reflects that.
- **`InMemoryAdapter` stays until migration completes.** The deprecation
  criterion is "no test imports it"; at that point it can be deleted in a
  one-line PR.

## Migration pattern

For each migrating test:
1. Replace `Arc<InMemoryAdapter>` with `Arc<MemoryAdapter>`.
2. Replace setup-via-`SurfaceInfo::window` + `HandleStore::insert` with
   `MemoryAdapter::builder().window(...).build()` plus the same handle
   insert (the daemon-side `HandleStore` still owns lifecycle bookkeeping).
3. Replace `set_test_scale_for_snapshot(scale)` with
   `builder.display(DisplayInfo { scale, ... })`.
4. Replace `set_next_X` for state-deterministic operations with the
   appropriate builder/`WindowSpec` override.
5. Replace `X_calls()` recorder assertions with state reads
   (`adapter.window(&id).outer_rect`, `adapter.cursor()`, etc.).
6. Tests that genuinely require error injection stay on
   `InMemoryAdapter` and gain a `// see #35` breadcrumb.
