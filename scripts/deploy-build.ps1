<# Complete a Windows helper build with a protected, explicitly registered runtime. #>
param([ValidateSet('debug', 'release')][string]$Profile = 'debug')
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
$targetRoot = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { '_build' }
if (-not [IO.Path]::IsPathRooted($targetRoot)) {
    $targetRoot = Join-Path $projectRoot $targetRoot
}
$source = Join-Path ([IO.Path]::GetFullPath($targetRoot)) $Profile
$thumbnailers = Join-Path $source 'thumbnailers'
foreach ($file in @('midnight-thumbnail-image.exe', 'midnight-thumbnail-image.json',
    'midnight-thumbnail-video.exe', 'midnight-thumbnail-video.json',
    'avcodec-63.dll', 'avformat-63.dll', 'avutil-61.dll', 'swscale-10.dll')) {
    if (-not (Test-Path -LiteralPath (Join-Path $thumbnailers $file) -PathType Leaf)) {
        throw "Normal builds require both thumbnail providers and the complete video runtime: $file"
    }
}
foreach ($name in @('midnight-thumbnail-image', 'midnight-thumbnail-video')) {
    $notices = Join-Path $thumbnailers "notices/$name"
    if (-not (Test-Path -LiteralPath $notices -PathType Container) -or
        -not (Get-ChildItem -LiteralPath $notices -Recurse -File | Select-Object -First 1)) {
        throw "Missing thumbnail dependency notices: $name"
    }
}
foreach ($file in @('ffmpeg-9.0.2.tar.xz', 'COPYING.LGPLv2.1', 'build-options.txt', 'config.h', 'config.mak')) {
    if (-not (Test-Path -LiteralPath (Join-Path $thumbnailers "notices/ffmpeg-9.0.2/$file") -PathType Leaf)) {
        throw "Missing corresponding FFmpeg source or build notice: $file"
    }
}
& python -B (Join-Path $PSScriptRoot 'package-release.py') validate-runtime --source $source --target 'x86_64-pc-windows-msvc'
if ($LASTEXITCODE -ne 0) {
    throw 'Normal deployment requires all viewer, filesystem, thumbnail and elevation modules with their complete runtime notices'
}
$deployment = @{
    SourceDirectory = $source
    RegisterRuntime = $true
    IncludeFilesystems = $true
    IncludeThumbnailers = $true
}
& (Join-Path $PSScriptRoot 'deploy-private.ps1') @deployment
