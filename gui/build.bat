@echo off
setlocal enabledelayedexpansion

set SCRIPT_DIR=%~dp0
cd /d "%SCRIPT_DIR%"

set DIST_DIR=%SCRIPT_DIR%..\dist-gui
if not exist "%DIST_DIR%" mkdir "%DIST_DIR%"

echo Building wrapper-lite GUI for all platforms...

echo ==^> Building Windows (amd64)...
set GOOS=windows
set GOARCH=amd64
set CGO_ENABLED=0
go build -ldflags="-s -w" -o "%DIST_DIR%\wrapper-lite-gui-windows-x86_64.exe" .

echo ==^> Building macOS (arm64)...
set GOOS=darwin
set GOARCH=arm64
set CGO_ENABLED=0
go build -ldflags="-s -w" -o "%DIST_DIR%\wrapper-lite-gui-macos-aarch64" .

echo ==^> Building macOS (amd64)...
set GOOS=darwin
set GOARCH=amd64
set CGO_ENABLED=0
go build -ldflags="-s -w" -o "%DIST_DIR%\wrapper-lite-gui-macos-x86_64" .

echo ==^> Building Linux (x86_64)...
set GOOS=linux
set GOARCH=amd64
set CGO_ENABLED=0
go build -ldflags="-s -w" -o "%DIST_DIR%\wrapper-lite-gui-linux-x86_64" .

echo ==^> Building Linux (aarch64)...
set GOOS=linux
set GOARCH=arm64
set CGO_ENABLED=0
go build -ldflags="-s -w" -o "%DIST_DIR%\wrapper-lite-gui-linux-aarch64" .

echo ==^> Building Android (aarch64)...
set GOOS=android
set GOARCH=arm64
set CGO_ENABLED=0
go build -ldflags="-s -w" -o "%DIST_DIR%\wrapper-lite-gui-android-aarch64" .

echo Build complete! Output binaries:
dir "%DIST_DIR%"
