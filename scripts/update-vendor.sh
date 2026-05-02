#!/usr/bin/env bash
# Regenerate vendor.tar.xz and vendor.tar.xz.sha256 from the current Cargo.lock.
#
# Run this any time you add, remove, or update a dependency in Cargo.toml/Cargo.lock.
# The generated archive is NOT committed to git — it is attached to GitHub releases
# automatically by the release workflow. For air-gapped builds, download it from the
# release page and run: bash scripts/airgap-build.sh vendor.tar.xz
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
echo "Both files are gitignored. The release workflow uploads them automatically"
echo "when a version tag is pushed. To test an air-gapped build locally:"
echo "  bash scripts/airgap-build.sh"
