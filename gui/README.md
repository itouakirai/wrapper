# wrapper-lite Cross-Platform GUI

A modern, lightweight, responsive cross-platform GUI for wrapper-lite, supporting **Windows**, **macOS**, **Linux**, and **Android**.

## Overview

- **Desktop (Windows, macOS, Linux)**:
  - Single standalone executable (wrapper-lite-gui.exe on Windows, wrapper-lite-gui on macOS & Linux).
  - Embedded web interface with real-time process monitoring, live console streaming via Server-Sent Events (SSE), and interactive API testing bench.
  - Automatically detects local environment. On non-Linux systems (Windows and macOS), directly leverages precompiled QEMU all-in-one packages.
  - Built-in 1-click downloader and extractor for nightly QEMU packages from 
ightly.link.
  - On Linux, supports both native mode (wrapper-lite / wrapper-lite-rootless) and QEMU virtualized guest mode.

- **Mobile (Android)**:
  - Native Android app with responsive touch UI.
  - **Embedded headless QEMU**: bundles qemu-system-x86-64-headless (Termux build) along with firmware and kernel assets.
  - Runs in an Android Foreground Service with background WakeLock & WifiLock, keeping Apple Music decryption active even when the screen is locked or another music player app is in the foreground.
  - Exposes the HTTP service on port 12340 locally and to other devices on the same Wi-Fi network.

## Precompiled QEMU Packages

The GUI directly utilizes the prebuilt QEMU packages compiled in CI:
- **Nightly link overview**: [nightly.link/itouakirai/wrapper](https://nightly.link/itouakirai/wrapper/workflows/build-lite/lite?preview)
- **Windows (x86_64)**: wrapper-lite-qemu-windows-x86_64.zip
- **macOS (Apple Silicon / aarch64)**: wrapper-lite-qemu-macos-aarch64.zip
- **Linux (x86_64)**: wrapper-lite-qemu-linux-x86_64.zip
- **Linux (aarch64)**: wrapper-lite-qemu-linux-aarch64.zip
- **Android (aarch64)**: wrapper-lite-qemu-android-aarch64.zip (contains headless QEMU binary + assets)

If QEMU package files are not present when you launch the GUI, you can click **Download / Update QEMU Package** in the **QEMU Package** tab to download and install them automatically with one click.

## Quick Start

### 1. Windows

1. Download wrapper-lite-qemu-windows-x86_64.zip (or run wrapper-lite-gui-windows-x86_64.exe).
2. Double-click wrapper-lite-gui.exe.
3. The GUI opens automatically in your browser / app window at http://127.0.0.1:12341.
4. Click **Start Service**. The service is accessible at http://127.0.0.1:12340.

### 2. macOS

1. Download and extract wrapper-lite-qemu-macos-aarch64.zip.
2. Run ./wrapper-lite-gui.
3. Configure settings and click **Start Service**.

### 3. Linux

1. Download and extract wrapper-lite-linux-x86_64.zip or wrapper-lite-qemu-linux-x86_64.zip.
2. Run ./wrapper-lite-gui.
3. In **Settings**, choose between **Native Rootless** or **QEMU Virtual Guest** mode.

### 4. Android

1. Install wrapper-lite-android.apk.
2. On first launch, the app unpacks the built-in qemu-system-x86-64-headless binary and firmware assets.
3. Tap **Start Service**. A notification will appear indicating that the QEMU service is running on :12340.
4. Point your music player or client to http://127.0.0.1:12340 (or http://<phone-ip>:12340).

### 5. Termux / Android CLI

For command-line / headless users in Termux:
`ash
./wrapper-lite-gui-android-aarch64 -no-browser -port 12341
`
Open http://127.0.0.1:12341 in your mobile browser to access the control panel.

## Building from Source

### Building Desktop GUI (Windows, macOS, Linux)

Prerequisites: Go 1.21+

`ash
# Build for host OS
cd gui
go build -ldflags=-s -w -o wrapper-lite-gui .

# Or build for all platforms at once:
./build.sh      # On Linux/macOS
.\build.bat     # On Windows
`

Output binaries are placed in dist-gui/:
- wrapper-lite-gui-windows-x86_64.exe
- wrapper-lite-gui-macos-aarch64
- wrapper-lite-gui-macos-x86_64
- wrapper-lite-gui-linux-x86_64
- wrapper-lite-gui-linux-aarch64
- wrapper-lite-gui-android-aarch64

### Building Android APK

Prerequisites: JDK 17+, Android SDK

`ash
# 1. Bundle headless QEMU package
cd android
./bundle_qemu.sh   # or bundle_qemu.bat on Windows

# 2. Build APK
./gradlew assembleDebug
`
Output APK is located at: ndroid/app/build/outputs/apk/debug/app-debug.apk.
