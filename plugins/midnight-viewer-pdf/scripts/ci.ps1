param(
    [Parameter(Mandatory)][string]$SdkConfig,
    [Parameter(Mandatory)][string]$BuildRoot,
    [ValidateSet('debug','release')][string]$Profile = 'release'
)
$ErrorActionPreference = 'Stop'
$pluginRoot = Split-Path -Parent $PSScriptRoot
$SdkConfig = (Resolve-Path -LiteralPath $SdkConfig).Path
$BuildRoot = [IO.Path]::GetFullPath($BuildRoot)
if ($BuildRoot -notmatch '[/\\]_build([/\\]|$)') { throw 'Generated output must be under _build' }
& cargo fmt --manifest-path (Join-Path $pluginRoot 'Cargo.toml') -- --check
if ($LASTEXITCODE -ne 0) { throw 'Formatting failed' }
foreach ($operation in @('test', 'clippy')) {
    $arguments = @($operation, '--config', $SdkConfig, '--manifest-path', (Join-Path $pluginRoot 'Cargo.toml'),
        '--target-dir', (Join-Path $BuildRoot 'target'), '--locked', '--offline', '--all-targets')
    if ($operation -eq 'clippy') { $arguments += @('--', '-D', 'warnings') }
    & cargo @arguments
    if ($LASTEXITCODE -ne 0) { throw "$operation failed" }
}
& (Join-Path $PSScriptRoot 'build.ps1') -SdkConfig $SdkConfig -BuildRoot $BuildRoot -Profile $Profile
