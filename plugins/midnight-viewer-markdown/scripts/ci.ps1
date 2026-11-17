param([Parameter(Mandatory)][string]$SdkConfig, [Parameter(Mandatory)][string]$BuildRoot)
$ErrorActionPreference = 'Stop'
$pluginRoot = Split-Path -Parent $PSScriptRoot
$BuildRoot = [IO.Path]::GetFullPath($ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($BuildRoot))
$config = & (Join-Path $PSScriptRoot 'prepare-sdk.ps1') -SdkConfig $SdkConfig -BuildRoot $BuildRoot
$manifest = Join-Path $pluginRoot 'Cargo.toml'
# The package build selects the native --target; test and lint with the same
# target so Cargo reuses one dependency build instead of compiling it twice.
$nativeTarget = ((& rustc -vV | Select-String '^host: ').ToString() -replace '^host: ', '').Trim()
if ($LASTEXITCODE -ne 0 -or -not $nativeTarget) { throw 'Native target discovery failed' }
$common = @('--manifest-path', $manifest, '--target-dir', (Join-Path $BuildRoot 'target'),
    '--target', $nativeTarget, '--all-targets', '--locked', '--offline')
& cargo fmt --manifest-path $manifest -- --check
if ($LASTEXITCODE -ne 0) { throw 'Formatting failed' }
& cargo --config $config test @common
if ($LASTEXITCODE -ne 0) { throw 'Plugin tests failed' }
& cargo clippy --config $config @common -- -D warnings
if ($LASTEXITCODE -ne 0) { throw 'Clippy failed' }
& (Join-Path $PSScriptRoot 'build.ps1') -SdkConfig $SdkConfig -BuildRoot $BuildRoot
