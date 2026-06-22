@echo off
echo ========================================
echo   imsViewer Release Build
echo ========================================
echo.

echo Building with Tauri (MSI error can be ignored)...
call npm run tauri build
echo.

if exist "src-tauri\target\release\imsViewer.exe" (
    echo SUCCESS: src-tauri\target\release\imsViewer.exe
) else (
    echo ERROR: EXE not found
)
pause
