<# Build the independent native filesystem package beside the application. #>
param(
    [ValidateSet('debug', 'release')][string]$Profile = 'debug',
    [string]$AppDirectory,
    [string]$BuildRoot,
    [switch]$Test,
    [switch]$Offline
)
$ErrorActionPreference = 'Stop'
if ($env:OS -ne 'Windows_NT') { throw 'Use build-native-filesystem.sh on Linux' }
$projectRoot = Split-Path -Parent $PSScriptRoot
$pluginRoot = Join-Path $projectRoot 'plugins/midnight-filesystem-native-windows'
$savedEnvironment = @{}
foreach ($name in @('CARGO_TARGET_DIR', 'CARGO_HOME', 'RUSTUP_HOME', 'PATH')) {
    $savedEnvironment[$name] = [Environment]::GetEnvironmentVariable($name)
}
Push-Location -LiteralPath $projectRoot
try {
    if (-not $AppDirectory) {
        $targetRoot = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { '_build' }
        $AppDirectory = [IO.Path]::GetFullPath((Join-Path $targetRoot $Profile))
    }
    if (-not $BuildRoot) { $BuildRoot = Join-Path $projectRoot '_build/native-filesystems/windows' }
    $env:CARGO_TARGET_DIR = [IO.Path]::GetFullPath($BuildRoot)
    $manifest = Join-Path $pluginRoot 'Cargo.toml'
    $options = @('--locked', '--manifest-path', $manifest)
    if ($Offline -or $env:MIDNIGHT_NATIVE_FILESYSTEM_OFFLINE -eq '1') { $options += '--offline' }
    if ($Profile -eq 'release') { $options += '--release' }
    $cargo = Join-Path $PSScriptRoot 'cargo.ps1'
    if ($Test) {
        $testArguments = @('test') + $options
        & $cargo @testArguments
        if ($LASTEXITCODE -ne 0) { throw 'Independent native filesystem tests failed' }
    }
    $buildArguments = @('build') + $options
    & $cargo @buildArguments
    if ($LASTEXITCODE -ne 0) { throw 'Independent native filesystem build failed' }
    $package = Join-Path $AppDirectory 'filesystems/native.windows'
    [IO.Directory]::CreateDirectory($package) | Out-Null
    $library = Join-Path $env:CARGO_TARGET_DIR "$Profile/midnight_filesystem_native_windows.dll"
    $libraryName = 'midnight_filesystem_native_windows.dll'
    $packageFiles = @($libraryName, 'plugin.toml', 'README.md', 'THIRD-PARTY.md')
    $stage = Join-Path $package ('.native-stage.' + [guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $stage | Out-Null
    try {
        # Resolve every input before publishing. A locked installed DLL fails first,
        # before any matching manifest or notices can replace the previous package.
        Copy-Item -LiteralPath $library -Destination (Join-Path $stage $libraryName)
        foreach ($name in $packageFiles | Select-Object -Skip 1) {
            Copy-Item -LiteralPath (Join-Path $pluginRoot $name) -Destination $stage
        }
        foreach ($name in $packageFiles) {
            Move-Item -LiteralPath (Join-Path $stage $name) -Destination (Join-Path $package $name) -Force
        }
    } finally {
        # Remove only the known files in our exclusive staging directory.
        foreach ($name in $packageFiles) {
            $stagedFile = Join-Path $stage $name
            if (Test-Path -LiteralPath $stagedFile -PathType Leaf) {
                Remove-Item -LiteralPath $stagedFile -Force
            }
        }
        [IO.Directory]::Delete($stage)
    }
    Write-Output "Native filesystem package: $package"
} finally {
    Pop-Location
    foreach ($name in $savedEnvironment.Keys) {
        if ($null -eq $savedEnvironment[$name]) {
            Remove-Item -LiteralPath "Env:$name" -ErrorAction SilentlyContinue
        } else {
            [Environment]::SetEnvironmentVariable($name, $savedEnvironment[$name])
        }
    }
}
