@echo off
setlocal

echo ========================================
echo   imsViewer Updater Key Generator
echo ========================================
echo.

set "KEY_DIR=%USERPROFILE%\.tauri"
set "KEY_PATH=%KEY_DIR%\imsviewer.key"

if not exist "%KEY_DIR%" mkdir "%KEY_DIR%"

if exist "%KEY_PATH%" (
    echo WARNING: Key already exists at %KEY_PATH%
    echo Delete it first if you want to regenerate.
    pause
    exit /b 1
)

echo Generating new signing key pair...
call powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0run_with_libclang.ps1" npm run tauri signer generate -- -w "%KEY_PATH%"

if not exist "%KEY_PATH%" (
    echo ERROR: Key generation failed
    pause
    exit /b 1
)

echo.
echo ========================================
echo   Key generated successfully!
echo ========================================
echo.
echo Private key: %KEY_PATH%
echo.
echo PUBLIC KEY (add to tauri.conf.json):
type "%KEY_PATH%.pub"
echo.
echo.
echo IMPORTANT: Keep the private key secure and never commit it to git!
pause
