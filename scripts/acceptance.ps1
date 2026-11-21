<#
Opt-in end-to-end acceptance suite on Windows (docs/developer/ACCEPTANCE.md), the
counterpart of scripts/acceptance.sh.

Usage: pwsh -File scripts\acceptance.ps1 [debug|release] [--dry-run] [--list]
           [--scale smoke|full] [--only NAME[,NAME...]] [--strict-review]
           [--keep-fixture] [--all-themes] [--run-dir DIR]
           [--plugins DIR | --validation-packages] [--test-binary FILE] [--offline]

Builds the `acceptance` test target of crates/midnight-acceptance (it needs the
`acceptance` feature: normal tests, CI and validation never run it) and runs it from a
private installation: a new folder below the user profile that only this account,
SYSTEM and Administrators can change, because the plugin host loads a plugin only when
no other account can change it or any folder above it. The folder holds a copy of the
test executable (a rebuild cannot replace the running one; staged scenarios start it
again) and the plugin packages the viewer scenarios stage; it is removed afterwards.
Run in-process instead (`& scripts\acceptance.ps1`), it restores the session's working
directory and the environment variables it sets when it ends, however it ends.
Exit codes: 0 every scenario passed or warned, 1 a failure, 2 invalid arguments,
127 cargo is missing.
#>
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

$usage = @'
Usage: pwsh -File scripts\acceptance.ps1 [debug|release] [--dry-run] [--list]
           [--scale smoke|full] [--only NAME[,NAME...]] [--strict-review]
           [--keep-fixture] [--all-themes] [--run-dir DIR]
           [--plugins DIR | --validation-packages] [--test-binary FILE] [--offline]
Tests always use the debug test profile; the profile names the build whose plugin
packages (<target>\<profile>\viewers, filesystems, thumbnailers) the viewer scenarios
stage. --plugins names another folder with that layout; --validation-packages takes the
packages scripts\validate.ps1 left in _build\validate. --test-binary runs an already
built test executable instead of building one. --run-dir names an absolute folder for
the run folders (default _build\acceptance).
Results: _build\acceptance\<run>\index.html; exit 0 pass, 1 failure, 2 usage, 127 no cargo.
'@

function Write-Line([string]$Text = '') { [Console]::Out.WriteLine($Text) }

function Stop-Usage([string]$Message) {
    if ($Message) { [Console]::Error.WriteLine("acceptance.ps1: $Message") }
    [Console]::Error.WriteLine($usage)
    exit 2
}

# GNU-style options, the same as scripts/acceptance.sh.
$profileName = $null
$dryRun = $false
$offline = $false
$validationPackages = $false
$plugins = $null
$testBinary = $null
$runDirectory = $null
$runner = New-Object Collections.Generic.List[string]
$listOnly = $false
$valueOptions = @('--scale', '--only', '--run-dir', '--plugins', '--test-binary')
for ($index = 0; $index -lt $args.Count; $index++) {
    $argument = [string]$args[$index]
    $name = $argument
    $value = $null
    if ($argument -match '^(--[a-z-]+)=(.*)$') { $name = $Matches[1]; $value = $Matches[2] }
    if ($valueOptions -contains $name -and $null -eq $value) {
        if ($index + 1 -ge $args.Count) { Stop-Usage "$name needs a value" }
        $index++
        $value = (@($args[$index]) | ForEach-Object { [string]$_ }) -join ','
    }
    switch -CaseSensitive ($name) {
        { $_ -in @('debug', 'release') } {
            if ($profileName) { Stop-Usage 'only one build profile' }
            $profileName = $_
        }
        '--dry-run' { $dryRun = $true }
        '--offline' { $offline = $true }
        '--validation-packages' { $validationPackages = $true }
        '--list' { $listOnly = $true; $runner.Add('--list') }
        { $_ -in @('--strict-review', '--keep-fixture', '--all-themes') } { $runner.Add($_) }
        '--scale' {
            if ($value -notin @('smoke', 'full')) { Stop-Usage "unknown scale $value" }
            $runner.Add('--scale'); $runner.Add($value)
        }
        '--only' {
            foreach ($filter in $value.Split(',')) {
                if (-not $filter -or $filter.StartsWith('-')) { Stop-Usage "invalid scenario name '$filter'" }
                $runner.Add($filter)
            }
        }
        '--run-dir' {
            if (-not [IO.Path]::IsPathRooted($value)) { Stop-Usage "--run-dir $value is not absolute" }
            $runDirectory = $value
        }
        '--plugins' { $plugins = $value }
        '--test-binary' { $testBinary = $value }
        { $_ -in @('--help', '-h', '-?') } { Write-Line $usage; exit 0 }
        default { Stop-Usage "unknown argument $argument" }
    }
}
if (-not $profileName) { $profileName = 'debug' }
if ($plugins -and $validationPackages) { Stop-Usage '--plugins and --validation-packages exclude each other' }

# Everything below changes the session's working directory and these variables. Run
# in-process, the script would otherwise leave them changed, MIDNIGHT_ACCEPTANCE_PLUGINS
# pointing at the removed installation's packages, so that the next run in that session
# would stage none. The `finally` at the end restores them on every exit.
$restoredVariables = @('TEMP', 'TMP', 'CARGO_HOME', 'RUSTUP_HOME', 'MIDNIGHT_ACCEPTANCE_PLUGINS',
    'MIDNIGHT_ACCEPTANCE_STAGE_ROOT', 'MIDNIGHT_ACCEPTANCE_RUN_DIR')
$savedVariables = @{}
foreach ($variable in $restoredVariables) {
    $savedVariables[$variable] = [Environment]::GetEnvironmentVariable($variable)
}
$root = Split-Path -Parent $PSScriptRoot
Push-Location -LiteralPath $root
try {
    $targetRoot = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { '_build' }
    if (-not [IO.Path]::IsPathRooted($targetRoot)) { $targetRoot = Join-Path $root $targetRoot }
    $targetRoot = [IO.Path]::GetFullPath($targetRoot)

    # Package folders, each copied below the private installation's packages\ folder.
    $validation = Join-Path $root '_build\validate'
    $packageSources = New-Object Collections.Generic.List[object]
    if ($validationPackages) {
        $packageSources.Add(@('viewers', (Join-Path $validation 'viewer-packages')))
        $packageSources.Add(@('filesystems', (Join-Path $validation 'filesystem-packages')))
        $packageSources.Add(@('filesystems', (Join-Path $validation 'archive-ci\package')))
    } else {
        if (-not $plugins) {
            $plugins = if ($env:MIDNIGHT_ACCEPTANCE_PLUGINS) { $env:MIDNIGHT_ACCEPTANCE_PLUGINS } else { Join-Path $targetRoot $profileName }
        }
        if (-not [IO.Path]::IsPathRooted($plugins)) { $plugins = Join-Path $root $plugins }
        foreach ($kind in @('viewers', 'filesystems', 'thumbnailers')) {
            $packageSources.Add(@($kind, (Join-Path $plugins $kind)))
        }
    }

    $cargoArguments = @('test', '--workspace', '--locked')
    if ($offline) { $cargoArguments += '--offline' }
    $cargoArguments += @('--features', 'midnight-acceptance/acceptance', '--test', 'acceptance', '--no-run',
        '--message-format', 'json-render-diagnostics')
    $installation = Join-Path $env:USERPROFILE ('MidnightAcceptance-' + [guid]::NewGuid().ToString('N'))

    if ($dryRun) {
        Write-Line "Working directory: $root"
        Write-Line "Build profile: $profileName (the suite uses the debug test profile)"
        if ($testBinary) { Write-Line "Test executable: $testBinary" }
        else { Write-Line ('Build: [cargo] ' + (($cargoArguments | ForEach-Object { "[$_]" }) -join ' ')) }
        foreach ($source in $packageSources) { Write-Line ("Plugin packages: {0} <- {1}" -f $source[0], $source[1]) }
        Write-Line "Private installation: $installation (owner, SYSTEM and Administrators only; removed afterwards)"
        if ($runDirectory) { Write-Line "Run folders: $runDirectory" }
        Write-Line ('Command: [<installation>\bin\acceptance.exe] ' + (($runner | ForEach-Object { "[$_]" }) -join ' '))
        exit 0
    }

    $temporary = Join-Path $root '_build\tmp'
    New-Item -ItemType Directory -Force -Path $temporary | Out-Null
    $env:TEMP = $temporary
    $env:TMP = $temporary

    if (-not $testBinary) {
        $localCargo = Join-Path $root '.tools\cargo\bin\cargo.exe'
        $cargo = $null
        if (Get-Command cargo -CommandType Application -ErrorAction Ignore) { $cargo = 'cargo' }
        elseif (Test-Path -LiteralPath $localCargo) {
            $env:CARGO_HOME = Join-Path $root '.tools\cargo'
            if (-not $env:RUSTUP_HOME) { $env:RUSTUP_HOME = Join-Path $root '.tools\rustup' }
            $cargo = $localCargo
        }
        if (-not $cargo) {
            [Console]::Error.WriteLine('cargo is not on PATH; run build.bat first.')
            exit 127
        }
        Write-Line 'Building the acceptance suite (debug test profile)...'
        $messages = & $cargo @cargoArguments
        if ($LASTEXITCODE -ne 0) {
            [Console]::Error.WriteLine("cargo exited with $LASTEXITCODE")
            exit 1
        }
        foreach ($line in $messages) {
            if (-not $line.StartsWith('{')) { continue }
            $message = $line | ConvertFrom-Json
            if ($message.reason -eq 'compiler-artifact' -and $message.target.name -eq 'acceptance' -and
                $message.executable) { $testBinary = $message.executable }
        }
        if (-not $testBinary) {
            [Console]::Error.WriteLine('cargo built no acceptance test executable')
            exit 1
        }
    }
    if (-not (Test-Path -LiteralPath $testBinary -PathType Leaf)) {
        [Console]::Error.WriteLine("no test executable at $testBinary")
        exit 1
    }

    # --list needs neither packages nor a private installation.
    if ($listOnly) {
        & $testBinary @runner
        exit $LASTEXITCODE
    }

    $exitCode = 1
    New-Item -ItemType Directory -Path $installation | Out-Null
    try {
        # Owner, SYSTEM and Administrators only, not inherited: the plugin host's trust check
        # (as scripts\validate.ps1 and CI's private test installation do).
        $sid = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
        $acl = New-Object Security.AccessControl.DirectorySecurity
        $acl.SetSecurityDescriptorSddlForm("D:P(A;OICI;FA;;;$sid)(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)",
            [Security.AccessControl.AccessControlSections]::Access)
        Set-Acl -LiteralPath $installation -AclObject $acl
        $bin = Join-Path $installation 'bin'
        New-Item -ItemType Directory -Path $bin | Out-Null
        $executable = Join-Path $bin (Split-Path -Leaf $testBinary)
        Copy-Item -LiteralPath $testBinary -Destination $executable
        $packages = Join-Path $installation 'packages'
        New-Item -ItemType Directory -Path $packages | Out-Null
        $installed = @{}
        foreach ($kind in @('viewers', 'filesystems', 'thumbnailers')) {
            $installed[$kind] = New-Object Collections.Generic.List[string]
            New-Item -ItemType Directory -Path (Join-Path $packages $kind) | Out-Null
        }
        foreach ($source in $packageSources) {
            $kind = $source[0]
            $from = $source[1]
            if (-not (Test-Path -LiteralPath $from -PathType Container)) { continue }
            foreach ($package in Get-ChildItem -LiteralPath $from -Directory) {
                # Network providers are never staged (they would reach real network shares);
                # a package already taken from an earlier source is kept.
                if ($kind -eq 'filesystems' -and $package.Name.StartsWith('network.')) { continue }
                $to = Join-Path (Join-Path $packages $kind) $package.Name
                if (Test-Path -LiteralPath $to) { continue }
                Copy-Item -LiteralPath $package.FullName -Destination $to -Recurse
                $installed[$kind].Add($package.Name)
            }
        }
        foreach ($kind in @('viewers', 'filesystems', 'thumbnailers')) {
            $text = if ($installed[$kind].Count -gt 0) { $installed[$kind] -join ', ' } else { 'none' }
            Write-Line "Plugin packages ($kind): $text"
        }
        $env:MIDNIGHT_ACCEPTANCE_PLUGINS = $packages
        $env:MIDNIGHT_ACCEPTANCE_STAGE_ROOT = $installation
        if ($runDirectory) { $env:MIDNIGHT_ACCEPTANCE_RUN_DIR = $runDirectory }
        Write-Line "Private installation: $installation"
        & $executable @runner
        $exitCode = $LASTEXITCODE
    } finally {
        Set-Location -LiteralPath $root
        # A staged child that is still exiting can hold a file for a moment.
        for ($attempt = 0; $attempt -lt 10 -and (Test-Path -LiteralPath $installation); $attempt++) {
            Remove-Item -LiteralPath $installation -Recurse -Force -ErrorAction SilentlyContinue
            if (Test-Path -LiteralPath $installation) { Start-Sleep -Milliseconds 500 }
        }
        if (Test-Path -LiteralPath $installation) {
            [Console]::Error.WriteLine("Could not remove the private installation $installation")
        }
    }
    exit $exitCode
} finally {
    Pop-Location
    foreach ($variable in $restoredVariables) {
        # PowerShell passes $null to a .NET string parameter as '', which would leave an
        # empty variable behind: a variable that was not set is removed instead.
        $value = $savedVariables[$variable]
        if ($null -eq $value) { Remove-Item -LiteralPath "Env:$variable" -ErrorAction Ignore }
        else { [Environment]::SetEnvironmentVariable($variable, $value) }
    }
}
