#!/usr/bin/env bash
set -euo pipefail

echo "==> Build (debug)"
cargo build --workspace
