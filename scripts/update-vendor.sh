#!/usr/bin/env bash
# Regenerate vendor.tar.xz and vendor.tar.xz.sha256 atomically.
# Run this any time you add, remove, or update a dependency in Cargo.toml/Cargo.lock.
# Never update vendor.tar.xz without running this script — the Docker build will fail
# checksum verification and the CI docker.yml workflow will break.
#
# Usage: bash scripts/update-vendor.sh
set -euo pipefail

ARCHIVE=vendor.tar.xz
SHA_FILE=vendor.tar.xz.sha256

echo "Removing old vendor directory..."
rm -rf vendor/

echo "Running cargo vendor..."
cargo vendor vendor/

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
echo "Stage both files before committing:"
echo "  git add ${ARCHIVE} ${SHA_FILE}"
