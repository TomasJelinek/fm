param(
    [Parameter(Mandatory = $true)][string]$InstallationDirectory,
    [Parameter(Mandatory = $true)][string]$ExpectedExecutableSha256
)
$ErrorActionPreference = 'Stop'
if ($env:OS -ne 'Windows_NT') { throw 'Desktop shortcut update requires Windows' }
. (Join-Path $PSScriptRoot 'windows-shortcut.ps1')
$desktop = [Environment]::GetFolderPath([Environment+SpecialFolder]::DesktopDirectory)
if (-not $desktop) { throw 'No resolved Desktop directory is available' }
Install-MidnightDesktopShortcut $InstallationDirectory $desktop $ExpectedExecutableSha256
