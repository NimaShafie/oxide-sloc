#!/usr/bin/env bash
# Air-gapped build setup for oxide-sloc.
#
# On a network-connected machine, download the vendor archive for the desired
# release from https://github.com/oxide-sloc/oxide-sloc/releases alongside the
# source tarball, then transfer both to the air-gapped system.
#
# On the air-gapped system, run:
#   bash scripts/airgap-build.sh [vendor.tar.xz]
#
# Requirements: Rust toolchain (see rust-toolchain.toml), tar, sha256sum.
set -euo pipefail

ARCHIVE="${1:-vendor.tar.xz}"

if [ ! -f "$ARCHIVE" ]; then
    echo "ERROR: vendor archive not found: $ARCHIVE"
    echo "Download it from the GitHub release page alongside the source tarball."
    exit 1
fi

echo "==> Verifying vendor archive checksum..."
if [ -f "${ARCHIVE}.sha256" ]; then
    sha256sum -c "${ARCHIVE}.sha256"
else
    echo "WARNING: ${ARCHIVE}.sha256 not found — skipping checksum verification."
fi

echo "==> Extracting vendor archive..."
tar -xJf "$ARCHIVE"

echo "==> Configuring cargo to use vendor directory..."
mkdir -p .cargo
cat > .cargo/config.toml <<'EOF'
[source.crates-io]
replace-with = "vendored-sources"

[source.vendored-sources]
directory = "vendor"
EOF

echo "==> Building oxide-sloc (offline)..."
cargo build --release -p oxide-sloc

echo ""
echo "Done. Binary: target/release/oxide-sloc"
