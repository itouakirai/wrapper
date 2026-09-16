#!/usr/bin/env bash
# bundle_qemu.sh - Embeds qemu-system-x86-64-headless and QEMU assets into the Android app
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
APP_ASSETS_DIR="$SCRIPT_DIR/app/src/main/assets/qemu"
TMP_DIR="$(mktemp -d)"

cleanup() {
  rm -rf "$TMP_DIR"
}
trap cleanup EXIT

echo "[bundle] Preparing QEMU assets for Android app..."
mkdir -p "$APP_ASSETS_DIR/bin"

# Check if local qemu assets exist in repo
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
if [ -f "$REPO_ROOT/qemu/vmlinuz-lite-qemu" ] && [ -f "$REPO_ROOT/qemu/lite-initramfs.cpio.gz" ] && [ -f "$REPO_ROOT/qemu/data.img" ]; then
  echo "[bundle] Copying local QEMU kernel and disk assets..."
  cp "$REPO_ROOT/qemu/vmlinuz-lite-qemu" "$APP_ASSETS_DIR/"
  cp "$REPO_ROOT/qemu/lite-initramfs.cpio.gz" "$APP_ASSETS_DIR/"
  cp "$REPO_ROOT/qemu/data.img" "$APP_ASSETS_DIR/"
fi

# Download official precompiled Android QEMU package from nightly.link
NIGHTLY_URL="https://nightly.link/WorldObservationLog/wrapper/workflows/build-lite/lite/wrapper-lite-qemu-android-aarch64.zip"
echo "[bundle] Downloading Termux headless QEMU package from: $NIGHTLY_URL"
curl -sL -o "$TMP_DIR/qemu-android.zip" "$NIGHTLY_URL"

echo "[bundle] Extracting QEMU headless binary and assets..."
unzip -q -o "$TMP_DIR/qemu-android.zip" -d "$TMP_DIR/unpacked"

# Copy QEMU headless binary + firmware
if [ -d "$TMP_DIR/unpacked/qemu" ]; then
  cp -R "$TMP_DIR/unpacked/qemu/"* "$APP_ASSETS_DIR/"
fi

if [ -f "$TMP_DIR/unpacked/wrapper-lite-qemu" ]; then
  cp "$TMP_DIR/unpacked/wrapper-lite-qemu" "$APP_ASSETS_DIR/" || true
fi

echo "[bundle] QEMU headless package successfully bundled into $APP_ASSETS_DIR!"
ls -la "$APP_ASSETS_DIR"
