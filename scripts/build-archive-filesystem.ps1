<# Normal builds install the independent archive worker. #>
param(
    [ValidateSet('debug', 'release')][string]$Profile = 'debug',
    [string]$WorkerSdkConfig,
    [string]$AppDirectory,
    [switch]$Offline
)
$ErrorActionPreference = 'Stop'
if (-not $WorkerSdkConfig) { $WorkerSdkConfig = $env:MIDNIGHT_ARCHIVE_SDK_CONFIG }
if ($env:MIDNIGHT_ARCHIVE_OFFLINE -eq '1' -or $env:MIDNIGHT_VIEWERS_OFFLINE -eq '1') { $Offline = $true }
$savedEnvironment = [Collections.Generic.Dictionary[string,string]]::new([StringComparer]::OrdinalIgnoreCase)
foreach ($entry in [Environment]::GetEnvironmentVariables('Process').GetEnumerator()) {
    $savedEnvironment[$entry.Key] = $entry.Value
}
$projectRoot = Split-Path -Parent $PSScriptRoot
$buildRoot = Join-Path $projectRoot '_build/archive-filesystems/windows'
Push-Location -LiteralPath $projectRoot
try {
    $localCargo = Join-Path $projectRoot '.tools/cargo/bin'
    if (Test-Path -LiteralPath (Join-Path $localCargo 'cargo.exe')) {
        $env:CARGO_HOME = Join-Path $projectRoot '.tools/cargo'
        $env:RUSTUP_HOME = Join-Path $projectRoot '.tools/rustup'
        $env:PATH = $localCargo + [IO.Path]::PathSeparator + $env:PATH
    }
    if (-not $AppDirectory) {
        $targetRoot = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { '_build' }
        $AppDirectory = [IO.Path]::GetFullPath((Join-Path $targetRoot $Profile))
    }
    if (-not $WorkerSdkConfig) {
        $stampRoots = @('api/tools/package', 'worker-api', 'thumbnail-api', 'filesystem-api', 'image-viewer-api')
        $stamp = ($stampRoots | ForEach-Object {
            Get-ChildItem -LiteralPath (Join-Path $projectRoot $_) -Recurse -File
        } | Sort-Object FullName | ForEach-Object {
            $_.FullName.Substring($projectRoot.Length) + ':' + (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash
        }) -join "`n"
        foreach ($cache in @('thumbnailers', 'image-viewer')) {
            $statePath = Join-Path $projectRoot "_build/$cache/sdk-state.json"
            if (Test-Path -LiteralPath $statePath) {
                $state = Get-Content -LiteralPath $statePath -Raw | ConvertFrom-Json
                if ($state.stamp -eq $stamp -and (Test-Path -LiteralPath $state.config -PathType Leaf)) {
                    $WorkerSdkConfig = $state.config
                    break
                }
            }
        }
        if (-not $WorkerSdkConfig) {
            throw 'A current public worker SDK is required. Run build.bat debug to prepare it through the viewer/thumbnail builds, or set MIDNIGHT_ARCHIVE_SDK_CONFIG to its .cargo/registry.toml.'
        }
    }
    $WorkerSdkConfig = (Resolve-Path -LiteralPath $WorkerSdkConfig).Path
    $temporaryRoot = Join-Path $projectRoot '_build/tmp'
    New-Item -ItemType Directory -Force $buildRoot, $temporaryRoot | Out-Null
    $env:TEMP = $temporaryRoot
    $env:TMP = $temporaryRoot
    $fetch = @('--config', $WorkerSdkConfig, 'fetch', '--manifest-path',
        (Join-Path $projectRoot 'plugins/midnight-fs-archive/Cargo.toml'), '--locked')
    if ($Offline) { $fetch += '--offline' }
    & cargo @fetch
    if ($LASTEXITCODE -ne 0) { throw 'Cannot fetch archive filesystem dependencies' }
    & (Join-Path $projectRoot 'plugins/midnight-fs-archive/scripts/build.ps1') `
        -SdkConfig $WorkerSdkConfig -BuildRoot $buildRoot `
        -OutputDirectory (Join-Path $AppDirectory 'filesystems/midnight.fs.archive') `
        -Profile $Profile -Offline:$Offline
    if ($LASTEXITCODE -ne 0) { throw 'Archive filesystem build failed' }
} finally {
    Pop-Location
    foreach ($variable in [Environment]::GetEnvironmentVariables('Process').Keys) {
        if (-not $savedEnvironment.ContainsKey($variable)) {
            Remove-Item -LiteralPath "Env:$variable" -ErrorAction SilentlyContinue
        }
    }
    foreach ($variable in $savedEnvironment.Keys) {
        [Environment]::SetEnvironmentVariable($variable, $savedEnvironment[$variable], 'Process')
    }
}
