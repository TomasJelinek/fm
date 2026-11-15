<# Normal builds always install image and video thumbnail providers. #>
param(
    [ValidateSet('debug', 'release')][string]$Profile = 'debug',
    [string]$WorkerSdkConfig,
    [string]$AppDirectory,
    [switch]$Offline
)
$ErrorActionPreference = 'Stop'
if (-not $WorkerSdkConfig) { $WorkerSdkConfig = $env:MIDNIGHT_THUMBNAIL_SDK_CONFIG }
if ($env:MIDNIGHT_THUMBNAILERS_OFFLINE -eq '1' -or $env:MIDNIGHT_VIEWERS_OFFLINE -eq '1') { $Offline = $true }
$savedEnvironment = [Collections.Generic.Dictionary[string,string]]::new([StringComparer]::OrdinalIgnoreCase)
foreach ($entry in [Environment]::GetEnvironmentVariables('Process').GetEnumerator()) {
    $savedEnvironment[$entry.Key] = $entry.Value
}
$projectRoot = Split-Path -Parent $PSScriptRoot
$buildRoot = Join-Path $projectRoot '_build/thumbnailers'
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
        foreach ($cache in @($buildRoot, (Join-Path $projectRoot '_build/image-viewer'))) {
            $statePath = Join-Path $cache 'sdk-state.json'
            if (Test-Path -LiteralPath $statePath) {
                $state = Get-Content -LiteralPath $statePath -Raw | ConvertFrom-Json
                if ($state.stamp -eq $stamp -and (Test-Path -LiteralPath $state.config -PathType Leaf)) {
                    $WorkerSdkConfig = $state.config
                    break
                }
            }
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
            if (-not (Test-Path -LiteralPath $WorkerSdkConfig -PathType Leaf)) { throw 'Worker SDK configuration is missing' }
            @{ stamp = $stamp; config = $WorkerSdkConfig } | ConvertTo-Json |
                Set-Content -LiteralPath (Join-Path $buildRoot 'sdk-state.json')
        }
    }
    $WorkerSdkConfig = (Resolve-Path -LiteralPath $WorkerSdkConfig).Path
    foreach ($name in @('image', 'video')) {
        $fetch = @('--config', $WorkerSdkConfig, 'fetch', '--manifest-path',
            (Join-Path $projectRoot "plugins/midnight-thumbnail-$name/Cargo.toml"), '--locked')
        if ($Offline) { $fetch += '--offline' }
        & cargo @fetch
        if ($LASTEXITCODE -ne 0) { throw "Cannot fetch $name thumbnail dependencies" }
    }
    $nativeArguments = @('-B', (Join-Path $PSScriptRoot 'prepare-thumbnail-ffmpeg.py'),
        '--build-root', (Join-Path $buildRoot 'ffmpeg'))
    if ($Offline) { $nativeArguments += '--offline' }
    $nativeOutput = & python @nativeArguments
    if ($LASTEXITCODE -ne 0) { throw 'Cannot prepare required thumbnail FFmpeg runtime' }
    $native = ($nativeOutput -join "`n") | ConvertFrom-Json
    foreach ($key in @('prefix', 'source_archive', 'libclang_path')) {
        if ($native.$key -isnot [string] -or [string]::IsNullOrWhiteSpace($native.$key)) {
            throw "Missing or invalid native thumbnail build path: $key"
        }
    }
    $notices = Join-Path $buildRoot 'notices'
    & python -B (Join-Path $PSScriptRoot 'thumbnail-notices.py') --config $WorkerSdkConfig --output $notices
    if ($LASTEXITCODE -ne 0) { throw 'Cannot prepare thumbnail dependency notices' }
    & (Join-Path $PSScriptRoot 'build-thumbnailers.ps1') -ApiConfig $WorkerSdkConfig `
        -Destination (Join-Path $AppDirectory 'thumbnailers') -BuildRoot (Join-Path $buildRoot 'plugins') `
        -NoticesRoot $notices -FfmpegPrefix $native.prefix -FfmpegSourceArchive $native.source_archive `
        -LibclangPath $native.libclang_path -Profile $Profile -NativeOffline:$Offline
    if ($LASTEXITCODE -ne 0) { throw 'Required thumbnail provider build failed' }
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
