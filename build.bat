@echo off
setlocal

rem Arguments in any order: at most one profile (default debug) and --validate.
set "MIDNIGHT_BUILD_PROFILE="
set "MIDNIGHT_VIEWER_PROFILE="
set "MIDNIGHT_VALIDATE="

:parse_arguments
if not "%~1"=="" goto parse_argument
if not "%1"=="" goto usage
goto arguments_parsed

:parse_argument
if /i "%~1"=="debug" goto choose_debug
if /i "%~1"=="release" goto choose_release
if /i "%~1"=="--validate" goto choose_validate
goto usage

:choose_debug
if defined MIDNIGHT_VIEWER_PROFILE goto usage
set "MIDNIGHT_BUILD_PROFILE="
set "MIDNIGHT_VIEWER_PROFILE=debug"
shift
goto parse_arguments

:choose_release
if defined MIDNIGHT_VIEWER_PROFILE goto usage
set "MIDNIGHT_BUILD_PROFILE=--release"
set "MIDNIGHT_VIEWER_PROFILE=release"
shift
goto parse_arguments

:choose_validate
if defined MIDNIGHT_VALIDATE goto usage
set "MIDNIGHT_VALIDATE=1"
shift
goto parse_arguments

:arguments_parsed
if not defined MIDNIGHT_VIEWER_PROFILE set "MIDNIGHT_VIEWER_PROFILE=debug"

:build
pushd "%~dp0" || exit /b 1
where pwsh.exe >nul 2>nul
if errorlevel 1 goto missing_pwsh
pwsh.exe -NoLogo -NoProfile -NonInteractive -Command "if ($PSVersionTable.PSVersion.Major -lt 7 -or -not $IsWindows -or -not [Environment]::Is64BitProcess) { exit 1 }"
if errorlevel 1 goto missing_pwsh
rem The module helpers run Python through the py launcher, or a real python.exe
rem when the launcher is absent; a Store alias stub fails this check, not a late step.
where py.exe >nul 2>nul
if errorlevel 1 goto check_python_exe
py.exe -3 -c "import sys; sys.exit(sys.version_info < (3, 12))" >nul 2>nul
if errorlevel 1 goto missing_python
goto python_ready
:check_python_exe
python.exe -c "import sys; sys.exit(sys.version_info < (3, 12))" >nul 2>nul
if errorlevel 1 goto missing_python
:python_ready
if not exist "_build\tmp\" mkdir "_build\tmp"
if errorlevel 1 goto temporary_failure
set "TEMP=%CD%\_build\tmp"
set "TMP=%TEMP%"
where cargo >nul 2>nul
if errorlevel 1 goto local_cargo
call cargo build -p midnight-explorer -p midnight-elevation --locked %MIDNIGHT_BUILD_PROFILE%
set "MIDNIGHT_BUILD_EXIT=%errorlevel%"
goto done

:local_cargo
if not exist ".tools\cargo\bin\cargo.exe" goto missing_cargo
pwsh.exe -NoLogo -NoProfile -File scripts\cargo.ps1 build -p midnight-explorer -p midnight-elevation --locked %MIDNIGHT_BUILD_PROFILE%
set "MIDNIGHT_BUILD_EXIT=%errorlevel%"
goto done

:missing_cargo
>&2 echo Rust Cargo was not found. Install Rust with rustup and reopen your terminal.
set "MIDNIGHT_BUILD_EXIT=127"
goto done

:missing_pwsh
>&2 echo 64-bit PowerShell 7 ^(pwsh.exe^) is required. Install PowerShell 7 and reopen your terminal.
set "MIDNIGHT_BUILD_EXIT=127"
goto exit_build

:missing_python
>&2 echo Python 3.12 or newer is required. Install it with the py launcher and reopen your terminal.
set "MIDNIGHT_BUILD_EXIT=127"
goto exit_build

:temporary_failure
>&2 echo Could not create the build temporary directory under _build.
set "MIDNIGHT_BUILD_EXIT=1"

:done
if not "%MIDNIGHT_BUILD_EXIT%"=="0" goto exit_build
pwsh.exe -NoLogo -NoProfile -File scripts\build-native-filesystem.ps1 -Profile %MIDNIGHT_VIEWER_PROFILE%
set "MIDNIGHT_BUILD_EXIT=%errorlevel%"
if not "%MIDNIGHT_BUILD_EXIT%"=="0" goto exit_build
pwsh.exe -NoLogo -NoProfile -File scripts\build-network-filesystem.ps1 -Profile %MIDNIGHT_VIEWER_PROFILE%
set "MIDNIGHT_BUILD_EXIT=%errorlevel%"
if not "%MIDNIGHT_BUILD_EXIT%"=="0" goto exit_build
pwsh.exe -NoLogo -NoProfile -File scripts\build-viewers.ps1 -Profile %MIDNIGHT_VIEWER_PROFILE%
set "MIDNIGHT_BUILD_EXIT=%errorlevel%"
if not "%MIDNIGHT_BUILD_EXIT%"=="0" goto exit_build
pwsh.exe -NoLogo -NoProfile -File scripts\build-default-thumbnailers.ps1 -Profile %MIDNIGHT_VIEWER_PROFILE%
set "MIDNIGHT_BUILD_EXIT=%errorlevel%"
if not "%MIDNIGHT_BUILD_EXIT%"=="0" goto exit_build
pwsh.exe -NoLogo -NoProfile -File scripts\build-archive-filesystem.ps1 -Profile %MIDNIGHT_VIEWER_PROFILE%
set "MIDNIGHT_BUILD_EXIT=%errorlevel%"
if not "%MIDNIGHT_BUILD_EXIT%"=="0" goto exit_build
rem A build that fails validation is not deployed.
if not defined MIDNIGHT_VALIDATE goto deploy
pwsh.exe -NoLogo -NoProfile -File scripts\validate.ps1 %MIDNIGHT_VIEWER_PROFILE%
set "MIDNIGHT_BUILD_EXIT=%errorlevel%"
if not "%MIDNIGHT_BUILD_EXIT%"=="0" goto exit_build
:deploy
pwsh.exe -NoLogo -NoProfile -File scripts\deploy-build.ps1 -Profile %MIDNIGHT_VIEWER_PROFILE%
set "MIDNIGHT_BUILD_EXIT=%errorlevel%"
:exit_build
popd
exit /b %MIDNIGHT_BUILD_EXIT%

:usage
>&2 echo Usage: build.bat [debug^|release] [--validate]
>&2 echo --validate runs scripts\validate.ps1 ^(the checks and tests that CI runs on Windows^) after the build and before its private deployment.
exit /b 2
