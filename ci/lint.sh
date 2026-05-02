#!/usr/bin/env bash
set -euo pipefail

echo "==> Format check"
cargo fmt --all -- --check

echo "==> Clippy"
cargo clippy --workspace --all-targets -- -D warnings
