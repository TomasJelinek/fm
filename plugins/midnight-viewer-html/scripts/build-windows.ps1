param(
    [Parameter(Mandatory=$true)][string]$SdkConfig,
    [string]$CefDistribution,
    [string]$CefSdk,
    [string]$TargetDirectory,
    [string]$OutputDirectory,
    [switch]$Release,
    # Compile only; scripts/build-html-viewer.ps1 packages when an input changed.
    [switch]$BuildOnly
)
$ErrorActionPreference = 'Stop'
$project = Split-Path $PSScriptRoot -Parent
$repository = Split-Path (Split-Path $project -Parent) -Parent
if (-not $TargetDirectory) { $TargetDirectory = Join-Path $repository '_build/html-viewer/provider-target' }
New-Item -ItemType Directory -Force $TargetDirectory | Out-Null
$TargetDirectory = (Resolve-Path -LiteralPath $TargetDirectory).Path
if ($CefSdk) {
    if ($CefDistribution) { throw 'Pass either -CefSdk or -CefDistribution, not both' }
    # A flattened SDK verified by scripts/prepare-html-cef.py is used in place: Cargo
    # compiles the wrapper from it and the package takes its runtime files from it,
    # checked against its receipt. Nothing of it is copied into the target directory.
    $cefStage = (Resolve-Path -LiteralPath $CefSdk).Path
    $receipt = $cefStage + '.verified.json'
    if (-not (Test-Path -LiteralPath $receipt -PathType Leaf) -or
        -not (Test-Path -LiteralPath (Join-Path $cefStage 'archive.json') -PathType Leaf)) {
        throw 'The CEF SDK must be a flattened SDK verified by scripts/prepare-html-cef.py (with its .verified.json receipt)'
    }
} else {
    # A manually extracted official distribution is flattened into the target directory
    # once, as cef-dll-sys requires; its receipt lists the staged files for the package.
    if (-not $CefDistribution) { $CefDistribution = Join-Path $repository '_build/html-viewer/cef/cef_binary_156.0.26+g8ea0c73+chromium-156.0.8078.88_windows64_minimal' }
    $CefDistribution = (Resolve-Path -LiteralPath $CefDistribution).Path
    $cefStage = Join-Path $TargetDirectory 'cef-sdk'
    $receipt = $cefStage + '.staged.json'
    if (-not (Test-Path -LiteralPath $receipt -PathType Leaf)) {
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
        $prefix = $cefStage.TrimEnd('\') + '\'
        $files = [ordered]@{}
        foreach ($file in (Get-ChildItem -LiteralPath $cefStage -Recurse -File | Sort-Object FullName)) {
            $files[$file.FullName.Substring($prefix.Length).Replace('\', '/')] =
                (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
        }
        # Written last: an interrupted staging is staged again.
        [IO.File]::WriteAllText($receipt, (@{ files = $files } | ConvertTo-Json -Depth 3), [Text.UTF8Encoding]::new($false))
    }
}
$env:CEF_PATH = $cefStage
$ninja = 'C:/Program Files/Microsoft Visual Studio/2022/Professional/Common7/IDE/CommonExtensions/Microsoft/CMake/Ninja'
if (Test-Path -LiteralPath $ninja) { $env:PATH = $ninja + ';' + $env:PATH }
$arguments = @('build','--manifest-path',(Join-Path $project 'Cargo.toml'),'--config',(Join-Path $project '.cargo/config.toml'),
    '--config',$SdkConfig,'--target-dir',$TargetDirectory,'--locked','--offline','--lib')
if ($Release) { $arguments += '--release' }
& cargo @arguments
if ($LASTEXITCODE -ne 0) { throw 'CEF provider build failed' }
if ($BuildOnly) { return }
& (Join-Path $PSScriptRoot 'package-windows.ps1') -SdkConfig $SdkConfig -TargetDirectory $TargetDirectory `
    -CefDirectory $cefStage -CefReceipt $receipt -OutputDirectory $OutputDirectory -Release:$Release
