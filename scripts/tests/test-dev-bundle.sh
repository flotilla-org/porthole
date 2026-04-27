#!/usr/bin/env bash
set -euo pipefail
REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$REPO_ROOT"
./scripts/dev-bundle.sh
codesign -v target/debug/Porthole.app
./target/debug/Porthole.app/Contents/MacOS/portholed --help > /dev/null 2>&1 || true
./target/debug/Porthole.app/Contents/MacOS/porthole --help > /dev/null
test -f target/debug/Porthole.app/Contents/Resources/icon.png || { echo "icon.png missing from bundle" >&2; exit 1; }
echo "dev-bundle: ok"
