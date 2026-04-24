#!/usr/bin/env bash
# Build porthole and wrap `portholed` in a .app bundle with ad-hoc codesigning.
# The bundle gives TCC a stable identity across rebuilds, so grants stick.

set -euo pipefail

PROFILE="debug"
REFRESH_ONLY=0
BUNDLE_ID="org.flotilla.porthole.dev"

usage() {
    cat <<EOF
Usage: $0 [--release] [--refresh]

  --release   Build release profile (default: debug).
  --refresh   Don't rebuild; just re-copy the binary into the existing bundle
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

BIN="target/$PROFILE/portholed"
if [[ ! -f "$BIN" ]]; then
    echo "missing binary: $BIN" >&2
    exit 1
fi

APP="target/$PROFILE/Portholed.app"
mkdir -p "$APP/Contents/MacOS"

cat > "$APP/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleIdentifier</key>
    <string>$BUNDLE_ID</string>
    <key>CFBundleName</key>
    <string>Portholed</string>
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
</dict>
</plist>
EOF

cp "$BIN" "$APP/Contents/MacOS/portholed"
chmod +x "$APP/Contents/MacOS/portholed"

codesign -s - --force --deep "$APP"

echo "bundle built: $APP"
echo "launch it: \"$APP/Contents/MacOS/portholed\""
echo "run onboarding: ./target/$PROFILE/porthole onboard"
