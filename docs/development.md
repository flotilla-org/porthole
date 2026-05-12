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
./target/release/Porthole.app/Contents/MacOS/porthole install --user --force
porthole onboard
```

Run the daemon via the LaunchAgent installed by `porthole install`, not by
starting `Porthole.app/Contents/MacOS/portholed` from a terminal. For
permission prompts, macOS tracks the responsible process; a terminal-launched
daemon can cause Privacy & Security to ask for Ghostty/Terminal permission
instead of Porthole permission.

`porthole onboard` walks through ungranted permissions one at a time:
1. Reads `/info` to see which permissions are ungranted.
2. For each one: fires the OS prompt, waits for you to press Enter once you've granted in Settings, restarts the daemon (`launchctl kickstart -k`) so its cached AX/SR trust state refreshes, then re-reads `/info` to verify.

Serial because TCC silently coalesces simultaneous prompt requests from one process and AX/SR trust state caches per-process; each grant needs its own daemon lifetime.

Exit codes:
- **0** — all granted and verified post-restart.
- **1** — at least one still missing (dismissed, or daemon not under launchd so we can't auto-verify), or a request to fire the prompt errored.
- **3** — `--no-wait` mode; prompts fired, no Enter wait, no restart, no verification — caller handles the rest.

## Rebuild workflow

Cargo replaces `target/<profile>/portholed` but the bundle's copy is stale. Two options:

```sh
./scripts/dev-bundle.sh --refresh    # re-copy and re-sign; keeps TCC grants
./target/debug/Porthole.app/Contents/MacOS/porthole install --user --force
```

The second command copies the refreshed bundle to `~/Applications/Porthole.app`
and restarts the LaunchAgent. Or just `cargo build` and run the binary from
`target/<profile>/portholed` directly — but that's a *different* path from
TCC's perspective and terminal-launched processes may be attributed to the
terminal. Prefer the installed bundle.

## If grants get stuck

macOS's TCC database can report stale state after crashes, force-quits, or bundle-identity changes. Reset:

```sh
tccutil reset Accessibility org.flotilla.porthole.dev
tccutil reset ScreenCapture org.flotilla.porthole.dev
./scripts/dev-bundle.sh --refresh
./target/debug/Porthole.app/Contents/MacOS/porthole install --user --force
porthole onboard
```

## Debug vs release bundle

They're separate TCC identities. If you switch frequently, grant both. Or stick to one profile and refresh it on rebuild.

## Integration tests

Tests marked `#[ignore]` in `porthole-adapter-macos` run against a real desktop session. Execute with:

```sh
cargo test -p porthole-adapter-macos -- --ignored
```

These tests use whatever daemon is currently running (or spawn their own from `CARGO_BIN_EXE_portholed` — a different path and thus a different TCC identity). Run the installed bundled daemon for the realistic path.

## Capture transfer SDL viewer

The first capture-transfer dogfood viewer lives in `tools/capture-viewer-sdl`.
It can run a synthetic producer in-process, or ask `portholed` to create a
synthetic daemon session and consume frames through the fd-passing transport.
Build and smoke-test the in-process path with:

```sh
cargo build -p capture-transfer -p porthole -p portholed --locked
cmake -S tools/capture-viewer-sdl -B target/capture-viewer-sdl \
  -DCAPTURE_TRANSFER_LIB="$PWD/target/debug/libcapture_transfer.dylib"
cmake --build target/capture-viewer-sdl
SDL_VIDEODRIVER=dummy target/capture-viewer-sdl/capture-viewer-sdl --frames 3
```

Smoke-test the daemon-backed path with:

```sh
runtime_dir="$(mktemp -d)"
PORTHOLE_RUNTIME_DIR="$runtime_dir" target/debug/portholed >"$runtime_dir/portholed.log" 2>&1 &
portholed_pid=$!

for _ in $(seq 1 50); do
  [ -S "$runtime_dir/porthole.sock" ] && break
  sleep 0.1
done

SDL_VIDEODRIVER=dummy target/capture-viewer-sdl/capture-viewer-sdl \
  --porthole-socket "$runtime_dir/porthole.sock" \
  --frames 3

kill "$portholed_pid"
rm -rf "$runtime_dir"
```

For the attach-only path, create the synthetic session separately and pass its
descriptor into the SDL viewer:

```sh
runtime_dir="$(mktemp -d)"
PORTHOLE_RUNTIME_DIR="$runtime_dir" target/debug/portholed >"$runtime_dir/portholed.log" 2>&1 &
portholed_pid=$!

for _ in $(seq 1 50); do
  [ -S "$runtime_dir/porthole.sock" ] && break
  sleep 0.1
done

descriptor="$(PORTHOLE_RUNTIME_DIR="$runtime_dir" target/debug/porthole capture-session synthetic)"
session_id="$(printf '%s\n' "$descriptor" | awk '/^session_id:/ { print $2 }')"

SDL_VIDEODRIVER=dummy target/capture-viewer-sdl/capture-viewer-sdl \
  --porthole-socket "$runtime_dir/porthole.sock" \
  --session-id "$session_id" \
  --frames 3

kill "$portholed_pid"
rm -rf "$runtime_dir"
```

For a real tracked surface, use ScreenCaptureKit-backed capture instead of the
synthetic producer:

```sh
surface_id="$(porthole attach --containing-pid $$ --frontmost --json | jq -r .surface_id)"
descriptor="$(porthole capture-session surface "$surface_id")"
session_id="$(printf '%s\n' "$descriptor" | awk '/^session_id:/ { print $2 }')"

target/capture-viewer-sdl/capture-viewer-sdl \
  --porthole-socket "$PORTHOLE_RUNTIME_DIR/porthole.sock" \
  --session-id "$session_id"
```

This path requires the normal porthole macOS permissions. If it returns
`system_permission_needed`, run `porthole onboard` for the daemon identity and
retry; do not route around the permission failure.

## What *not* to do when permissions are missing

Per `AGENTS.md`: stop, state the missing permission, tell the user to run `porthole onboard`, wait. Do not build mock layers, feature flags, or "degrade to empty" paths. Preflight returns `system_permission_needed` with remediation — surface that, don't route around it.
