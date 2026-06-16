@echo off
echo ========================================
echo   imsViewer Release Build
echo ========================================
echo.

echo [1/3] Building frontend (Vite)...
call npx vite build
if %errorlevel% neq 0 (
    echo ERROR: Vite build failed
    exit /b %errorlevel%
)
echo.

echo [2/3] Building backend (Cargo release)...
cd /d "%~dp0src-tauri"
cargo build --release
if %errorlevel% neq 0 (
    echo ERROR: Cargo build failed
    exit /b %errorlevel%
)
cd /d "%~dp0"
echo.

echo [3/3] Done!
echo EXE: src-tauri\target\release\imsViewer.exe
echo.

pause
