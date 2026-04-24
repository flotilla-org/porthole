#!/usr/bin/env bash
set -euo pipefail
REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$REPO_ROOT"
./scripts/dev-bundle.sh
codesign -v target/debug/Portholed.app
./target/debug/Portholed.app/Contents/MacOS/portholed --help > /dev/null 2>&1 || true
echo "dev-bundle: ok"
