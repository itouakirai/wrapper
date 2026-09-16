@echo off
setlocal enabledelayedexpansion

set SCRIPT_DIR=%~dp0
set APP_ASSETS_DIR=%SCRIPT_DIR%app\src\main\assets\qemu
set NIGHTLY_URL=https://nightly.link/itouakirai/wrapper/workflows/build-lite/lite/wrapper-lite-qemu-android-aarch64.zip
set TMP_ZIP=%TEMP%\qemu-android.zip
set TMP_EXTRACT=%TEMP%\qemu-android-unpacked

echo [bundle] Preparing QEMU assets for Android app...
if not exist "%APP_ASSETS_DIR%\bin" mkdir "%APP_ASSETS_DIR%\bin"

set REPO_ROOT=%SCRIPT_DIR%..
if exist "%REPO_ROOT%\qemu\vmlinuz-lite-qemu" (
    if exist "%REPO_ROOT%\qemu\lite-initramfs.cpio.gz" (
        if exist "%REPO_ROOT%\qemu\data.img" (
            echo [bundle] Copying local QEMU kernel and disk assets...
            copy /Y "%REPO_ROOT%\qemu\vmlinuz-lite-qemu" "%APP_ASSETS_DIR%\"
            copy /Y "%REPO_ROOT%\qemu\lite-initramfs.cpio.gz" "%APP_ASSETS_DIR%\"
            copy /Y "%REPO_ROOT%\qemu\data.img" "%APP_ASSETS_DIR%\"
        )
    )
)

echo [bundle] Checking precompiled QEMU package from: %NIGHTLY_URL%
powershell -Command "try { Invoke-WebRequest -Uri '%NIGHTLY_URL%' -OutFile '%TMP_ZIP%' -UseBasicParsing } catch { exit 1 }"

set ZIP_OK=0
if exist "%TMP_ZIP%" (
    tar -tf "%TMP_ZIP%" >nul 2>&1
    if !errorlevel! equ 0 set ZIP_OK=1
)

if !ZIP_OK! equ 1 (
    echo [bundle] Extracting QEMU headless binary, firmware, and libraries...
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
) else (
    echo [bundle] Precompiled nightly package unavailable; resolving and bundling Termux QEMU + shared libraries directly...
    del /f /q "%TMP_ZIP%" 2>nul
    python "%SCRIPT_DIR%bundle_termux_deps.py" --target-dir "%APP_ASSETS_DIR%\bin"
)

echo [bundle] QEMU headless package successfully bundled into %APP_ASSETS_DIR%!
dir "%APP_ASSETS_DIR%"
