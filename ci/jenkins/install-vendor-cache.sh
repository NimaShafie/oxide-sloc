#!/usr/bin/env bash
# Pre-stage the vendor.tar.xz archive on the Jenkins agent's persistent cache
# so that offline (air-gapped) builds never have to reach crates.io.
#
# The archive is NOT committed to git — it is attached to each GitHub release.
# Download it once on any machine that can reach GitHub, then call this script
# to place it at the path the Jenkinsfile's Setup stage checks first.
#
# Usage:
#   # Option A — download from GitHub releases (requires internet, run once):
#   bash ci/jenkins/install-vendor-cache.sh --download vX.Y.Z
#
#   # Option B — use a local copy you already have:
#   bash ci/jenkins/install-vendor-cache.sh /path/to/vendor.tar.xz
#
#   # Option C — regenerate from the current workspace (requires Cargo online once):
#   bash scripts/update-vendor.sh
#   bash ci/jenkins/install-vendor-cache.sh vendor.tar.xz
#
# After staging, transfer the persistent cache directory to the air-gapped host:
#   tar -czf rust-cache.tar.gz -C ~ .rust-cache
#   # Copy to agent host, then:
#   tar -xzf rust-cache.tar.gz -C /var/lib/docker/volumes/<jenkins_home_volume>/_data
#
# The Jenkinsfile expects the archive at:
#   /var/jenkins_home/.rust-cache/vendor.tar.xz
#   /var/jenkins_home/.rust-cache/vendor.tar.xz.sha256  (optional but recommended)

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
CACHE_DIR="${HOME}/.rust-cache"
DEST_ARCHIVE="${CACHE_DIR}/vendor.tar.xz"
DEST_SHA="${CACHE_DIR}/vendor.tar.xz.sha256"

mkdir -p "${CACHE_DIR}"

# ── Argument handling ─────────────────────────────────────────────────────────
if [ "${1:-}" = "--download" ]; then
    TAG="${2:-}"
    if [ -z "${TAG}" ]; then
        echo "Usage: $0 --download vX.Y.Z" >&2
        exit 1
    fi
    BASE="https://github.com/oxide-sloc/oxide-sloc/releases/download/${TAG}"
    echo "Downloading vendor.tar.xz from release ${TAG}..."
    curl -fL --retry 3 --retry-delay 2 -o "${DEST_ARCHIVE}" "${BASE}/vendor.tar.xz"
    echo "Downloading vendor.tar.xz.sha256..."
    curl -fL --retry 3 --retry-delay 2 -o "${DEST_SHA}"    "${BASE}/vendor.tar.xz.sha256"
else
    SRC_ARCHIVE="${1:-}"
    if [ -z "${SRC_ARCHIVE}" ]; then
        # Check if it already exists in the workspace root
        if [ -f "${REPO_ROOT}/vendor.tar.xz" ]; then
            SRC_ARCHIVE="${REPO_ROOT}/vendor.tar.xz"
        else
            echo "Usage: $0 /path/to/vendor.tar.xz" >&2
            echo "       $0 --download vX.Y.Z" >&2
            echo "" >&2
            echo "Generate a fresh archive first with:" >&2
            echo "  bash scripts/update-vendor.sh" >&2
            exit 1
        fi
    fi

    SRC_ARCHIVE="$(realpath "${SRC_ARCHIVE}")"
    if [ ! -f "${SRC_ARCHIVE}" ]; then
        echo "Error: archive not found: ${SRC_ARCHIVE}" >&2
        exit 1
    fi

    echo "Copying ${SRC_ARCHIVE} → ${DEST_ARCHIVE}..."
    cp "${SRC_ARCHIVE}" "${DEST_ARCHIVE}"

    # Copy the .sha256 file if it lives beside the source archive
    SRC_SHA="${SRC_ARCHIVE}.sha256"
    if [ -f "${SRC_SHA}" ]; then
        echo "Copying ${SRC_SHA} → ${DEST_SHA}..."
        cp "${SRC_SHA}" "${DEST_SHA}"
    else
        echo "No .sha256 beside source — computing checksum from copied archive..."
        sha256sum "${DEST_ARCHIVE}" > "${DEST_SHA}"
    fi
fi

# ── Verify the staged archive ──────────────────────────────────────────────────
echo "Verifying checksum..."
(cd "${CACHE_DIR}" && sha256sum -c "$(basename "${DEST_SHA}")")

echo ""
echo "Vendor cache staged successfully:"
echo "  Archive  : ${DEST_ARCHIVE}"
echo "  Checksum : ${DEST_SHA}"
echo "  Size     : $(du -sh "${DEST_ARCHIVE}" | cut -f1)"
echo ""
echo "To bundle the full offline cache for transfer to an air-gapped host:"
echo "  tar -czf rust-cache.tar.gz -C '${HOME}' .rust-cache"
echo ""
echo "On the air-gapped Jenkins host:"
echo "  tar -xzf rust-cache.tar.gz -C /var/lib/docker/volumes/<jenkins_home>/_data"
