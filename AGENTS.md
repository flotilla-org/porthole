# AGENTS.md

Repo-specific guidance for agents working in porthole. Short and load-bearing — read before starting.

## Permissions-blocker rule

Porthole's macOS adapter depends on Accessibility and Screen Recording permissions. When a task hits a permission-dependent call and the permission isn't granted:

- **Do not** invent a code-level workaround — no mock layer, no feature flag to skip the call, no refactor-to-avoid, no "I'll just return empty for now." These always make the system worse.
- **Do** stop with status `BLOCKED`, state which permission is missing, and wait for the user to grant it (via `porthole onboard` or manually in System Settings → Privacy & Security).
- Resume exactly where you left off after the user confirms the grant.

Waiting is cheaper than invention. The permission dependency is deliberate — porthole's job is desktop orchestration and you cannot do that without the permissions the OS reserves for that purpose. Architectural retreats to "not need the permission" are almost always a mistake.

## CI gates

Before claiming a change is done, these four must all be clean:

```
cargo build --workspace --locked
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo +nightly-2026-03-12 fmt --check
```

The fmt check uses a pinned nightly so output is reproducible across machines and CI. To apply: `cargo +nightly-2026-03-12 fmt`.

`cargo test` runs only non-ignored tests. `#[ignore]`-gated integration tests need a real macOS desktop with permissions and are verified manually, not in CI.

`Cargo.lock` may show as locally modified from rebuilds — that's normal working-tree drift; do not commit it as part of an unrelated change.

## No-backwards-compat phase

Porthole is pre-release. Wire contracts, error shapes, and public types can change freely without migration logic or deprecation paths. Prefer clean renames and honest breaks over legacy shims.

## Where to look

- `docs/roadmap.md` — phase plan and what's next
- `docs/superpowers/specs/` — design specs, one per slice
- `docs/superpowers/plans/` — implementation plans derived from the specs
- `README.md` — agent-first usage guide with curl and CLI examples
- `docs/2026-04-20-window-evidence-experience-report.md` — the experience report that motivated porthole

## Testing conventions

- **Core tests (`porthole-core`)** run against the `InMemoryAdapter` — fully deterministic, no OS dependency.
- **Daemon tests (`portholed`)** use oneshot `axum::Router` calls with the in-memory adapter. No real daemon spawned except in `tests/*_e2e.rs` integration tests that spawn on a tempfile UDS.
- **macOS adapter integration tests** are `#[ignore]`'d by default and run with `cargo test -p porthole-adapter-macos -- --ignored`. They require a real desktop session and the two permissions granted.
