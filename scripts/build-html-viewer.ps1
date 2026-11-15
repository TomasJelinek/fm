<# HTML is a required independent runtime; missing inputs fail the normal build. #>
param(
    [ValidateSet('debug', 'release')][string]$Profile = 'debug',
    [string]$SdkConfig,
    [string]$AppDirectory,
    [switch]$Offline
)
$ErrorActionPreference = 'Stop'
if (-not $SdkConfig) { $SdkConfig = $env:MIDNIGHT_HTML_SDK_CONFIG }
if ($env:MIDNIGHT_VIEWERS_OFFLINE -eq '1') { $Offline = $true }
$savedEnvironment = [Collections.Generic.Dictionary[string,string]]::new([StringComparer]::OrdinalIgnoreCase)
foreach ($entry in [Environment]::GetEnvironmentVariables('Process').GetEnumerator()) {
    $savedEnvironment[$entry.Key] = $entry.Value
}
$projectRoot = Split-Path -Parent $PSScriptRoot
$buildRoot = Join-Path $projectRoot '_build/html-viewer'
$staging = $null
$cleanStaging = $true
Push-Location -LiteralPath $projectRoot
try {
    $localCargo = Join-Path $projectRoot '.tools/cargo/bin'
    if (Test-Path -LiteralPath (Join-Path $localCargo 'cargo.exe')) {
        if (-not $env:CARGO_HOME) { $env:CARGO_HOME = Join-Path $projectRoot '.tools/cargo' }
        if (-not $env:RUSTUP_HOME) { $env:RUSTUP_HOME = Join-Path $projectRoot '.tools/rustup' }
        $env:PATH = $localCargo + [IO.Path]::PathSeparator + $env:PATH
    }
    if (-not $AppDirectory) {
        $targetRoot = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { '_build' }
        $AppDirectory = Join-Path $targetRoot $Profile
    }
    $AppDirectory = [IO.Path]::GetFullPath($AppDirectory)
    $destination = Join-Path $AppDirectory 'viewers/midnight-viewer-html'
    foreach ($target in @($buildRoot, $destination)) {
        if (($target -split '[/\\]') -notcontains '_build') { throw 'Generated HTML output must be under _build' }
        for ($part = $target; $part; $part = Split-Path -Parent $part) {
            if ((Test-Path -LiteralPath $part) -and
                ((Get-Item -LiteralPath $part).Attributes -band [IO.FileAttributes]::ReparsePoint)) {
                throw 'Generated HTML output must not traverse reparse points'
            }
        }
    }
    New-Item -ItemType Directory -Force $buildRoot, (Join-Path $projectRoot '_build/tmp') | Out-Null
    $env:TEMP = Join-Path $projectRoot '_build/tmp'
    $env:TMP = $env:TEMP
    if (-not $SdkConfig) {
        $stamp = (Get-ChildItem -LiteralPath (Join-Path $projectRoot 'web-viewer-api') -Recurse -File |
            Sort-Object FullName | ForEach-Object {
                $_.FullName.Substring($projectRoot.Length) + ':' + (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash
            }) -join "`n"
        $statePath = Join-Path $buildRoot 'windows-web-sdk-state.json'
        if (Test-Path -LiteralPath $statePath) {
            $state = Get-Content -LiteralPath $statePath -Raw | ConvertFrom-Json
            if ($state.stamp -eq $stamp -and (Test-Path -LiteralPath $state.config -PathType Leaf)) {
                $SdkConfig = $state.config
            }
        }
        if (-not $SdkConfig) {
            $fetch = @('fetch', '--manifest-path', (Join-Path $projectRoot 'web-viewer-api/Cargo.toml'), '--locked')
            if ($Offline) { $fetch += '--offline' }
            & cargo @fetch
            if ($LASTEXITCODE -ne 0) { throw 'Cannot fetch public web SDK dependencies' }
            $SdkConfig = & (Join-Path $projectRoot 'web-viewer-api/scripts/package.ps1')
            if ($LASTEXITCODE -ne 0 -or -not (Test-Path -LiteralPath $SdkConfig -PathType Leaf)) {
                throw 'Public web SDK packager did not report a configuration'
            }
            @{ stamp = $stamp; config = $SdkConfig } | ConvertTo-Json | Set-Content -LiteralPath $statePath
        }
    }
    $SdkConfig = (Resolve-Path -LiteralPath $SdkConfig).Path
    $target = Join-Path $buildRoot 'provider-target/windows'
    $nativeArguments = @('-B', (Join-Path $PSScriptRoot 'prepare-html-cef.py'), '--output', (Join-Path $target 'cef-sdk'))
    if ($Offline) { $nativeArguments += '--offline' }
    $nativeOutput = & python @nativeArguments
    if ($LASTEXITCODE -ne 0) { throw 'Required CEF SDK preparation failed' }
    $native = ($nativeOutput -join "`n") | ConvertFrom-Json
    if ($native.cef_path -isnot [string] -or -not (Test-Path -LiteralPath $native.cef_path -PathType Container)) {
        throw 'CEF preparation returned an invalid SDK directory'
    }
    $fetch = @('--config', $SdkConfig, 'fetch', '--manifest-path',
        (Join-Path $projectRoot 'plugins/midnight-viewer-html/Cargo.toml'), '--locked')
    if ($Offline) { $fetch += '--offline' }
    & cargo @fetch
    if ($LASTEXITCODE -ne 0) { throw 'Cannot fetch HTML provider dependencies' }
    New-Item -ItemType Directory -Force (Join-Path $AppDirectory 'viewers') | Out-Null
    $staging = Join-Path $AppDirectory ('.html-build.' + [Guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $staging | Out-Null
    $package = Join-Path $staging 'package'
    & (Join-Path $projectRoot 'plugins/midnight-viewer-html/scripts/build-windows.ps1') `
        -SdkConfig $SdkConfig -CefDistribution $native.cef_path -TargetDirectory $target `
        -OutputDirectory $package -Release:($Profile -eq 'release')
    if ($LASTEXITCODE -ne 0) { throw 'HTML provider build failed' }
    foreach ($name in @('runtime-files.json', 'web-viewer.json')) {
        if (-not (Test-Path -LiteralPath (Join-Path $package $name) -PathType Leaf)) {
            throw 'HTML package is missing its runtime inventory or viewer manifest'
        }
    }
    $previous = Join-Path $staging 'previous'
    if (Test-Path -LiteralPath $destination) { Move-Item -LiteralPath $destination -Destination $previous }
    try { Move-Item -LiteralPath $package -Destination $destination }
    catch {
        if (Test-Path -LiteralPath $previous) {
            try { Move-Item -LiteralPath $previous -Destination $destination }
            catch { $cleanStaging = $false; throw "Previous HTML module retained at $previous; restoration failed: $_" }
        }
        throw
    }
    Write-Output "HTML viewer package: $destination"
} finally {
    if ($staging -and $cleanStaging -and (Test-Path -LiteralPath $staging)) {
        Remove-Item -LiteralPath $staging -Recurse -Force
    }
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
