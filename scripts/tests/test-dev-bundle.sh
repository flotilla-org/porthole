#!/usr/bin/env bash
set -euo pipefail
REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$REPO_ROOT"
./scripts/dev-bundle.sh
codesign -v target/debug/Porthole.app
if security find-identity -v -p codesigning 2>/dev/null | grep -q '"Apple Development:'; then
    signature="$(codesign -dv target/debug/Porthole.app 2>&1 | sed -n 's/^Signature=//p')"
    if [[ "$signature" == "adhoc" ]]; then
        echo "expected dev bundle to use Apple Development signing identity, got ad-hoc signature" >&2
        exit 1
    fi
fi
timeout 5 ./target/debug/Porthole.app/Contents/MacOS/portholed --help > /dev/null 2>&1 || true
./target/debug/Porthole.app/Contents/MacOS/porthole --help > /dev/null
test -f target/debug/Porthole.app/Contents/Resources/icon.png || { echo "icon.png missing from bundle" >&2; exit 1; }
echo "dev-bundle: ok"
