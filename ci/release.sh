#!/usr/bin/env bash
# Usage: bash ci/release.sh [TARGET]
# Omit TARGET to build for the host. Provide a Rust target triple for cross-compilation.
set -euo pipefail

TARGET="${1:-}"

# Cross-compiled Linux targets (musl, aarch64) have no sysroot for wayland-client
# or GTK, and they run as headless servers, so disable the native file-dialog feature.
EXTRA_FLAGS=""
if [[ "$TARGET" == *"-unknown-linux-musl"* || "$TARGET" == "aarch64-unknown-linux-gnu" ]]; then
    EXTRA_FLAGS="--no-default-features"
fi

if [ -n "$TARGET" ]; then
    echo "==> Release build (target: $TARGET)"
    # shellcheck disable=SC2086
    cargo build --release --target "$TARGET" -p oxide-sloc $EXTRA_FLAGS
else
    echo "==> Release build (host target)"
    cargo build --release -p oxide-sloc
fi
