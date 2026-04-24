#!/usr/bin/env bash
# oxide-sloc launcher
# Usage: bash run.sh   (Windows via Git Bash; Linux/macOS)
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Detect Windows (Git Bash / MSYS2 / Cygwin)
if [[ -n "${WINDIR+x}" ]] || [[ "${OSTYPE:-}" == msys* ]] || [[ "${OSTYPE:-}" == cygwin* ]]; then
    PLATFORM=windows
    EXE="$SCRIPT_DIR/oxide-sloc.exe"
    EXE_DIST="$SCRIPT_DIR/dist/oxide-sloc.exe"
    EXE_BUILD="$SCRIPT_DIR/target/release/oxide-sloc.exe"
    BUNDLE="$SCRIPT_DIR/dist/oxide-sloc-windows-x64.zip"
else
    PLATFORM=linux
    EXE="$SCRIPT_DIR/oxide-sloc"
    EXE_DIST="$SCRIPT_DIR/dist/oxide-sloc"
    EXE_BUILD="$SCRIPT_DIR/target/release/oxide-sloc"
    BUNDLE="$SCRIPT_DIR/dist/oxide-sloc-linux-x86_64.tar.gz"
fi

launch() {
    [[ "$PLATFORM" == linux ]] && chmod +x "$1"
    printf '\n  oxide-sloc starting \xe2\x86\x92 http://127.0.0.1:4317\n  Press Ctrl+C to stop.\n\n'
    "$1"
}

extract_bundle() {
    echo "Extracting oxide-sloc..."
    if [[ "$PLATFORM" == windows ]]; then
        WIN_BUNDLE="$(cygpath -w "$BUNDLE")"
        WIN_DEST="$(cygpath -w "$SCRIPT_DIR")"
        powershell -NoProfile -Command "Expand-Archive -Path '$WIN_BUNDLE' -DestinationPath '$WIN_DEST' -Force"
    else
        tar xzf "$BUNDLE" -C "$SCRIPT_DIR"
    fi
}

if   [[ -f "$EXE" ]];       then launch "$EXE";       exit 0
elif [[ -f "$EXE_DIST" ]];  then launch "$EXE_DIST";  exit 0
elif [[ -f "$EXE_BUILD" ]]; then launch "$EXE_BUILD"; exit 0
elif [[ -f "$BUNDLE" ]]; then
    extract_bundle
    if [[ -f "$EXE" ]]; then
        launch "$EXE"
        exit 0
    fi
    echo "ERROR: extraction completed but binary not found — archive may be corrupt." >&2
    exit 1
fi

printf '\noxide-sloc: no binary found.\n\n' >&2
printf '  Option 1 - Download: https://github.com/NimaShafie/oxide-sloc/releases\n' >&2
printf '             Place binary next to this script, then: bash run.sh\n' >&2
printf '  Option 2 - Build:    cargo build --release -p oxide-sloc\n' >&2
printf '  Option 3 - Docker:   docker compose up\n\n' >&2
exit 1
