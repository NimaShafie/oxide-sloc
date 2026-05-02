#!/usr/bin/env bash
# Install the system packages required by oxide-sloc's build on a native
# (non-Docker) Jenkins agent — the same packages that Dockerfile.agent provides
# to container-based agents.
#
# Supports Debian/Ubuntu (apt-get) and RHEL/CentOS/Fedora/Rocky/AlmaLinux (dnf/yum).
# Must be run as root or via sudo.
#
# Usage (run once per agent host):
#   sudo bash ci/jenkins/install-system-deps.sh
#
# After this, run ci/jenkins/install-rust-cache.sh (as the jenkins user) to
# pre-populate the Rust toolchain cache, then verify with preflight.sh.

set -euo pipefail

if [ "$(id -u)" -ne 0 ]; then
    echo "Re-running with sudo..."
    exec sudo bash "$0" "$@"
fi

# ── Debian / Ubuntu ────────────────────────────────────────────────────────────
if command -v apt-get &>/dev/null; then
    echo "==> Detected Debian/Ubuntu — installing via apt-get..."
    apt-get update -qq
    apt-get install -y --no-install-recommends \
        python3 \
        python3-minimal \
        build-essential \
        pkg-config \
        libssl-dev \
        libwayland-dev \
        libgtk-3-dev \
        libxdo-dev \
        curl \
        xz-utils
    echo "==> Done (apt-get)."

# ── RHEL / CentOS / Fedora / Rocky / AlmaLinux ────────────────────────────────
elif command -v dnf &>/dev/null; then
    echo "==> Detected RHEL/Fedora family — installing via dnf..."
    dnf install -y \
        python3 \
        gcc \
        make \
        pkg-config \
        openssl-devel \
        wayland-devel \
        gtk3-devel \
        libxdo-devel \
        curl \
        xz
    echo "==> Done (dnf)."

elif command -v yum &>/dev/null; then
    echo "==> Detected older RHEL/CentOS — installing via yum..."
    yum install -y \
        python3 \
        gcc \
        make \
        pkgconfig \
        openssl-devel \
        wayland-devel \
        gtk3-devel \
        libxdo-devel \
        curl \
        xz
    echo "==> Done (yum)."

else
    echo "ERROR: No supported package manager found (apt-get, dnf, or yum)." >&2
    echo "" >&2
    echo "Install the following packages manually, then re-run preflight.sh:" >&2
    echo "  Debian names: python3 build-essential pkg-config libssl-dev" >&2
    echo "                libwayland-dev libgtk-3-dev libxdo-dev curl xz-utils" >&2
    echo "  RHEL names:   python3 gcc make pkg-config openssl-devel" >&2
    echo "                wayland-devel gtk3-devel libxdo-devel curl xz" >&2
    exit 1
fi

echo ""
echo "System dependencies installed."
echo ""
echo "Next steps to complete native agent setup:"
echo "  1. Pre-populate the Rust toolchain (run as the jenkins user):"
echo "       sudo -u jenkins bash ci/jenkins/install-rust-cache.sh"
echo "  2. Verify the agent is build-ready:"
echo "       bash ci/jenkins/preflight.sh"
