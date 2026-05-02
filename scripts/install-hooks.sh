#!/usr/bin/env bash
# Point git at the project-local hooks directory.
# Run once after cloning: bash scripts/install-hooks.sh
set -euo pipefail
git config core.hooksPath .githooks
chmod +x .githooks/*
echo "Git hooks installed from .githooks/"
