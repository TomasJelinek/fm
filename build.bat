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
set "FM_BUILD_PROFILE="
goto build

:release
set "FM_BUILD_PROFILE=--release"

:build
pushd "%~dp0" || exit /b 1
where cargo >nul 2>nul
if errorlevel 1 goto local_cargo
call cargo build -p fm-app --locked %FM_BUILD_PROFILE%
set "FM_BUILD_EXIT=%errorlevel%"
goto done

:local_cargo
if not exist ".tools\cargo\bin\cargo.exe" goto missing_cargo
powershell.exe -NoLogo -NoProfile -File scripts\cargo.ps1 build -p fm-app --locked %FM_BUILD_PROFILE%
set "FM_BUILD_EXIT=%errorlevel%"
goto done

:missing_cargo
>&2 echo Rust Cargo was not found. Install Rust with rustup and reopen your terminal.
set "FM_BUILD_EXIT=127"

:done
popd
exit /b %FM_BUILD_EXIT%

:usage
>&2 echo Usage: build.bat [debug^|release]
exit /b 2
