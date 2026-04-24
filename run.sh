#!/usr/bin/env bash
# Linux / macOS launcher for oxide-sloc.
# Usage: bash run.sh   (or chmod +x run.sh && ./run.sh)

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

EXE="$SCRIPT_DIR/oxidesloc"
EXE_DIST="$SCRIPT_DIR/dist/oxidesloc"
EXE_BUILD="$SCRIPT_DIR/target/release/oxidesloc"
BUNDLE="$SCRIPT_DIR/dist/oxidesloc-linux-x86_64.tar.gz"

launch() {
    chmod +x "$1"
    printf '\n  oxide-sloc starting \xe2\x86\x92 http://127.0.0.1:4317\n  Press Ctrl+C to stop.\n\n'
    "$1"
}

if   [[ -f "$EXE" ]];       then launch "$EXE";       exit 0
elif [[ -f "$EXE_DIST" ]];  then launch "$EXE_DIST";  exit 0
elif [[ -f "$EXE_BUILD" ]]; then launch "$EXE_BUILD"; exit 0
elif [[ -f "$BUNDLE" ]]; then
    echo "Extracting oxide-sloc..."
    tar xzf "$BUNDLE" -C "$SCRIPT_DIR"
    if [[ -f "$EXE" ]]; then
        launch "$EXE"
        exit 0
    fi
    echo "ERROR: extraction completed but binary not found — archive may be corrupt." >&2
    exit 1
fi

printf '\noxide-sloc: no binary found.\n\n' >&2
printf '  Option 1 - Download: https://github.com/NimaShafie/oxide-sloc/releases\n' >&2
printf '             Place binary as "oxidesloc" next to this script, then: bash run.sh\n' >&2
printf '  Option 2 - Build:    cargo build --release -p oxidesloc\n' >&2
printf '  Option 3 - Docker:   docker compose up\n\n' >&2
exit 1
