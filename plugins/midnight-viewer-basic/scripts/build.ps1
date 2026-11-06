param(
    [Parameter(Mandatory)][string]$SdkConfig,
    [Parameter(Mandatory)][string]$BuildRoot,
    [string]$OutputDirectory
)
$ErrorActionPreference = 'Stop'
$pluginRoot = Split-Path -Parent $PSScriptRoot
$SdkConfig = (Resolve-Path -LiteralPath $SdkConfig).Path
$BuildRoot = [IO.Path]::GetFullPath($BuildRoot)
if (-not $OutputDirectory) { $OutputDirectory = Join-Path $BuildRoot 'package/midnight-viewer-basic' }
$OutputDirectory = [IO.Path]::GetFullPath($OutputDirectory)
New-Item -ItemType Directory -Force $BuildRoot, $OutputDirectory | Out-Null
$nativeTarget = ((& rustc -vV | Select-String '^host: ').ToString() -replace '^host: ', '').Trim()
if ($LASTEXITCODE -ne 0) { throw 'rustc target discovery failed' }
if ($nativeTarget -match '-windows-') { $library = 'midnight_viewer_basic.dll' }
elseif ($nativeTarget -match '-linux-') { $library = 'libmidnight_viewer_basic.so' }
else { throw "Unsupported native target: $nativeTarget" }
& cargo --config $SdkConfig build --manifest-path (Join-Path $pluginRoot 'Cargo.toml') `
    --target-dir (Join-Path $BuildRoot 'target') --target $nativeTarget --release --locked --offline
if ($LASTEXITCODE -ne 0) { throw 'Plugin release build failed' }
Copy-Item -LiteralPath (Join-Path $BuildRoot "target/$nativeTarget/release/$library") `
    -Destination (Join-Path $OutputDirectory $library)
$manifest = Get-Content -LiteralPath (Join-Path $pluginRoot 'plugin.toml') -Raw
$manifest = $manifest -replace 'entry_library = "[^"]+"', "entry_library = `"$library`""
$manifest = $manifest -replace 'target = "[^"]+"', "target = `"$nativeTarget`""
[IO.File]::WriteAllText((Join-Path $OutputDirectory 'plugin.toml'), $manifest)
foreach ($notice in @('LICENSE', 'UNICODE-LICENSE.txt', 'README.md', 'THIRD-PARTY.md')) {
    Copy-Item -LiteralPath (Join-Path $pluginRoot $notice) -Destination $OutputDirectory
}
$notices = Join-Path $OutputDirectory 'notices'
New-Item -ItemType Directory -Path $notices -Force | Out-Null
Get-ChildItem -LiteralPath (Join-Path $pluginRoot 'notices') | ForEach-Object {
    Copy-Item -LiteralPath $_.FullName -Destination $notices -Recurse -Force
}
Write-Output "Basic viewer package: $OutputDirectory"
