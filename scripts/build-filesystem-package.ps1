<# Build an independent filesystem package beside the application. #>
param(
    [Parameter(Mandatory)][ValidateSet('native', 'network')][string]$Provider,
    [ValidateSet('debug', 'release')][string]$Profile = 'debug',
    [string]$AppDirectory,
    [string]$BuildRoot,
    [switch]$Test,
    [switch]$Offline
)
$ErrorActionPreference = 'Stop'
if ($env:OS -ne 'Windows_NT') { throw 'Use the .sh filesystem helper on Linux' }
$projectRoot = Split-Path -Parent $PSScriptRoot
$pluginName = if ($Provider -eq 'native') { 'midnight-filesystem-native-windows' } else { 'midnight-filesystem-network' }
$pluginRoot = Join-Path $projectRoot "plugins/$pluginName"
$sourceManifest = if ($Provider -eq 'native') { 'plugin.toml' } else { 'plugin.windows.toml' }
$libraryName = if ($Provider -eq 'native') { 'midnight_filesystem_native_windows.dll' } else { 'midnight_filesystem_network.dll' }
$environmentPrefix = 'MIDNIGHT_' + $Provider.ToUpperInvariant() + '_FILESYSTEM'
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
    if (-not $BuildRoot) { $BuildRoot = [Environment]::GetEnvironmentVariable($environmentPrefix + '_BUILD_ROOT') }
    if (-not $BuildRoot) { $BuildRoot = Join-Path $projectRoot "_build/$Provider-filesystems/windows" }
    $env:CARGO_TARGET_DIR = [IO.Path]::GetFullPath($BuildRoot)
    $manifest = Join-Path $pluginRoot 'Cargo.toml'
    $options = @('--locked', '--manifest-path', $manifest)
    if ($Offline -or [Environment]::GetEnvironmentVariable($environmentPrefix + '_OFFLINE') -eq '1') { $options += '--offline' }
    if ($Profile -eq 'release') { $options += '--release' }
    $cargo = Join-Path $PSScriptRoot 'cargo.ps1'
    if ($Test -or [Environment]::GetEnvironmentVariable('MIDNIGHT_TEST_' + $Provider.ToUpperInvariant() + '_FILESYSTEM') -eq '1') {
        $testArguments = @('test') + $options
        & $cargo @testArguments
        if ($LASTEXITCODE -ne 0) { throw 'Independent filesystem tests failed' }
    }
    $buildArguments = @('build') + $options
    & $cargo @buildArguments
    if ($LASTEXITCODE -ne 0) { throw 'Independent filesystem build failed' }
    $package = Join-Path $AppDirectory "filesystems/$Provider.windows"
    [IO.Directory]::CreateDirectory($package) | Out-Null
    $library = Join-Path $env:CARGO_TARGET_DIR "$Profile/$libraryName"
    $packageFiles = @($libraryName, 'plugin.toml', 'README.md', 'THIRD-PARTY.md')
    $stage = Join-Path $package ('.' + $Provider + '-stage.' + [guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $stage | Out-Null
    try {
        # Resolve every input before publishing. A locked installed DLL fails first,
        # before any matching manifest or notices can replace the previous package.
        Copy-Item -LiteralPath $library -Destination (Join-Path $stage $libraryName)
        Copy-Item -LiteralPath (Join-Path $pluginRoot $sourceManifest) -Destination (Join-Path $stage 'plugin.toml')
        foreach ($name in @('README.md', 'THIRD-PARTY.md')) {
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
    Write-Output "Filesystem package: $package"
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
