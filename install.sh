#!/usr/bin/env bash
# oxide-sloc installer
# Usage:  bash install.sh            (Windows via Git Bash, Linux, macOS)
#         bash install.sh --rebuild  (force a fresh build even if binary exists)
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

FORCE_REBUILD=false
for arg in "$@"; do
    case "$arg" in
        --rebuild|--force|-f) FORCE_REBUILD=true ;;
    esac
done

# Detect Windows (Git Bash / MSYS2 / Cygwin)
if [[ -n "${WINDIR+x}" ]] || [[ "${OSTYPE:-}" == msys* ]] || [[ "${OSTYPE:-}" == cygwin* ]]; then
    PLATFORM=windows
    EXE="$SCRIPT_DIR/oxide-sloc.exe"
    DIST_ARCHIVE="$SCRIPT_DIR/dist/oxide-sloc-windows-x64.zip"
    BUILD_OUTPUT="$SCRIPT_DIR/target/release/oxide-sloc.exe"
else
    PLATFORM=linux
    EXE="$SCRIPT_DIR/oxide-sloc"
    DIST_ARCHIVE="$SCRIPT_DIR/dist/oxide-sloc-linux-x86_64.tar.gz"
    BUILD_OUTPUT="$SCRIPT_DIR/target/release/oxide-sloc"
fi

VENDOR_ARCHIVE="$SCRIPT_DIR/vendor.tar.xz"
VENDOR_DIR="$SCRIPT_DIR/vendor"

echo ""
echo " oxide-sloc installer"
echo " ════════════════════"

# ── 1. Already installed ────────────────────────────────────────────────────
if [[ -f "$EXE" ]] && [[ "$FORCE_REBUILD" == false ]]; then
    echo " [OK] $(basename "$EXE") already present."
    echo " Run: bash run.sh"
    echo " To rebuild from source:  bash install.sh --rebuild"
    exit 0
fi

if [[ -f "$EXE" ]] && [[ "$FORCE_REBUILD" == true ]]; then
    echo " [--rebuild] Removing existing binary to force a fresh build..."
    rm -f "$EXE"
fi

# ── 2. Pre-built binary ─────────────────────────────────────────────────────
if [[ -f "$DIST_ARCHIVE" ]]; then
    echo " Extracting pre-built binary from dist/..."
    if [[ "$PLATFORM" == windows ]]; then
        WIN_ARCHIVE="$(cygpath -w "$DIST_ARCHIVE")"
        WIN_DEST="$(cygpath -w "$SCRIPT_DIR")"
        powershell -NoProfile -Command \
            "Expand-Archive -Path '$WIN_ARCHIVE' -DestinationPath '$WIN_DEST' -Force"
    else
        tar xzf "$DIST_ARCHIVE" -C "$SCRIPT_DIR"
    fi
    if [[ -f "$EXE" ]]; then
        [[ "$PLATFORM" == linux ]] && chmod +x "$EXE"
        echo " [OK] Extracted $(basename "$EXE")"
        echo ""
        echo " Start the web UI:  bash run.sh"
        exit 0
    fi
    echo " [WARN] Extraction completed but binary not found — archive may be corrupt."
fi

# ── 3. Build from vendored sources ──────────────────────────────────────────
if command -v cargo &>/dev/null; then
    if [[ ! -d "$VENDOR_DIR" ]]; then
        if [[ -f "$VENDOR_ARCHIVE" ]]; then
            echo " Decompressing vendor.tar.xz (22 MB → 362 MB, one-time)..."
            tar -xJf "$VENDOR_ARCHIVE" -C "$SCRIPT_DIR"
            echo " [OK] Vendor sources ready."
        else
            echo " [ERROR] Neither vendor/ nor vendor.tar.xz found. Clone the full repository." >&2
            exit 1
        fi
    fi

    echo " Rust found. Building from vendored sources (this may take a few minutes)..."
    cd "$SCRIPT_DIR"
    cargo build --release --offline -p oxide-sloc

    if [[ -f "$BUILD_OUTPUT" ]]; then
        cp "$BUILD_OUTPUT" "$EXE"
        [[ "$PLATFORM" == linux ]] && chmod +x "$EXE"
        echo " [OK] Built and installed $(basename "$EXE")"
        echo ""
        echo " Start the web UI:  bash run.sh"
        exit 0
    fi
    echo " [ERROR] Build failed. Check output above." >&2
    exit 1
fi

# ── 4. No Rust — air-gapped instructions ────────────────────────────────────
echo ""
echo " No pre-built binary and no Rust toolchain found."
echo ""
echo " Option A — pre-built binary (easiest):"
echo "   Download from https://github.com/NimaShafie/oxide-sloc/releases"
echo "   Place the binary next to this script, then run:  bash install.sh"
echo ""
echo " Option B — build from source (air-gapped, no internet on target):"
echo "   On a NETWORKED machine, bundle the Rust toolchain:"
echo ""
echo "   Windows (PowerShell):"
echo "     rustup-init.exe --default-toolchain stable --no-modify-path"
echo "     Compress-Archive \"\$env:USERPROFILE\.rustup\",\"\$env:USERPROFILE\.cargo\" rust-toolchain-windows.zip"
echo ""
echo "   Linux:"
echo "     curl -sSf https://sh.rustup.rs | sh -s -- --default-toolchain stable --no-modify-path"
echo "     tar -czf rust-toolchain-linux.tar.gz ~/.rustup ~/.cargo"
echo ""
echo "   Transfer the archive to this machine, then:"
echo ""
echo "   Windows (PowerShell):"
echo "     Expand-Archive rust-toolchain-windows.zip -DestinationPath \$env:USERPROFILE"
echo "     [Environment]::SetEnvironmentVariable('PATH', \"\$env:USERPROFILE\.cargo\bin;\$env:PATH\", 'User')"
echo "   Open a new terminal, then:  bash install.sh"
echo ""
echo "   Linux:"
echo "     tar xzf rust-toolchain-linux.tar.gz -C ~"
echo "     echo 'export PATH=\"\$HOME/.cargo/bin:\$PATH\"' >> ~/.bashrc && source ~/.bashrc"
echo "   Then:  bash install.sh"
echo ""
exit 1
