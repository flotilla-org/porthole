#!/usr/bin/env bash
set -euo pipefail

args=(bundle --platform macos)
while [[ $# -gt 0 ]]; do
    case "$1" in
        --release) args+=(--release); shift ;;
        --refresh) args+=(--refresh); shift ;;
        --sign)
            if [[ $# -lt 2 ]]; then
                echo "--sign requires an identity" >&2
                exit 1
            fi
            args+=(--sign "$2")
            shift 2
            ;;
        --adhoc)
            echo "--adhoc is not supported for Porthole dev bundles." >&2
            echo "Ad-hoc signatures change designated requirement on rebuild and invalidate TCC grants." >&2
            exit 1
            ;;
        -h|--help)
            cargo xtask bundle --help
            exit 0
            ;;
        *) echo "unknown arg: $1" >&2; exit 1 ;;
    esac
done

exec cargo xtask "${args[@]}"
