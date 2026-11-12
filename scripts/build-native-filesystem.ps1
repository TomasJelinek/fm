<# Build the independent native filesystem package beside the application. #>
param(
    [ValidateSet('debug', 'release')][string]$Profile = 'debug',
    [string]$AppDirectory,
    [string]$BuildRoot,
    [switch]$Test,
    [switch]$Offline
)
$ErrorActionPreference = 'Stop'
& (Join-Path $PSScriptRoot 'build-filesystem-package.ps1') -Provider native @PSBoundParameters
