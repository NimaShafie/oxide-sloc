#!/usr/bin/env bash
# Run this ONCE on a networked machine to pre-populate the Rust toolchain cache.
# Then transfer the resulting archive to the air-gapped Jenkins agent host.
#
# Usage:
#   bash ci/jenkins/install-rust-cache.sh
#   tar -czf rust-cache.tar.gz -C ~ .rust-cache
#
# On the Jenkins host (before starting the container):
#   tar -xzf rust-cache.tar.gz -C /var/lib/docker/volumes/<jenkins_home_volume>/_data
#
# The Jenkinsfile expects the toolchain at:
#   /var/jenkins_home/.rust-cache/cargo
#   /var/jenkins_home/.rust-cache/rustup

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
TOOLCHAIN=$(grep '^channel' "${REPO_ROOT}/rust-toolchain.toml" | cut -d'"' -f2)
CACHE_DIR="${HOME}/.rust-cache"

export CARGO_HOME="${CACHE_DIR}/cargo"
export RUSTUP_HOME="${CACHE_DIR}/rustup"

echo "Installing Rust ${TOOLCHAIN} into ${CACHE_DIR}..."
mkdir -p "${CARGO_HOME}" "${RUSTUP_HOME}"

if rustup toolchain list 2>/dev/null | grep -q "${TOOLCHAIN}"; then
    echo "Toolchain ${TOOLCHAIN} already present — nothing to do."
else
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
        | sh -s -- -y \
            --default-toolchain "${TOOLCHAIN}" \
            --no-modify-path \
            --component rustfmt clippy
fi

echo ""
echo "Done. Toolchain is at ${CACHE_DIR}."
echo ""
echo "To bundle for transfer:"
echo "  tar -czf rust-cache.tar.gz -C '${HOME}' .rust-cache"
echo ""
echo "On the air-gapped Jenkins host:"
echo "  tar -xzf rust-cache.tar.gz -C /var/lib/docker/volumes/<jenkins_home>/_data"
