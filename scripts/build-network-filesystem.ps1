<# Build the independent Network Neighborhood package beside the application. #>
param(
    [ValidateSet('debug', 'release')][string]$Profile = 'debug',
    [string]$AppDirectory,
    [string]$BuildRoot,
    [switch]$Test,
    [switch]$Offline
)
$ErrorActionPreference = 'Stop'
& (Join-Path $PSScriptRoot 'build-filesystem-package.ps1') -Provider network @PSBoundParameters
