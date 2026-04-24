:<<'_END_'
@echo off
setlocal

set "ROOT=%~dp0"
set "EXE=%ROOT%oxidesloc.exe"
set "EXE_DIST=%ROOT%dist\oxidesloc.exe"
set "EXE_BUILD=%ROOT%target\release\oxidesloc.exe"
set "ZIP=%ROOT%dist\oxidesloc-windows-x64.zip"

if exist "%EXE%"       goto :run
if exist "%EXE_DIST%"  ( set "EXE=%EXE_DIST%"  & goto :run )
if exist "%EXE_BUILD%" ( set "EXE=%EXE_BUILD%" & goto :run )

if exist "%ZIP%" (
    echo Extracting oxide-sloc...
    powershell -NoProfile -Command "Expand-Archive -Path '%ZIP%' -DestinationPath '%ROOT%' -Force"
    if exist "%EXE%" goto :run
    echo Extraction failed. Try extracting dist\oxidesloc-windows-x64.zip manually.
    pause
    exit /b 1
)

echo oxide-sloc: no binary found.
echo.
echo   Option 1 - Download: https://github.com/NimaShafie/oxide-sloc/releases
echo              Place oxidesloc.exe next to this script and run again.
echo   Option 2 - Build:    cargo build --release -p oxidesloc
echo   Option 3 - Docker:   docker compose up
echo.
pause
exit /b 1

:run
start "" "%EXE%"
exit /b 0
_END_

# ── Linux / macOS ────────────────────────────────────────────────────────────
# Windows: double-click run.bat
# Linux:   bash run.bat
# Requires: bash, tar — present on every RHEL/Ubuntu/Debian install, nothing else.

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
printf '             Place binary as "oxidesloc" next to this script, then: bash run.bat\n' >&2
printf '  Option 2 - Build:    cargo build --release -p oxidesloc\n' >&2
printf '  Option 3 - Docker:   docker compose up\n\n' >&2
exit 1
