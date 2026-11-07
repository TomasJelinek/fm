param(
    [Parameter(Mandatory=$true)][string]$SdkConfig,
    [Parameter(Mandatory=$true)][string]$BuildRoot,
    [string]$OutputDirectory,
    [switch]$Test,
    [switch]$Offline
)
$ErrorActionPreference = 'Stop'
$pluginRoot = Split-Path -Parent $PSScriptRoot
$buildDirectory = [IO.Path]::GetFullPath($BuildRoot)
if (($buildDirectory -split '[\\/]') -notcontains '_build') { throw 'BuildRoot must be under _build' }
if ($buildDirectory.StartsWith($pluginRoot + [IO.Path]::DirectorySeparatorChar)) { throw 'BuildRoot must not modify plugin sources' }
New-Item -ItemType Directory -Force -Path $buildDirectory | Out-Null
$savedTarget = $env:CARGO_TARGET_DIR
$savedIncremental = $env:CARGO_INCREMENTAL
$savedDevDebug = $env:CARGO_PROFILE_DEV_DEBUG
$savedTestDebug = $env:CARGO_PROFILE_TEST_DEBUG
try {
    $env:CARGO_TARGET_DIR = $buildDirectory
    $env:CARGO_INCREMENTAL = '0'
    $env:CARGO_PROFILE_DEV_DEBUG = '0'
    $env:CARGO_PROFILE_TEST_DEBUG = '0'
    $common = @('--manifest-path', (Join-Path $pluginRoot 'Cargo.toml'), '--locked', '--config', (Resolve-Path -LiteralPath $SdkConfig).Path)
    if ($Offline) { $common += '--offline' }
    if ($Test) {
        & cargo test @common
        if ($LASTEXITCODE -ne 0) { throw 'Archive plugin tests failed' }
    }
    & cargo build --release @common
    if ($LASTEXITCODE -ne 0) { throw 'Archive plugin build failed' }
    if ($OutputDirectory) {
        $package = [IO.Path]::GetFullPath($OutputDirectory)
        if (($package -split '[\\/]') -notcontains '_build') { throw 'Package staging must be under _build' }
        New-Item -ItemType Directory -Force -Path $package | Out-Null
        Copy-Item -LiteralPath (Join-Path $buildDirectory 'release/midnight-fs-archive.exe') -Destination $package
        Copy-Item -LiteralPath (Join-Path $pluginRoot 'THIRD-PARTY.md') -Destination $package
        $target = ((& rustc -vV | Select-String '^host: ').ToString() -replace '^host: ', '').Trim()
        if (-not $target) { throw 'Cannot determine plugin target' }
        $manifest = @"
schema_version = 1
id = "midnight.fs.archive"
kind = "filesystem-worker"
version = "0.1.0"
entry_executable = "midnight-fs-archive.exe"
target = "$target"
protocol = 1
formats = ["zip", "tar", "gzip", "tar-gzip", "7z", "xz", "tar-xz", "bzip2", "tar-bzip2", "zstd", "tar-zstd"]
"@
        [IO.File]::WriteAllText((Join-Path $package 'plugin.toml'), $manifest.Replace("`r`n", "`n"), [Text.UTF8Encoding]::new($false))
        & python (Join-Path $PSScriptRoot 'notices.py') --config $SdkConfig --output (Join-Path $package 'licenses')
        if ($LASTEXITCODE -ne 0) { throw 'Archive distribution notices failed' }
    }
} finally {
    $env:CARGO_TARGET_DIR = $savedTarget
    $env:CARGO_INCREMENTAL = $savedIncremental
    $env:CARGO_PROFILE_DEV_DEBUG = $savedDevDebug
    $env:CARGO_PROFILE_TEST_DEBUG = $savedTestDebug
}
