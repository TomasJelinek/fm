$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$deploy = Join-Path $projectRoot 'scripts/deploy-private.ps1'
$parent = if ($env:HOST_TEST_INSTALL_ROOT) { $env:HOST_TEST_INSTALL_ROOT } else { Join-Path $env:USERPROFILE '_build' }
New-Item -ItemType Directory -Path $parent -Force | Out-Null
$testRoot = Join-Path $parent ('runtime-registration-test-' + [guid]::NewGuid().ToString('N'))
$sid = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
function Protect-Fixture([string]$Path) {
    $acl = New-Object Security.AccessControl.DirectorySecurity
    $acl.SetSecurityDescriptorSddlForm("D:P(A;OICI;FA;;;$sid)(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)",
        [Security.AccessControl.AccessControlSections]::Access)
    Set-Acl -LiteralPath $Path -AclObject $acl
}
function Refuses([scriptblock]$Action, [string]$Reason) {
    $refused = $false
    try { & $Action | Out-Null } catch { $refused = $true }
    if (-not $refused) { throw "Expected refusal: $Reason" }
}
function Start-BriefHold([string]$Path, [string]$Trigger, [int]$Milliseconds) {
    # Opens $Path without delete sharing, as endpoint protection opens a file just written,
    # and closes it $Milliseconds after $Trigger appears; returns the releasing thread.
    if (-not ('MidnightRegistrationHoldFixture' -as [type])) {
        Add-Type -TypeDefinition @'
using System.IO;
using System.Threading;
public static class MidnightRegistrationHoldFixture {
    public static Thread Hold(string path, string trigger, int milliseconds) {
        FileStream stream = new FileStream(path, FileMode.Open, FileAccess.Read, FileShare.Read);
        Thread release = new Thread(delegate() {
            try {
                for (int waited = 0; !File.Exists(trigger) && waited < 120000; waited += 5) Thread.Sleep(5);
                Thread.Sleep(milliseconds);
            } finally { stream.Dispose(); }
        });
        release.IsBackground = true;
        release.Start();
        return release;
    }
}
'@
    }
    return [MidnightRegistrationHoldFixture]::Hold($Path, $Trigger, $Milliseconds)
}
function Assert-DeploymentReadLease {
    $source = Join-Path $testRoot 'held-source.exe'
    $replacement = Join-Path $testRoot 'held-replacement.exe'
    $renamed = Join-Path $testRoot 'held-renamed.exe'
    [IO.File]::WriteAllText($source, 'original lease fixture')
    [IO.File]::WriteAllText($replacement, 'replacement lease fixture')
    $lease = [MidnightRuntimeRegistrationV1.LaunchStamp]::OpenForDeployment($source)
    try {
        $perFile = [IO.File]::Open($source,[IO.FileMode]::Open,[IO.FileAccess]::Read,[IO.FileShare]::Read)
        $perFile.Dispose()
        Refuses {
            $writer = [IO.File]::Open($source,[IO.FileMode]::Open,[IO.FileAccess]::Write,[IO.FileShare]::Read)
            $writer.Dispose()
        } 'data writer while actual deployment lease is held'
        Refuses { [IO.File]::Delete($source) } 'delete while actual deployment lease is held'
        Refuses { [IO.File]::Move($source,$renamed) } 'rename while actual deployment lease is held'
        Refuses { [IO.File]::Replace($replacement,$source,[NullString]::Value) } 'replace while actual deployment lease is held'
        if ([IO.File]::ReadAllText($source) -cne 'original lease fixture' -or
            -not (Test-Path -LiteralPath $replacement)) { throw 'Refused access changed lease fixtures' }
    } finally { $lease.Dispose() }
    $writer = [IO.File]::Open($source,[IO.FileMode]::Open,[IO.FileAccess]::Write,[IO.FileShare]::Read)
    $writer.Dispose()
    [IO.File]::Move($source,$renamed)
    [IO.File]::Move($renamed,$source)
    [IO.File]::Replace($replacement,$source,[NullString]::Value)
    if ([IO.File]::ReadAllText($source) -cne 'replacement lease fixture') { throw 'Replacement after deployment lease release failed' }
    [IO.File]::Delete($source)
    if (Test-Path -LiteralPath $source) { throw 'Delete after deployment lease release failed' }
}
function Read-Utf16Field([string]$Text, [string]$Name) {
    $matches = [regex]::Matches($Text, '(?m)^' + [regex]::Escape($Name) + ' = \[([0-9]+(?:,[0-9]+)*)\]$')
    if ($matches.Count -ne 1) { throw "Invalid UTF-16 field: $Name" }
    return -join ($matches[0].Groups[1].Value.Split(',') | ForEach-Object { [char][uint16]$_ })
}
function Read-IntegerField([string]$Text, [string]$Name) {
    $matches = [regex]::Matches($Text, '(?m)^' + [regex]::Escape($Name) + ' = ([0-9]+)$')
    if ($matches.Count -ne 1) { throw "Invalid integer field: $Name" }
    return [int64]::Parse($matches[0].Groups[1].Value, [Globalization.CultureInfo]::InvariantCulture)
}
function Assert-NoTemporaryRecords([string]$Registry) {
    if (@(Get-ChildItem -LiteralPath $Registry -Recurse -Force -File |
        Where-Object { $_.Name.EndsWith('.tmp', [StringComparison]::Ordinal) }).Count) {
        throw 'Registration temporary file leaked'
    }
}
function Registry-Contents([string]$Registry) {
    return [string]::Join(';', @(Get-ChildItem -LiteralPath $Registry -Recurse -Force -File |
        Sort-Object FullName | ForEach-Object {
            $_.FullName.Substring($Registry.Length + 1) + '=' +
                [Convert]::ToBase64String([IO.File]::ReadAllBytes($_.FullName))
        }))
}
function Assert-LauncherPointer([string]$Registry, [string]$SourceExecutable, [string]$Hash) {
    $launchers = Join-Path $Registry 'launchers'
    if (-not (Test-Path -LiteralPath $launchers -PathType Container)) {
        throw 'Source-launcher pointer missing after registration'
    }
    $pointers = @(Get-ChildItem -LiteralPath $launchers -Filter '*.toml' -File)
    if ($pointers.Count -ne 1) { throw 'Expected one source-launcher pointer' }
    $stamp = [MidnightRuntimeRegistrationV1.LaunchStamp]::Open($SourceExecutable)
    try {
        if ($pointers[0].Name -cne ($stamp.Key + '.toml')) { throw 'Launcher key differs from native source path' }
        $text = [IO.File]::ReadAllText($pointers[0].FullName)
        if ((Read-IntegerField $text 'schema_version') -ne 1) { throw 'Pointer schema differs' }
        $actual = Read-Utf16Field $text 'launch_executable_utf16'
        if (-not [string]::Equals($actual, $stamp.CanonicalPath, [StringComparison]::Ordinal)) {
            throw 'Pointer does not retain the original source canonical UTF-16 path'
        }
        if ((Read-IntegerField $text 'launch_length') -ne $stamp.Length -or
            (Read-IntegerField $text 'launch_last_write_time_100ns') -ne $stamp.LastWriteTime100ns) {
            throw 'Pointer source length/last-write differs from metadata handle'
        }
        $expected = '(?m)^registration_sha256 = "' + [regex]::Escape($Hash) + '"$'
        if ($Hash -cnotmatch '^[0-9a-f]{64}$' -or $text -cnotmatch $expected) { throw 'Pointer registration digest differs' }
        if (-not (Get-Acl -LiteralPath $launchers).AreAccessRulesProtected) { throw 'Launcher directory ACL is inherited' }
        Assert-NoTemporaryRecords $Registry
        return [pscustomobject]@{ Path=$pointers[0].FullName; Key=$stamp.Key; Text=$text; CanonicalPath=$stamp.CanonicalPath }
    } finally { $stamp.Dispose() }
}
function Assert-KeyVectors {
    $vectors = @(
        @('C:\Build\midnight-explorer.exe', '06bf529c3d1adb10987aae5aa43980c7fc433bcaaf32ec9be96c451251261122'),
        @('\\?\C:\Build\midnight-explorer.exe', '419b90ed499c757debe70e3b11ad37eaad2adf0c145fc6535a4de3e5b2861f33'),
        @(('\\?\UNC\server\share\' + [char]0x76ee + [char]0x5f55 + '\midnight-explorer.exe'), 'a271a30dfbebe7c54a8b05cb2e2b4275d51895451ad9a478cea3708f274fa3be'),
        @(('\\?\C:\' + ('segment\' * 40) + 'midnight-explorer.exe'), 'b326d9854b671a38b0b240b6d7ee9f05d3a823f6d1e2812be455158588d0fd16'),
        @(('\\?\C:\raw-' + [char]0xd800 + '-tail-' + [char]0xdc00 + '\midnight-explorer.exe'), '3adfc4d47c610729bf2f1544e8dc991fab5629b41a3ab1ce696e0bb74409866e')
    )
    foreach ($vector in $vectors) {
        $actual = [MidnightRuntimeRegistrationV1.LaunchStamp]::KeyForUtf16Units($vector[0])
        if ($actual -cne $vector[1]) { throw 'Launcher UTF-16LE/domain codec differs from independent golden vector' }
    }
}
function Assert-NativeLongPath {
    if (-not ('MidnightRegistrationLongFixture' -as [type])) {
        Add-Type -TypeDefinition @'
using System;
using System.ComponentModel;
using System.Runtime.InteropServices;
using Microsoft.Win32.SafeHandles;
public static class MidnightRegistrationLongFixture {
    [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
    static extern bool CreateDirectoryW(string path, IntPtr security);
    [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
    static extern SafeFileHandle CreateFileW(string path,uint access,uint share,IntPtr security,uint disposition,uint flags,IntPtr template);
    [DllImport("kernel32.dll", SetLastError=true)]
    static extern bool WriteFile(SafeFileHandle file,byte[] data,uint length,out uint written,IntPtr overlapped);
    [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
    static extern bool DeleteFileW(string path);
    [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
    static extern bool RemoveDirectoryW(string path);
    public static void Directory(string path) {
        if(!CreateDirectoryW(path,IntPtr.Zero)) throw new Win32Exception(Marshal.GetLastWin32Error());
    }
    public static void File(string path) {
        using(SafeFileHandle file=CreateFileW(path,0x40000000,0,IntPtr.Zero,1,0,IntPtr.Zero)) {
            if(file.IsInvalid) throw new Win32Exception(Marshal.GetLastWin32Error());
            byte[] data=new byte[]{97,98,99};uint written;
            if(!WriteFile(file,data,3,out written,IntPtr.Zero)||written!=3)
                throw new Win32Exception(Marshal.GetLastWin32Error());
        }
    }
    public static void DeleteFile(string path) {
        if(!DeleteFileW(path)) throw new Win32Exception(Marshal.GetLastWin32Error());
    }
    public static void DeleteDirectory(string path) {
        if(!RemoveDirectoryW(path)) throw new Win32Exception(Marshal.GetLastWin32Error());
    }
}
'@
    }
    $prefix = '\\?\' + [IO.Path]::GetFullPath($testRoot).TrimEnd('\') + '\'
    $directories = New-Object 'System.Collections.Generic.List[string]'
    $cursor = $prefix + 'long-launcher'
    $file = $null
    try {
        [MidnightRegistrationLongFixture]::Directory($cursor)
        $directories.Add($cursor)
        for ($index=0; $index -lt 9; $index++) {
            $cursor += '\segment-0123456789-0123456789'
            [MidnightRegistrationLongFixture]::Directory($cursor)
            $directories.Add($cursor)
        }
        $candidate = $cursor + '\midnight-explorer.exe'
        [MidnightRegistrationLongFixture]::File($candidate)
        $file = $candidate
        $stamp = [MidnightRuntimeRegistrationV1.LaunchStamp]::Open($file)
        try {
            if ($stamp.CanonicalPath.Length -le 260 -or $stamp.Length -ne 3) { throw 'Native long-path metadata differs' }
            if ($stamp.Key -cne [MidnightRuntimeRegistrationV1.LaunchStamp]::KeyForUtf16Units($stamp.CanonicalPath)) {
                throw 'Native long-path key differs'
            }
        } finally { $stamp.Dispose() }
    } finally {
        if ($file) {
            if (-not $file.StartsWith($prefix,[StringComparison]::Ordinal)) { throw 'Unexpected long fixture file' }
            [MidnightRegistrationLongFixture]::DeleteFile($file)
        }
        for ($index=$directories.Count-1; $index -ge 0; $index--) {
            if (-not $directories[$index].StartsWith($prefix,[StringComparison]::Ordinal)) { throw 'Unexpected long fixture directory' }
            [MidnightRegistrationLongFixture]::DeleteDirectory($directories[$index])
        }
    }
}

New-Item -ItemType Directory -Path $testRoot | Out-Null
Protect-Fixture $testRoot
try {
    $sourceName = 'source-' + [char]0x539f + [char]0x59cb + ' source'
    $source = Join-Path $testRoot $sourceName
    New-Item -ItemType Directory -Path $source | Out-Null
    $sourceExecutable = Join-Path $source 'midnight-explorer.exe'
    [IO.File]::WriteAllText($sourceExecutable, 'registered executable fixture')
    [IO.File]::WriteAllText((Join-Path $source 'midnight-elevated-listing.exe'), 'authorization helper fixture')
    foreach ($name in @('basic', 'source', 'markdown')) {
        $package = Join-Path $source "viewers/midnight-viewer-$name"
        New-Item -ItemType Directory -Path $package -Force | Out-Null
        foreach ($file in @('plugin.toml', "midnight_viewer_$name.dll", 'LICENSE', 'THIRD-PARTY.md')) {
            [IO.File]::WriteAllText((Join-Path $package $file), "fixture $name $file")
        }
    }
    $registry = Join-Path $testRoot 'profile/plugin-runtimes'
    $destination = Join-Path $testRoot ('installed-' + [char]0x6587 + [char]0x6863)
    & $deploy -SourceDirectory $source -Destination $destination -RegisterRuntime -RegistrationDirectory $registry | Out-Null
    $hash = (Get-FileHash -LiteralPath $sourceExecutable -Algorithm SHA256).Hash.ToLowerInvariant()
    $recordPath = Join-Path $registry "$hash.toml"
    if (-not (Test-Path -LiteralPath $recordPath -PathType Leaf)) { throw 'Matching registration missing' }
    $record = [IO.File]::ReadAllText($recordPath)
    if ((Read-IntegerField $record 'schema_version') -ne 1 -or
        $record -cnotmatch ('(?m)^sha256 = "' + $hash + '"$')) { throw 'Registration schema/digest differs' }
    if ((Read-Utf16Field $record 'executable_utf16') -cne (Join-Path $destination 'midnight-explorer.exe')) {
        throw 'Native target path changed in legacy registration'
    }
    if (-not (Get-Acl -LiteralPath $registry).AreAccessRulesProtected) { throw 'Registry ACL is inherited' }
    $pointer = Assert-LauncherPointer $registry $sourceExecutable $hash
    Assert-DeploymentReadLease
    Assert-KeyVectors
    Assert-NativeLongPath
    $alias = [MidnightRuntimeRegistrationV1.LaunchStamp]::Open($sourceExecutable.ToUpperInvariant())
    try {
        if ($alias.Key -cne $pointer.Key -or
            -not [string]::Equals($alias.CanonicalPath,$pointer.CanonicalPath,[StringComparison]::Ordinal)) {
            throw 'Native case alias canonicalizes to a different source key'
        }
    } finally { $alias.Dispose() }
    $verbatim = [MidnightRuntimeRegistrationV1.LaunchStamp]::Open($pointer.CanonicalPath)
    try { if ($verbatim.Key -cne $pointer.Key) { throw 'DOS/verbatim aliases differ' } }
    finally { $verbatim.Dispose() }

    $beforeNoRegister = Registry-Contents $registry
    & $deploy -SourceDirectory $source -Destination "$testRoot/no-registration" | Out-Null
    if ((Registry-Contents $registry) -cne $beforeNoRegister) { throw 'No-register deployment changed registration' }
    Refuses { & $deploy -SourceDirectory $source -Destination "$testRoot/no-switch" -RegistrationDirectory $registry } 'registration directory without switch'

    $second = Join-Path $testRoot 'updated-runtime'
    & $deploy -SourceDirectory $source -Destination $second -RegisterRuntime -RegistrationDirectory $registry | Out-Null
    if ([IO.File]::ReadAllText($recordPath) -ceq $record) { throw 'Legacy registration did not update atomically' }
    $pointer = Assert-LauncherPointer $registry $sourceExecutable $hash
    if (@(Get-ChildItem -LiteralPath $registry -Force).Count -ne 2) { throw 'Unexpected registration root entries' }
    Remove-Item -LiteralPath $pointer.Path
    if (-not (Test-Path -LiteralPath $recordPath)) { throw 'Legacy-only migration fixture lost its hash record' }
    & $deploy -SourceDirectory $source -Destination "$testRoot/migrated-runtime" -RegisterRuntime -RegistrationDirectory $registry | Out-Null
    $pointer = Assert-LauncherPointer $registry $sourceExecutable $hash

    # A pointer replacement failure must leave the old pointer, after publishing the new legacy record.
    [IO.File]::WriteAllText($sourceExecutable, 'a new and longer registered executable fixture')
    $nextHash = (Get-FileHash -LiteralPath $sourceExecutable -Algorithm SHA256).Hash.ToLowerInvariant()
    $oldPointer = [IO.File]::ReadAllText($pointer.Path)
    $lock = [IO.File]::Open($pointer.Path,[IO.FileMode]::Open,[IO.FileAccess]::Read,[IO.FileShare]::Read)
    # The lock outlasts the publication's retry; a 100 ms budget keeps that wait short
    # (the deployment itself always uses the 2 s default).
    $PSDefaultParameterValues['Invoke-FreshRename:TimeoutMilliseconds'] = 100
    try {
        Refuses { & $deploy -SourceDirectory $source -Destination "$testRoot/pointer-locked" -RegisterRuntime -RegistrationDirectory $registry } 'locked pointer publication'
    } finally {
        $null = $PSDefaultParameterValues.Remove('Invoke-FreshRename:TimeoutMilliseconds')
        $lock.Dispose()
    }
    if ([IO.File]::ReadAllText($pointer.Path) -cne $oldPointer) { throw 'Failed publication changed old pointer' }
    if (-not (Test-Path -LiteralPath (Join-Path $registry "$nextHash.toml") -PathType Leaf)) {
        throw 'Pointer publication was attempted before the new legacy record'
    }
    Assert-NoTemporaryRecords $registry
    & $deploy -SourceDirectory $source -Destination "$testRoot/refreshed-runtime" -RegisterRuntime -RegistrationDirectory $registry | Out-Null
    $hash = $nextHash
    $pointer = Assert-LauncherPointer $registry $sourceExecutable $hash

    # A brief hold of the pointer while it is replaced, like endpoint protection's, is
    # waited out: the publication retries for about 2 s, which the lock above outlasts.
    [IO.File]::WriteAllText($sourceExecutable, 'a registered executable fixture published through a brief hold')
    $heldHash = (Get-FileHash -LiteralPath $sourceExecutable -Algorithm SHA256).Hash.ToLowerInvariant()
    $release = Start-BriefHold $pointer.Path (Join-Path $registry "$heldHash.toml") 300
    try { & $deploy -SourceDirectory $source -Destination "$testRoot/briefly-held" -RegisterRuntime -RegistrationDirectory $registry | Out-Null }
    finally { $release.Join() }
    $hash = $heldHash
    $pointer = Assert-LauncherPointer $registry $sourceExecutable $hash
    Assert-NoTemporaryRecords $registry

    $unsafe = Join-Path $testRoot 'shared-registry'
    New-Item -ItemType Directory -Path $unsafe | Out-Null
    $acl = Get-Acl -LiteralPath $unsafe
    $acl.SetSecurityDescriptorSddlForm("D:P(A;OICI;FA;;;$sid)(A;OICI;FA;;;WD)",
        [Security.AccessControl.AccessControlSections]::Access)
    Set-Acl -LiteralPath $unsafe -AclObject $acl
    $before = (Get-Acl -LiteralPath $unsafe).Sddl
    Refuses { & $deploy -SourceDirectory $source -Destination "$testRoot/refused" -RegisterRuntime -RegistrationDirectory $unsafe } 'shared registry'
    if ((Get-Acl -LiteralPath $unsafe).Sddl -cne $before) { throw 'Shared ACL was silently repaired' }
    if (Test-Path -LiteralPath "$testRoot/refused") { throw 'Registry refusal left a deployment' }

    foreach ($kind in @('unsafe-launchers','unsafe-pointer','launcher-junction','pointer-junction')) {
        $hostile = Join-Path $testRoot $kind
        $launchers = Join-Path $hostile 'launchers'
        New-Item -ItemType Directory -Path $hostile | Out-Null
        if ($kind -eq 'launcher-junction') {
            New-Item -ItemType Junction -Path $launchers -Target $registry | Out-Null
            $redirect = $launchers
        } else {
            New-Item -ItemType Directory -Path $launchers | Out-Null
            $recordPointer = Join-Path $launchers ($pointer.Key + '.toml')
            if ($kind -eq 'pointer-junction') {
                New-Item -ItemType Junction -Path $recordPointer -Target $registry | Out-Null
                $redirect = $recordPointer
            } else {
                $redirect = $null
                if ($kind -eq 'unsafe-pointer') { [IO.File]::WriteAllText($recordPointer,$pointer.Text) }
                $bad = if ($kind -eq 'unsafe-launchers') { $launchers } else { $recordPointer }
                $badAcl = Get-Acl -LiteralPath $bad
                $badAcl.SetSecurityDescriptorSddlForm("D:P(A;OICI;FA;;;$sid)(A;OICI;FA;;;WD)",
                    [Security.AccessControl.AccessControlSections]::Access)
                Set-Acl -LiteralPath $bad -AclObject $badAcl
                $badBefore = (Get-Acl -LiteralPath $bad).Sddl
            }
        }
        try {
            $refusedDestination = Join-Path $testRoot ($kind + '-refused')
            Refuses { & $deploy -SourceDirectory $source -Destination $refusedDestination -RegisterRuntime -RegistrationDirectory $hostile } $kind
            if (Test-Path -LiteralPath $refusedDestination) { throw 'Hostile pointer path left a deployment' }
            if (-not $redirect -and (Get-Acl -LiteralPath $bad).Sddl -cne $badBefore) {
                throw 'Hostile registration ACL was silently repaired'
            }
        } finally { if ($redirect) { [IO.Directory]::Delete($redirect) } }
    }

    Refuses { & $deploy -SourceDirectory ($source + '\..\' + $sourceName) -Destination "$testRoot/traversal-refused" -RegisterRuntime -RegistrationDirectory $registry } 'original launcher parent traversal'
    if (Test-Path -LiteralPath "$testRoot/traversal-refused") { throw 'Parent traversal left deployment' }

    $originalProfile = $env:USERPROFILE
    $originalLocalData = $env:LOCALAPPDATA
    try {
        $env:USERPROFILE = Join-Path $testRoot 'isolated-user-profile'
        $env:LOCALAPPDATA = $unsafe
        New-Item -ItemType Directory -Path $env:USERPROFILE | Out-Null
        & $deploy -SourceDirectory $source -Destination "$testRoot/default-registration" -RegisterRuntime | Out-Null
        $defaultRegistry = Join-Path $env:USERPROFILE '.midnight-explorer/plugin-runtimes'
        if (-not (Test-Path -LiteralPath (Join-Path $defaultRegistry "$hash.toml") -PathType Leaf)) {
            throw 'Default registration must use the protected profile namespace'
        }
        $null = Assert-LauncherPointer $defaultRegistry $sourceExecutable $hash
        if ((Get-Acl -LiteralPath $unsafe).Sddl -cne $before) { throw 'AppData ACL was changed' }
    } finally {
        $env:USERPROFILE = $originalProfile
        $env:LOCALAPPDATA = $originalLocalData
    }

    $redirect = Join-Path $testRoot 'registry-junction'
    New-Item -ItemType Junction -Path $redirect -Target $registry | Out-Null
    try {
        Refuses { & $deploy -SourceDirectory $source -Destination "$testRoot/redirected" -RegisterRuntime -RegistrationDirectory $redirect } 'registry junction'
    } finally { [IO.Directory]::Delete($redirect) }
    if (Test-Path -LiteralPath "$testRoot/redirected") { throw 'Redirected registry left deployment' }
    Write-Output 'Runtime registration checks passed: unchanged legacy schema, exact source pointer metadata, five UTF-16 codec vectors, native case/verbatim aliases and long path, atomic publication order, explicit migration, a brief publication hold waited out, no-register invariance, private ACL and hostile/reparse refusals.'
    Write-Output 'Not run: native UNC resolution (no controlled network share); the UNC codec vector is covered.'
} finally {
    $resolvedParent = [IO.Path]::GetFullPath($parent).TrimEnd('\')
    if ((Split-Path -Parent ([IO.Path]::GetFullPath($testRoot))) -cne $resolvedParent) { throw 'Unexpected fixture cleanup path' }
    Remove-Item -LiteralPath $testRoot -Recurse -Force
}
