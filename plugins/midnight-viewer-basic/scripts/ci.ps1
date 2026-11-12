param([Parameter(Mandatory)][string]$SdkConfig, [Parameter(Mandatory)][string]$BuildRoot)
$ErrorActionPreference = 'Stop'
$pluginRoot = Split-Path -Parent $PSScriptRoot
$SdkConfig = (Resolve-Path -LiteralPath $SdkConfig).Path
$BuildRoot = [IO.Path]::GetFullPath($BuildRoot)
& cargo fmt --manifest-path (Join-Path $pluginRoot 'Cargo.toml') -- --check
if ($LASTEXITCODE -ne 0) { throw 'Formatting failed' }
foreach ($operation in @('test', 'clippy')) {
    $cargoArguments = @($operation, '--config', $SdkConfig, '--manifest-path',
        (Join-Path $pluginRoot 'Cargo.toml'), '--target-dir', (Join-Path $BuildRoot 'target'),
        '--locked', '--offline', '--all-targets')
    if ($operation -eq 'clippy') { $cargoArguments += @('--', '-D', 'warnings') }
    & cargo @cargoArguments
    if ($LASTEXITCODE -ne 0) { throw "$operation failed" }
}
& (Join-Path $PSScriptRoot 'build.ps1') -SdkConfig $SdkConfig -BuildRoot $BuildRoot
