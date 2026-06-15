@echo off
cd /d "%~dp0"

:: Add Rust/Cargo to PATH
set PATH=%USERPROFILE%\.cargo\bin;%PATH%

:: Build Release version (frontend + Rust + installer)
echo ========================================
echo   imsViewer Release Build
echo ========================================
echo.
npm run tauri build

echo.
echo ========================================
echo   Build Complete!
echo ========================================
echo Output:
echo   exe:    src-tauri\target\release\imsviewer.exe
echo   msi:    src-tauri\target\release\bundle\msi\imsViewer_*.msi
echo   setup:  src-tauri\target\release\bundle\nsis\imsViewer_*-setup.exe
echo.
pause
