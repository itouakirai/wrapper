#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

DIST_DIR="$SCRIPT_DIR/../dist-gui"
mkdir -p "$DIST_DIR"

echo "Building wrapper-lite GUI for all platforms..."

# Windows x86_64
echo "==> Building Windows (amd64)..."
GOOS=windows GOARCH=amd64 CGO_ENABLED=0 go build -ldflags="-s -w" -o "$DIST_DIR/wrapper-lite-gui-windows-x86_64.exe" .

# macOS Apple Silicon (arm64)
echo "==> Building macOS (arm64)..."
GOOS=darwin GOARCH=arm64 CGO_ENABLED=0 go build -ldflags="-s -w" -o "$DIST_DIR/wrapper-lite-gui-macos-aarch64" .

# macOS Intel (amd64)
echo "==> Building macOS (x86_64)..."
GOOS=darwin GOARCH=amd64 CGO_ENABLED=0 go build -ldflags="-s -w" -o "$DIST_DIR/wrapper-lite-gui-macos-x86_64" .

# Linux x86_64
echo "==> Building Linux (x86_64)..."
GOOS=linux GOARCH=amd64 CGO_ENABLED=0 go build -ldflags="-s -w" -o "$DIST_DIR/wrapper-lite-gui-linux-x86_64" .

# Linux aarch64
echo "==> Building Linux (aarch64)..."
GOOS=linux GOARCH=arm64 CGO_ENABLED=0 go build -ldflags="-s -w" -o "$DIST_DIR/wrapper-lite-gui-linux-aarch64" .

# Android CLI / Termux (aarch64)
echo "==> Building Android (aarch64)..."
GOOS=android GOARCH=arm64 CGO_ENABLED=0 go build -ldflags="-s -w" -o "$DIST_DIR/wrapper-lite-gui-android-aarch64" .

echo "Build complete! Output binaries:"
ls -lh "$DIST_DIR"
