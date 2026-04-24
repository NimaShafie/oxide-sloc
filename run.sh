#!/usr/bin/env bash
# oxide-sloc — zero-dependency Linux launcher
# Requires: bash, tar  (both present on every RHEL/Ubuntu/Debian install)
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

EXE="$SCRIPT_DIR/oxidesloc"
EXE_DIST="$SCRIPT_DIR/dist/oxidesloc"
EXE_BUILD="$SCRIPT_DIR/target/release/oxidesloc"
BUNDLE="$SCRIPT_DIR/dist/oxidesloc-linux-x86_64.tar.gz"

launch() {
    chmod +x "$1"
    echo ""
    echo "  oxide-sloc starting → http://127.0.0.1:4317"
    echo "  Press Ctrl+C to stop."
    echo ""
    "$1"
}

# Pre-extracted binary next to this script
if [[ -f "$EXE" ]]; then
    launch "$EXE"
    exit 0
fi

# Previously extracted into dist/
if [[ -f "$EXE_DIST" ]]; then
    launch "$EXE_DIST"
    exit 0
fi

# Built from source
if [[ -f "$EXE_BUILD" ]]; then
    launch "$EXE_BUILD"
    exit 0
fi

# Auto-extract from the bundled tar.gz — tar is always present on Linux
if [[ -f "$BUNDLE" ]]; then
    echo "Extracting oxide-sloc..."
    tar xzf "$BUNDLE" -C "$SCRIPT_DIR"
    if [[ -f "$EXE" ]]; then
        launch "$EXE"
        exit 0
    fi
    echo "ERROR: extraction completed but binary not found — archive may be corrupt." >&2
    exit 1
fi

echo "" >&2
echo "oxide-sloc: no binary found." >&2
echo "" >&2
echo "To get started, choose one of the following:" >&2
echo "" >&2
echo "  Option 1 — Download a pre-built binary (no Rust required):" >&2
echo "    https://github.com/NimaShafie/oxide-sloc/releases" >&2
echo "    Download oxide-sloc-linux-x86_64, place it as 'oxidesloc' next to this" >&2
echo "    script, then run:  bash run.sh" >&2
echo "" >&2
echo "  Option 2 — Build from source (requires Rust 1.78+):" >&2
echo "    cargo build --release -p oxidesloc" >&2
echo "    Then run:  bash run.sh" >&2
echo "" >&2
exit 1
