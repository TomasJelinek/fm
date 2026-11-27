<#
.SYNOPSIS
Captures Cargo metadata and Cargo trees for the dependency inventory.

.DESCRIPTION
Writes <scope>-<target>.json and <scope>-<target>.tree for every scope of
scripts/dependency-inventory.py, for the host the script runs on: x86_64-pc-windows-msvc
on Windows x64, x86_64-unknown-linux-gnu on Linux x86-64. Build dependencies and
procedural macros follow the host, so each target is captured on its own host
(docs/third-party.md, "Regenerating the inventory"). -IncludeMacOS adds the two Darwin
pairs of native-macos after checking that its graph still has no build or
procedural-macro dependency, which is what makes them independent of the host.

The captures compile nothing. First the script checks that the Cargo manifests, lock
files, Cargo configuration and toolchain file equal -ExpectedCommit (HEAD by default,
that is, no local changes) and that the checkout is not nested in another Cargo project.
capture-<target>.txt records the commit.

The plugin scopes need the released SDK bundles, named with -ViewerSdkConfig,
-WorkerSdkConfig and -WebSdkConfig or -WebSdkRegistry (or MIDNIGHT_VIEWER_SDK_CONFIG,
MIDNIGHT_IMAGE_VIEWER_SDK_CONFIG and MIDNIGHT_HTML_SDK_CONFIG). Otherwise the script
reuses a current bundle that a Windows build recorded, or packages one with the project's
packagers, which compile the packaging tool and need Python 3.12 or later.

Output: -OutputDirectory (default _build/verification/dependency-inventory-<target>) and
the same files in a .zip archive beside it.
#>
param(
    [string]$OutputDirectory,
    [string]$ExpectedCommit = 'HEAD',
    [string]$ViewerSdkConfig = $env:MIDNIGHT_VIEWER_SDK_CONFIG,
    [string]$WorkerSdkConfig = $env:MIDNIGHT_IMAGE_VIEWER_SDK_CONFIG,
    [string]$WebSdkConfig = $env:MIDNIGHT_HTML_SDK_CONFIG,
    [string]$WebSdkRegistry,
    [switch]$IncludeMacOS,
    [switch]$Offline
)
Set-StrictMode -Version 3.0
$ErrorActionPreference = 'Stop'
if ($PSVersionTable.PSVersion.Major -lt 7) { throw 'Run this script with PowerShell 7 (pwsh).' }

# Resolve caller paths against the PowerShell location before Push-Location.
foreach ($name in 'OutputDirectory', 'ViewerSdkConfig', 'WorkerSdkConfig', 'WebSdkConfig', 'WebSdkRegistry') {
    $value = Get-Variable -Name $name -ValueOnly
    if ($value) {
        Set-Variable -Name $name -Value $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($value)
    }
}
$root = Split-Path -Parent $PSScriptRoot
. (Join-Path $PSScriptRoot 'utf8-native-output.ps1')
# vendor/femtovg has no [workspace] table of its own: Cargo looks above the checkout for
# one, so in a checkout nested in another (such as a worktree below .claude/worktrees of a
# second checkout) it would join the outer workspace and fail.
for ($parent = Split-Path -Parent $root; $parent; $parent = Split-Path -Parent $parent) {
    if (Test-Path -LiteralPath (Join-Path $parent 'Cargo.toml') -PathType Leaf) {
        throw "Use a checkout outside other Cargo projects: $parent contains a Cargo.toml."
    }
}

# The scopes of scripts/dependency-inventory.py with the SDK and features of their CI.
$hostTargets = @('x86_64-pc-windows-msvc', 'x86_64-unknown-linux-gnu')
$scopes = @(
    @{ Name = 'app'; Manifest = 'Cargo.toml' }
    @{ Name = 'renderer-tests'; Manifest = 'vendor/femtovg/Cargo.toml'
        Options = @('--no-default-features', '--features', 'wgpu') }
    @{ Name = 'api'; Manifest = 'api/Cargo.toml' }
    @{ Name = 'basic'; Manifest = 'plugins/midnight-viewer-basic/Cargo.toml'; Sdk = 'viewer' }
    @{ Name = 'source'; Manifest = 'plugins/midnight-viewer-source/Cargo.toml'; Sdk = 'source' }
    @{ Name = 'markdown'; Manifest = 'plugins/midnight-viewer-markdown/Cargo.toml'; Sdk = 'markdown' }
    @{ Name = 'conformance'; Manifest = 'plugins/viewer-conformance/Cargo.toml'; Sdk = 'viewer' }
    @{ Name = 'filesystem-api'; Manifest = 'filesystem-api/Cargo.toml' }
    @{ Name = 'native-windows'; Manifest = 'plugins/midnight-filesystem-native-windows/Cargo.toml' }
    @{ Name = 'native-linux'; Manifest = 'plugins/midnight-filesystem-native-linux/Cargo.toml' }
    @{ Name = 'network'; Manifest = 'plugins/midnight-filesystem-network/Cargo.toml' }
    @{ Name = 'worker-api'; Manifest = 'worker-api/Cargo.toml' }
    @{ Name = 'thumbnail-api'; Manifest = 'thumbnail-api/Cargo.toml' }
    @{ Name = 'image-viewer-api'; Manifest = 'image-viewer-api/Cargo.toml' }
    @{ Name = 'web-viewer-api'; Manifest = 'web-viewer-api/Cargo.toml' }
    @{ Name = 'html-viewer'; Manifest = 'plugins/midnight-viewer-html/Cargo.toml'; Sdk = 'web' }
    @{ Name = 'image-viewer'; Manifest = 'plugins/midnight-viewer-image/Cargo.toml'; Sdk = 'worker' }
    @{ Name = 'image-decoder'; Manifest = 'plugins/image-decoder/Cargo.toml' }
    @{ Name = 'pdf-viewer'; Manifest = 'plugins/midnight-viewer-pdf/Cargo.toml'; Sdk = 'worker' }
    @{ Name = 'archive'; Manifest = 'plugins/midnight-fs-archive/Cargo.toml'; Sdk = 'worker' }
    @{ Name = 'thumbnail-image'; Manifest = 'plugins/midnight-thumbnail-image/Cargo.toml'; Sdk = 'worker' }
    @{ Name = 'thumbnail-video'; Manifest = 'plugins/midnight-thumbnail-video/Cargo.toml'; Sdk = 'worker'
        Options = @('--features', 'system,playback-worker') }
)
$macOS = @{ Name = 'native-macos'; Manifest = 'plugins/midnight-filesystem-native-macos/Cargo.toml' }
$darwinTargets = @('x86_64-apple-darwin', 'aarch64-apple-darwin')
$registryIndex = 'https://midnight-explorer.invalid/api-index'
$utf8 = [Text.UTF8Encoding]::new($false)

function Invoke-Cargo {
    & (Join-Path $root 'scripts/cargo.ps1') @args
    if ($LASTEXITCODE -ne 0) { throw "cargo $($args -join ' ') failed" }
}

# The stamps and state files of scripts/build-viewers.ps1, build-image-viewer.ps1 and
# build-html-viewer.ps1, read but never written: a current bundle is reused as is.
function Get-SourceStamp([string[]]$Roots) {
    ($Roots | ForEach-Object { Get-ChildItem -LiteralPath (Join-Path $root $_) -Recurse -File } |
        Sort-Object FullName | ForEach-Object {
            $_.FullName.Substring($root.Length) + ':' + (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash
        }) -join "`n"
}

function Find-CurrentSdk([string]$Stamp, [string[]]$StatePaths) {
    foreach ($statePath in $StatePaths) {
        if (-not (Test-Path -LiteralPath $statePath -PathType Leaf)) { continue }
        $state = Get-Content -LiteralPath $statePath -Raw | ConvertFrom-Json
        $names = @($state.PSObject.Properties.Name)
        if ($names -contains 'stamp' -and $names -contains 'config' -and $state.stamp -eq $Stamp -and
                $state.config -and (Test-Path -LiteralPath $state.config -PathType Leaf)) {
            Write-Host "Reusing the SDK bundle recorded in $statePath"
            return $state.config
        }
    }
    return $null
}

function Get-ReportedBundle([object[]]$Output, [string]$Prefix) {
    $line = @($Output | Where-Object { "$_".StartsWith($Prefix) }) | Select-Object -Last 1
    if (-not $line) { throw "The packager did not report its bundle ('$Prefix')" }
    return "$line".Substring($Prefix.Length).Trim()
}

# Only this script's own output may be replaced.
function Assert-CaptureDirectory([string]$Path) {
    if (-not (Test-Path -LiteralPath $Path)) { return }
    $foreign = @(Get-ChildItem -LiteralPath $Path -Force | Where-Object {
        $_.PSIsContainer -or $_.Name -notmatch '^(capture-.+\.txt|.+-.+\.(json|tree))$' })
    if ($foreign.Count) { throw "Not a capture directory (it holds $($foreign[0].Name)): $Path" }
}

function Remove-CaptureDirectory([string]$Path) {
    Assert-CaptureDirectory $Path
    if (Test-Path -LiteralPath $Path) { Remove-Item -LiteralPath $Path -Recurse -Force }
}

# Decodes Cargo's UTF-8 output as UTF-8 and writes it without a byte order mark.
function Save-Capture([hashtable]$Scope, [string]$Triple, [string]$Directory, [hashtable]$Configs) {
    $common = @()
    if ($Scope.ContainsKey('Sdk')) { $common += @('--config', $Configs[$Scope.Sdk]) }
    $options = @()
    if ($Scope.ContainsKey('Options')) { $options += $Scope.Options }
    $commands = [ordered]@{
        json = @($common + @('metadata', '--locked', '--format-version', '1', '--manifest-path', $Scope.Manifest,
            '--filter-platform', $Triple) + $options)
        tree = @($common + @('tree', '--locked', '--workspace', '--manifest-path', $Scope.Manifest,
            '--target', $Triple, '--edges', 'normal,build,dev', '--prefix', 'none') + $options)
    }
    $texts = @{}
    foreach ($kind in $commands.Keys) {
        $arguments = $commands[$kind]
        $lines = @(Invoke-Utf8NativeCommand { & (Join-Path $root 'scripts/cargo.ps1') @arguments })
        if ($LASTEXITCODE -ne 0) { throw "cargo $($arguments -join ' ') failed" }
        $texts[$kind] = ($lines -join "`n") + "`n"
        if (-not $texts[$kind].Trim()) { throw "cargo $($arguments -join ' ') printed nothing" }
        [IO.File]::WriteAllText((Join-Path $Directory "$($Scope.Name)-$Triple.$kind"), $texts[$kind], $utf8)
    }
    Write-Host "captured $($Scope.Name)-$Triple"
    return $texts['json']
}

Push-Location -LiteralPath $root
try {
    if (-not (Get-Command git -CommandType Application -ErrorAction Ignore)) {
        throw 'git must be on PATH: the script checks the dependency inputs with it.'
    }
    $resolved = @(& git -C $root rev-parse --verify --quiet "$ExpectedCommit^{commit}")
    if ($LASTEXITCODE -ne 0 -or $resolved.Count -ne 1) {
        throw "Unknown commit $ExpectedCommit. Run 'git fetch origin' and start again."
    }
    $expected = "$($resolved[0])".Trim()
    $inputs = @(':(glob)**/Cargo.toml', ':(glob)**/Cargo.lock', '.cargo/config.toml', 'rust-toolchain.toml')
    $changed = @(& git -C $root diff --name-only $expected '--' @inputs)
    if ($LASTEXITCODE -ne 0) { throw 'git diff failed' }
    $untracked = @(& git -C $root ls-files --others --exclude-standard '--' @inputs)
    if ($LASTEXITCODE -ne 0) { throw 'git ls-files failed' }
    if ($changed.Count -or $untracked.Count) {
        throw ("These dependency inputs differ from ${expected}: " + (($changed + $untracked) -join ', ') +
            '. Capture from a checkout without such changes, for example a separate worktree: ' +
            "git worktree add --detach ..\fm-inventory $expected")
    }

    $localCargo = Join-Path $root '.tools/cargo/bin'
    if (Test-Path -LiteralPath (Join-Path $localCargo 'cargo.exe')) {
        $env:CARGO_HOME = Join-Path $root '.tools/cargo'
        $env:RUSTUP_HOME = Join-Path $root '.tools/rustup'
        $env:PATH = $localCargo + [IO.Path]::PathSeparator + $env:PATH
    }
    $temporary = Join-Path $root '_build/tmp'
    $verification = Join-Path $root '_build/verification'
    $work = Join-Path $verification 'dependency-inventory-sdk'
    New-Item -ItemType Directory -Force $temporary, $work | Out-Null
    $env:TEMP = $temporary
    $env:TMP = $temporary
    $env:TMPDIR = $temporary
    # Python before 3.15 writes and decodes text in the ANSI code page unless UTF-8 mode is on.
    $env:PYTHONUTF8 = '1'
    $fetchOptions = @('--locked')
    if ($Offline) { $fetchOptions += '--offline' }

    $hostTriple = @(Invoke-Utf8NativeCommand { & rustc -vV }) | Where-Object { "$_".StartsWith('host: ') } |
        Select-Object -First 1
    if ($LASTEXITCODE -ne 0 -or -not $hostTriple) { throw 'Cannot determine the Rust host target' }
    $hostTriple = "$hostTriple".Substring(6).Trim()
    if ($hostTargets -notcontains $hostTriple) {
        throw "The inventory captures Windows x64 and Linux x86-64, each on its own host; this host is $hostTriple."
    }
    $cargoVersion = (@(Invoke-Utf8NativeCommand { & cargo --version }) -join ' ').Trim()
    $destination = if ($OutputDirectory) { $OutputDirectory } else {
        Join-Path $verification "dependency-inventory-$hostTriple" }
    $partial = "$destination.partial"
    Assert-CaptureDirectory $destination
    Assert-CaptureDirectory $partial

    if (-not $ViewerSdkConfig) {
        $ViewerSdkConfig = Find-CurrentSdk (Get-SourceStamp @('api')) @(
            Join-Path $root '_build/viewers/sdk-state.json')
        if (-not $ViewerSdkConfig) {
            Write-Host 'Packaging the public viewer SDK...'
            Invoke-Cargo fetch --manifest-path api/Cargo.toml @fetchOptions
            $env:CARGO_TARGET_DIR = Join-Path $work 'viewer-sdk-target'
            try {
                $output = Invoke-Utf8NativeCommand { & (Join-Path $root 'api/scripts/package.ps1') }
                if ($LASTEXITCODE -ne 0) { throw 'Cannot package the public viewer SDK' }
            } finally {
                Remove-Item Env:CARGO_TARGET_DIR -ErrorAction Ignore
            }
            $ViewerSdkConfig = Join-Path (Get-ReportedBundle $output 'Public API/SDK release bundle: ') '.cargo/config.toml'
        }
    }
    if (-not $WorkerSdkConfig) {
        $stamp = Get-SourceStamp @('api/tools/package', 'worker-api', 'thumbnail-api', 'filesystem-api', 'image-viewer-api')
        $WorkerSdkConfig = Find-CurrentSdk $stamp @('image-viewer', 'pdf-viewer', 'thumbnailers' | ForEach-Object {
            Join-Path $root "_build/$_/sdk-state.json" })
        if (-not $WorkerSdkConfig) {
            Write-Host 'Packaging the public worker SDK...'
            foreach ($workspace in 'api', 'worker-api', 'thumbnail-api', 'filesystem-api', 'image-viewer-api') {
                Invoke-Cargo fetch --manifest-path "$workspace/Cargo.toml" @fetchOptions
            }
            $releases = Join-Path $work 'worker-releases'
            $env:CARGO_TARGET_DIR = Join-Path $work 'worker-sdk-target'
            try {
                $output = Invoke-Utf8NativeCommand {
                    & cargo run --manifest-path api/Cargo.toml -p midnight-api-package --bin worker-packages `
                        --locked --offline '--' $root $releases
                }
                if ($LASTEXITCODE -ne 0) { throw 'Cannot package the public worker SDK' }
            } finally {
                Remove-Item Env:CARGO_TARGET_DIR -ErrorAction Ignore
            }
            $WorkerSdkConfig = Join-Path (Get-ReportedBundle $output 'Public worker API/SDK release bundle: ') '.cargo/registry.toml'
        }
    }
    if (-not $WebSdkConfig -and $WebSdkRegistry) {
        # A released web SDK registry moved elsewhere, such as a CI artifact: the
        # configuration that web-viewer-api/scripts/package.py writes, for its new place.
        $registry = (Resolve-Path -LiteralPath $WebSdkRegistry).Path.Replace('\', '/')
        $WebSdkConfig = Join-Path $work 'web-sdk.config.toml'
        $content = @('[registries.midnight]', "index = `"$registryIndex`"", '[source.midnight]',
            "registry = `"$registryIndex`"", 'replace-with = "web-viewer-releases"',
            '[source.web-viewer-releases]', "local-registry = $(ConvertTo-Json -Compress $registry)")
        [IO.File]::WriteAllText($WebSdkConfig, ($content -join "`n") + "`n", $utf8)
    }
    if (-not $WebSdkConfig) {
        $WebSdkConfig = Find-CurrentSdk (Get-SourceStamp @('web-viewer-api')) @(
            Join-Path $root '_build/html-viewer/windows-web-sdk-state.json')
        if (-not $WebSdkConfig) {
            Write-Host 'Packaging the public web SDK...'
            Invoke-Cargo fetch --manifest-path web-viewer-api/Cargo.toml @fetchOptions
            $release = Join-Path $work "web-sdk-$([DateTime]::UtcNow.ToString('yyyyMMddHHmmssfff'))"
            $output = @(Invoke-Utf8NativeCommand { & (Join-Path $root 'web-viewer-api/scripts/package.ps1') -Output $release })
            if ($LASTEXITCODE -ne 0 -or -not $output.Count) { throw 'Cannot package the public web SDK' }
            $WebSdkConfig = "$($output[-1])".Trim()
        }
    }
    $configs = @{}
    $found = [ordered]@{ viewer = $ViewerSdkConfig; worker = $WorkerSdkConfig; web = $WebSdkConfig }
    foreach ($kind in $found.Keys) {
        if (-not (Test-Path -LiteralPath $found[$kind] -PathType Leaf)) { throw "Missing $kind SDK configuration: $($found[$kind])" }
        $configs[$kind] = (Resolve-Path -LiteralPath $found[$kind]).Path
    }
    # The source and markdown viewers derive their configuration as their CI does.
    foreach ($viewer in 'source', 'markdown') {
        $prepare = Join-Path $root "plugins/midnight-viewer-$viewer/scripts/prepare-sdk.ps1"
        $generated = @(& $prepare -SdkConfig $configs['viewer'] -BuildRoot (Join-Path $work $viewer))
        $configs[$viewer] = "$($generated[-1])".Trim()
        if (-not (Test-Path -LiteralPath $configs[$viewer] -PathType Leaf)) { throw "prepare-sdk.ps1 failed for $viewer" }
    }

    Remove-CaptureDirectory $partial
    New-Item -ItemType Directory -Force $partial | Out-Null
    foreach ($scope in $scopes) {
        Save-Capture $scope $hostTriple $partial $configs | Out-Null
    }
    if ($IncludeMacOS) {
        foreach ($triple in $darwinTargets) {
            $graph = Save-Capture $macOS $triple $partial $configs | ConvertFrom-Json
            $build = @($graph.resolve.nodes | ForEach-Object { $_.deps } | ForEach-Object { $_.dep_kinds } |
                Where-Object { $_.kind -eq 'build' })
            $macros = @($graph.packages | Where-Object { @($_.targets | Where-Object { $_.kind -contains 'proc-macro' }).Count })
            if ($build.Count -or $macros.Count) {
                throw "native-macos now has build or procedural-macro dependencies; capture $triple on a Mac (docs/third-party.md)."
            }
        }
    }
    $record = @(
        "commit: $(@(& git -C $root rev-parse HEAD)[0])",
        "dependency inputs equal: $expected",
        "captured: $([DateTime]::UtcNow.ToString('yyyy-MM-ddTHH:mm:ssZ'))",
        "host: $hostTriple",
        "cargo: $cargoVersion",
        "powershell: $($PSVersionTable.PSVersion)",
        "viewer sdk: $($configs['viewer'])",
        "worker sdk: $($configs['worker'])",
        "web sdk: $($configs['web'])"
    )
    [IO.File]::WriteAllText((Join-Path $partial "capture-$hostTriple.txt"), ($record -join "`n") + "`n", $utf8)
    Remove-CaptureDirectory $destination
    Move-Item -LiteralPath $partial -Destination $destination
    $archive = "$destination.zip"
    Compress-Archive -Path (Join-Path $destination '*') -DestinationPath $archive -Force
    $count = @(Get-ChildItem -LiteralPath $destination -File | Where-Object { $_.Extension -in '.json', '.tree' }).Count
    Write-Host ''
    Write-Host "Captured $count files for $hostTriple$(if ($IncludeMacOS) { ' and macOS' })."
    Write-Host "Folder:  $destination"
    Write-Host "Archive: $archive"
} finally {
    Pop-Location
}
