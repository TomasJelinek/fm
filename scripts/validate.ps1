<#
Full local validation on Windows: the tests that CI (.github/workflows/ci.yml) runs for
this host, with CI's own test commands and the builds and packages those tests need, in
CI's job order. build.bat --validate runs it after its build steps and before the
private deployment.

Usage: scripts\validate.ps1 [debug|release] [--strict] [--offline] [--dry-run]
           [--only ID[,ID...]] [--plugin-sdk CONFIG] [--worker-sdk CONFIG] [--web-sdk CONFIG]

Each step runs in its own PowerShell 7 process (Windows PowerShell 5.1 when PowerShell 7
is not installed) and logs to _build\validate\logs\<run>\. A step whose prerequisite is
missing is SKIPPED with the reason. Exit codes: 0 no step failed (with --strict: also
none skipped for a missing prerequisite), 1 a step failed, 2 invalid arguments,
127 cargo, rustc or Python 3.12+ is missing. See docs/developer/TEST_EXECUTION.md.
#>
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

$usage = @'
Usage: scripts\validate.ps1 [debug|release] [--strict] [--offline] [--dry-run]
           [--only ID[,ID...]] [--plugin-sdk CONFIG] [--worker-sdk CONFIG] [--web-sdk CONFIG]
Runs the tests that CI runs for this platform; debug|release selects the
profile of the host build and smoke test (default debug). --strict fails when a step is
skipped for a missing prerequisite. --offline (or CARGO_NET_OFFLINE=true) forbids
downloads. --dry-run prints the plan and its commands. --only runs the named steps and
the steps they need. The --*-sdk options use an already packaged SDK bundle instead
of packaging one, as CI's plugin jobs download theirs.
'@

function Write-Line([string]$Text = '') { [Console]::Out.WriteLine($Text) }

function Stop-Usage([string]$Message) {
    if ($Message) { [Console]::Error.WriteLine("validate.ps1: $Message") }
    [Console]::Error.WriteLine($usage)
    exit 2
}

# GNU-style options, the same as scripts/validate.sh; PowerShell-style spellings work too.
$profileName = $null
$strict = $false
$dryRun = $false
$offline = $false
$selection = @()
$supplied = @{ PluginSdk = $null; WorkerSdk = $null; WebSdk = $null }
$optionNames = @{
    '--strict' = 'strict'; '-strict' = 'strict'; '--dry-run' = 'dry-run'; '-dryrun' = 'dry-run'
    '--offline' = 'offline'; '-offline' = 'offline'; '--only' = 'only'; '-only' = 'only'
    '--plugin-sdk' = 'PluginSdk'; '-pluginsdk' = 'PluginSdk'; '--worker-sdk' = 'WorkerSdk'
    '-workersdk' = 'WorkerSdk'; '--web-sdk' = 'WebSdk'; '-websdk' = 'WebSdk'
    '--profile' = 'profile'; '-profile' = 'profile'; '--help' = 'help'; '-help' = 'help'
    '-h' = 'help'; '-?' = 'help'
}
# Called in a session (& scripts\validate.ps1 --only a,b), PowerShell passes a list as an array.
function ConvertTo-ArgumentText($Value) { return (@($Value) | ForEach-Object { [string]$_ }) -join ',' }
for ($index = 0; $index -lt $args.Count; $index++) {
    $argument = ConvertTo-ArgumentText $args[$index]
    $value = $null
    $name = $argument
    if ($argument -match '^(--[a-z-]+)=(.*)$') { $name = $Matches[1]; $value = $Matches[2] }
    $option = $optionNames[$name.ToLowerInvariant()]
    if ($argument -in @('debug', 'release')) { $option = 'profile'; $value = $argument }
    if (-not $option) { Stop-Usage "unknown argument '$argument'" }
    if ($option -in @('help', 'strict', 'dry-run', 'offline') -and $null -ne $value) { Stop-Usage "$name takes no value" }
    if ($option -in @('only', 'PluginSdk', 'WorkerSdk', 'WebSdk', 'profile') -and $null -eq $value) {
        $index++
        if ($index -ge $args.Count) { Stop-Usage "$name needs a value" }
        $value = ConvertTo-ArgumentText $args[$index]
    }
    switch ($option) {
        'help' { Write-Line $usage; exit 0 }
        'strict' { $strict = $true }
        'dry-run' { $dryRun = $true }
        'offline' { $offline = $true }
        'only' {
            $ids = @($value -split ',' | ForEach-Object { $_.Trim() } | Where-Object { $_ })
            if (-not $ids.Count) { Stop-Usage "$name needs at least one step id" }
            $selection += $ids
        }
        'profile' {
            if ($value -notin @('debug', 'release')) { Stop-Usage "unknown profile '$value'" }
            if ($profileName) { Stop-Usage 'choose one profile' }
            $profileName = $value.ToLowerInvariant()
        }
        default {
            if ($supplied[$option]) { Stop-Usage "$name was given twice" }
            if (-not $value) { Stop-Usage "$name needs a value" }
            $supplied[$option] = $value
        }
    }
}
if (-not $profileName) { $profileName = 'debug' }
if ($env:CARGO_NET_OFFLINE -eq 'true') { $offline = $true }

$Root = Split-Path -Parent $PSScriptRoot
foreach ($key in @($supplied.Keys)) {
    if (-not $supplied[$key]) { continue }
    $path = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($supplied[$key])
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { Stop-Usage "SDK configuration not found: $path" }
    $supplied[$key] = [IO.Path]::GetFullPath($path)
}

# --- Environment, restored when the script ends (it may run inside a caller's session). ---
$savedEnvironment = [Collections.Generic.Dictionary[string, string]]::new([StringComparer]::OrdinalIgnoreCase)
foreach ($entry in [Environment]::GetEnvironmentVariables('Process').GetEnumerator()) {
    $savedEnvironment[$entry.Key] = $entry.Value
}
$savedConsoleEncoding = $null

function Restore-Environment {
    # Remove added variables: SetEnvironmentVariable(name, $null) would leave them empty on
    # PowerShell 7 with .NET 11, because PowerShell passes $null to a string parameter as ''.
    foreach ($variable in @([Environment]::GetEnvironmentVariables('Process').Keys)) {
        if (-not $savedEnvironment.ContainsKey($variable)) {
            Remove-Item -LiteralPath "Env:$variable" -ErrorAction SilentlyContinue
        }
    }
    foreach ($variable in $savedEnvironment.Keys) {
        [Environment]::SetEnvironmentVariable($variable, $savedEnvironment[$variable], 'Process')
    }
}

function Get-Application([string]$Name) {
    $command = Get-Command $Name -CommandType Application -ErrorAction Ignore | Select-Object -First 1
    if ($command) { return $command.Source }
    return $null
}

# Python 3.12+ through the py launcher, or a real python.exe when the launcher is absent;
# the same rule as build.bat. Steps run the resolved interpreter directly.
function Resolve-Python {
    $ErrorActionPreference = 'Continue'
    $probe = 'import sys; print(sys.executable) if sys.version_info >= (3, 12) else sys.exit(1)'
    $launcher = Get-Application 'py'
    $output = if ($launcher) { & $launcher -3 -c $probe 2>$null } else {
        $python = Get-Application 'python'
        if (-not $python) { return $null }
        & $python -c $probe 2>$null
    }
    if ($LASTEXITCODE -ne 0 -or -not $output) { return $null }
    return ([string](@($output)[-1])).Trim()
}

# The pinned web SDK archives are classic-zlib deflate output (web-viewer-api/scripts/package.ps1).
function Test-ClassicZlibPython {
    $ErrorActionPreference = 'Continue'
    $probe = 'import sys,zlib; sys.exit(zlib.ZLIB_RUNTIME_VERSION.lower().find(chr(110)+chr(103)) >= 0)'
    $launcher = Get-Application 'py'
    if ($launcher) {
        foreach ($version in @('-3', '-3.13', '-3.12', '-3.11')) {
            & $launcher $version -c $probe 2>$null | Out-Null
            if ($LASTEXITCODE -eq 0) { return $null }
        }
    } elseif ($Python) {
        & $Python -c $probe 2>$null | Out-Null
        if ($LASTEXITCODE -eq 0) { return $null }
    }
    return 'no Python with classic zlib: Python 3.14+ on Windows bundles zlib-ng, whose output ' +
        'differs from the pinned web SDK archives; install Python 3.13 (py -3.13) or pass --web-sdk'
}

function Test-Msvc {
    $ErrorActionPreference = 'Continue'
    $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
    if (-not (Test-Path -LiteralPath $vswhere -PathType Leaf)) { return 'the Visual Studio Installer (vswhere.exe) is missing' }
    $installations = @(& $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath 2>$null)
    if ($LASTEXITCODE -ne 0 -or $installations.Count -ne 1 -or -not $installations[0]) {
        return 'no Visual Studio installation with the x64 MSVC tools'
    }
    return $null
}

function Test-PowerShell7 {
    if ($Pwsh) { return $null }
    return 'PowerShell 7 (pwsh.exe) is not installed; the pinned native AV1 decoder bootstrap requires it'
}

# Offline runs can only reuse the pinned dav1d tool and source downloads of an earlier run.
function Test-Dav1dInputs([string]$BuildRoot) {
    if (-not $offline) { return $null }
    foreach ($receipt in @('native-tools\tool-provenance.json', 'native-dav1d\source.json')) {
        if (-not (Test-Path -LiteralPath (Join-Path $BuildRoot $receipt) -PathType Leaf)) {
            return "offline, and the pinned dav1d tools and source are not cached below $BuildRoot (run once online)"
        }
    }
    return $null
}

# Offline, Cargo resolves the HTML viewer's whole lockfile from its cache, including the
# optional CEF bindings that a build without native CEF does not compile. The dry run
# and a run before the web SDK exists leave the decision to the step.
function Test-HtmlCrates {
    if (-not $offline -or $dryRun -or -not $WebSdk) { return $null }
    $ErrorActionPreference = 'Continue'
    & $Cargo --config $WebSdk fetch --manifest-path plugins/midnight-viewer-html/Cargo.toml --locked --offline 2>&1 | Out-Null
    if ($LASTEXITCODE -eq 0) { return $null }
    return "offline, and the HTML viewer's locked crates (with the optional CEF bindings) are not in the Cargo cache (run once online)"
}

function Test-FfmpegInputs {
    foreach ($name in @('MIDNIGHT_FFMPEG_PREFIX', 'MIDNIGHT_FFMPEG_SOURCE_ARCHIVE', 'LIBCLANG_PATH')) {
        if (-not [Environment]::GetEnvironmentVariable($name)) {
            return 'no FFmpeg 9.0.2 MSVC prefix: set MIDNIGHT_FFMPEG_PREFIX, MIDNIGHT_FFMPEG_SOURCE_ARCHIVE ' +
                'and LIBCLANG_PATH as for the build (plugins/midnight-thumbnail-video/README.md)'
        }
    }
    return $null
}

# A PowerShell single-quoted literal. PowerShell also ends such strings at the typographic
# single quotes U+2018-U+201B, so those are doubled too (this file stays ASCII for 5.1).
$quoteCharacters = '[''' + [string][char]0x2018 + [char]0x2019 + [char]0x201A + [char]0x201B + ']'
function ConvertTo-Literal($Value) {
    if ($null -eq $Value) { return '$null' }
    if ($Value -is [bool]) { if ($Value) { return '$true' } else { return '$false' } }
    if ($Value -is [array]) { return '@(' + ((@($Value) | ForEach-Object { ConvertTo-Literal $_ }) -join ', ') + ')' }
    return "'" + ([string]$Value -replace $quoteCharacters, '$0$0') + "'"
}

# Script block text without its common indentation, for the plan and the step scripts.
function Format-Body([string]$Text) {
    $lines = @($Text -split "`r?`n" | ForEach-Object { $_.TrimEnd() })
    $indents = @($lines | Where-Object { $_ } | ForEach-Object { $_.Length - $_.TrimStart().Length })
    $indent = 0
    if ($indents.Count) { $indent = ($indents | Measure-Object -Minimum).Minimum }
    return @($lines | Where-Object { $_ } | ForEach-Object { $_.Substring($indent) })
}

function Format-Duration([TimeSpan]$Duration) {
    $culture = [Globalization.CultureInfo]::InvariantCulture
    if ($Duration.TotalHours -ge 1) {
        return [string]::Format($culture, '{0}h {1:D2}m', [int][Math]::Floor($Duration.TotalHours), $Duration.Minutes)
    }
    if ($Duration.TotalMinutes -ge 1) { return [string]::Format($culture, '{0}m {1:D2}s', $Duration.Minutes, $Duration.Seconds) }
    return [string]::Format($culture, '{0:F1}s', $Duration.TotalSeconds)
}

# libtest prints one "test result:" line per test executable and doc-test run;
# unittest prints "Ran N tests" and then "OK (skipped=K)" or "FAILED (failures=F, ...)".
function Get-TestCounts([string]$Log) {
    $counts = @{ Passed = 0; Failed = 0; Ignored = 0; Found = $false }
    if (-not (Test-Path -LiteralPath $Log -PathType Leaf)) { return $counts }
    $ran = $null
    foreach ($line in [IO.File]::ReadLines($Log)) {
        if ($line -match '^test result: \w+\. (\d+) passed; (\d+) failed; (\d+) ignored;') {
            $counts.Passed += [int]$Matches[1]; $counts.Failed += [int]$Matches[2]; $counts.Ignored += [int]$Matches[3]
            $counts.Found = $true
        } elseif ($line -match '^Ran (\d+) tests? in ') {
            $ran = [int]$Matches[1]
        } elseif ($null -ne $ran -and $line -match '^(OK|FAILED)(?: \((.*)\))?\s*$') {
            $details = @{}
            foreach ($part in ($Matches[2] -split ',\s*')) {
                if ($part -match '^([a-z ]+)=(\d+)$') { $details[$Matches[1]] = [int]$Matches[2] }
            }
            $failed = 0
            foreach ($key in @('failures', 'errors', 'unexpected successes')) { if ($details[$key]) { $failed += $details[$key] } }
            $skipped = 0
            if ($details['skipped']) { $skipped = $details['skipped'] }
            $counts.Passed += $ran - $failed - $skipped; $counts.Failed += $failed; $counts.Ignored += $skipped
            $counts.Found = $true
            $ran = $null
        }
    }
    return $counts
}

function Get-LoggedValue([string]$Log, [string]$Prefix) {
    $value = $null
    foreach ($line in [IO.File]::ReadLines($Log)) {
        if ($line.StartsWith($Prefix)) { $value = $line.Substring($Prefix.Length).Trim() }
    }
    if (-not $value) { throw "the step did not report '$Prefix'" }
    return $value
}

# Only directories below _build\validate are recreated.
function Reset-ValidationDirectory([string]$Path) {
    $full = [IO.Path]::GetFullPath($Path)
    if (-not $full.StartsWith($Out.TrimEnd('\') + '\', [StringComparison]::OrdinalIgnoreCase)) {
        throw "Refusing to reset a directory outside _build\validate: $full"
    }
    if (Test-Path -LiteralPath $full) {
        if ((Get-Item -LiteralPath $full -Force).Attributes -band [IO.FileAttributes]::ReparsePoint) {
            throw "Refusing to reset a linked directory: $full"
        }
        Remove-Item -LiteralPath $full -Recurse -Force
    }
    New-Item -ItemType Directory -Path $full -Force | Out-Null
}

# --- Tools and paths ---
Push-Location -LiteralPath $Root
try {
    $Out = Join-Path $Root '_build\validate'
    $temporaryRoot = Join-Path $Root '_build\tmp'
    if (-not (Get-Application 'cargo')) {
        $localCargo = Join-Path $Root '.tools\cargo\bin'
        if (Test-Path -LiteralPath (Join-Path $localCargo 'cargo.exe') -PathType Leaf) {
            $env:CARGO_HOME = Join-Path $Root '.tools\cargo'
            $env:RUSTUP_HOME = Join-Path $Root '.tools\rustup'
            $env:PATH = $localCargo + [IO.Path]::PathSeparator + $env:PATH
        }
    }
    $Cargo = Get-Application 'cargo'
    $Rustc = Get-Application 'rustc'
    $Python = Resolve-Python
    $Pwsh = Get-Application 'pwsh'
    if ($PSVersionTable.PSVersion.Major -ge 7) { $Pwsh = (Get-Process -Id $PID).Path }
    $WindowsPowerShell = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
    $Shell = if ($Pwsh) { $Pwsh } else { (Get-Process -Id $PID).Path }
    $missing = @()
    if (-not $Cargo) { $missing += 'cargo (install Rust with rustup)' }
    if (-not $Rustc) { $missing += 'rustc on PATH (the user-command probe tests compile with it)' }
    if (-not $Python) { $missing += 'Python 3.12+ (py launcher or python.exe)' }
    $Target = $null
    if ($Rustc) {
        $hostLine = @(& $Rustc -vV | Where-Object { $_ -like 'host: *' })
        if ($hostLine.Count -eq 1) { $Target = $hostLine[0].Substring(6).Trim() }
    }
    $CargoTarget = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { '_build' }
    if (-not [IO.Path]::IsPathRooted($CargoTarget)) { $CargoTarget = Join-Path $Root $CargoTarget }
    $CargoTarget = [IO.Path]::GetFullPath($CargoTarget)
    $RunId = (Get-Date).ToString('yyyyMMdd-HHmmss')
    $RunDirectory = Join-Path $Out "logs\$RunId"
    for ($suffix = 2; Test-Path -LiteralPath $RunDirectory; $suffix++) { $RunDirectory = Join-Path $Out "logs\$RunId-$suffix" }
    $OfflineFlag = @()
    if ($offline) { $OfflineFlag = @('--offline') }
    $PluginSdk = $supplied.PluginSdk
    $WorkerSdk = $supplied.WorkerSdk
    $WebSdk = $supplied.WebSdk
    $WebSdkOutput = Join-Path $Out ('web-sdk\release-' + (Split-Path -Leaf $RunDirectory))
    $TestRoot = $null
    # SDK bundles this run packages (60-80 MB each run); removed when it ends.
    $PackagedBundles = New-Object Collections.Generic.List[string]

    # --- Steps, in CI's job order: SDK jobs, plugin jobs, check, release. ---
    $steps = New-Object Collections.Generic.List[object]
    function Add-Step([hashtable]$Step) {
        if (-not $Step.ContainsKey('Needs')) { $Step.Needs = @() }
        if (-not $Step.ContainsKey('Variables')) { $Step.Variables = @{} }
        $steps.Add($Step)
    }

    function Install-TestPackages {
        # Mirrors .github/actions/private-test-installation and plugin-packages with the
        # packages built above.
        $viewerPackages = Join-Path $Out 'viewer-packages'
        $filesystemPackages = Join-Path $Out 'filesystem-packages'
        Reset-ValidationDirectory $viewerPackages
        Reset-ValidationDirectory $filesystemPackages
        $packages = @(
            @('viewer-basic', 'viewer-ci\package\midnight-viewer-basic', $viewerPackages),
            @('viewer-source', 'viewer-ci\package\midnight-viewer-source', $viewerPackages),
            @('viewer-markdown', 'viewer-ci\package\midnight-viewer-markdown', $viewerPackages),
            @('viewer-image', 'raster-ci\package\midnight-viewer-image', $viewerPackages),
            @('viewer-pdf', 'raster-ci\package\midnight-viewer-pdf', $viewerPackages),
            @('filesystem-native', 'filesystem-library\package\native.windows', $filesystemPackages),
            @('filesystem-network', 'filesystem-library\package\network.windows', $filesystemPackages),
            @('archive', 'archive-ci\package\archive', $filesystemPackages))
        foreach ($package in $packages) {
            # Only packages built by this run; a skipped step leaves no stale package behind.
            if (-not $results[$package[0]] -or $results[$package[0]].Status -ne 'PASS') { continue }
            Copy-Item -LiteralPath (Join-Path $Out $package[1]) -Destination $package[2] -Recurse
            Write-Output ('Installed the package of ' + $package[0] + ': ' + (Split-Path -Leaf $package[1]))
        }
        $testRoot = Join-Path $env:USERPROFILE ('MidnightViewerTests-' + [guid]::NewGuid().ToString('N'))
        New-Item -ItemType Directory -Path $testRoot | Out-Null
        $script:TestRoot = $testRoot
        $sid = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
        $acl = New-Object Security.AccessControl.DirectorySecurity
        $acl.SetSecurityDescriptorSddlForm("D:P(A;OICI;FA;;;$sid)(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)",
            [Security.AccessControl.AccessControlSections]::Access)
        Set-Acl -LiteralPath $testRoot -AclObject $acl
        Copy-Item -LiteralPath $viewerPackages -Destination (Join-Path $testRoot 'viewers') -Recurse
        Copy-Item -LiteralPath $filesystemPackages -Destination (Join-Path $testRoot 'filesystems') -Recurse
        $env:HOST_TEST_INSTALL_ROOT = $testRoot
        $env:HOST_TEST_PLUGIN = Join-Path $viewerPackages 'midnight-viewer-basic\midnight_viewer_basic.dll'
        $env:MIDNIGHT_VIEWER_TEST_INSTALL = $testRoot
        $env:MIDNIGHT_NATIVE_TEST_INSTALLATION = $testRoot
        $env:MIDNIGHT_IMAGE_VIEWER_TEST_APPLICATION = Join-Path $testRoot 'midnight-explorer.exe'
        $env:MIDNIGHT_PDF_VIEWER_TEST_APPLICATION = $env:MIDNIGHT_IMAGE_VIEWER_TEST_APPLICATION
        $env:ARCHIVE_WORKER = Join-Path $filesystemPackages 'archive\midnight-fs-archive.exe'
        $env:ARCHIVE_TEST_INSTALL_ROOT = $testRoot
        Write-Output "HOST_TEST_INSTALL_ROOT=$testRoot (owner, SYSTEM and Administrators only)"
    }
    # The test command of the plugin's scripts/ci.ps1 (CI's viewers job), then the package
    # build that the private test installation needs.
    $viewerBody = {
        $pluginRoot = "plugins/midnight-viewer-$Plugin"
        $build = Join-Path $Out 'viewer-ci'
        $config = $PluginSdk
        if (Test-Path "$pluginRoot/scripts/prepare-sdk.ps1") {
            $config = & "$pluginRoot/scripts/prepare-sdk.ps1" -SdkConfig $PluginSdk -BuildRoot $build
        }
        cargo --config $config fetch --manifest-path "$pluginRoot/Cargo.toml" --locked @OfflineFlag
        if ($LASTEXITCODE -ne 0) { throw 'Dependency fetch failed' }
        cargo --config $config test --manifest-path "$pluginRoot/Cargo.toml" --target-dir (Join-Path $build 'target') --target $Target --locked --offline --all-targets
        if ($LASTEXITCODE -ne 0) { throw 'Viewer tests failed' }
        & "$pluginRoot/scripts/build.ps1" -SdkConfig $PluginSdk -BuildRoot $build
        if ($LASTEXITCODE -ne 0) { throw 'Viewer package build failed' }
    }
    # The image viewer, PDF viewer and image thumbnailer share one build root, so the
    # pinned dav1d tools and library are prepared once. Each runs the test commands of
    # its scripts/ci.ps1 (the image viewer also the shared decoder's); the two viewers
    # then build the package that the private test installation needs.
    $rasterBody = {
        $build = Join-Path $Out 'raster-ci'
        $pluginRoot = "plugins/midnight-$Plugin"
        cargo --config $WorkerSdk fetch --manifest-path "$pluginRoot/Cargo.toml" --locked @OfflineFlag
        if ($LASTEXITCODE -ne 0) { throw 'Dependency fetch failed' }
        if ($Plugin -ne 'viewer-pdf') {
            $jobs = if ($env:CARGO_BUILD_JOBS) { [int]$env:CARGO_BUILD_JOBS } else { 2 }
            & plugins/image-decoder/native/bootstrap-windows.ps1 -BuildRoot (Join-Path $build 'native-tools') -Offline:$Offline
            & plugins/image-decoder/native/build-windows.ps1 -BuildRoot (Join-Path $build 'native-dav1d') -Jobs $jobs -Offline:$Offline
        }
        $projects = @($pluginRoot)
        if ($Plugin -eq 'viewer-image') { $projects = @('plugins/image-decoder', $pluginRoot) }
        $targetOption = @('--target', $Target)
        if ($Plugin -eq 'thumbnail-image') { $targetOption = @() }
        foreach ($project in $projects) {
            cargo test --config $WorkerSdk --manifest-path "$project/Cargo.toml" --target-dir (Join-Path $build 'target') @targetOption --locked --offline --all-targets
            if ($LASTEXITCODE -ne 0) { throw "$project tests failed" }
        }
        if ($Plugin -eq 'viewer-image') {
            & "$pluginRoot/scripts/build.ps1" -SdkConfig $WorkerSdk -BuildRoot $build -NativeOffline:$Offline
            if ($LASTEXITCODE -ne 0) { throw 'Image viewer package build failed' }
        } elseif ($Plugin -eq 'viewer-pdf') {
            & "$pluginRoot/scripts/build.ps1" -SdkConfig $WorkerSdk -BuildRoot $build -Profile release
            if ($LASTEXITCODE -ne 0) { throw 'PDF viewer package build failed' }
        }
    }
    $filesystemBody = {
        $build = Join-Path $Out 'filesystem-library'
        $env:CARGO_TARGET_DIR = $build
        $pluginRoot = "plugins/$PluginName"
        cargo test --manifest-path "$pluginRoot/Cargo.toml" --locked
        if ($LASTEXITCODE -ne 0) { throw 'Filesystem plugin tests failed' }
        cargo build --manifest-path "$pluginRoot/Cargo.toml" --release --locked --offline
        if ($LASTEXITCODE -ne 0) { throw 'Filesystem plugin release build failed' }
        $package = Join-Path $build "package/$Provider.windows"
        if (Test-Path -LiteralPath $package) { Remove-Item -LiteralPath $package -Recurse -Force }
        New-Item -ItemType Directory -Force $package | Out-Null
        Copy-Item -LiteralPath (Join-Path $build "release/$Library") -Destination $package
        Copy-Item -LiteralPath "$pluginRoot/$Manifest" -Destination (Join-Path $package 'plugin.toml')
        foreach ($name in @('README.md', 'THIRD-PARTY.md')) { Copy-Item -LiteralPath "$pluginRoot/$name" -Destination $package }
    }
    $ignoredBody = {
        & $Python -B scripts/ci-workspace-tests.py run @Selection
        if ($LASTEXITCODE -ne 0) { throw 'Ignored qualification tests failed' }
    }
    $allViewers = @('viewer-basic', 'viewer-source', 'viewer-markdown', 'viewer-image', 'viewer-pdf')

    Add-Step @{ Id = 'sdk-api'; Job = 'public-sdk'; Title = 'Public API and SDK tests'; Body = {
        $env:CARGO_TARGET_DIR = Join-Path $Out 'sdk-target'
        cargo fetch --manifest-path api/Cargo.toml --locked @OfflineFlag
        if ($LASTEXITCODE -ne 0) { throw 'API dependency fetch failed' }
        cargo test --manifest-path api/Cargo.toml --all-targets --locked --offline
        if ($LASTEXITCODE -ne 0) { throw 'API tests failed' }
    } }
    Add-Step @{ Id = 'sdk-abi'; Job = 'public-sdk'; Title = 'C and Rust ABI layout agreement'
        Prerequisite = { Test-Msvc }; Body = {
        $env:CARGO_TARGET_DIR = Join-Path $Out 'sdk-target'
        & api/scripts/check-abi.ps1 -BuildRoot (Join-Path $Out 'api-abi/windows')
        if ($LASTEXITCODE -ne 0) { throw 'ABI layout check failed' }
    } }
    Add-Step @{ Id = 'sdk-package'; Job = 'public-sdk'; Title = 'Build public SDK release bundle'
        Supplied = 'PluginSdk'; Body = {
        $env:CARGO_TARGET_DIR = Join-Path $Out 'sdk-target'
        cargo fetch --manifest-path api/Cargo.toml --locked @OfflineFlag
        if ($LASTEXITCODE -ne 0) { throw 'API dependency fetch failed' }
        & api/scripts/package.ps1
        if ($LASTEXITCODE -ne 0) { throw 'Public SDK packaging failed' }
    }; OnPass = {
        param($Log)
        $bundle = Get-LoggedValue $Log 'Public API/SDK release bundle: '
        $PackagedBundles.Add($bundle)
        $script:PluginSdk = Join-Path $bundle '.cargo\config.toml'
        if (-not (Test-Path -LiteralPath $PluginSdk -PathType Leaf)) { throw "missing $PluginSdk" }
    } }
    Add-Step @{ Id = 'sdk-protocols'; Job = 'public-worker-sdk'; Title = 'Test public protocol workspaces'; Body = {
        $env:CARGO_TARGET_DIR = Join-Path $Out 'sdk-target'
        foreach ($api in @('api', 'worker-api', 'thumbnail-api', 'filesystem-api', 'image-viewer-api', 'web-viewer-api')) {
            cargo fetch --manifest-path "$api/Cargo.toml" --locked @OfflineFlag
            if ($LASTEXITCODE -ne 0) { throw "$api dependency fetch failed" }
        }
        foreach ($api in @('worker-api', 'thumbnail-api', 'filesystem-api', 'image-viewer-api', 'web-viewer-api')) {
            cargo test --manifest-path "$api/Cargo.toml" --workspace --all-targets --locked --offline
            if ($LASTEXITCODE -ne 0) { throw "$api tests failed" }
        }
    } }
    Add-Step @{ Id = 'sdk-web-package'; Job = 'public-worker-sdk'; Title = 'Package the public web SDK'
        Supplied = 'WebSdk'; Prerequisite = { Test-ClassicZlibPython }; Body = {
        cargo fetch --manifest-path web-viewer-api/Cargo.toml --locked @OfflineFlag
        if ($LASTEXITCODE -ne 0) { throw 'Web SDK dependency fetch failed' }
        & web-viewer-api/scripts/package.ps1 -Output $WebSdkOutput
        if ($LASTEXITCODE -ne 0) { throw 'Public web SDK packaging failed' }
    }; OnPass = {
        $PackagedBundles.Add($WebSdkOutput)
        $script:WebSdk = Join-Path $WebSdkOutput 'config.toml'
        if (-not (Test-Path -LiteralPath $WebSdk -PathType Leaf)) { throw "missing $WebSdk" }
    } }
    Add-Step @{ Id = 'sdk-worker-package'; Job = 'public-worker-sdk'; Title = 'Package public image and worker APIs'
        Supplied = 'WorkerSdk'; Body = {
        $env:CARGO_TARGET_DIR = Join-Path $Out 'sdk-target'
        foreach ($api in @('api', 'worker-api', 'thumbnail-api', 'filesystem-api', 'image-viewer-api')) {
            cargo fetch --manifest-path "$api/Cargo.toml" --locked @OfflineFlag
            if ($LASTEXITCODE -ne 0) { throw "$api dependency fetch failed" }
        }
        cargo run --manifest-path api/Cargo.toml -p midnight-api-package --bin worker-packages --locked --offline -- $Root (Join-Path $Out 'releases')
        if ($LASTEXITCODE -ne 0) { throw 'Worker SDK packaging failed' }
    }; OnPass = {
        param($Log)
        $bundle = Get-LoggedValue $Log 'Public worker API/SDK release bundle: '
        $PackagedBundles.Add($bundle)
        $script:WorkerSdk = Join-Path $bundle '.cargo\registry.toml'
        if (-not (Test-Path -LiteralPath $WorkerSdk -PathType Leaf)) { throw "missing $WorkerSdk" }
    } }
    foreach ($name in @('basic', 'source', 'markdown')) {
        Add-Step @{ Id = "viewer-$name"; Job = 'viewers'; Title = "Independent $name viewer tests and package"
            Needs = @('sdk-package'); Variables = @{ Plugin = $name }; Body = $viewerBody }
    }
    Add-Step @{ Id = 'viewer-conformance'; Job = 'viewer-conformance'; Title = 'Released SDK ABI conformance'
        Needs = @('sdk-package'); Prerequisite = { Test-Msvc }; Body = {
        $include = Join-Path (Split-Path -Parent (Split-Path -Parent $PluginSdk)) 'include'
        $manifest = 'plugins/viewer-conformance/Cargo.toml'
        $env:CARGO_TARGET_DIR = Join-Path $Out 'conformance-target'
        cargo --config $PluginSdk fetch --manifest-path $manifest --locked @OfflineFlag
        if ($LASTEXITCODE -ne 0) { throw 'Conformance dependency fetch failed' }
        cargo test --config $PluginSdk --manifest-path $manifest --locked --offline
        if ($LASTEXITCODE -ne 0) { throw 'Conformance Rust tests failed' }
        cargo build --config $PluginSdk --manifest-path $manifest --release --locked --offline
        if ($LASTEXITCODE -ne 0) { throw 'Conformance fixture build failed' }
        # The native loader test compiles C with an x64 MSVC developer environment.
        $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio/Installer/vswhere.exe'
        $studio = @(& $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath)
        if ($LASTEXITCODE -ne 0 -or $studio.Count -ne 1) { throw 'Could not find installed x64 MSVC tools' }
        & (Join-Path $studio[0] 'Common7/Tools/Launch-VsDevShell.ps1') -Arch amd64 -HostArch amd64 -SkipAutomaticLocation | Out-Host
        & plugins/viewer-conformance/tests/test-windows.ps1 `
            -Library (Join-Path $env:CARGO_TARGET_DIR 'release/midnight_viewer_conformance.dll') `
            -HeaderDirectory $include -BuildRoot (Join-Path $Out 'conformance/windows')
        if ($LASTEXITCODE -ne 0) { throw 'Native viewer ABI conformance failed' }
    } }
    Add-Step @{ Id = 'viewer-image'; Job = 'image-viewer'; Title = 'Independent raster image viewer'
        Needs = @('sdk-worker-package'); Variables = @{ Plugin = 'viewer-image' }; Body = $rasterBody
        Prerequisite = { Test-PowerShell7; Test-Msvc; Test-Dav1dInputs (Join-Path $Out 'raster-ci') } }
    Add-Step @{ Id = 'viewer-pdf'; Job = 'image-viewer'; Title = 'Independent PDF viewer'
        Needs = @('sdk-worker-package'); Variables = @{ Plugin = 'viewer-pdf' }; Body = $rasterBody }
    Add-Step @{ Id = 'thumbnail-image'; Job = 'thumbnail-image'; Title = 'Independent image thumbnailer'
        Needs = @('sdk-worker-package'); Variables = @{ Plugin = 'thumbnail-image' }; Body = $rasterBody
        Prerequisite = { Test-PowerShell7; Test-Msvc; Test-Dav1dInputs (Join-Path $Out 'raster-ci') } }
    Add-Step @{ Id = 'thumbnail-video'; Job = 'not in CI'; Title = 'Video thumbnailer (pinned FFmpeg)'
        Needs = @('sdk-worker-package'); Prerequisite = { Test-FfmpegInputs }; Body = {
        $manifest = 'plugins/midnight-thumbnail-video/Cargo.toml'
        # The build's own preparation validates the FFmpeg prefix and finds libclang.
        $state = & $Python -B scripts/prepare-thumbnail-ffmpeg.py --build-root (Join-Path $Root '_build/thumbnailers/ffmpeg') @OfflineFlag
        if ($LASTEXITCODE -ne 0) { throw 'FFmpeg preparation failed' }
        $native = ($state -join "`n") | ConvertFrom-Json
        $env:FFMPEG_DIR = $native.prefix
        $env:LIBCLANG_PATH = $native.libclang_path
        $env:PATH = (Join-Path $native.prefix 'bin') + [IO.Path]::PathSeparator + $env:PATH
        $env:CARGO_TARGET_DIR = Join-Path $Out 'thumbnail-video-target'
        cargo test --config $WorkerSdk --manifest-path $manifest --all-targets --locked --offline
        if ($LASTEXITCODE -ne 0) { throw 'Video thumbnailer tests failed' }
    } }
    Add-Step @{ Id = 'html-viewer'; Job = 'not in CI'; Title = 'HTML viewer without native CEF'
        Needs = @('sdk-web-package'); Prerequisite = { Test-HtmlCrates }; Body = {
        $manifest = 'plugins/midnight-viewer-html/Cargo.toml'
        # The Windows static CRT configuration the provider build uses (plugin README).
        $crt = 'plugins/midnight-viewer-html/.cargo/config.toml'
        $env:CARGO_TARGET_DIR = Join-Path $Out 'html-viewer-target'
        cargo --config $WebSdk fetch --manifest-path $manifest --locked @OfflineFlag
        if ($LASTEXITCODE -ne 0) { throw 'HTML viewer dependency fetch failed' }
        cargo test --config $WebSdk --config $crt --manifest-path $manifest --no-default-features --all-targets --locked --offline
        if ($LASTEXITCODE -ne 0) { throw 'HTML viewer tests failed' }
    } }
    Add-Step @{ Id = 'archive'; Job = 'archive-filesystem'; Title = 'Archive filesystem tests, package and fixtures'
        Needs = @('sdk-worker-package'); Before = { Reset-ValidationDirectory (Join-Path $Out 'archive-fixtures') }; Body = {
        $env:CARGO_INCREMENTAL = '0'; $env:CARGO_PROFILE_DEV_DEBUG = '0'; $env:CARGO_PROFILE_TEST_DEBUG = '0'; $env:RUST_TEST_THREADS = '1'
        $build = Join-Path $Out 'archive-ci'
        $package = Join-Path $build 'package/archive'
        $plugin = 'plugins/midnight-fs-archive'
        $env:CARGO_TARGET_DIR = $build
        cargo --config $WorkerSdk fetch --manifest-path "$plugin/Cargo.toml" --locked @OfflineFlag
        if ($LASTEXITCODE -ne 0) { throw 'Archive dependency fetch failed' }
        & "$plugin/scripts/build.ps1" -SdkConfig $WorkerSdk -BuildRoot $build -OutputDirectory $package -Profile release -Test -Offline
        if ($LASTEXITCODE -ne 0) { throw 'Independent archive tests or packaging failed' }
        $env:MIDNIGHT_ARCHIVE_FIXTURES = Join-Path $Out 'archive-fixtures'
        cargo test --config $WorkerSdk --manifest-path "$plugin/Cargo.toml" --test worker_fixtures --locked --offline -- --ignored
        if ($LASTEXITCODE -ne 0) { throw 'Archive worker fixture export failed' }
    } }
    Add-Step @{ Id = 'filesystem-api'; Job = 'filesystem-libraries'; Title = 'Public filesystem API and SDK'; Body = {
        $env:CARGO_TARGET_DIR = Join-Path $Out 'filesystem-library'
        cargo test --manifest-path filesystem-api/Cargo.toml --workspace --locked
        if ($LASTEXITCODE -ne 0) { throw 'Filesystem API/SDK tests failed' }
    } }
    Add-Step @{ Id = 'filesystem-native'; Job = 'filesystem-libraries'; Title = 'Native filesystem plugin and package'
        Variables = @{ Provider = 'native'; PluginName = 'midnight-filesystem-native-windows'
            Library = 'midnight_filesystem_native_windows.dll'; Manifest = 'plugin.toml' }; Body = $filesystemBody }
    Add-Step @{ Id = 'filesystem-network'; Job = 'filesystem-libraries'; Title = 'Network filesystem plugin and package'
        Variables = @{ Provider = 'network'; PluginName = 'midnight-filesystem-network'
            Library = 'midnight_filesystem_network.dll'; Manifest = 'plugin.windows.toml' }; Body = $filesystemBody }
    Add-Step @{ Id = 'test-install'; Job = 'scripts-and-lint'; Title = 'Private test installation'; Local = { Install-TestPackages } }
    Add-Step @{ Id = 'script-tests'; Job = 'scripts-and-lint'; Title = 'Build script and packaging tests'; Needs = @('test-install'); Body = {
        & $Python -B -m unittest discover -s scripts/tests -v
        if ($LASTEXITCODE -ne 0) { throw 'Script tests failed' }
    } }
    Add-Step @{ Id = 'decoder-native-tests'; Job = 'scripts-and-lint'; Title = 'Shared image decoder native helper tests'; Body = {
        & $Python -B -m unittest discover -s plugins/image-decoder/native/tests -v
        if ($LASTEXITCODE -ne 0) { throw 'Native helper tests failed' }
    } }
    $powerShellTests = {
        foreach ($test in @('test-private-deployment.ps1', 'test-runtime-registration.ps1',
                'test-build-deployment-retirement.ps1', 'test-desktop-shortcut.ps1')) {
            & $TestShell -NoLogo -NoProfile -File "scripts/tests/$test"
            if ($LASTEXITCODE -ne 0) { throw "$test failed under $TestShell" }
        }
    }
    Add-Step @{ Id = 'powershell-tests-7'; Job = 'scripts-and-lint'; Title = 'Deployment and shortcut tests (PowerShell 7)'
        Platform = 'windows'; Needs = @('test-install'); Variables = @{ TestShell = $Pwsh }; Body = $powerShellTests
        Prerequisite = { if (-not $Pwsh) { 'PowerShell 7 (pwsh.exe) is not installed' } } }
    Add-Step @{ Id = 'powershell-tests-5'; Job = 'scripts-and-lint'; Title = 'Deployment and shortcut tests (PowerShell 5.1)'
        Platform = 'windows'; Needs = @('test-install'); Variables = @{ TestShell = $WindowsPowerShell }; Body = $powerShellTests }
    Add-Step @{ Id = 'workspace-tests'; Job = 'check'; Title = 'Test (workspace gates, doc tests included)'
        Needs = @('test-install'); Body = {
        # CI's gates without its disk-saving artifact retirement; every gate runs.
        & $Python -B scripts/ci-workspace-tests.py --keep-test-artifacts --no-fail-fast
        if ($LASTEXITCODE -ne 0) { throw 'Workspace tests failed' }
    } }
    Add-Step @{ Id = 'trash-qualification'; Job = 'check'; Title = 'Qualify Linux cross-filesystem Trash'; Platform = 'linux' }
    Add-Step @{ Id = 'bad-descriptor'; Job = 'check'; Title = 'Native viewer ABI rejection fixture'; Body = {
        cargo build --manifest-path crates/midnight-plugin-host/tests/fixtures/bad-descriptor/Cargo.toml --locked --offline --target-dir (Join-Path $Out 'bad-viewer-descriptor')
        if ($LASTEXITCODE -ne 0) { throw 'Bad descriptor fixture build failed' }
    }; OnPass = {
        $env:HOST_BAD_PLUGIN = Join-Path $Out 'bad-viewer-descriptor\debug\midnight_host_bad_descriptor.dll'
        if (-not (Test-Path -LiteralPath $env:HOST_BAD_PLUGIN -PathType Leaf)) { throw "missing $env:HOST_BAD_PLUGIN" }
    } }
    Add-Step @{ Id = 'host-plugin-ignored'; Job = 'check'; Title = 'Native viewer open, viewport, search and close'
        Needs = @('test-install', 'bad-descriptor', 'viewer-basic'); Body = $ignoredBody
        Variables = @{ Selection = @('midnight-plugin-host', 'lib', '--ignored') } }
    Add-Step @{ Id = 'viewer-engine-ignored'; Job = 'check'; Title = 'Native viewer engine integration'
        Needs = @('test-install', 'filesystem-native') + $allViewers; Body = $ignoredBody
        # As in CI: the native CEF Office/HTML and qualified-product Source acceptance tests are manual.
        Variables = @{ Selection = @('midnight-viewer-engine', 'lib', '--ignored', '--nocapture',
                '--skip', 'office_preview::native_tests', '--skip', 'tests::source_native_acceptance') } }
    Add-Step @{ Id = 'native-fs-ignored'; Job = 'check'; Title = 'Installed native filesystem browse and reads'
        Needs = @('test-install', 'filesystem-native'); Body = $ignoredBody
        Variables = @{ Selection = @('midnight-native-filesystem-host', 'test:native', '--ignored', '--test-threads=1') } }
    Add-Step @{ Id = 'network-metadata-ignored'; Job = 'check'; Title = 'Installed network metadata boundary'
        Needs = @('test-install', 'filesystem-network'); Body = {
        $env:MIDNIGHT_NETWORK_TEST_INSTALLATION = $env:MIDNIGHT_NATIVE_TEST_INSTALLATION
        & $Python -B scripts/ci-workspace-tests.py run midnight-native-filesystem-host lib installed_network_metadata --ignored --test-threads=1
        if ($LASTEXITCODE -ne 0) { throw 'Installed network metadata boundary failed' }
    } }
    Add-Step @{ Id = 'app-native-viewers'; Job = 'check'; Title = 'Application viewers use the native provider'
        Needs = @('test-install', 'filesystem-native') + $allViewers; Body = {
        $executable = & $Python -B scripts/ci-workspace-tests.py artifact midnight-explorer bin:me
        if ($LASTEXITCODE -ne 0 -or -not $executable) { throw 'Application test executable not reported' }
        $probe = Join-Path $env:HOST_TEST_INSTALL_ROOT 'native-app-test.exe'
        Copy-Item -LiteralPath ($executable | Select-Object -Last 1) -Destination $probe -Force
        $fixture = Join-Path $Out 'native-app-fixture'
        New-Item -ItemType Directory -Force $fixture | Out-Null
        $first = 'first ' + [char]0x6587 + [char]0x6863 + '.txt'
        $utf8 = New-Object Text.UTF8Encoding $false
        [IO.File]::WriteAllText((Join-Path $fixture $first), "First native provider content`r`n", $utf8)
        [IO.File]::WriteAllText((Join-Path $fixture 'second.txt'), "Second native provider content`r`n", $utf8)
        $env:MIDNIGHT_APP_NATIVE_TEST_FILES = $fixture
        & $probe viewers::tests::native_plugin_ --ignored --nocapture --test-threads=1
        if ($LASTEXITCODE -ne 0) { throw 'Application F3 native plugin integration failed' }
    } }
    Add-Step @{ Id = 'image-process-ignored'; Job = 'check'; Title = 'Restricted image process qualification'
        Needs = @('test-install', 'filesystem-native', 'viewer-image'); Body = $ignoredBody
        Variables = @{ Selection = @('midnight-image-viewer-host', 'test:native', '--ignored', '--test-threads=1') } }
    Add-Step @{ Id = 'pdf-process-ignored'; Job = 'check'; Title = 'Restricted PDF process qualification'
        Needs = @('test-install', 'filesystem-native', 'viewer-pdf'); Body = $ignoredBody
        Variables = @{ Selection = @('midnight-image-viewer-host', 'test:pdf_native', '--ignored', '--test-threads=1') } }
    Add-Step @{ Id = 'archive-worker-ignored'; Job = 'check'; Title = 'Restricted archive worker listing'
        Needs = @('test-install', 'archive'); Body = $ignoredBody
        # As in CI: listing archives of millions of members is a manual measurement.
        Variables = @{ Selection = @('midnight-filesystem-host', 'test:worker', '--ignored', '--test-threads=1',
                '--skip', 'real_sandboxed_worker_lists_archives_of_millions_of_members') } }
    Add-Step @{ Id = 'archive-nested-ignored'; Job = 'check'; Title = 'Restricted archive workers inside archives'
        Needs = @('test-install', 'archive'); Body = $ignoredBody
        Variables = @{ Selection = @('midnight-filesystem-host', 'test:nested_worker', '--ignored', '--test-threads=1') } }
    Add-Step @{ Id = 'archive-streaming-ignored'; Job = 'check'; Title = 'Installed archive worker streaming and discovery'
        Needs = @('test-install', 'archive'); Body = {
        $env:ARCHIVE_CAPTURE_INSTALLATION = $env:HOST_TEST_INSTALL_ROOT
        $env:MIDNIGHT_ARCHIVE_FIXTURES = Join-Path $Out 'archive-fixtures'
        $env:MIDNIGHT_ARCHIVE_TEST_ZIP = Join-Path $Out 'archive-test.zip'
        # The fixture script never replaces a file; this one is regenerated identically.
        if (Test-Path -LiteralPath $env:MIDNIGHT_ARCHIVE_TEST_ZIP) { Remove-Item -LiteralPath $env:MIDNIGHT_ARCHIVE_TEST_ZIP }
        & $Python -B scripts/archive-test-zip.py $env:MIDNIGHT_ARCHIVE_TEST_ZIP
        if ($LASTEXITCODE -ne 0) { throw 'Large ZIP fixture generation failed' }
        # Decoded members are staged on the TEMP volume, which must keep max(1 GiB, 2%) + 64 MiB free.
        Get-PSDrive -Name $env:TEMP.Substring(0, 1) | Format-Table Name, Used, Free | Out-Host
        & $Python -B scripts/ci-workspace-tests.py run midnight-filesystem-host test:streaming_worker --ignored --test-threads=1
        if ($LASTEXITCODE -ne 0) { throw 'Installed archive worker streaming qualification failed' }
        & $Python -B scripts/ci-workspace-tests.py run midnight-filesystem-host lib captured_source_tests:: --ignored --test-threads=1
        if ($LASTEXITCODE -ne 0) { throw 'Installed archive worker pathless content failed' }
        & $Python -B scripts/ci-workspace-tests.py run midnight-filesystem-host test:discovery_kinds --ignored --test-threads=1
        if ($LASTEXITCODE -ne 0) { throw 'Protected filesystem package discovery failed' }
    } }
    Add-Step @{ Id = 'x11-dnd'; Job = 'check'; Title = 'Native X11 drag and drop (isolated display)'; Platform = 'linux' }
    Add-Step @{ Id = 'x11-clipboard'; Job = 'check'; Title = 'Native X11 clipboard (isolated display)'; Platform = 'linux' }
    Add-Step @{ Id = 'host-build'; Job = 'release'; Title = "Host build ($profileName)"; Body = {
        if ($BuildProfile -eq 'release') {
            cargo build -p midnight-explorer -p midnight-elevation --locked --release
        } else {
            cargo build -p midnight-explorer -p midnight-elevation --locked
        }
        if ($LASTEXITCODE -ne 0) { throw 'Host build failed' }
    } }
    Add-Step @{ Id = 'smoke-test'; Job = 'release'; Title = "Installed filesystem smoke test ($profileName)"
        Needs = @('test-install', 'host-build'); Body = {
        $executable = Join-Path $env:HOST_TEST_INSTALL_ROOT 'me.exe'
        Copy-Item -LiteralPath (Join-Path $CargoTarget "$BuildProfile/me.exe") -Destination $executable -Force
        # The Administrator helper is built but never installed in a private test installation.
        if (-not (Test-Path -LiteralPath (Join-Path $CargoTarget "$BuildProfile/midnight-elevated-listing.exe") -PathType Leaf)) {
            throw "The $BuildProfile build is missing midnight-elevated-listing.exe"
        }
        $process = Start-Process -FilePath $executable -ArgumentList '--smoke-test' -WindowStyle Hidden -Wait -PassThru
        if ($process.ExitCode -ne 0) { throw "Installed native filesystem smoke failed (exit $($process.ExitCode))" }
        Write-Output 'Installed native filesystem smoke test passed.'
    } }

    # --- Selection, dependencies and prerequisites ---
    $byId = @{}
    foreach ($step in $steps) { $byId[$step.Id] = $step }
    $selected = $null
    if ($selection.Count) {
        $selected = New-Object 'Collections.Generic.HashSet[string]'
        $pending = New-Object Collections.Generic.Stack[string]
        foreach ($id in $selection) {
            if (-not $byId.ContainsKey($id)) { Stop-Usage "unknown step '$id' (see --dry-run for the step list)" }
            # Ids match case-insensitively; the set below holds their canonical spelling.
            $pending.Push($byId[$id].Id)
        }
        while ($pending.Count) {
            $id = $pending.Pop()
            if ($selected.Add($id)) { foreach ($need in $byId[$id].Needs) { $pending.Push($need) } }
        }
    }

    function Get-SkipReason($Step, [hashtable]$Results) {
        if ($Step.Platform -and $Step.Platform -ne 'windows') {
            return @{ Kind = 'platform'; Reason = 'Linux only (CI runs it on ubuntu-24.04)' }
        }
        if ($Step.Supplied -and $supplied[$Step.Supplied]) {
            return @{ Kind = 'supplied'; Reason = "uses the bundle given with --$($Step.Supplied.Replace('Sdk', '').ToLowerInvariant())-sdk" }
        }
        foreach ($need in $Step.Needs) {
            $neededStep = $byId[$need]
            if ($neededStep.Supplied -and $supplied[$neededStep.Supplied]) { continue }
            if ($null -ne $Results) {
                $result = $Results[$need]
                if ($result.Status -ne 'PASS') { return @{ Kind = 'needs'; Reason = "needs $need ($($result.Status))" } }
            }
        }
        if ($Step.Prerequisite) {
            foreach ($reason in @(& $Step.Prerequisite)) {
                if ($reason) { return @{ Kind = 'prerequisite'; Reason = $reason } }
            }
        }
        return $null
    }

    $globals = [ordered]@{
        Root = $Root; Out = $Out; RunId = (Split-Path -Leaf $RunDirectory); BuildProfile = $profileName
        Target = $Target; Python = $Python; Pwsh = $Pwsh; Offline = $offline; OfflineFlag = $OfflineFlag
        CargoTarget = $CargoTarget; WebSdkOutput = $WebSdkOutput
    }

    function Get-StepScript($Step) {
        $lines = New-Object Collections.Generic.List[string]
        $lines.Add('$ErrorActionPreference = ''Stop''')
        $lines.Add('$ProgressPreference = ''SilentlyContinue''')
        foreach ($name in $globals.Keys) { $lines.Add('$' + $name + ' = ' + (ConvertTo-Literal $globals[$name])) }
        foreach ($name in @('PluginSdk', 'WorkerSdk', 'WebSdk')) {
            $lines.Add('$' + $name + ' = ' + (ConvertTo-Literal (Get-Variable -Name $name -ValueOnly)))
        }
        foreach ($name in ($Step.Variables.Keys | Sort-Object)) { $lines.Add('$' + $name + ' = ' + (ConvertTo-Literal $Step.Variables[$name])) }
        $lines.Add('Set-Location -LiteralPath $Root')
        $lines.Add('try {')
        foreach ($line in (Format-Body $Step.Body.ToString())) { $lines.Add('    ' + $line) }
        $lines.Add('} catch {')
        $lines.Add('    [Console]::Out.WriteLine(''Step failed: '' + ($_ | Out-String).Trim())')
        $lines.Add('    exit 1')
        $lines.Add('}')
        $lines.Add('if ((Test-Path -LiteralPath variable:\LASTEXITCODE) -and $LASTEXITCODE -ne 0) { exit $LASTEXITCODE }')
        $lines.Add('exit 0')
        return ($lines -join "`n") + "`n"
    }

    # TMPDIR as well, as in CI: Python's tempfile and MSYS tools prefer it to TEMP.
    $environment = [ordered]@{
        TEMP = $temporaryRoot; TMP = $temporaryRoot; TMPDIR = $temporaryRoot; PYTHONUTF8 = '1'
        CARGO_REGISTRIES_MIDNIGHT_INDEX = $(if ($env:CARGO_REGISTRIES_MIDNIGHT_INDEX) { $env:CARGO_REGISTRIES_MIDNIGHT_INDEX } else { 'https://midnight-explorer.invalid/api-index' })
    }
    if ($offline) { $environment['CARGO_NET_OFFLINE'] = 'true' }

    if ($dryRun) {
        Write-Line "Validation plan (dry run) for $Root"
        Write-Line "Platform: windows ($Target); profile: $profileName; offline: $offline; strict: $strict"
        Write-Line ('Tools: cargo=' + $Cargo + '; rustc=' + $Rustc + '; python=' + $Python + '; step shell=' + $Shell)
        if ($missing.Count) { Write-Line ('Missing tools (a real run stops with exit 127): ' + ($missing -join '; ')) }
        Write-Line "Logs: $RunDirectory"
        Write-Line ('Environment: ' + (($environment.Keys | ForEach-Object { "$_=$($environment[$_])" }) -join '; '))
        Write-Line ('Private test installation: a new owner-only directory below ' + $env:USERPROFILE +
            ' (HOST_TEST_INSTALL_ROOT), removed afterwards')
        Write-Line 'Step variables:'
        foreach ($name in $globals.Keys) { Write-Line ('  $' + $name + ' = ' + (ConvertTo-Literal $globals[$name])) }
        Write-Line ('  $PluginSdk = ' + $(if ($PluginSdk) { ConvertTo-Literal $PluginSdk } else { '<the bundle that sdk-package reports>' }))
        Write-Line ('  $WorkerSdk = ' + $(if ($WorkerSdk) { ConvertTo-Literal $WorkerSdk } else { '<the bundle that sdk-worker-package reports>' }))
        Write-Line ('  $WebSdk = ' + $(if ($WebSdk) { ConvertTo-Literal $WebSdk } else { ConvertTo-Literal (Join-Path $WebSdkOutput 'config.toml') }))
        $number = 0
        foreach ($step in $steps) {
            $number++
            if ($selected -and -not $selected.Contains($step.Id)) { continue }
            $skip = Get-SkipReason $step $null
            $state = if ($skip) { 'skip: ' + $skip.Reason } else { 'run' }
            if (-not $skip -and $step.Needs.Count) { $state += ' if these pass: ' + ($step.Needs -join ', ') }
            Write-Line ''
            Write-Line ('{0,2}. {1} - {2} [CI job: {3}] ({4})' -f $number, $step.Id, $step.Title, $step.Job, $state)
            foreach ($name in ($step.Variables.Keys | Sort-Object)) { Write-Line ('      $' + $name + ' = ' + (ConvertTo-Literal $step.Variables[$name])) }
            if ($step.Local) {
                Write-Line '      # Runs in the validation process; its variables reach every later step.'
                $text = (Get-Command Install-TestPackages).ScriptBlock.ToString()
            } elseif ($step.Body) { $text = $step.Body.ToString() } else { $text = '' }
            foreach ($line in (Format-Body $text)) { Write-Line ('      ' + $line) }
        }
        exit 0
    }

    if ($missing.Count) {
        foreach ($item in $missing) { [Console]::Error.WriteLine("validate.ps1: missing $item") }
        exit 127
    }

    # --- Run ---
    foreach ($name in $environment.Keys) { [Environment]::SetEnvironmentVariable($name, $environment[$name], 'Process') }
    New-Item -ItemType Directory -Force $temporaryRoot, $RunDirectory | Out-Null
    try {
        $savedConsoleEncoding = [Console]::OutputEncoding
        [Console]::OutputEncoding = New-Object Text.UTF8Encoding $false
    } catch { $savedConsoleEncoding = $null }
    if (-not ('MidnightValidationProcess' -as [type])) {
        Add-Type -TypeDefinition @'
using System;
using System.Diagnostics;
using System.IO;
using System.Text;

public static class MidnightValidationProcess {
    // Streams the child's output to the console and the step log as it arrives.
    public static int Run(string fileName, string arguments, string workingDirectory, string logPath) {
        UTF8Encoding encoding = new UTF8Encoding(false);
        object gate = new object();
        using (StreamWriter log = new StreamWriter(logPath, false, encoding)) {
            log.AutoFlush = true;
            ProcessStartInfo start = new ProcessStartInfo(fileName, arguments);
            start.UseShellExecute = false;
            start.RedirectStandardOutput = true;
            start.RedirectStandardError = true;
            start.StandardOutputEncoding = encoding;
            start.StandardErrorEncoding = encoding;
            start.WorkingDirectory = workingDirectory;
            using (Process process = new Process()) {
                process.StartInfo = start;
                DataReceivedEventHandler write = delegate(object sender, DataReceivedEventArgs line) {
                    if (line.Data == null) { return; }
                    lock (gate) { Console.Out.WriteLine(line.Data); log.WriteLine(line.Data); }
                };
                process.OutputDataReceived += write;
                process.ErrorDataReceived += write;
                process.Start();
                process.BeginOutputReadLine();
                process.BeginErrorReadLine();
                process.WaitForExit();
                return process.ExitCode;
            }
        }
    }
}
'@
    }

    $results = @{}
    $started = Get-Date
    $number = 0
    $selectedCount = 0
    foreach ($step in $steps) { if (-not $selected -or $selected.Contains($step.Id)) { $selectedCount++ } }
    Write-Line "Validating $Root on windows ($Target); profile $profileName; logs in $RunDirectory"
    Write-Line ('Step shell: ' + $Shell)
    try {
        foreach ($step in $steps) {
            $number++
            if ($selected -and -not $selected.Contains($step.Id)) { continue }
            $log = Join-Path $RunDirectory ('{0:D2}-{1}.log' -f $number, $step.Id)
            $skip = Get-SkipReason $step $results
            if ($skip) {
                $results[$step.Id] = @{ Status = 'SKIPPED'; Kind = $skip.Kind; Reason = $skip.Reason; Duration = [TimeSpan]::Zero; Counts = $null }
                Write-Line ('== [{0}/{1}] {2}: SKIPPED ({3})' -f $number, $steps.Count, $step.Id, $skip.Reason)
                continue
            }
            Write-Line ''
            Write-Line ('== [{0}/{1}] {2}: {3} [CI job: {4}]' -f $number, $steps.Count, $step.Id, $step.Title, $step.Job)
            $clock = [Diagnostics.Stopwatch]::StartNew()
            $status = 'PASS'
            $reason = $null
            try {
                if ($step.Before) { & $step.Before }
                if ($step.Local) {
                    $output = @(& $step.Local | ForEach-Object { [string]$_ })
                    foreach ($line in $output) { Write-Line $line }
                    [IO.File]::WriteAllLines($log, [string[]]$output, (New-Object Text.UTF8Encoding $false))
                } else {
                    $script = Join-Path $RunDirectory ('{0:D2}-{1}.ps1' -f $number, $step.Id)
                    # A byte order mark lets Windows PowerShell 5.1 read the script as UTF-8.
                    [IO.File]::WriteAllText($script, (Get-StepScript $step), (New-Object Text.UTF8Encoding $true))
                    $arguments = '-NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "' + $script + '"'
                    $code = [MidnightValidationProcess]::Run($Shell, $arguments, $Root, $log)
                    if ($code -ne 0) { $status = 'FAIL'; $reason = "exit code $code" }
                }
                if ($status -eq 'PASS' -and $step.OnPass) { & $step.OnPass $log }
            } catch {
                $status = 'FAIL'
                $reason = $_.Exception.Message
                Write-Line ('Validation error: ' + $reason)
            }
            $clock.Stop()
            $counts = Get-TestCounts $log
            $results[$step.Id] = @{ Status = $status; Kind = $null; Reason = $reason; Duration = $clock.Elapsed; Counts = $counts }
            Write-Line ('-- {0}: {1} in {2}{3}' -f $step.Id, $status, (Format-Duration $clock.Elapsed), $(if ($reason) { " ($reason)" } else { '' }))
        }
    } finally {
        $removals = @()
        if ($TestRoot) { $removals += ,@($TestRoot, 'the private test installation') }
        $buildPrefix = (Join-Path $Root '_build').TrimEnd('\') + '\'
        foreach ($bundle in $PackagedBundles) {
            # Only bundles that this run's packagers reported, below this checkout's _build.
            if ([IO.Path]::GetFullPath($bundle).StartsWith($buildPrefix, [StringComparison]::OrdinalIgnoreCase)) {
                $removals += ,@($bundle, 'the SDK bundle packaged for this run')
            }
        }
        foreach ($removal in $removals) {
            if (-not (Test-Path -LiteralPath $removal[0])) { continue }
            if ((Get-Item -LiteralPath $removal[0] -Force).Attributes -band [IO.FileAttributes]::ReparsePoint) { continue }
            $removed = $false
            for ($attempt = 0; $attempt -lt 5 -and -not $removed; $attempt++) {
                try { Remove-Item -LiteralPath $removal[0] -Recurse -Force; $removed = $true }
                catch { Start-Sleep -Milliseconds 500 }
            }
            if ($removed) { Write-Line "Removed $($removal[1]) $($removal[0])" }
            else { [Console]::Error.WriteLine("validate.ps1: could not remove $($removal[1]) $($removal[0])") }
        }
    }

    # --- Summary ---
    $lines = New-Object Collections.Generic.List[string]
    $lines.Add('')
    $lines.Add("Validation summary: windows ($Target), profile $profileName, started $($started.ToString('yyyy-MM-dd HH:mm:ss'))")
    $lines.Add(('{0,-2} {1,-26} {2,-7} {3,8} {4,7} {5,6} {6,7}  {7}' -f '#', 'Step', 'Result', 'Time', 'Passed', 'Failed', 'Ignored', 'CI job / reason'))
    $tally = @{ PASS = 0; FAIL = 0; SKIPPED = 0 }
    $tests = @{ Passed = 0; Failed = 0; Ignored = 0 }
    $strictFailures = 0
    $ciSkips = New-Object Collections.Generic.List[string]
    $number = 0
    foreach ($step in $steps) {
        $number++
        $result = $results[$step.Id]
        if (-not $result) { continue }
        $tally[$result.Status]++
        if ($result.Status -eq 'SKIPPED' -and $result.Kind -notin @('platform', 'supplied')) {
            $strictFailures++
            if ($step.Job -ne 'not in CI') { $ciSkips.Add($step.Id) }
        }
        $passed = ''; $failed = ''; $ignored = ''
        if ($result.Counts -and $result.Counts.Found) {
            $passed = $result.Counts.Passed; $failed = $result.Counts.Failed; $ignored = $result.Counts.Ignored
            $tests.Passed += $passed; $tests.Failed += $failed; $tests.Ignored += $ignored
        }
        $time = if ($result.Status -eq 'SKIPPED') { '-' } else { Format-Duration $result.Duration }
        $note = if ($result.Reason) { $result.Reason } else { $step.Job }
        $lines.Add(('{0,2} {1,-26} {2,-7} {3,8} {4,7} {5,6} {6,7}  {7}' -f $number, $step.Id, $result.Status, $time, $passed, $failed, $ignored, $note))
    }
    $elapsed = (Get-Date) - $started
    $lines.Add(('Steps: {0} passed, {1} failed, {2} skipped in {3}. Tests: {4} passed, {5} failed, {6} ignored.' -f
        $tally.PASS, $tally.FAIL, $tally.SKIPPED, (Format-Duration $elapsed), $tests.Passed, $tests.Failed, $tests.Ignored))
    if ($selected) { $lines.Add("Only the selected steps and the steps they need ran (--only $($selection -join ','))." ) }
    # Without --strict such a run still passes; this line keeps the gap in CI coverage visible.
    if ($ciSkips.Count) {
        $lines.Add(('WARNING: {0} CI step(s) SKIPPED for a missing prerequisite or needed step: {1} (use --strict to fail)' -f
            $ciSkips.Count, ($ciSkips -join ', ')))
    }
    $exitCode = 0
    if ($tally.FAIL) { $exitCode = 1; $lines.Add('VALIDATION FAILED') }
    elseif ($strict -and $strictFailures) { $exitCode = 1; $lines.Add("VALIDATION FAILED: --strict and $strictFailures step(s) skipped") }
    else { $lines.Add('VALIDATION PASSED') }
    $lines.Add("Logs: $RunDirectory")
    foreach ($line in $lines) { Write-Line $line }
    [IO.File]::WriteAllLines((Join-Path $RunDirectory 'summary.txt'), [string[]]$lines, (New-Object Text.UTF8Encoding $false))
    exit $exitCode
} finally {
    if ($null -ne $savedConsoleEncoding) { try { [Console]::OutputEncoding = $savedConsoleEncoding } catch { } }
    Pop-Location
    Restore-Environment
}
