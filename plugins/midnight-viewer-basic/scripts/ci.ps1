param([Parameter(Mandatory)][string]$SdkConfig, [Parameter(Mandatory)][string]$BuildRoot)
$ErrorActionPreference = 'Stop'
$pluginRoot = Split-Path -Parent $PSScriptRoot
$SdkConfig = (Resolve-Path -LiteralPath $SdkConfig).Path
$BuildRoot = [IO.Path]::GetFullPath($ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($BuildRoot))
# The package build selects the native --target; test and lint with the same
# target so Cargo reuses one dependency build instead of compiling it twice.
$nativeTarget = ((& rustc -vV | Select-String '^host: ').ToString() -replace '^host: ', '').Trim()
if ($LASTEXITCODE -ne 0 -or -not $nativeTarget) { throw 'rustc target discovery failed' }
& cargo fmt --manifest-path (Join-Path $pluginRoot 'Cargo.toml') -- --check
if ($LASTEXITCODE -ne 0) { throw 'Formatting failed' }
foreach ($operation in @('test', 'clippy')) {
    $cargoArguments = @($operation, '--config', $SdkConfig, '--manifest-path',
        (Join-Path $pluginRoot 'Cargo.toml'), '--target-dir', (Join-Path $BuildRoot 'target'),
        '--target', $nativeTarget, '--locked', '--offline', '--all-targets')
    if ($operation -eq 'clippy') { $cargoArguments += @('--', '-D', 'warnings') }
    & cargo @cargoArguments
    if ($LASTEXITCODE -ne 0) { throw "$operation failed" }
}
& (Join-Path $PSScriptRoot 'build.ps1') -SdkConfig $SdkConfig -BuildRoot $BuildRoot
