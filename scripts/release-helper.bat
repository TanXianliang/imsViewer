@echo off
setlocal

echo ========================================
echo   imsViewer Release Helper
echo ========================================
echo.

set "KEY_PATH=%USERPROFILE%\.tauri\imsviewer.key"
set "BUNDLE_DIR=src-tauri\target\release\bundle\nsis"

if not exist "%KEY_PATH%" (
    echo ERROR: Private key not found at %KEY_PATH%
    echo Run scripts\generate-keys.bat first.
    pause
    exit /b 1
)

if not exist "%BUNDLE_DIR%" (
    echo ERROR: Bundle directory not found: %BUNDLE_DIR%
    echo Run npm run tauri build first.
    pause
    exit /b 1
)

echo Found bundle directory: %BUNDLE_DIR%
echo.

REM Find the latest installer
for /f "delims=" %%F in ('dir /b /o-d "%BUNDLE_DIR%\*.exe" 2^>nul') do (
    set "SETUP_EXE=%%F"
    goto :found
)

echo ERROR: No setup.exe found in %BUNDLE_DIR%
pause
exit /b 1

:found
echo Found installer: %SETUP_EXE%
echo.

REM Extract version number
for /f "tokens=2 delims=_" %%V in ("%SETUP_EXE%") do set "VERSION=%%V"
echo Version: %VERSION%
echo.

REM Sign the installer
echo Signing installer...
call powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0run_with_libclang.ps1" npm run tauri signer sign -- -k "%KEY_PATH%" "%BUNDLE_DIR%\%SETUP_EXE%"

if errorlevel 1 (
    echo ERROR: Signing failed
    pause
    exit /b 1
)

set "SIG_FILE=%BUNDLE_DIR%\%SETUP_EXE%.sig"
if not exist "%SIG_FILE%" (
    echo ERROR: Signature file not generated
    pause
    exit /b 1
)

echo.
echo ========================================
echo   Signing completed!
echo ========================================
echo.
echo Installer: %BUNDLE_DIR%\%SETUP_EXE%
echo Signature: %SIG_FILE%
echo.

REM Generate latest.json
set "LATEST_JSON=%BUNDLE_DIR%\latest.json"
echo Generating latest.json...

REM Read signature content
set /p SIGNATURE=<"%SIG_FILE%"

REM Get file size
for %%A in ("%BUNDLE_DIR%\%SETUP_EXE%") do set "FILE_SIZE=%%~zA"

REM Get current date
for /f "tokens=2 delims==" %%I in ('wmic os get localdatetime /value') do set datetime=%%I
set "PUB_DATE=%datetime:~0,4%-%datetime:~4,2%-%datetime:~6,2%T%datetime:~8,2%:%datetime:~10,2%:%datetime:~12,2%Z"

(
echo {
echo   "version": "%VERSION%",
echo   "notes": "imsViewer v%VERSION%",
echo   "pub_date": "%PUB_DATE%",
echo   "platforms": {
echo     "windows-x86_64": {
echo       "signature": "%SIGNATURE%",
echo       "url": "https://github.com/TanXianliang/imsViewer/releases/download/v%VERSION%/imsViewer_%VERSION%_x64-setup.exe"
echo     }
echo   }
echo }
) > "%LATEST_JSON%"

echo.
echo latest.json generated: %LATEST_JSON%
echo.
echo ========================================
echo   Next steps:
echo ========================================
echo 1. Create a new release on GitHub
echo 2. Upload: %SETUP_EXE%
echo 3. Upload: %SETUP_EXE%.sig
echo 4. Upload: latest.json
echo.
pause
