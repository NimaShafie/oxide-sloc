#!/usr/bin/env bash
# Regenerate vendor.tar.xz and vendor.tar.xz.sha256 from the current Cargo.lock.
#
# Run this any time you add, remove, or update a dependency in Cargo.toml/Cargo.lock,
# then stage and commit BOTH generated files:
#
#   git add vendor.tar.xz vendor.tar.xz.sha256
#   git commit -m "chore: update vendor archive"
#
# Both files are tracked by git so that a plain `git clone` is sufficient for a fully
# offline (air-gapped) build — no separate download step required.
#
# Usage: bash scripts/update-vendor.sh
set -euo pipefail

ARCHIVE=vendor.tar.xz
SHA_FILE=vendor.tar.xz.sha256

echo "Removing old vendor directory..."
rm -rf vendor/

echo "Running cargo vendor..."
# --sync ci/tools/Cargo.toml pulls CI tooling (cargo-llvm-cov, etc.) into the
# vendor archive so they can be installed offline on air-gapped agents.
cargo vendor --sync ci/tools/Cargo.toml vendor/

echo "Packing ${ARCHIVE} (xz compression, deterministic sort)..."
# Use LC_ALL=C + sorted find output for a reproducible archive on any OS.
LC_ALL=C tar \
  --sort=name \
  --mtime="@0" \
  --owner=0 --group=0 --numeric-owner \
  -cJf "${ARCHIVE}" vendor/

echo "Writing ${SHA_FILE}..."
sha256sum "${ARCHIVE}" > "${SHA_FILE}"

echo "Done."
echo "  Archive : ${ARCHIVE}"
echo "  Checksum: $(cat ${SHA_FILE})"
echo ""
echo "Stage and commit both files:"
echo "  git add vendor.tar.xz vendor.tar.xz.sha256"
echo "  git commit -m \"chore: update vendor archive\""
