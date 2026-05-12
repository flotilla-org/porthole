#!/usr/bin/env bash
# Manual smoke test for real capture-transfer sessions.
#
# Requires:
# - portholed running from Porthole.app for stable TCC identity
# - Accessibility and Screen Recording granted (`porthole onboard`)
# - porthole, jq, cargo, cmake, and SDL build dependencies
#
# Default behavior attaches the current frontmost window. Click the target
# window first, or pass --surface-id if you already have a tracked surface.

set -euo pipefail

FRAMES="${FRAMES:-300}"
SURFACE_ID="${SURFACE_ID:-}"
SKIP_BUILD=0

usage() {
    cat <<EOF
Usage: $0 [--surface-id ID] [--frames N] [--skip-build]

  --surface-id ID  Use an existing porthole surface id instead of attaching
                   the current frontmost window.
  --frames N       Exit the viewer after N frames (default: $FRAMES).
  --skip-build     Reuse an existing target/capture-viewer-sdl build.
EOF
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --surface-id)
            if [[ $# -lt 2 ]]; then
                echo "--surface-id requires a value" >&2
                usage
                exit 1
            fi
            SURFACE_ID="$2"
            shift 2
            ;;
        --frames)
            if [[ $# -lt 2 ]]; then
                echo "--frames requires a value" >&2
                usage
                exit 1
            fi
            FRAMES="$2"
            shift 2
            ;;
        --skip-build)
            SKIP_BUILD=1
            shift
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            echo "unknown arg: $1" >&2
            usage
            exit 1
            ;;
    esac
done

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"

for cmd in porthole jq; do
    if ! command -v "$cmd" >/dev/null; then
        echo "$cmd required" >&2
        exit 1
    fi
done

if ! porthole info | grep -q "system permission accessibility: granted"; then
    echo "Accessibility is not granted. Run: porthole onboard" >&2
    exit 1
fi
if ! porthole info | grep -q "system permission screen_recording: granted"; then
    echo "Screen Recording is not granted. Run: porthole onboard" >&2
    exit 1
fi

if [[ "$SKIP_BUILD" -eq 0 ]]; then
    cargo build -p capture-transfer -p porthole -p portholed --locked
    cmake -S tools/capture-viewer-sdl -B target/capture-viewer-sdl \
        -DCAPTURE_TRANSFER_LIB="$PWD/target/debug/libcapture_transfer.dylib"
    cmake --build target/capture-viewer-sdl
fi

if [[ -z "$SURFACE_ID" ]]; then
    SURFACE_ID="$(porthole attach --frontmost --json | jq -r .surface_id)"
fi

echo "surface_id=$SURFACE_ID"
descriptor="$(porthole capture-session surface "$SURFACE_ID")"
printf '%s\n' "$descriptor"

session_id="$(printf '%s\n' "$descriptor" | awk '/^session_id:/ { print $2 }')"
porthole_socket="$(printf '%s\n' "$descriptor" | awk '/^porthole_socket:/ { print $2 }')"

if [[ -z "$session_id" || -z "$porthole_socket" ]]; then
    echo "failed to parse capture-session descriptor" >&2
    exit 1
fi

target/capture-viewer-sdl/capture-viewer-sdl \
    --porthole-socket "$porthole_socket" \
    --session-id "$session_id" \
    --frames "$FRAMES"
