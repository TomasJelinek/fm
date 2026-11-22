<# HTML is a required independent runtime; missing inputs fail the normal build. #>
param(
    [ValidateSet('debug', 'release')][string]$Profile = 'debug',
    [string]$SdkConfig,
    [string]$AppDirectory,
    [switch]$Offline
)
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'utf8-native-output.ps1')

# Endpoint protection briefly holds files that were just written. While it does, Windows
# refuses to rename them or a folder above them: access denied (5), a sharing (32) or a
# lock (33) violation, or File.Replace cannot remove the replaced file (1175, both files
# unchanged; its partial outcomes 1176/1177 are not retried). A rename of fresh output
# retries only those errors, with a short backoff for about 2 s, and then fails with the
# first error. This script always uses the 2000 ms default; tests shorten it with
# -TimeoutMilliseconds or $PSDefaultParameterValues.
function Invoke-FreshRename {
    [CmdletBinding()]
    param([Parameter(Mandatory)][scriptblock]$Rename, [int]$TimeoutMilliseconds = 2000)
    $firstHold = $null
    $holdDelay = 20
    while ($true) {
        try { & $Rename; return }
        catch {
            $holdCode = 0
            for ($failure = $_.Exception; $failure -and -not $holdCode; $failure = $failure.InnerException) {
                if ((($failure.HResult -shr 16) -band 0x1FFF) -eq 7) { $holdCode = $failure.HResult -band 0xFFFF }
            }
            if ($holdCode -notin @(5, 32, 33, 1175)) { throw }
            if (-not $firstHold) { $firstHold = $_; $holdWatch = [Diagnostics.Stopwatch]::StartNew() }
            $holdLeft = $TimeoutMilliseconds - $holdWatch.ElapsedMilliseconds
            if ($holdLeft -le 0) { throw $firstHold }
            Start-Sleep -Milliseconds ([Math]::Max(1, [Math]::Min($holdDelay, $holdLeft)))
            $holdDelay = [Math]::Min(2 * $holdDelay, 200)
        }
    }
}

function Test-HtmlPackageCurrent([string]$StatePath, [string]$Fingerprint, [string]$Destination) {
    if (-not (Test-Path -LiteralPath $StatePath -PathType Leaf)) { return $false }
    $state = Get-Content -LiteralPath $StatePath -Raw -Encoding UTF8 | ConvertFrom-Json
    $inventory = Join-Path $Destination 'runtime-files.json'
    if ($state.fingerprint -cne $Fingerprint -or -not [IO.File]::Exists($inventory) -or
        (Get-FileHash -LiteralPath $inventory -Algorithm SHA256).Hash -cne $state.runtime) { return $false }
    # Deployment re-hashes every file against this inventory; here a missing or
    # resized file is enough to package again.
    foreach ($file in (Get-Content -LiteralPath $inventory -Raw | ConvertFrom-Json).files) {
        $path = Join-Path $Destination $file.path
        if (-not [IO.File]::Exists($path) -or [IO.FileInfo]::new($path).Length -ne $file.bytes) { return $false }
    }
    return $true
}

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
# Resolve caller paths against the PowerShell location before Push-Location;
# [IO.Path]::GetFullPath would use the unrelated process working directory.
if ($AppDirectory) { $AppDirectory = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($AppDirectory) }
if ($SdkConfig) { $SdkConfig = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($SdkConfig) }
# Python 3.12+ runs through the documented py launcher; python may be a Store alias.
$python = 'python'
$pythonArguments = @('-B')
if (Get-Command py -CommandType Application -ErrorAction Ignore) {
    $python = 'py'
    $pythonArguments = @('-3', '-B')
}
Push-Location -LiteralPath $projectRoot
try {
    $localCargo = Join-Path $projectRoot '.tools/cargo/bin'
    if (Test-Path -LiteralPath (Join-Path $localCargo 'cargo.exe')) {
        if (-not $env:CARGO_HOME) { $env:CARGO_HOME = Join-Path $projectRoot '.tools/cargo' }
        if (-not $env:RUSTUP_HOME) { $env:RUSTUP_HOME = Join-Path $projectRoot '.tools/rustup' }
        $env:PATH = $localCargo + [IO.Path]::PathSeparator + $env:PATH
    }
    if (-not $AppDirectory) {
        # Cargo resolves a relative CARGO_TARGET_DIR from the project root here too.
        $targetRoot = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { '_build' }
        if (-not [IO.Path]::IsPathRooted($targetRoot)) { $targetRoot = Join-Path $projectRoot $targetRoot }
        $AppDirectory = Join-Path $targetRoot $Profile
    }
    $AppDirectory = [IO.Path]::GetFullPath($AppDirectory)
    $destination = Join-Path $AppDirectory 'viewers/midnight-viewer-html'
    # Generated output lies below _build: the checkout's own for a path in the checkout,
    # otherwise the first _build of a custom target directory. That _build and everything
    # below it must not be a reparse point (symlink or junction). The checkout and the
    # folders above it may be links, a subst drive or hidden folders such as AppData.
    $checkout = [IO.Path]::GetFullPath($projectRoot).TrimEnd('\')
    foreach ($target in @($buildRoot, $destination)) {
        $ancestors = @()
        $inside = $false
        for ($part = [IO.Path]::GetFullPath($target).TrimEnd('\'); $part; $part = [IO.Path]::GetDirectoryName($part)) {
            # TrimEnd also matches a checkout at a drive root, such as a subst drive's M:\.
            if ($part.TrimEnd('\') -eq $checkout) { $inside = $true; break }
            $ancestors += $part
        }
        $anchor = 0
        for ($index = $ancestors.Count - 1; $index -gt 0; $index--) {
            if ([IO.Path]::GetFileName($ancestors[$index]) -eq '_build') { $anchor = $index; break }
            if ($inside) { break }
        }
        if ($anchor -eq 0) { throw "Generated HTML output must be under _build: $target" }
        foreach ($part in $ancestors[0..$anchor]) {
            $item = Get-Item -LiteralPath $part -Force -ErrorAction Ignore
            if ($item -and ($item.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
                throw "Generated HTML output must not traverse reparse points: $part"
            }
        }
    }
    New-Item -ItemType Directory -Force $buildRoot, (Join-Path $projectRoot '_build/tmp') | Out-Null
    $env:TEMP = Join-Path $projectRoot '_build/tmp'
    $env:TMP = $env:TEMP
    # Python before 3.15 writes and decodes text in the ANSI code page unless UTF-8 mode is on.
    $env:PYTHONUTF8 = '1'
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
            $SdkConfig = Invoke-Utf8NativeCommand { & (Join-Path $projectRoot 'web-viewer-api/scripts/package.ps1') }
            if ($LASTEXITCODE -ne 0 -or -not (Test-Path -LiteralPath $SdkConfig -PathType Leaf)) {
                throw 'Public web SDK packager did not report a configuration'
            }
            @{ stamp = $stamp; config = $SdkConfig } | ConvertTo-Json | Set-Content -LiteralPath $statePath
        }
    }
    $SdkConfig = (Resolve-Path -LiteralPath $SdkConfig).Path
    # cl.exe cannot address paths beyond 260 characters, and CEF's C++ wrapper objects
    # lie about 170 characters below the Cargo target directory. A deep checkout can
    # select a short target for Cargo's output (for example a junction such as D:\mh
    # into its _build). The verified CEF SDK stays under _build (no links there) and is
    # used in place; the package copies its runtime files only from there.
    $defaultTarget = [IO.Path]::GetFullPath((Join-Path $buildRoot 'provider-target/windows'))
    $target = $defaultTarget
    if ($env:MIDNIGHT_HTML_TARGET_DIR) {
        $target = $env:MIDNIGHT_HTML_TARGET_DIR.Replace('/', '\').TrimEnd('\')
        if ($target -notmatch '^[A-Za-z]:\\.' -or @($target.Substring(3).Split('\') | Where-Object {
                $_ -in @('', '.', '..') -or $_ -match '[<>:"|?*\x00-\x1f]' -or $_ -match '[. ]$' }).Count) {
            throw 'MIDNIGHT_HTML_TARGET_DIR must be an absolute drive path of plain directory names, such as D:\mh'
        }
        $projectPrefix = $projectRoot.TrimEnd('\') + '\'
        if (($target + '\').StartsWith($projectPrefix, [StringComparison]::OrdinalIgnoreCase) -and
            -not ($target + '\').StartsWith($projectPrefix + '_build\', [StringComparison]::OrdinalIgnoreCase)) {
            throw "MIDNIGHT_HTML_TARGET_DIR inside the checkout must be below its _build directory: $target"
        }
    }
    if ($target.Length -gt 80) {
        Write-Warning ("The HTML provider target directory has $($target.Length) characters; CEF wrapper " +
            'objects may then exceed the 260-character MSVC path limit. Set MIDNIGHT_HTML_TARGET_DIR to a ' +
            "shorter directory (for example a junction such as D:\mh to this checkout's _build\mh) if cl.exe fails.")
    }
    $nativeArguments = $pythonArguments + @((Join-Path $PSScriptRoot 'prepare-html-cef.py'), '--output', (Join-Path $defaultTarget 'cef-sdk'))
    if ($Offline) { $nativeArguments += '--offline' }
    $nativeOutput = Invoke-Utf8NativeCommand { & $python @nativeArguments }
    if ($LASTEXITCODE -ne 0) { throw 'Required CEF SDK preparation failed' }
    $native = ($nativeOutput -join "`n") | ConvertFrom-Json
    if ($native.cef_path -isnot [string] -or -not (Test-Path -LiteralPath $native.cef_path -PathType Container)) {
        throw 'CEF preparation returned an invalid SDK directory'
    }
    # The wrapper sources are compiled in place, at most 80 characters below the SDK.
    if ($native.cef_path.Length + 81 -gt 259) {
        Write-Warning ("The verified CEF SDK path has $($native.cef_path.Length) characters; its wrapper sources " +
            'may then exceed the 260-character MSVC path limit. Build from a checkout with a shorter path if cl.exe fails.')
    }
    $fetch = @('--config', $SdkConfig, 'fetch', '--manifest-path',
        (Join-Path $projectRoot 'plugins/midnight-viewer-html/Cargo.toml'), '--locked')
    if ($Offline) { $fetch += '--offline' }
    & cargo @fetch
    if ($LASTEXITCODE -ne 0) { throw 'Cannot fetch HTML provider dependencies' }
    $pluginRoot = Join-Path $projectRoot 'plugins/midnight-viewer-html'
    $release = $Profile -eq 'release'
    & (Join-Path $pluginRoot 'scripts/build-windows.ps1') -SdkConfig $SdkConfig -CefSdk $native.cef_path `
        -TargetDirectory $target -Release:$release -BuildOnly
    if ($LASTEXITCODE -ne 0) { throw 'HTML provider build failed' }
    # Packaging copies and hashes about 370 MB. An installed package made from the same
    # inputs (provider library, verified CEF SDK receipt, SDK configuration, packaging
    # scripts, manifest, licence and notices) is kept while its runtime inventory is unchanged.
    $receipt = $native.cef_path + '.verified.json'
    $inputs = @((Join-Path $target "$Profile/midnight_viewer_html.dll"), $receipt, $SdkConfig) +
        @('Cargo.toml', 'Cargo.lock', '.cargo/config.toml', 'web-viewer.json', 'LICENSE', 'THIRD_PARTY.md', 'README.md' |
            ForEach-Object { Join-Path $pluginRoot $_ }) +
        @(Get-ChildItem -LiteralPath (Join-Path $pluginRoot 'scripts'), (Join-Path $pluginRoot 'notices') -Recurse -File |
            Sort-Object FullName | ForEach-Object FullName)
    $fingerprint = (@("profile=$Profile", "destination=$destination") + @($inputs | ForEach-Object {
        $_ + ':' + (Get-FileHash -LiteralPath $_ -Algorithm SHA256).Hash })) -join "`n"
    $packageState = Join-Path $buildRoot "windows-package-state.$Profile.json"
    if (Test-HtmlPackageCurrent $packageState $fingerprint $destination) {
        Write-Output "HTML viewer package is current: $destination"
        return
    }
    New-Item -ItemType Directory -Force (Join-Path $AppDirectory 'viewers') | Out-Null
    $staging = Join-Path $AppDirectory ('.html-build.' + [Guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $staging | Out-Null
    $package = Join-Path $staging 'package'
    & (Join-Path $pluginRoot 'scripts/package-windows.ps1') -SdkConfig $SdkConfig -TargetDirectory $target `
        -CefDirectory $native.cef_path -CefReceipt $receipt -OutputDirectory $package -Release:$release
    if ($LASTEXITCODE -ne 0) { throw 'HTML provider packaging failed' }
    foreach ($name in @('runtime-files.json', 'web-viewer.json')) {
        if (-not (Test-Path -LiteralPath (Join-Path $package $name) -PathType Leaf)) {
            throw 'HTML package is missing its runtime inventory or viewer manifest'
        }
    }
    $previous = Join-Path $staging 'previous'
    # Directory.Move is one rename that leaves nothing behind when it is refused; Move-Item
    # would then move item by item and leave a created destination that a retry or the
    # restoration below would nest the package in.
    if (Test-Path -LiteralPath $destination) {
        # Retried: endpoint protection may still hold files of a package built moments ago.
        Invoke-FreshRename { [IO.Directory]::Move($destination, $previous) }
    }
    # Retried: endpoint protection may still hold the files just packaged.
    try { Invoke-FreshRename { [IO.Directory]::Move($package, $destination) } }
    catch {
        if (Test-Path -LiteralPath $previous) {
            # Retried like the publication: the previous package returns to its place.
            try { Invoke-FreshRename { [IO.Directory]::Move($previous, $destination) } }
            catch { $cleanStaging = $false; throw "Previous HTML module retained at $previous; restoration failed: $_" }
        }
        throw
    }
    # Recorded after the swap only: a failure above keeps describing the previous package.
    @{ fingerprint = $fingerprint
       runtime = (Get-FileHash -LiteralPath (Join-Path $destination 'runtime-files.json') -Algorithm SHA256).Hash } |
        ConvertTo-Json | Set-Content -LiteralPath $packageState -Encoding UTF8
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
