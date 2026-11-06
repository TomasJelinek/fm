param([Parameter(Mandatory)][string]$SdkConfig, [Parameter(Mandatory)][string]$BuildRoot)
$ErrorActionPreference = 'Stop'
$pluginRoot = Split-Path -Parent $PSScriptRoot
$config = & (Join-Path $PSScriptRoot 'prepare-sdk.ps1') -SdkConfig $SdkConfig -BuildRoot $BuildRoot
$manifest = Join-Path $pluginRoot 'Cargo.toml'
& cargo fmt --manifest-path $manifest -- --check
if ($LASTEXITCODE -ne 0) { throw 'Formatting failed' }
& cargo --config $config test --manifest-path $manifest --target-dir (Join-Path $BuildRoot 'target') --all-targets --locked --offline
if ($LASTEXITCODE -ne 0) { throw 'Plugin tests failed' }
& cargo clippy --config $config --manifest-path $manifest --target-dir (Join-Path $BuildRoot 'target') --all-targets --locked --offline -- -D warnings
if ($LASTEXITCODE -ne 0) { throw 'Clippy failed' }
& (Join-Path $PSScriptRoot 'build.ps1') -SdkConfig $SdkConfig -BuildRoot $BuildRoot
