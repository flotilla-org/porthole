#!/usr/bin/env bash
# Build porthole and wrap `portholed` + `porthole` in a single .app bundle.
# Both binaries inside the same bundle share TCC identity: one Privacy &
# Security entry covers the daemon, and the CLI inherits the same bundle
# context for free.

set -euo pipefail

PROFILE="debug"
REFRESH_ONLY=0
BUNDLE_ID="org.flotilla.porthole.dev"
SIGN_IDENTITY=""
FORCE_ADHOC=0

usage() {
    cat <<EOF
Usage: $0 [--release] [--refresh] [--sign IDENTITY] [--adhoc]

  --release   Build release profile (default: debug).
  --refresh   Don't rebuild; just re-copy the binaries into the existing bundle
              and re-sign. Use after cargo build to keep TCC grants.
  --sign      Codesign with the given identity.
  --adhoc     Force ad-hoc signing, even if an Apple Development identity exists.
EOF
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --release) PROFILE="release"; shift ;;
        --refresh) REFRESH_ONLY=1; shift ;;
        --sign)
            if [[ $# -lt 2 ]]; then
                echo "--sign requires an identity" >&2
                usage
                exit 1
            fi
            SIGN_IDENTITY="$2"
            shift 2
            ;;
        --adhoc) FORCE_ADHOC=1; shift ;;
        -h|--help) usage; exit 0 ;;
        *) echo "unknown arg: $1" >&2; usage; exit 1 ;;
    esac
done

if [[ "$FORCE_ADHOC" -eq 1 && -n "$SIGN_IDENTITY" ]]; then
    echo "--sign and --adhoc are mutually exclusive" >&2
    usage
    exit 1
fi

choose_sign_identity() {
    if [[ "$FORCE_ADHOC" -eq 1 ]]; then
        echo "-"
        return
    fi
    if [[ -n "$SIGN_IDENTITY" ]]; then
        echo "$SIGN_IDENTITY"
        return
    fi

    security find-identity -v -p codesigning 2>/dev/null \
        | sed -n 's/^[[:space:]]*[0-9]*) [A-Fa-f0-9]* "\(Apple Development:.*\)"$/\1/p' \
        | head -n 1
}

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
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"

cp assets/icon.png "$APP/Contents/Resources/icon.png"

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
    <key>CFBundleIconFile</key>
    <string>icon</string>
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

SIGNING_IDENTITY="$(choose_sign_identity)"
if [[ -z "$SIGNING_IDENTITY" ]]; then
    SIGNING_IDENTITY="-"
fi

codesign -s "$SIGNING_IDENTITY" --force --deep "$APP"

echo "bundle built: $APP"
if [[ "$SIGNING_IDENTITY" == "-" ]]; then
    echo "signed with:      ad-hoc"
else
    echo "signed with:      $SIGNING_IDENTITY"
fi
echo "install/restart:   \"$APP/Contents/MacOS/porthole\" install --user --force"
echo "grant permissions: porthole onboard"
echo
echo "Do not launch portholed directly from a terminal for onboarding; macOS may"
echo "attribute TCC prompts to the terminal app instead of Porthole."
