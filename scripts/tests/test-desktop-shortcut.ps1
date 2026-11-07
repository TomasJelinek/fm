param([switch]$ReadRealCloudDesktop)
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
. (Join-Path $projectRoot 'scripts/windows-shortcut.ps1')
if (-not ('ShortcutReparseFixture' -as [type])) {
    Add-Type -Path (Join-Path $PSScriptRoot 'windows-shortcut-fixtures.cs')
}
$testParent = if ($env:HOST_TEST_INSTALL_ROOT) { $env:HOST_TEST_INSTALL_ROOT } else {
    Join-Path $env:USERPROFILE '_build'
}
New-Item -ItemType Directory -Path $testParent -Force | Out-Null
$testRoot = Join-Path $testParent ('desktop-shortcut-test-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $testRoot | Out-Null
$sid = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
$acl = New-Object Security.AccessControl.DirectorySecurity
$acl.SetSecurityDescriptorSddlForm("D:P(A;OICI;FA;;;$sid)(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)",
    [Security.AccessControl.AccessControlSections]::Access)
Set-Acl -LiteralPath $testRoot -AclObject $acl
function Refuses([scriptblock]$Action, [string]$Reason) {
    $refused = $false
    try { & $Action | Out-Null } catch { $refused = $true }
    if (-not $refused) { throw "Expected refusal: $Reason" }
}
function Read-Shortcut([string]$Path) {
    return [MidnightShortcut.Native]::ReadShortcut($Path)
}
try {
    # Build native Unicode fixtures without depending on the PS5.1 source code page.
    $installation = Join-Path $testRoot ('installed-' + [char]0x6587 + [char]0x010d)
    $desktop = Join-Path $testRoot ('desktop-' + [char]0x010d + [char]0x6587)
    $outside = Join-Path $testRoot 'outside'
    New-Item -ItemType Directory -Path $installation, $desktop, $outside | Out-Null
    $executable = Join-Path $installation 'me.exe'
    [IO.File]::WriteAllText($executable, 'owned native shortcut fixture')
    $hash = (Get-FileHash -LiteralPath $executable -Algorithm SHA256).Hash
    $linkPath = Join-Path $desktop 'Midnight Explorer.lnk'
    Install-MidnightDesktopShortcut $installation $desktop $hash | Out-Null
    $values = Read-Shortcut $linkPath
    if ($values[0] -cne $executable -or $values[1] -cne ($executable + ',0') -or $values[3]) {
        throw 'Created shortcut values differ'
    }
    [IO.File]::WriteAllText($linkPath, 'old shortcut contents')
    Install-MidnightDesktopShortcut $installation $desktop $hash | Out-Null
    if ((Read-Shortcut $linkPath)[0] -cne $executable) { throw 'Existing shortcut was not replaced' }
    $before = (Get-FileHash -LiteralPath $linkPath).Hash
    Refuses { Install-MidnightDesktopShortcut $installation $desktop ('0' * 64) } 'wrong runtime hash'
    if ((Get-FileHash -LiteralPath $linkPath).Hash -cne $before) { throw 'Hash refusal modified shortcut' }
    $runtimeJunction = Join-Path $testRoot 'runtime-junction'
    New-Item -ItemType Junction -Path $runtimeJunction -Target $installation | Out-Null
    try { Refuses { Install-MidnightDesktopShortcut $runtimeJunction $desktop $hash } 'runtime junction' }
    finally { [IO.Directory]::Delete($runtimeJunction) }
    $originalRuntimeAcl = Get-Acl -LiteralPath $installation
    $unsafeRuntimeAcl = Get-Acl -LiteralPath $installation
    $unsafeRuntimeAcl.AddAccessRule([Security.AccessControl.FileSystemAccessRule]::new(
        [Security.Principal.SecurityIdentifier]::new('S-1-1-0'),
        [Security.AccessControl.FileSystemRights]::Write,
        [Security.AccessControl.AccessControlType]::Allow))
    try {
        Set-Acl -LiteralPath $installation -AclObject $unsafeRuntimeAcl
        Refuses { Install-MidnightDesktopShortcut $installation $desktop $hash } 'untrusted runtime writer'
        if ((Get-FileHash -LiteralPath $linkPath).Hash -cne $before) { throw 'Unsafe runtime changed shortcut' }
    } finally { Set-Acl -LiteralPath $installation -AclObject $originalRuntimeAcl }

    $junction = Join-Path $testRoot 'desktop-junction'
    $sentinel = Join-Path $outside 'Midnight Explorer.lnk'
    [IO.File]::WriteAllText($sentinel, 'outside must remain unchanged')
    $sentinelHash = (Get-FileHash -LiteralPath $sentinel).Hash
    New-Item -ItemType Junction -Path $junction -Target $outside | Out-Null
    try { Refuses { Install-MidnightDesktopShortcut $installation $junction $hash } 'Desktop junction' }
    finally { [IO.Directory]::Delete($junction) }
    if ((Get-FileHash -LiteralPath $sentinel).Hash -cne $sentinelHash) { throw 'Junction target changed' }

    Remove-Item -LiteralPath $linkPath
    New-Item -ItemType Junction -Path $linkPath -Target $outside | Out-Null
    try { Refuses { Install-MidnightDesktopShortcut $installation $desktop $hash } 'final destination junction' }
    finally { [IO.Directory]::Delete($linkPath) }
    if ((Get-FileHash -LiteralPath $sentinel).Hash -cne $sentinelHash) { throw 'Final-link target changed' }

    $pins = [MidnightShortcut.Native]::PinDirectory($desktop, $true)
    try {
        Refuses { [IO.Directory]::Move($desktop, ($desktop + '-moved')) } 'pinned ancestor rename'
        $stage = Join-Path $installation 'native-stage.lnk'
        [IO.File]::WriteAllText($stage, 'new native shortcut contents')
        $existing = Join-Path $desktop 'existing.lnk'
        [IO.File]::WriteAllText($existing, 'existing must survive no-replace')
        Refuses { [MidnightShortcut.Native]::Publish($stage, $pins, 'existing.lnk', $false) } 'atomic no-replace'
        if ([IO.File]::ReadAllText($existing) -cne 'existing must survive no-replace' -or
            -not [IO.File]::Exists($stage)) { throw 'No-replace changed existing/source file' }
        [MidnightShortcut.Native]::Publish($stage, $pins, 'existing.lnk', $true)
        if ([IO.File]::ReadAllText($existing) -cne 'new native shortcut contents') {
            throw 'Native replacement did not publish the staged contents'
        }
        # A final hard link introduced after validation must never write through to its target.
        $fileLink = Join-Path $desktop 'late-link.lnk'
        New-Item -ItemType HardLink -Path $fileLink -Target $sentinel | Out-Null
        [IO.File]::WriteAllText($stage, 'replacement directory entry')
        [MidnightShortcut.Native]::Publish($stage, $pins, 'late-link.lnk', $true)
        if ((Get-FileHash -LiteralPath $sentinel).Hash -cne $sentinelHash -or
            [IO.File]::ReadAllText($fileLink) -cne 'replacement directory entry') {
            throw 'Native rename wrote through a late destination hard link'
        }
        # Different actual local volumes must fail, with source and old target untouched.
        $crossRoot = Join-Path $projectRoot ('_build/goal/desktop/cross-' + [guid]::NewGuid().ToString('N'))
        New-Item -ItemType Directory -Path $crossRoot | Out-Null
        try {
            $crossPins = [MidnightShortcut.Native]::PinDirectory($crossRoot, $true)
            try {
                if ($crossPins.Volume -ne $pins.Volume) {
                    $crossStage = Join-Path $crossRoot 'cross-stage.lnk'
                    [IO.File]::WriteAllText($crossStage, 'cross-volume must stay here')
                    $existingHash = (Get-FileHash -LiteralPath $existing).Hash
                    Refuses { [MidnightShortcut.Native]::Publish($crossStage, $pins, 'existing.lnk', $true) } 'cross-volume rename'
                    if (-not [IO.File]::Exists($crossStage) -or
                        (Get-FileHash -LiteralPath $existing).Hash -cne $existingHash) {
                        throw 'Cross-volume refusal modified files'
                    }
                    Write-Output 'Cross-volume publication refusal verified on distinct local volumes.'
                } else { Write-Output 'Cross-volume native fixture unavailable: both roots use one volume.' }
            } finally { $crossPins.Dispose() }
        } finally {
            $crossParent = [IO.Path]::GetFullPath((Join-Path $projectRoot '_build/goal/desktop')).TrimEnd('\')
            if ((Split-Path -Parent ([IO.Path]::GetFullPath($crossRoot))) -cne $crossParent) {
                throw 'Unexpected cross-volume fixture cleanup path'
            }
            Remove-Item -LiteralPath $crossRoot -Recurse -Force
        }
    } finally { $pins.Dispose() }
    # Retag the very same retained leaf after validation. Publishing must still use
    # that directory object, never follow the new junction to the outside sentinel.
    $retagged = Join-Path $testRoot 'retagged-desktop'
    New-Item -ItemType Directory -Path $retagged | Out-Null
    $retagPins = [MidnightShortcut.Native]::PinDirectory($retagged, $true)
    $retagSet = $false
    try {
        $retagStage = Join-Path $installation 'retag-stage.lnk'
        [IO.File]::WriteAllText($retagStage, 'retained directory object')
        [ShortcutReparseFixture]::SetJunction($retagged, $outside, $false)
        $retagSet = $true
        $retagPublished = $false
        try {
            [MidnightShortcut.Native]::Publish($retagStage, $retagPins, 'Midnight Explorer.lnk', $true)
            $retagPublished = $true
        } catch {
            # Windows may refuse a rename relative to a now-reparsed directory.
            # Fail closed is valid; losing/modifying the outside object is never valid.
            if (-not [IO.File]::Exists($retagStage)) { throw 'Failed retagged rename lost its source' }
        }
        if ((Get-FileHash -LiteralPath $sentinel).Hash -cne $sentinelHash) {
            throw 'Publication followed a retagged destination directory'
        }
        [ShortcutReparseFixture]::SetJunction($retagged, $outside, $true)
        $retagSet = $false
        if ($retagPublished) {
            if ([IO.File]::ReadAllText((Join-Path $retagged 'Midnight Explorer.lnk')) -cne
                'retained directory object') { throw 'Publication lost the pinned directory object' }
        } else { Remove-Item -LiteralPath $retagStage }
        Write-Output 'Retagged destination leaf: outside sentinel unchanged; publication stayed contained or failed closed.'
    } finally {
        if ($retagSet) { [ShortcutReparseFixture]::SetJunction($retagged, $outside, $true) }
        $retagPins.Dispose()
    }
    for ($variant = 0; $variant -lt 16; $variant++) {
        $tag = [uint32]([uint64]2415919130 + ([uint64]$variant * 4096))
        if (-not [MidnightShortcut.Native]::IsCloudTag($tag)) { throw 'Known cloud family refused' }
    }
    foreach ($value in @('00000000', 'a0000003', 'a000000c', '9001001a', '8000001b')) {
        $tag = [Convert]::ToUInt32($value, 16)
        if ([MidnightShortcut.Native]::IsCloudTag($tag)) { throw 'Unknown/name-surrogate tag accepted' }
    }
    if (@(Get-ChildItem -LiteralPath $installation -Filter '*.lnk' -Force).Count) {
        throw 'Staged shortcut leaked'
    }
    if ($ReadRealCloudDesktop) {
        $realDesktop = [Environment]::GetFolderPath([Environment+SpecialFolder]::DesktopDirectory)
        $realPins = [MidnightShortcut.Native]::PinDirectory($realDesktop, $true)
        try {
            $tag = $realPins.Tag
            if (-not [MidnightShortcut.Native]::IsCloudTag($tag)) { throw 'Expected real cloud Desktop' }
            Write-Output ('Real Desktop read validation: {0}, tag 0x{1:x8}' -f $realDesktop, $tag)
        } finally { $realPins.Dispose() }
    }
    Write-Output 'Desktop shortcut checks passed: create/update, hash, junctions, pins, atomic no-replace/replace, late hard link, cloud tags, cleanup.'
} finally {
    if ((Split-Path -Parent ([IO.Path]::GetFullPath($testRoot))) -cne
        [IO.Path]::GetFullPath($testParent).TrimEnd('\')) { throw 'Unexpected fixture cleanup path' }
    Remove-Item -LiteralPath $testRoot -Recurse -Force
}
