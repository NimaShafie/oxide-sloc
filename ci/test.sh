#!/usr/bin/env bash
set -euo pipefail

echo "==> Unit tests"
cargo test --workspace
