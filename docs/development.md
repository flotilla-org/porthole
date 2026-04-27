# Porthole development playbook

This covers first-time setup, day-to-day workflow, and what to do when grants go sideways.

## First-time setup

Porthole's macOS adapter needs **Accessibility** and **Screen Recording** system permissions. Grants are tied to a binary's code signature + path; the dev bundle gives a stable identity so grants persist across rebuilds.

```sh
git clone <repo>
cd porthole
cargo build --workspace --release
./scripts/dev-bundle.sh --release
open -R target/release/Porthole.app    # reveal in Finder
./target/release/Porthole.app/Contents/MacOS/portholed &
./target/release/Porthole.app/Contents/MacOS/porthole onboard
```

`porthole onboard` does three things:
1. Reads `/info` to see which permissions are ungranted.
2. Asks the daemon to trigger the OS prompt for each one (Settings opens automatically).
3. Polls until granted or 60 seconds pass, then prints a summary.

Exit codes:
- **0** — all granted already; no action taken.
- **1** — at least one permission still ungranted after waiting. Grant in Settings and re-run.
- **2** — all granted now, but **restart the daemon**. Accessibility grants need a process restart for the AX runtime to pick them up.
- **3** — `--no-wait` mode; prompts fired, grant state unknown.

## Rebuild workflow

Cargo replaces `target/<profile>/portholed` but the bundle's copy is stale. Two options:

```sh
./scripts/dev-bundle.sh --refresh    # re-copy and re-sign; keeps TCC grants
```

or just `cargo build` and run the binary from `target/<profile>/portholed` directly — but that's a *different* path from TCC's perspective, so you'll be prompted to grant again. Prefer the bundle.

## If grants get stuck

macOS's TCC database can report stale state after crashes, force-quits, or bundle-identity changes. Reset:

```sh
tccutil reset Accessibility org.flotilla.porthole.dev
tccutil reset ScreenCapture org.flotilla.porthole.dev
./scripts/dev-bundle.sh --refresh
./target/debug/Porthole.app/Contents/MacOS/portholed &
./target/debug/Porthole.app/Contents/MacOS/porthole onboard
```

## Debug vs release bundle

They're separate TCC identities. If you switch frequently, grant both. Or stick to one profile and refresh it on rebuild.

## Integration tests

Tests marked `#[ignore]` in `porthole-adapter-macos` run against a real desktop session. Execute with:

```sh
cargo test -p porthole-adapter-macos -- --ignored
```

These tests use whatever daemon is currently running (or spawn their own from `CARGO_BIN_EXE_portholed` — a different path and thus a different TCC identity). Run the bundled daemon for the realistic path.

## What *not* to do when permissions are missing

Per `AGENTS.md`: stop, state the missing permission, tell the user to run `porthole onboard`, wait. Do not build mock layers, feature flags, or "degrade to empty" paths. Preflight returns `system_permission_needed` with remediation — surface that, don't route around it.
