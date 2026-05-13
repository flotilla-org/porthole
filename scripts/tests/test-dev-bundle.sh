#!/usr/bin/env bash
set -euo pipefail
REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$REPO_ROOT"

if ./scripts/dev-bundle.sh --adhoc > /tmp/porthole-dev-bundle-test-adhoc.out 2>&1; then
    cat /tmp/porthole-dev-bundle-test-adhoc.out >&2
    echo "expected dev-bundle.sh --adhoc to fail" >&2
    exit 1
fi
grep -q "not supported" /tmp/porthole-dev-bundle-test-adhoc.out || {
    cat /tmp/porthole-dev-bundle-test-adhoc.out >&2
    echo "expected --adhoc rejection error" >&2
    exit 1
}

if ! security find-identity -v -p codesigning 2>/dev/null | grep -q '"Apple Development:'; then
    if ./scripts/dev-bundle.sh --refresh > /tmp/porthole-dev-bundle-test.out 2>&1; then
        cat /tmp/porthole-dev-bundle-test.out >&2
        echo "expected dev-bundle.sh to fail without an Apple Development signing identity" >&2
        exit 1
    fi
    grep -q "Apple Development signing identity required" /tmp/porthole-dev-bundle-test.out || {
        cat /tmp/porthole-dev-bundle-test.out >&2
        echo "expected missing signing identity error" >&2
        exit 1
    }
    echo "dev-bundle: signing identity required"
    exit 0
fi

./scripts/dev-bundle.sh
codesign -v target/debug/Porthole.app
signature="$(codesign -dv target/debug/Porthole.app 2>&1 | sed -n 's/^Signature=//p')"
if [[ "$signature" == "adhoc" ]]; then
    echo "expected dev bundle to use Apple Development signing identity, got ad-hoc signature" >&2
    exit 1
fi
timeout 5 ./target/debug/Porthole.app/Contents/MacOS/portholed --help > /dev/null 2>&1 || true
./target/debug/Porthole.app/Contents/MacOS/porthole --help > /dev/null
test -f target/debug/Porthole.app/Contents/Resources/icon.png || { echo "icon.png missing from bundle" >&2; exit 1; }
echo "dev-bundle: ok"
