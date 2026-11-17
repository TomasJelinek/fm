param(
    [Parameter(Mandatory = $true)][string]$InstallationDirectory,
    [Parameter(Mandatory = $true)][string]$ExpectedExecutableSha256
)
$ErrorActionPreference = 'Stop'
if ($env:OS -ne 'Windows_NT') { throw 'Desktop shortcut update requires Windows' }
# build.bat retires its earlier build deployments; a shortcut needs an installation
# made by deploy-private.ps1 directly (for example with -CreateDesktopShortcut).
$InstallationDirectory = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($InstallationDirectory)
if (Test-Path -LiteralPath (Join-Path $InstallationDirectory '.midnight-build-deployment')) {
    throw 'A build.bat deployment is retired by a later build; deploy a persistent installation with scripts/deploy-private.ps1 -CreateDesktopShortcut'
}
. (Join-Path $PSScriptRoot 'windows-shortcut.ps1')
$desktop = [Environment]::GetFolderPath([Environment+SpecialFolder]::DesktopDirectory)
if (-not $desktop) { throw 'No resolved Desktop directory is available' }
Install-MidnightDesktopShortcut $InstallationDirectory $desktop $ExpectedExecutableSha256
