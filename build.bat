@echo off
setlocal

if not "%~2"=="" goto usage
if not "%2"=="" goto usage
if not "%~1"=="" goto profile
if not "%1"=="" goto usage
goto debug

:profile
if /i "%~1"=="debug" goto debug
if /i "%~1"=="release" goto release
goto usage

:debug
set "MIDNIGHT_BUILD_PROFILE="
set "MIDNIGHT_VIEWER_PROFILE=debug"
goto build

:release
set "MIDNIGHT_BUILD_PROFILE=--release"
set "MIDNIGHT_VIEWER_PROFILE=release"

:build
pushd "%~dp0" || exit /b 1
if not exist "_build\tmp\" mkdir "_build\tmp"
if errorlevel 1 goto temporary_failure
set "TEMP=%CD%\_build\tmp"
set "TMP=%TEMP%"
where cargo >nul 2>nul
if errorlevel 1 goto local_cargo
call cargo build -p midnight-explorer --locked %MIDNIGHT_BUILD_PROFILE%
set "MIDNIGHT_BUILD_EXIT=%errorlevel%"
goto done

:local_cargo
if not exist ".tools\cargo\bin\cargo.exe" goto missing_cargo
powershell.exe -NoLogo -NoProfile -File scripts\cargo.ps1 build -p midnight-explorer --locked %MIDNIGHT_BUILD_PROFILE%
set "MIDNIGHT_BUILD_EXIT=%errorlevel%"
goto done

:missing_cargo
>&2 echo Rust Cargo was not found. Install Rust with rustup and reopen your terminal.
set "MIDNIGHT_BUILD_EXIT=127"
goto done

:temporary_failure
>&2 echo Could not create the build temporary directory under _build.
set "MIDNIGHT_BUILD_EXIT=1"

:done
if not "%MIDNIGHT_BUILD_EXIT%"=="0" goto exit_build
powershell.exe -NoLogo -NoProfile -File scripts\build-native-filesystem.ps1 -Profile %MIDNIGHT_VIEWER_PROFILE%
set "MIDNIGHT_BUILD_EXIT=%errorlevel%"
if not "%MIDNIGHT_BUILD_EXIT%"=="0" goto exit_build
powershell.exe -NoLogo -NoProfile -File scripts\build-viewers.ps1 -Profile %MIDNIGHT_VIEWER_PROFILE%
set "MIDNIGHT_BUILD_EXIT=%errorlevel%"
if not "%MIDNIGHT_BUILD_EXIT%"=="0" goto exit_build
powershell.exe -NoLogo -NoProfile -File scripts\deploy-build.ps1 -Profile %MIDNIGHT_VIEWER_PROFILE%
set "MIDNIGHT_BUILD_EXIT=%errorlevel%"
:exit_build
popd
exit /b %MIDNIGHT_BUILD_EXIT%

:usage
>&2 echo Usage: build.bat [debug^|release]
exit /b 2
