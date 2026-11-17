<# Independent checks of the image thumbnailer: the pinned native dav1d build, then
formatting, tests and strict Clippy, and an optimized build. #>
param([Parameter(Mandatory)][string]$SdkConfig, [Parameter(Mandatory)][string]$BuildRoot, [switch]$NativeOffline)
$ErrorActionPreference = 'Stop'
$pluginRoot = Split-Path -Parent $PSScriptRoot
$SdkConfig = (Resolve-Path -LiteralPath $SdkConfig).Path
$BuildRoot = [IO.Path]::GetFullPath($ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($BuildRoot))
if ($BuildRoot -notmatch '[/\\]_build([/\\]|$)') { throw 'Generated output must be under _build' }
$nativeScripts = Join-Path $pluginRoot '../image-decoder/native'
$nativeJobs = if ($env:CARGO_BUILD_JOBS) { [int]$env:CARGO_BUILD_JOBS } else { 2 }
& (Join-Path $nativeScripts 'bootstrap-windows.ps1') -BuildRoot (Join-Path $BuildRoot 'native-tools') -Offline:$NativeOffline
& (Join-Path $nativeScripts 'build-windows.ps1') -BuildRoot (Join-Path $BuildRoot 'native-dav1d') -Jobs $nativeJobs -Offline:$NativeOffline
$manifest = Join-Path $pluginRoot 'Cargo.toml'
& cargo fmt --manifest-path $manifest -- --check
if ($LASTEXITCODE -ne 0) { throw 'Formatting failed' }
$common = @('--config', $SdkConfig, '--manifest-path', $manifest, '--target-dir', (Join-Path $BuildRoot 'target'),
    '--locked', '--offline')
foreach ($operation in @('test', 'clippy')) {
    $arguments = @($operation) + $common + @('--all-targets')
    if ($operation -eq 'clippy') { $arguments += @('--', '-D', 'warnings') }
    & cargo @arguments
    if ($LASTEXITCODE -ne 0) { throw "$operation failed" }
}
& cargo build @common --release
if ($LASTEXITCODE -ne 0) { throw 'Optimized build failed' }
