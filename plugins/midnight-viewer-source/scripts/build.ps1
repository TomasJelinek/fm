param(
    [Parameter(Mandatory)][string]$SdkConfig,
    [Parameter(Mandatory)][string]$BuildRoot,
    [string]$OutputDirectory
)
$ErrorActionPreference = 'Stop'
$pluginRoot = Split-Path -Parent $PSScriptRoot
$BuildRoot = [IO.Path]::GetFullPath($BuildRoot)
if (-not $OutputDirectory) { $OutputDirectory = Join-Path $BuildRoot 'package/midnight-viewer-source' }
$OutputDirectory = [IO.Path]::GetFullPath($OutputDirectory)
$config = & (Join-Path $PSScriptRoot 'prepare-sdk.ps1') -SdkConfig $SdkConfig -BuildRoot $BuildRoot
$nativeTarget = ((& rustc -vV | Select-String '^host: ').ToString() -replace '^host: ', '').Trim()
if ($LASTEXITCODE -ne 0) { throw 'Native target discovery failed' }
if ($nativeTarget -match '-windows-') { $library = 'midnight_viewer_source.dll' }
elseif ($nativeTarget -match '-linux-') { $library = 'libmidnight_viewer_source.so' }
else { throw "Unsupported native target: $nativeTarget" }
& cargo --config $config build --manifest-path (Join-Path $pluginRoot 'Cargo.toml') --target-dir (Join-Path $BuildRoot 'target') --target $nativeTarget --release --locked --offline
if ($LASTEXITCODE -ne 0) { throw 'Plugin release build failed' }
New-Item -ItemType Directory -Force $OutputDirectory | Out-Null
Copy-Item -LiteralPath (Join-Path $BuildRoot "target/$nativeTarget/release/$library") -Destination $OutputDirectory
$manifest = Get-Content -LiteralPath (Join-Path $pluginRoot 'plugin.toml') -Raw
$manifest = $manifest -replace 'entry_library = "[^"]+"', "entry_library = `"$library`""
$manifest = $manifest -replace 'target = "[^"]+"', "target = `"$nativeTarget`""
[IO.File]::WriteAllText((Join-Path $OutputDirectory 'plugin.toml'), $manifest)
foreach ($notice in @('LICENSE', 'README.md', 'THIRD-PARTY.md')) {
    Copy-Item -LiteralPath (Join-Path $pluginRoot $notice) -Destination $OutputDirectory
}
Copy-Item -LiteralPath (Join-Path $pluginRoot 'notices') -Destination $OutputDirectory -Recurse -Force
Write-Output "Viewer package: $OutputDirectory"
