param([Parameter(Mandatory)][string]$SdkConfig, [Parameter(Mandatory)][string]$BuildRoot, [switch]$NativeOffline)
$ErrorActionPreference = 'Stop'
$pluginRoot = Split-Path -Parent $PSScriptRoot
$SdkConfig = (Resolve-Path -LiteralPath $SdkConfig).Path
$BuildRoot = [IO.Path]::GetFullPath($ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($BuildRoot))
if ($BuildRoot -notmatch '[/\\]_build([/\\]|$)') { throw 'Generated output must be under _build' }
# The package build selects the native --target; test and lint with the same
# target so a Debug package build reuses the test dependency build.
$nativeTarget = ((& rustc -vV | Select-String '^host: ').ToString() -replace '^host: ', '').Trim()
if ($LASTEXITCODE -ne 0 -or -not $nativeTarget) { throw 'rustc target discovery failed' }
$decoderRoot = Join-Path (Split-Path -Parent $pluginRoot) 'image-decoder'
$nativeTools = Join-Path $BuildRoot 'native-tools'
$nativeBuild = Join-Path $BuildRoot 'native-dav1d'
$nativeScripts = Join-Path $pluginRoot '../image-decoder/native'
$nativeJobs = if ($env:CARGO_BUILD_JOBS) { [int]$env:CARGO_BUILD_JOBS } else { [Environment]::ProcessorCount }
& (Join-Path $nativeScripts 'bootstrap-windows.ps1') -BuildRoot $nativeTools -Offline:$NativeOffline
& (Join-Path $nativeScripts 'build-windows.ps1') -BuildRoot $nativeBuild -Jobs $nativeJobs -Offline:$NativeOffline
foreach ($project in @($decoderRoot, $pluginRoot)) {
    & cargo fmt --manifest-path (Join-Path $project 'Cargo.toml') -- --check
    if ($LASTEXITCODE -ne 0) { throw 'Formatting failed' }
    foreach ($operation in @('test', 'clippy')) {
        $arguments = @($operation, '--config', $SdkConfig, '--manifest-path', (Join-Path $project 'Cargo.toml'),
            '--target-dir', (Join-Path $BuildRoot 'target'), '--target', $nativeTarget, '--locked', '--offline', '--all-targets')
        if ($operation -eq 'clippy') { $arguments += @('--', '-D', 'warnings') }
        & cargo @arguments
        if ($LASTEXITCODE -ne 0) { throw "$operation failed" }
    }
}
& (Join-Path $PSScriptRoot 'build.ps1') -SdkConfig $SdkConfig -BuildRoot $BuildRoot -NativeOffline:$NativeOffline
