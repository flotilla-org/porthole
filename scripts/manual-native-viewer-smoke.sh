#!/usr/bin/env bash
# Manual smoke test for the macOS native (IOSurface/Metal) capture viewer (#85).
#
# Exercises the full native chain: SCK live capture -> IOSurface staged into
# the producer pool -> ring publication -> XPC attach over portholed's named
# mach service -> the viewer presents zero-copy with a GPU fence wait.
#
# Requires:
# - portholed running from Porthole.app under launchd, so it owns the
#   `work.flotilla.porthole.attach` MachServices name (a plain `cargo run`
#   binary cannot register it). Install with `porthole install`.
# - Accessibility and Screen Recording granted (`porthole onboard`).
# - An agent token the daemon will honour for the protected capture route,
#   in PORTHOLE_AGENT_TOKEN (see `porthole agents`), with the surface
#   Observe+Record grant approved.
# - porthole, jq, cargo, cmake, and SDL build dependencies.
#
# Watch for: live frames updating as you interact with the window; no tearing
# while dragging/resizing it (GPU fence wait); kill portholed mid-run and the
# viewer should hold a placeholder rather than crash.

set -euo pipefail

FRAMES="${FRAMES:-600}"
SURFACE_ID="${SURFACE_ID:-}"
SKIP_BUILD=0

usage() {
    cat <<EOF
Usage: $0 [--surface-id ID] [--frames N] [--skip-build]

  --surface-id ID  Use an existing porthole surface id instead of attaching
                   the current frontmost window.
  --frames N       Exit the viewer after N presented frames (default: $FRAMES).
  --skip-build     Reuse an existing target/capture-viewer-sdl build.
EOF
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --surface-id)
            [[ $# -ge 2 ]] || { echo "--surface-id requires a value" >&2; usage; exit 1; }
            SURFACE_ID="$2"; shift 2 ;;
        --frames)
            [[ $# -ge 2 ]] || { echo "--frames requires a value" >&2; usage; exit 1; }
            FRAMES="$2"; shift 2 ;;
        --skip-build)
            SKIP_BUILD=1; shift ;;
        -h|--help)
            usage; exit 0 ;;
        *)
            echo "unknown arg: $1" >&2; usage; exit 1 ;;
    esac
done

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"

for cmd in porthole jq cmake; do
    if ! command -v "$cmd" >/dev/null; then
        echo "$cmd required" >&2
        exit 1
    fi
done

if ! porthole info | grep -q "system permission screen_recording: granted"; then
    echo "Screen Recording is not granted. Run: porthole onboard" >&2
    exit 1
fi

if [[ "$SKIP_BUILD" -eq 0 ]]; then
    # The dylib must carry the ft_native_* symbols, so build with backend-macos.
    cargo build -p capture-transfer --features backend-macos --locked
    cargo build -p porthole -p portholed --locked
    cmake -S tools/capture-viewer-sdl -B target/capture-viewer-sdl \
        -DCAPTURE_TRANSFER_LIB="$PWD/target/debug/libcapture_transfer.dylib"
    cmake --build target/capture-viewer-sdl
fi

if [[ -z "$SURFACE_ID" ]]; then
    SURFACE_ID="$(porthole attach --frontmost --json | jq -r .surface_id)"
fi

echo "surface_id=$SURFACE_ID"
descriptor="$(porthole capture-session surface "$SURFACE_ID" --native --json)"
printf '%s\n' "$descriptor"

mach_service="$(printf '%s\n' "$descriptor" | jq -r '.native.mach_service_name // empty')"
attach_token="$(printf '%s\n' "$descriptor" | jq -r '.native.attach_token // empty')"

if [[ -z "$mach_service" || -z "$attach_token" ]]; then
    echo "failed to parse native capture-session descriptor (is this a native session?)" >&2
    exit 1
fi

target/capture-viewer-sdl/capture-viewer-sdl \
    --native \
    --mach-service "$mach_service" \
    --token "$attach_token" \
    --frames "$FRAMES"
