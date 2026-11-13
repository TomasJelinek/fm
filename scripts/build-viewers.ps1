param(
    [ValidateSet('debug', 'release')][string]$Profile = 'debug',
    [string]$SdkConfig,
    [string]$WorkerSdkConfig,
    [string]$AppDirectory,
    [switch]$Test,
    [switch]$Offline
)
$ErrorActionPreference = 'Stop'
if (-not $SdkConfig) { $SdkConfig = $env:MIDNIGHT_VIEWER_SDK_CONFIG }
if ($env:MIDNIGHT_VIEWERS_OFFLINE -eq '1') { $Offline = $true }
$savedEnvironment = @{}
foreach ($variable in @('TEMP','TMP','CARGO_HOME','RUSTUP_HOME','PATH','CARGO_TARGET_DIR')) {
    $savedEnvironment[$variable] = [Environment]::GetEnvironmentVariable($variable)
}
$projectRoot = Split-Path -Parent $PSScriptRoot
$buildRoot = Join-Path $projectRoot '_build/viewers'
$temporaryRoot = Join-Path $projectRoot '_build/tmp'
New-Item -ItemType Directory -Force $buildRoot, $temporaryRoot | Out-Null
$env:TEMP = $temporaryRoot
$env:TMP = $temporaryRoot
$localCargo = Join-Path $projectRoot '.tools/cargo/bin'
if (Test-Path -LiteralPath (Join-Path $localCargo 'cargo.exe')) {
    $env:CARGO_HOME = Join-Path $projectRoot '.tools/cargo'
    $env:RUSTUP_HOME = Join-Path $projectRoot '.tools/rustup'
    $env:PATH = $localCargo + [IO.Path]::PathSeparator + $env:PATH
}
Push-Location -LiteralPath $projectRoot
try {
    if (-not $AppDirectory) {
        $targetRoot = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { '_build' }
        $AppDirectory = [IO.Path]::GetFullPath((Join-Path $targetRoot $Profile))
    }
    if (-not $SdkConfig) {
        $stamp = (Get-ChildItem -LiteralPath (Join-Path $projectRoot 'api') -Recurse -File |
            Sort-Object FullName | ForEach-Object {
                $_.FullName.Substring($projectRoot.Length) + ':' +
                    (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash
            }) -join "`n"
        $statePath = Join-Path $buildRoot 'sdk-state.json'
        if (Test-Path -LiteralPath $statePath) {
            $state = Get-Content -LiteralPath $statePath -Raw | ConvertFrom-Json
            if ($state.stamp -eq $stamp -and (Test-Path -LiteralPath $state.config)) {
                $SdkConfig = $state.config
            }
        }
        if (-not $SdkConfig) {
            $fetch = @('fetch','--manifest-path','api/Cargo.toml','--locked')
            if ($Offline) { $fetch += '--offline' }
            & cargo @fetch
            if ($LASTEXITCODE -ne 0) { throw 'Cannot fetch public SDK dependencies' }
            $env:CARGO_TARGET_DIR = Join-Path $buildRoot 'sdk-target'
            $output = & (Join-Path $projectRoot 'api/scripts/package.ps1')
            if ($LASTEXITCODE -ne 0) { throw 'Cannot package public SDK' }
            $prefix = 'Public API/SDK release bundle: '
            $line = @($output | Where-Object { $_.ToString().StartsWith($prefix) })[-1]
            if (-not $line) { throw 'SDK packager did not report its output directory' }
            $SdkConfig = Join-Path ($line.ToString().Substring($prefix.Length)) '.cargo/config.toml'
            @{ stamp = $stamp; config = $SdkConfig } | ConvertTo-Json |
                Set-Content -LiteralPath $statePath
        }
    }
    $SdkConfig = (Resolve-Path -LiteralPath $SdkConfig).Path
    $nativeTarget = ((& rustc -vV | Select-String '^host: ').ToString() -replace '^host: ', '').Trim()
    if ($LASTEXITCODE -ne 0 -or -not $nativeTarget) { throw 'Cannot determine native plugin target' }
    foreach ($name in @('basic', 'source', 'markdown')) {
        $package = "midnight-viewer-$name"
        $pluginRoot = Join-Path $projectRoot "plugins/$package"
        $pluginBuild = Join-Path $buildRoot "$nativeTarget/$name"
        $prepare = Join-Path $pluginRoot 'scripts/prepare-sdk.ps1'
        $config = if (Test-Path -LiteralPath $prepare) {
            & $prepare -SdkConfig $SdkConfig -BuildRoot $pluginBuild
        } else { $SdkConfig }
        $fetch = @('--config',$config,'fetch','--manifest-path',(Join-Path $pluginRoot 'Cargo.toml'),'--locked')
        if ($Offline) { $fetch += '--offline' }
        & cargo @fetch
        if ($LASTEXITCODE -ne 0) { throw "Cannot fetch $package dependencies" }
        if ($Test) {
            & (Join-Path $pluginRoot 'scripts/ci.ps1') -SdkConfig $SdkConfig -BuildRoot $pluginBuild
            if ($LASTEXITCODE -ne 0) { throw "$package tests failed" }
        }
        & (Join-Path $pluginRoot 'scripts/build.ps1') -SdkConfig $SdkConfig -BuildRoot $pluginBuild `
            -OutputDirectory (Join-Path $AppDirectory "viewers/$package") -Profile $Profile
        if ($LASTEXITCODE -ne 0) { throw "$package build failed" }
    }
    foreach ($viewer in @('image', 'pdf')) {
        & (Join-Path $PSScriptRoot 'build-image-viewer.ps1') -Viewer $viewer -Profile $Profile -WorkerSdkConfig $WorkerSdkConfig `
            -AppDirectory $AppDirectory -Test:$Test -Offline:$Offline
        if ($LASTEXITCODE -ne 0) { throw "$viewer viewer build failed" }
    }
    & (Join-Path $PSScriptRoot 'build-html-viewer.ps1') -Profile $Profile `
        -AppDirectory $AppDirectory -Offline:$Offline
    if ($LASTEXITCODE -ne 0) { throw 'HTML viewer build failed' }
} finally {
    Pop-Location
    foreach ($variable in $savedEnvironment.Keys) {
        if ($null -eq $savedEnvironment[$variable]) {
            Remove-Item -LiteralPath "Env:$variable" -ErrorAction SilentlyContinue
        } else {
            [Environment]::SetEnvironmentVariable($variable, $savedEnvironment[$variable])
        }
    }
}
