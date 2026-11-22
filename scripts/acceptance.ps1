<#
Opt-in end-to-end acceptance suite on Windows (docs/developer/ACCEPTANCE.md), the
counterpart of scripts/acceptance.sh.

Usage: pwsh -File scripts\acceptance.ps1 [debug|release] [--dry-run] [--list]
           [--scale smoke|full] [--only NAME[,NAME...]] [--strict-review]
           [--keep-fixture] [--all-themes] [--stress] [--run-dir DIR]
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
Exit codes: 0 every scenario passed or warned, 1 a failure, 2 invalid arguments or run
folders on a volume that is not qualified for Copy and Move, 127 cargo is missing.
#>
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

$usage = @'
Usage: pwsh -File scripts\acceptance.ps1 [debug|release] [--dry-run] [--list]
           [--scale smoke|full] [--only NAME[,NAME...]] [--strict-review]
           [--keep-fixture] [--all-themes] [--stress] [--run-dir DIR]
           [--plugins DIR | --validation-packages] [--test-binary FILE] [--offline]
Tests always use the debug test profile; the profile names the build whose plugin
packages (<target>\<profile>\viewers, filesystems, thumbnailers) the viewer scenarios
stage. --plugins names another folder with that layout; --validation-packages takes the
packages scripts\validate.ps1 left in _build\validate. --test-binary runs an already
built test executable instead of building one. --run-dir names an absolute folder for
the run folders (default _build\acceptance). --stress runs the opt-in stress sizes
(MIDNIGHT_ACCEPTANCE_STRESS=1). The gated volumes' variables (MIDNIGHT_ACCEPTANCE_FAT_DIR,
_REMOVABLE_DIR, _NETWORK_DIR, _OTHER_EXT_DIR, _NO_TMPFILE_DIR, _WIN_SYMLINKS) name
absolute folders; the suite and its staged children get them as they are. A run stops
(exit 2) when the run folders' volume is not qualified for Copy and Move: NTFS with 8.3
short-name creation disabled (docs\developer\TEST_EXECUTION.md).
Results: _build\acceptance\<run>\index.html; exit 0 pass, 1 failure, 2 usage, 127 no cargo.
'@

# The gated volumes' folders (crates/midnight-acceptance/tests/acceptance/fixtures.rs,
# `GATED`): the scenarios that need such a volume report SKIP without theirs.
$gatedVariables = @('MIDNIGHT_ACCEPTANCE_FAT_DIR', 'MIDNIGHT_ACCEPTANCE_REMOVABLE_DIR',
    'MIDNIGHT_ACCEPTANCE_NETWORK_DIR', 'MIDNIGHT_ACCEPTANCE_OTHER_EXT_DIR',
    'MIDNIGHT_ACCEPTANCE_NO_TMPFILE_DIR', 'MIDNIGHT_ACCEPTANCE_WIN_SYMLINKS')

function Write-Line([string]$Text = '') { [Console]::Out.WriteLine($Text) }

# The SID of the restricted workers' AppContainer, MidnightExplorer.RestrictedWorker.v1, as
# the unpackaged application gets it, computed as scripts\deploy-private.ps1 and
# build-thumbnailers.ps1 compute it: S-1-15-2 and the first 28 bytes of the SHA-256 of the
# lowercase name in UTF-16LE as seven little-endian numbers. Never
# DeriveAppContainerSidFromAppContainerName: inside a packaged PowerShell (the Microsoft
# Store pwsh) it returns a child AppContainer of that package, which no worker runs as.
function Get-RestrictedWorkerSid {
    $name = 'MidnightExplorer.RestrictedWorker.v1'.ToLowerInvariant()
    $sha = [Security.Cryptography.SHA256]::Create()
    try { $hash = $sha.ComputeHash([Text.Encoding]::Unicode.GetBytes($name)) } finally { $sha.Dispose() }
    $parts = foreach ($index in 0..6) { [BitConverter]::ToUInt32($hash, 4 * $index) }
    [Security.Principal.SecurityIdentifier]::new('S-1-15-2-' + ($parts -join '-'))
}

function Stop-Usage([string]$Message) {
    if ($Message) { [Console]::Error.WriteLine("acceptance.ps1: $Message") }
    [Console]::Error.WriteLine($usage)
    exit 2
}

# Whether the volume of `Folder` (or of its nearest existing parent) is qualified for Copy
# and Move as the platform decides it (docs/developer/TEST_EXECUTION.md, "Windows Copy/Move
# qualification volume"): NTFS whose persistent volume state has 8.3 short-name creation
# disabled, read with FSCTL_QUERY_PERSISTENT_VOLUME_STATE on the volume's root folder (no
# elevation needed). Elsewhere the engine takes its opaque lane, where Copy and Move jobs
# wait for each other. The script tests stand in for the probe with
# MIDNIGHT_ACCEPTANCE_SCRIPT_TEST_VOLUME (qualified or unqualified); nothing else sets it.
function Get-CopyVolume([string]$Folder) {
    $existing = [IO.Path]::GetFullPath($Folder)
    while (-not (Test-Path -LiteralPath $existing -PathType Container)) {
        $parent = Split-Path -Parent $existing
        if (-not $parent -or $parent -eq $existing) { break }
        $existing = $parent
    }
    $volume = [ordered]@{ Root = $existing; Qualified = $false; Reason = '' }
    $forced = $env:MIDNIGHT_ACCEPTANCE_SCRIPT_TEST_VOLUME
    if ($forced -in @('qualified', 'unqualified')) {
        $volume.Qualified = $forced -eq 'qualified'
        $volume.Reason = "$forced by MIDNIGHT_ACCEPTANCE_SCRIPT_TEST_VOLUME"
        return [pscustomobject]$volume
    }
    if (-not ('MidnightAcceptance.CopyVolume' -as [type])) {
        Add-Type -TypeDefinition @'
using System;
using System.ComponentModel;
using System.Runtime.InteropServices;
using System.Text;
using Microsoft.Win32.SafeHandles;

namespace MidnightAcceptance {
    public static class CopyVolume {
        [StructLayout(LayoutKind.Sequential)]
        struct PersistentVolumeInformation {
            public uint VolumeFlags;
            public uint FlagMask;
            public uint Version;
            public uint Reserved;
        }

        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        static extern bool GetVolumePathNameW(string fileName, StringBuilder volumePath, uint length);

        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        static extern bool GetVolumeInformationW(string root, StringBuilder name, uint nameLength,
            IntPtr serial, IntPtr maximumComponent, IntPtr flags, StringBuilder fileSystem,
            uint fileSystemLength);

        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        static extern SafeFileHandle CreateFileW(string name, uint access, uint share, IntPtr security,
            uint disposition, uint flags, IntPtr template);

        [DllImport("kernel32.dll", SetLastError = true)]
        static extern bool DeviceIoControl(SafeFileHandle device, uint code,
            ref PersistentVolumeInformation input, uint inputLength,
            out PersistentVolumeInformation output, uint outputLength, out uint returned,
            IntPtr overlapped);

        // The root (mount point) of the volume an existing folder is on.
        public static string Root(string folder) {
            var root = new StringBuilder(1024);
            if (!GetVolumePathNameW(folder, root, (uint)root.Capacity)) {
                throw new Win32Exception(Marshal.GetLastWin32Error());
            }
            return root.ToString();
        }

        public static string FileSystem(string root) {
            var fileSystem = new StringBuilder(64);
            if (!GetVolumeInformationW(root, null, 0, IntPtr.Zero, IntPtr.Zero, IntPtr.Zero,
                    fileSystem, (uint)fileSystem.Capacity)) {
                throw new Win32Exception(Marshal.GetLastWin32Error());
            }
            return fileSystem.ToString();
        }

        // PERSISTENT_VOLUME_STATE_SHORT_NAME_CREATION_DISABLED in the volume's persistent
        // state, read on its root folder (FILE_READ_ATTRIBUTES, every sharing mode,
        // OPEN_EXISTING, FILE_FLAG_BACKUP_SEMANTICS) with FSCTL_QUERY_PERSISTENT_VOLUME_STATE.
        public static bool ShortNamesDisabled(string root) {
            using (var handle = CreateFileW(root, 0x80, 7, IntPtr.Zero, 3, 0x02000000, IntPtr.Zero)) {
                if (handle.IsInvalid) {
                    throw new Win32Exception(Marshal.GetLastWin32Error());
                }
                var request = new PersistentVolumeInformation { FlagMask = 1, Version = 1 };
                PersistentVolumeInformation response;
                uint returned;
                if (!DeviceIoControl(handle, 0x9023C, ref request, 16, out response, 16, out returned,
                        IntPtr.Zero)) {
                    throw new Win32Exception(Marshal.GetLastWin32Error());
                }
                return returned >= 16 && (response.VolumeFlags & 1) != 0;
            }
        }
    }
}
'@
    }
    try {
        $volume.Root = [MidnightAcceptance.CopyVolume]::Root($existing)
        $fileSystem = [MidnightAcceptance.CopyVolume]::FileSystem($volume.Root)
        if ($fileSystem -ne 'NTFS') {
            $volume.Reason = "$fileSystem, not NTFS"
        } elseif ([MidnightAcceptance.CopyVolume]::ShortNamesDisabled($volume.Root)) {
            $volume.Qualified = $true
            $volume.Reason = 'NTFS, 8.3 short-name creation disabled'
        } else {
            $volume.Reason = 'NTFS whose volume state does not disable 8.3 short names'
        }
    } catch {
        $volume.Reason = "its 8.3 short-name state cannot be read: $($_.Exception.Message)"
    }
    [pscustomobject]$volume
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
$stress = $false
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
        '--stress' { $stress = $true; $runner.Add('--stress') }
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
# A gated folder that is set must be absolute, as the suite requires.
$gated = [ordered]@{}
foreach ($variable in $gatedVariables) {
    $value = [Environment]::GetEnvironmentVariable($variable)
    if (-not $value) { continue }
    if (-not [IO.Path]::IsPathFullyQualified($value)) { Stop-Usage "$variable $value is not an absolute folder" }
    $gated[$variable] = $value
}

# Everything below changes the session's working directory and these variables. Run
# in-process, the script would otherwise leave them changed, MIDNIGHT_ACCEPTANCE_PLUGINS
# pointing at the removed installation's packages, so that the next run in that session
# would stage none. The `finally` at the end restores them on every exit.
$restoredVariables = @('TEMP', 'TMP', 'CARGO_HOME', 'RUSTUP_HOME', 'MIDNIGHT_ACCEPTANCE_PLUGINS',
    'MIDNIGHT_ACCEPTANCE_STAGE_ROOT', 'MIDNIGHT_ACCEPTANCE_RUN_DIR', 'MIDNIGHT_ACCEPTANCE_STRESS')
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

    # Copy and Move run in the run folder. On a volume that is not qualified for them
    # (Get-CopyVolume) their jobs wait for each other, so the scenarios that run jobs side
    # by side would fail for no fault of the product: such a run is refused (--list runs
    # no scenario).
    $runFolders = if ($runDirectory) { $runDirectory }
        elseif ($env:MIDNIGHT_ACCEPTANCE_RUN_DIR) { $env:MIDNIGHT_ACCEPTANCE_RUN_DIR }
        else { Join-Path $root '_build\acceptance' }
    $copyVolume = if ($dryRun -or -not $listOnly) { Get-CopyVolume $runFolders }

    if ($dryRun) {
        Write-Line "Working directory: $root"
        Write-Line "Build profile: $profileName (the suite uses the debug test profile)"
        if ($testBinary) { Write-Line "Test executable: $testBinary" }
        else { Write-Line ('Build: [cargo] ' + (($cargoArguments | ForEach-Object { "[$_]" }) -join ' ')) }
        foreach ($source in $packageSources) { Write-Line ("Plugin packages: {0} <- {1}" -f $source[0], $source[1]) }
        Write-Line "Private installation: $installation (owner, SYSTEM and Administrators only; removed afterwards)"
        if ($runDirectory) { Write-Line "Run folders: $runDirectory" }
        $verdict = if ($copyVolume.Qualified) { '' } else { '; a run there stops' }
        Write-Line "Run folder volume: $($copyVolume.Root) ($($copyVolume.Reason))$verdict"
        if ($stress) { Write-Line 'Stress sizes: on (MIDNIGHT_ACCEPTANCE_STRESS=1)' }
        foreach ($variable in $gated.Keys) { Write-Line "Gated folder: $variable = $($gated[$variable])" }
        Write-Line ('Command: [<installation>\bin\acceptance.exe] ' + (($runner | ForEach-Object { "[$_]" }) -join ' '))
        exit 0
    }
    if (-not $listOnly -and -not $copyVolume.Qualified) {
        $refusal = ("acceptance.ps1: the run folders {0} are on {1} ({2}). Copy and Move jobs " +
            "there take the engine's opaque lane and wait for each other, so the scenarios " +
            "that run jobs side by side would fail for no fault of the product. Run on an NTFS " +
            "volume whose 8.3 short-name creation is disabled, with --run-dir " +
            "(docs/developer/TEST_EXECUTION.md, `"Windows Copy/Move qualification volume`").") -f
            $runFolders, $copyVolume.Root, $copyVolume.Reason
        [Console]::Error.WriteLine($refusal)
        exit 2
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
        }
        # A kind's folder exists only when a package of it is staged, as in a build folder:
        # the staged scenarios report SKIP without viewers\ and filesystems\, as on Linux.
        foreach ($source in $packageSources) {
            $kind = $source[0]
            $from = $source[1]
            if (-not (Test-Path -LiteralPath $from -PathType Container)) { continue }
            # Viewers and filesystems are one folder per package. Thumbnail providers are
            # installed flat, as the thumbnail host discovers them: each provider's manifest
            # (<name>.json) and executable, the FFmpeg libraries they share and notices\, so
            # every entry of that folder is copied and each manifest names a provider.
            $entries = if ($kind -eq 'thumbnailers') { Get-ChildItem -LiteralPath $from }
                else { Get-ChildItem -LiteralPath $from -Directory }
            foreach ($package in $entries) {
                # Network providers are never staged (they would reach real network shares);
                # a package already taken from an earlier source is kept.
                if ($kind -eq 'filesystems' -and $package.Name.StartsWith('network.')) { continue }
                $folder = Join-Path $packages $kind
                $to = Join-Path $folder $package.Name
                if (Test-Path -LiteralPath $to) { continue }
                if (-not (Test-Path -LiteralPath $folder)) { New-Item -ItemType Directory -Path $folder | Out-Null }
                Copy-Item -LiteralPath $package.FullName -Destination $to -Recurse
                if ($kind -ne 'thumbnailers') { $installed[$kind].Add($package.Name) }
                elseif (-not $package.PSIsContainer -and $package.Extension -eq '.json') {
                    $installed[$kind].Add([IO.Path]::GetFileNameWithoutExtension($package.Name))
                }
            }
        }
        foreach ($kind in @('viewers', 'filesystems', 'thumbnailers')) {
            $text = if ($installed[$kind].Count -gt 0) { $installed[$kind] -join ', ' } else { 'none' }
            Write-Line "Plugin packages ($kind): $text"
        }
        # The video worker's FFmpeg libraries and playback helpers load in the restricted
        # workers' AppContainer only when their ACL grants it, which a copy does not keep:
        # grant exactly what deploy-private.ps1 grants, read and execute for that SID on these
        # files only (no inheritance, no folder, no write). Stages hard-link these files.
        $thumbnailers = Join-Path $packages 'thumbnailers'
        if (Test-Path -LiteralPath (Join-Path $thumbnailers 'midnight-thumbnail-video.json') -PathType Leaf) {
            $workerSid = Get-RestrictedWorkerSid
            foreach ($name in @('avcodec-63.dll', 'avformat-63.dll', 'avutil-61.dll', 'swscale-10.dll',
                    'swresample-7.dll', 'midnight-video-playback.exe', 'midnight-video-playback-system.exe')) {
                $path = Join-Path $thumbnailers $name
                if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { continue }
                $acl = Get-Acl -LiteralPath $path
                $acl.AddAccessRule([Security.AccessControl.FileSystemAccessRule]::new($workerSid,
                    [Security.AccessControl.FileSystemRights]::ReadAndExecute,
                    [Security.AccessControl.AccessControlType]::Allow))
                Set-Acl -LiteralPath $path -AclObject $acl
            }
        }
        $env:MIDNIGHT_ACCEPTANCE_PLUGINS = $packages
        $env:MIDNIGHT_ACCEPTANCE_STAGE_ROOT = $installation
        if ($runDirectory) { $env:MIDNIGHT_ACCEPTANCE_RUN_DIR = $runDirectory }
        # Staged children read the stress sizes from the environment, as they inherit the
        # gated volumes' variables.
        if ($stress) { $env:MIDNIGHT_ACCEPTANCE_STRESS = '1' }
        foreach ($variable in $gated.Keys) { Write-Line "Gated folder: $variable = $($gated[$variable])" }
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
