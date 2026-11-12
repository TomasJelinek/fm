param(
    [ValidateSet('debug', 'release')][string]$Profile = 'debug',
    [string]$WorkerSdkConfig,
    [string]$AppDirectory,
    [switch]$Test,
    [switch]$Offline,
    [ValidateSet('image', 'pdf')][string]$Viewer = 'image'
)
$ErrorActionPreference = 'Stop'
if (-not $WorkerSdkConfig) { $WorkerSdkConfig = $env:MIDNIGHT_IMAGE_VIEWER_SDK_CONFIG }
if ($env:MIDNIGHT_VIEWERS_OFFLINE -eq '1') { $Offline = $true }
# Native compiler activation can introduce many variables. Restore the complete
# process environment, including the difference between absent and empty values.
$savedEnvironment = [Collections.Generic.Dictionary[string,string]]::new([StringComparer]::OrdinalIgnoreCase)
foreach ($entry in [Environment]::GetEnvironmentVariables('Process').GetEnumerator()) {
    $savedEnvironment[$entry.Key] = $entry.Value
}
$projectRoot = Split-Path -Parent $PSScriptRoot
$buildRoot = Join-Path $projectRoot "_build/$Viewer-viewer"
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
    if (-not $WorkerSdkConfig) {
        $stampRoots = @('api/tools/package', 'worker-api', 'thumbnail-api', 'filesystem-api', 'image-viewer-api')
        $stamp = ($stampRoots | ForEach-Object {
            Get-ChildItem -LiteralPath (Join-Path $projectRoot $_) -Recurse -File
        } | Sort-Object FullName | ForEach-Object {
            $_.FullName.Substring($projectRoot.Length) + ':' + (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash
        }) -join "`n"
        $statePath = Join-Path $buildRoot 'sdk-state.json'
        if (Test-Path -LiteralPath $statePath) {
            $state = Get-Content -LiteralPath $statePath -Raw | ConvertFrom-Json
            if ($state.stamp -eq $stamp -and (Test-Path -LiteralPath $state.config)) { $WorkerSdkConfig = $state.config }
        }
        if (-not $WorkerSdkConfig) {
            foreach ($workspace in @('api', 'worker-api', 'thumbnail-api', 'filesystem-api', 'image-viewer-api')) {
                $fetch = @('fetch', '--manifest-path', "$workspace/Cargo.toml", '--locked')
                if ($Offline) { $fetch += '--offline' }
                & cargo @fetch
                if ($LASTEXITCODE -ne 0) { throw "Cannot fetch $workspace dependencies" }
            }
            $env:CARGO_TARGET_DIR = Join-Path $buildRoot 'sdk-target'
            $output = & cargo run --manifest-path api/Cargo.toml -p midnight-api-package --bin worker-packages --locked --offline -- $projectRoot (Join-Path $buildRoot 'releases')
            if ($LASTEXITCODE -ne 0) { throw 'Cannot package public worker SDK' }
            $prefix = 'Public worker API/SDK release bundle: '
            $line = @($output | Where-Object { $_.ToString().StartsWith($prefix) }) | Select-Object -Last 1
            if (-not $line) { throw 'Worker SDK packager did not report its output directory' }
            $WorkerSdkConfig = Join-Path ($line.ToString().Substring($prefix.Length)) '.cargo/registry.toml'
            @{ stamp = $stamp; config = $WorkerSdkConfig } | ConvertTo-Json | Set-Content -LiteralPath $statePath
        }
    }
    $WorkerSdkConfig = (Resolve-Path -LiteralPath $WorkerSdkConfig).Path
    $pluginRoot = Join-Path $projectRoot "plugins/midnight-viewer-$Viewer"
    $pluginBuild = Join-Path $buildRoot 'windows-plugin'
    $fetch = @('--config', $WorkerSdkConfig, 'fetch', '--manifest-path', (Join-Path $pluginRoot 'Cargo.toml'), '--locked')
    if ($Offline) { $fetch += '--offline' }
    & cargo @fetch
    if ($LASTEXITCODE -ne 0) { throw 'Cannot fetch image-viewer dependencies' }
    $nativeOptions = @{}
    if ($Viewer -eq 'image') { $nativeOptions.NativeOffline = [bool]$Offline }
    if ($Test) {
        & (Join-Path $pluginRoot 'scripts/ci.ps1') -SdkConfig $WorkerSdkConfig -BuildRoot $pluginBuild @nativeOptions
        if ($LASTEXITCODE -ne 0) { throw 'Image-viewer tests failed' }
    }
    & (Join-Path $pluginRoot 'scripts/build.ps1') -SdkConfig $WorkerSdkConfig -BuildRoot $pluginBuild `
        -OutputDirectory (Join-Path $AppDirectory "viewers/midnight-viewer-$Viewer") -Profile $Profile @nativeOptions
    if ($LASTEXITCODE -ne 0) { throw 'Image-viewer build failed' }
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
