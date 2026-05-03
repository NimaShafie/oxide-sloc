#!/usr/bin/env bash
# Generate LCOV and Cobertura coverage reports for SonarQube import.
#
# Usage: bash ci/sonar/generate-coverage.sh [output-dir]
# Default output directory: coverage/
#
# Tool priority:
#   1. cargo-llvm-cov  — preferred; vendored in vendor.tar.xz via ci/tools/Cargo.toml.
#        In Jenkins CI, the SonarQube scan stage installs it automatically from vendor
#        when GENERATE_COVERAGE=true (no internet required).
#        For local use: cargo install cargo-llvm-cov
#   2. cargo-tarpaulin — cross-platform fallback; install with:
#        cargo install cargo-tarpaulin
#
# cargo-llvm-cov requires the llvm-tools rustup component, which is declared in
# rust-toolchain.toml and will be present in any toolchain bundle built from it.
#
# Output files written to OUTPUT_DIR:
#   lcov.info            — consumed by sonar.lcov.reportPaths
#   sonar-coverage.xml   — consumed by sonar.coverageReportPaths (Cobertura/generic XML)

set -euo pipefail

OUTPUT_DIR="${1:-coverage}"
mkdir -p "$OUTPUT_DIR"

has_cargo_subcommand() {
    cargo "$1" --version &>/dev/null 2>&1
}

# ── cargo-llvm-cov ────────────────────────────────────────────────────────────
if has_cargo_subcommand llvm-cov; then
    echo "==> Generating coverage with cargo-llvm-cov"

    rustup component add llvm-tools 2>/dev/null || true

    cargo llvm-cov \
        --workspace \
        --all-features \
        --lcov \
        --output-path "${OUTPUT_DIR}/lcov.info"

    cargo llvm-cov \
        --workspace \
        --all-features \
        --cobertura \
        --output-path "${OUTPUT_DIR}/sonar-coverage.xml"

    echo "Coverage reports written to ${OUTPUT_DIR}/"
    exit 0
fi

# ── cargo-tarpaulin ───────────────────────────────────────────────────────────
if has_cargo_subcommand tarpaulin; then
    echo "==> Generating coverage with cargo-tarpaulin"

    cargo tarpaulin \
        --workspace \
        --all-features \
        --out Lcov Xml \
        --output-dir "${OUTPUT_DIR}" \
        --exclude-files "tests/*" \
        --timeout 120

    # tarpaulin names the XML file "cobertura.xml"; rename to what SonarQube expects.
    if [ -f "${OUTPUT_DIR}/cobertura.xml" ]; then
        mv -f "${OUTPUT_DIR}/cobertura.xml" "${OUTPUT_DIR}/sonar-coverage.xml"
    fi

    echo "Coverage reports written to ${OUTPUT_DIR}/"
    exit 0
fi

# ── Neither tool found ────────────────────────────────────────────────────────
cat >&2 <<'EOF'
WARNING: Neither cargo-llvm-cov nor cargo-tarpaulin is installed.
  Coverage data will NOT be included in the SonarQube scan.

  To install cargo-llvm-cov (preferred):
    cargo install cargo-llvm-cov     # vendored in vendor.tar.xz for air-gapped use
    rustup component add llvm-tools  # included in rust-toolchain.toml

  To install cargo-tarpaulin (fallback):
    cargo install cargo-tarpaulin
EOF
exit 0
