#!/usr/bin/env bash
# Build porthole and wrap `portholed` + `porthole` in a single .app bundle with
# ad-hoc codesigning. Both binaries inside the same bundle share TCC identity:
# one Privacy & Security entry covers the daemon, and the CLI inherits the
# same bundle context for free.

set -euo pipefail

PROFILE="debug"
REFRESH_ONLY=0
BUNDLE_ID="org.flotilla.porthole.dev"

usage() {
    cat <<EOF
Usage: $0 [--release] [--refresh]

  --release   Build release profile (default: debug).
  --refresh   Don't rebuild; just re-copy the binaries into the existing bundle
              and re-sign. Use after cargo build to keep TCC grants.
EOF
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --release) PROFILE="release"; shift ;;
        --refresh) REFRESH_ONLY=1; shift ;;
        -h|--help) usage; exit 0 ;;
        *) echo "unknown arg: $1" >&2; usage; exit 1 ;;
    esac
done

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"

if [[ "$REFRESH_ONLY" -eq 0 ]]; then
    if [[ "$PROFILE" == "release" ]]; then
        cargo build --workspace --release
    else
        cargo build --workspace
    fi
fi

DAEMON_BIN="target/$PROFILE/portholed"
CLI_BIN="target/$PROFILE/porthole"
for bin in "$DAEMON_BIN" "$CLI_BIN"; do
    if [[ ! -f "$bin" ]]; then
        echo "missing binary: $bin" >&2
        exit 1
    fi
done

APP="target/$PROFILE/Porthole.app"
mkdir -p "$APP/Contents/MacOS"

cat > "$APP/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleIdentifier</key>
    <string>$BUNDLE_ID</string>
    <key>CFBundleName</key>
    <string>Porthole</string>
    <key>CFBundleExecutable</key>
    <string>portholed</string>
    <key>CFBundleVersion</key>
    <string>0.0.0-dev</string>
    <key>CFBundleShortVersionString</key>
    <string>0.0.0-dev</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>LSBackgroundOnly</key>
    <true/>
    <key>NSAccessibilityUsageDescription</key>
    <string>Porthole needs Accessibility to inject input and inspect window state.</string>
    <key>NSScreenCaptureUsageDescription</key>
    <string>Porthole needs Screen Recording to capture window screenshots and detect frame changes.</string>
</dict>
</plist>
EOF

cp "$DAEMON_BIN" "$APP/Contents/MacOS/portholed"
cp "$CLI_BIN"    "$APP/Contents/MacOS/porthole"
chmod +x "$APP/Contents/MacOS/portholed" "$APP/Contents/MacOS/porthole"

codesign -s - --force --deep "$APP"

echo "bundle built: $APP"
echo "launch the daemon: \"$APP/Contents/MacOS/portholed\""
echo "run the CLI:       \"$APP/Contents/MacOS/porthole\" onboard"
