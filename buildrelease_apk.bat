@echo off
setlocal

echo ========================================
echo   imsViewer Android Release APK Build
echo ========================================
echo.

if not defined JAVA_HOME (
    if exist "C:\Program Files\Eclipse Adoptium\jdk-17.0.19.10-hotspot" (
        set "JAVA_HOME=C:\Program Files\Eclipse Adoptium\jdk-17.0.19.10-hotspot"
    )
)

if not defined ANDROID_HOME (
    if exist "%LOCALAPPDATA%\Android\Sdk" (
        set "ANDROID_HOME=%LOCALAPPDATA%\Android\Sdk"
    )
)

echo JAVA_HOME=%JAVA_HOME%
echo ANDROID_HOME=%ANDROID_HOME%
echo.

pushd android
call gradlew.bat :app:assembleRelease
set "BUILD_EXIT=%ERRORLEVEL%"
popd

echo.
if not "%BUILD_EXIT%"=="0" (
    echo ERROR: Android release APK build failed
    pause
    exit /b %BUILD_EXIT%
)

set "SIGNED_APK=android\app\build\outputs\apk\release\app-release.apk"
set "UNSIGNED_APK=android\app\build\outputs\apk\release\app-release-unsigned.apk"
set "OUT_DIR=bin\android"
set "OUT_APK=%OUT_DIR%\imsViewer-android-release.apk"

if exist "%SIGNED_APK%" (
    if not exist "%OUT_DIR%" mkdir "%OUT_DIR%"
    copy /Y "%SIGNED_APK%" "%OUT_APK%" >nul
    echo SUCCESS: %OUT_APK%
) else if exist "%UNSIGNED_APK%" (
    if not exist "%OUT_DIR%" mkdir "%OUT_DIR%"
    copy /Y "%UNSIGNED_APK%" "%OUT_APK%" >nul
    echo SUCCESS: %OUT_APK%
    echo NOTE: Source APK is unsigned: %UNSIGNED_APK%
) else (
    echo ERROR: Release APK not found
    pause
    exit /b 1
)

pause
