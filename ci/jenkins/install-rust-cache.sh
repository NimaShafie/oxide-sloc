#!/usr/bin/env bash
# Pre-populate the Rust toolchain cache on a Jenkins agent (Docker or native).
#
# Run this ONCE on the agent host — or on any networked machine and then transfer
# the resulting archive to an air-gapped host.  After running, every subsequent
# Jenkins build finds the toolchain in place and skips all network access.
#
# Works with both Docker-managed and native (systemd / bare-metal) agents because
# the Jenkinsfile reads CARGO_HOME/RUSTUP_HOME from ${HOME}/.rust-cache, which
# this script also uses.
#
# ── Run directly on the agent (internet available) ───────────────────────────
#   bash ci/jenkins/install-rust-cache.sh
#
# ── Transfer to an air-gapped agent ──────────────────────────────────────────
#   # On a networked machine:
#   bash ci/jenkins/install-rust-cache.sh
#   tar -czf rust-cache.tar.gz -C "${HOME}" .rust-cache
#
#   # On the air-gapped agent — Docker:
#   #   Copy rust-cache.tar.gz to the host, then:
#   tar -xzf rust-cache.tar.gz \
#       -C /var/lib/docker/volumes/<jenkins_home_volume>/_data
#
#   # On the air-gapped agent — native systemd (jenkins user home = /var/lib/jenkins):
#   sudo -u jenkins tar -xzf rust-cache.tar.gz -C /var/lib/jenkins
#   # Adjust the path if the jenkins user's home differs (check: getent passwd jenkins).
#
# After transfer, the Jenkinsfile Setup stage detects the cached toolchain on
# first build and skips all Rust-related network access automatically.

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

JENKINS_HOME_GUESS="$(getent passwd jenkins 2>/dev/null | cut -d: -f6 || echo '/var/lib/jenkins')"

echo ""
echo "Done. Toolchain cached at: ${CACHE_DIR}"
echo ""
echo "To bundle for transfer to an air-gapped host:"
echo "  tar -czf rust-cache.tar.gz -C '${HOME}' .rust-cache"
echo ""
echo "Extract on the air-gapped agent — Docker:"
echo "  tar -xzf rust-cache.tar.gz \\"
echo "      -C /var/lib/docker/volumes/<jenkins_home_volume>/_data"
echo ""
echo "Extract on the air-gapped agent — native (jenkins user home = ${JENKINS_HOME_GUESS}):"
echo "  sudo -u jenkins tar -xzf rust-cache.tar.gz -C '${JENKINS_HOME_GUESS}'"
