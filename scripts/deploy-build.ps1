<# Complete a Windows helper build with a protected, explicitly registered runtime. #>
param([ValidateSet('debug', 'release')][string]$Profile = 'debug')
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
$targetRoot = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { '_build' }
if (-not [IO.Path]::IsPathRooted($targetRoot)) {
    $targetRoot = Join-Path $projectRoot $targetRoot
}
$source = Join-Path ([IO.Path]::GetFullPath($targetRoot)) $Profile
# One validator checks every viewer, filesystem and thumbnail module, the FFmpeg
# runtime with its corresponding source/build notices and the elevation helper.
# Python 3.12+ runs through the documented py launcher; python may be a Store alias.
$python = 'python'
$pythonArguments = @('-B')
if (Get-Command py -CommandType Application -ErrorAction Ignore) {
    $python = 'py'
    $pythonArguments = @('-3', '-B')
}
& $python @pythonArguments (Join-Path $PSScriptRoot 'package-release.py') validate-runtime --source $source --target 'x86_64-pc-windows-msvc'
if ($LASTEXITCODE -ne 0) {
    throw 'Normal deployment requires all viewer, filesystem, thumbnail and elevation modules with their complete runtime notices'
}
# Each build replaces the previous build deployment of this output directory
# instead of accumulating complete private installations in the user profile.
$deployment = @{
    SourceDirectory = $source
    RegisterRuntime = $true
    IncludeFilesystems = $true
    IncludeThumbnailers = $true
    ReplaceBuildDeployment = $true
}
& (Join-Path $PSScriptRoot 'deploy-private.ps1') @deployment
