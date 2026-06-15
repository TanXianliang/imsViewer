@echo off
cd /d "%~dp0"

:: Add Rust/Cargo to PATH
set PATH=%USERPROFILE%\.cargo\bin;%PATH%

:: Launch Tauri dev mode (Vite + Rust debug)
npm run tauri dev
pause
