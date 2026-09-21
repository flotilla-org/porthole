#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")"
mkdir -p .build/PortholeAuthorizationPrototype.bundle/Contents/MacOS
xcrun clang -Wall -Wextra -Werror -bundle -framework Security Mechanism.c -o .build/PortholeAuthorizationPrototype.bundle/Contents/MacOS/PortholeAuthorizationPrototype
cp Info.plist .build/PortholeAuthorizationPrototype.bundle/Contents/Info.plist
xcrun swiftc -swift-version 5 -warnings-as-errors -framework Security -framework LocalAuthentication Probe.swift -o .build/authorization-probe
printf '%s\n' "Built .build/authorization-probe and custom authorization mechanism bundle. Nothing installed."
