@echo off
setlocal

echo ========================================
echo   imsViewer Release Build
echo ========================================
echo.

echo Building with Tauri (MSI error can be ignored)...
call npm run tauri build
set "BUILD_EXIT=%ERRORLEVEL%"
echo.

if not "%BUILD_EXIT%"=="0" (
    echo ERROR: Windows release build failed
    pause
    exit /b %BUILD_EXIT%
)

set "SRC_EXE=src-tauri\target\release\imsViewer.exe"
set "OUT_DIR=bin\win32"
set "OUT_EXE=%OUT_DIR%\imsViewer.exe"

if exist "%SRC_EXE%" (
    if not exist "%OUT_DIR%" mkdir "%OUT_DIR%"
    copy /Y "%SRC_EXE%" "%OUT_EXE%" >nul
    copy /Y "src-tauri\target\release\*.dll" "%OUT_DIR%\" >nul
    xcopy /E /I /Y "src-tauri\target\release\resources" "%OUT_DIR%\resources" >nul
    echo SUCCESS: %OUT_DIR%
) else (
    echo ERROR: EXE not found
    pause
    exit /b 1
)
pause
