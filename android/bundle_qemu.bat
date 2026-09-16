@echo off
setlocal enabledelayedexpansion

set SCRIPT_DIR=%~dp0
set APP_ASSETS_DIR=%SCRIPT_DIR%app\src\main\assets\qemu
set NIGHTLY_URL=https://nightly.link/WorldObservationLog/wrapper/workflows/build-lite/lite/wrapper-lite-qemu-android-aarch64.zip
set TMP_ZIP=%TEMP%\qemu-android.zip
set TMP_EXTRACT=%TEMP%\qemu-android-unpacked

echo [bundle] Preparing QEMU assets for Android app...
if not exist "%APP_ASSETS_DIR%" mkdir "%APP_ASSETS_DIR%"

echo [bundle] Downloading Termux headless QEMU package from: %NIGHTLY_URL%
powershell -Command "Invoke-WebRequest -Uri '%NIGHTLY_URL%' -OutFile '%TMP_ZIP%' -UseBasicParsing"

echo [bundle] Extracting QEMU headless binary and assets...
if exist "%TMP_EXTRACT%" rd /s /q "%TMP_EXTRACT%"
mkdir "%TMP_EXTRACT%"
tar -xf "%TMP_ZIP%" -C "%TMP_EXTRACT%"

if exist "%TMP_EXTRACT%\qemu" (
    xcopy /E /Y /I "%TMP_EXTRACT%\qemu" "%APP_ASSETS_DIR%"
)

if exist "%TMP_EXTRACT%\wrapper-lite-qemu" (
    copy /Y "%TMP_EXTRACT%\wrapper-lite-qemu" "%APP_ASSETS_DIR%\"
)

del /f /q "%TMP_ZIP%" 2>nul
rd /s /q "%TMP_EXTRACT%" 2>nul

echo [bundle] QEMU headless package successfully bundled into %APP_ASSETS_DIR%!
dir "%APP_ASSETS_DIR%"
