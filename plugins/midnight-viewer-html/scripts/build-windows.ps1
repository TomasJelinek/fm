param(
    [Parameter(Mandatory=$true)][string]$SdkConfig,
    [string]$CefDistribution,
    [string]$TargetDirectory,
    [string]$OutputDirectory,
    [switch]$Release
)
$ErrorActionPreference = 'Stop'
$project = Split-Path $PSScriptRoot -Parent
$repository = Split-Path (Split-Path $project -Parent) -Parent
if (-not $TargetDirectory) { $TargetDirectory = Join-Path $repository '_build/html-viewer/provider-target' }
if (-not $CefDistribution) { $CefDistribution = Join-Path $repository '_build/html-viewer/cef/cef_binary_156.0.26+g8ea0c73+chromium-156.0.8078.88_windows64_minimal' }
$CefDistribution = (Resolve-Path -LiteralPath $CefDistribution).Path
New-Item -ItemType Directory -Force $TargetDirectory | Out-Null
$TargetDirectory = (Resolve-Path -LiteralPath $TargetDirectory).Path
$cefStage = Join-Path $TargetDirectory 'cef-sdk'
if (-not (Test-Path -LiteralPath (Join-Path $cefStage 'archive.json'))) {
    New-Item -ItemType Directory -Force $cefStage | Out-Null
    foreach ($directory in @('Release','Resources')) {
        Get-ChildItem -LiteralPath (Join-Path $CefDistribution $directory) | Copy-Item -Destination $cefStage -Recurse -Force
    }
    foreach ($directory in @('include','cmake','libcef_dll')) {
        Copy-Item -LiteralPath (Join-Path $CefDistribution $directory) -Destination $cefStage -Recurse -Force
    }
    foreach ($file in @('CMakeLists.txt','LICENSE.txt','CREDITS.html')) {
        Copy-Item -LiteralPath (Join-Path $CefDistribution $file) -Destination $cefStage -Force
    }
    @{type='minimal';name='cef_binary_156.0.26+g8ea0c73+chromium-156.0.8078.88_windows64_minimal.tar.bz2';sha1='15d29eea209d857c22dc163706080295aea69b5c'} |
        ConvertTo-Json | Set-Content -LiteralPath (Join-Path $cefStage 'archive.json')
}
$env:CEF_PATH = $cefStage
$ninja = 'C:/Program Files/Microsoft Visual Studio/2022/Professional/Common7/IDE/CommonExtensions/Microsoft/CMake/Ninja'
if (Test-Path -LiteralPath $ninja) { $env:PATH = $ninja + ';' + $env:PATH }
$arguments = @('build','--manifest-path',(Join-Path $project 'Cargo.toml'),'--config',(Join-Path $project '.cargo/config.toml'),
    '--config',$SdkConfig,'--target-dir',$TargetDirectory,'--locked','--offline','--lib')
if ($Release) { $arguments += '--release' }
& cargo @arguments
if ($LASTEXITCODE -ne 0) { throw 'CEF provider build failed' }
& (Join-Path $PSScriptRoot 'package-windows.ps1') -SdkConfig $SdkConfig -TargetDirectory $TargetDirectory `
    -OutputDirectory $OutputDirectory -Release:$Release
