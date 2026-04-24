@echo off
setlocal

set "ROOT=%~dp0"
set "EXE=%ROOT%oxidesloc.exe"
set "EXE_DIST=%ROOT%dist\oxidesloc.exe"
set "EXE_BUILD=%ROOT%target\release\oxidesloc.exe"
set "ZIP=%ROOT%dist\oxidesloc-windows-x64.zip"

:: Already extracted next to this script
if exist "%EXE%" goto :run

:: Already present in dist\ (e.g. previously extracted)
if exist "%EXE_DIST%" (
    set "EXE=%EXE_DIST%"
    goto :run
)

:: Built from source
if exist "%EXE_BUILD%" (
    set "EXE=%EXE_BUILD%"
    goto :run
)

:: Auto-extract from the bundled zip using PowerShell (no extra tools required)
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
echo To get started, choose one of the following:
echo.
echo   Option 1 - Download a pre-built binary (no Rust required):
echo     https://github.com/NimaShafie/oxide-sloc/releases
echo     Place oxidesloc.exe next to this script and run again.
echo.
echo   Option 2 - Build from source (requires Rust 1.78+):
echo     cargo build --release -p oxidesloc
echo     Then run this script again.
echo.
echo   Option 3 - Docker (no Rust required):
echo     docker compose up
echo     Then open http://localhost:4317
echo.
pause
exit /b 1

:run
start "" "%EXE%"
