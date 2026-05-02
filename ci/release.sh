#!/usr/bin/env bash
# Usage: bash ci/release.sh [TARGET]
# Omit TARGET to build for the host. Provide a Rust target triple for cross-compilation.
set -euo pipefail

TARGET="${1:-}"

if [ -n "$TARGET" ]; then
    echo "==> Release build (target: $TARGET)"
    cargo build --release --target "$TARGET" -p oxide-sloc
else
    echo "==> Release build (host target)"
    cargo build --release -p oxide-sloc
fi
