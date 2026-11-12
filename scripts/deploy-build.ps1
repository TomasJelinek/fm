<# Complete a Windows helper build with a protected, explicitly registered runtime. #>
param([ValidateSet('debug', 'release')][string]$Profile = 'debug')
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
$targetRoot = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { '_build' }
if (-not [IO.Path]::IsPathRooted($targetRoot)) {
    $targetRoot = Join-Path $projectRoot $targetRoot
}
$source = Join-Path ([IO.Path]::GetFullPath($targetRoot)) $Profile
$deployment = @{
    SourceDirectory = $source
    RegisterRuntime = $true
    IncludeFilesystems = (Test-Path -LiteralPath (Join-Path $source 'filesystems') -PathType Container)
    IncludeThumbnailers = (Test-Path -LiteralPath (Join-Path $source 'thumbnailers') -PathType Container)
}
& (Join-Path $PSScriptRoot 'deploy-private.ps1') @deployment
