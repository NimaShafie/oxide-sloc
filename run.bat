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
