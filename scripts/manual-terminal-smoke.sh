#!/usr/bin/env bash
# Manual smoke test for porthole terminal orchestration.
# Exercises every verb a kitty-graphics-protocol harness would need:
# launch, focus, wait-stable, text, key, key+modifier, screenshot,
# scrollback (via Cmd+ArrowUp), place (reflow), close.
#
# Requires:
# - portholed running (via Porthole.app for stable TCC identity)
# - `porthole` on PATH (or invoke the absolute bundle path)
# - permissions granted (`porthole onboard` first)
# - a terminal app at $TERMINAL_APP (default /Applications/Ghostty.app)
# - jq for JSON parsing

set -euo pipefail

TERMINAL_APP="${TERMINAL_APP:-/Applications/Ghostty.app}"
OUT="${OUT:-/tmp/porthole-smoke}"

if ! command -v porthole >/dev/null; then
    echo "porthole not on PATH. Either install via the .app bundle's symlink or run:" >&2
    echo "    export PATH=\$PWD/target/debug/Porthole.app/Contents/MacOS:\$PATH" >&2
    exit 1
fi
if ! command -v jq >/dev/null; then
    echo "jq required for parsing launch JSON" >&2
    exit 1
fi
if [[ ! -d "$TERMINAL_APP" ]]; then
    echo "terminal app not found at $TERMINAL_APP" >&2
    echo "set TERMINAL_APP=/Applications/Terminal.app (or similar) and re-run" >&2
    exit 1
fi

mkdir -p "$OUT"
echo "smoke outputs -> $OUT"

# 1. Launch + focus + wait
SID=$(porthole launch --app "$TERMINAL_APP" --kind process --json | jq -r .surface_id)
echo "surface_id=$SID"
porthole focus "$SID"
porthole wait "$SID" --condition stable --window-ms 1500 --threshold-pct 1.0

# 2. Run a known command, screenshot the output
porthole text "$SID" 'printf "hello porthole\n"; seq 1 80'
porthole key  "$SID" --key Enter
porthole wait "$SID" --condition stable --window-ms 1500 --threshold-pct 1.0
porthole screenshot "$SID" --out "$OUT/01-after-output.png"
echo "  ✓ 01-after-output.png"

# 3. Scrollback via Cmd+ArrowUp
for _ in 1 2 3; do
    porthole key "$SID" --key ArrowUp --mod Cmd
done
porthole wait "$SID" --condition stable --window-ms 1000
porthole screenshot "$SID" --out "$OUT/02-scrolled.png"
echo "  ✓ 02-scrolled.png"

# 4. Reflow: narrow then wide
porthole place "$SID" --x 100 --y 100 --w 500 --h 800
porthole wait  "$SID" --condition stable --window-ms 1000
porthole screenshot "$SID" --out "$OUT/03-narrow.png"
echo "  ✓ 03-narrow.png"

porthole place "$SID" --x 100 --y 100 --w 1200 --h 800
porthole wait  "$SID" --condition stable --window-ms 1000
porthole screenshot "$SID" --out "$OUT/04-wide.png"
echo "  ✓ 04-wide.png"

# 5. Close
porthole close "$SID"
echo "done. four PNGs in $OUT"
