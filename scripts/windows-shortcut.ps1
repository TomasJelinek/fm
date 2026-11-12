# Shortcut-only cloud handling. Never use this policy to validate executable/plugin paths.
$shortcutNative = Join-Path $PSScriptRoot 'windows-shortcut-native.cs'
if (-not ('MidnightShortcut.Native' -as [type])) { Add-Type -Path $shortcutNative }

function Assert-ShortcutRuntimeSecurity([string]$Path, [bool]$Contents) {
    $sid = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
    $installer = (New-Object Security.Principal.NTAccount('NT SERVICE', 'TrustedInstaller')).Translate(
        [Security.Principal.SecurityIdentifier]).Value
    $trusted = @($sid, 'S-1-5-18', 'S-1-5-32-544', $installer)
    $first = $true
    $current = $Path
    while ($current) {
        $acl = Get-Acl -LiteralPath $current
        $owner = $acl.GetOwner([Security.Principal.SecurityIdentifier]).Value
        $raw = New-Object Security.AccessControl.RawSecurityDescriptor(
            $acl.GetSecurityDescriptorBinaryForm(), 0)
        if ($owner -notin $trusted -or $null -eq $raw.DiscretionaryAcl) {
            throw "Untrusted shortcut runtime owner or unrestricted DACL: $current"
        }
        $mutable = if ($first -and $Contents) { 0x500d0156 } else { 0x500d0150 }
        foreach ($ace in $raw.DiscretionaryAcl) {
            if (([int]$ace.AceFlags -band [int][Security.AccessControl.AceFlags]::InheritOnly) -ne 0) { continue }
            if ($ace.AceType -eq [Security.AccessControl.AceType]::AccessDenied) { continue }
            if ($ace.AceType -ne [Security.AccessControl.AceType]::AccessAllowed -or
                (($ace.AccessMask -band $mutable) -ne 0 -and $ace.SecurityIdentifier.Value -notin $trusted)) {
                throw "Unsafe shortcut runtime ACL: $current"
            }
        }
        $parent = [IO.Directory]::GetParent($current)
        $current = if ($parent) { $parent.FullName } else { $null }
        $first = $false
    }
}

function Install-MidnightDesktopShortcut([string]$InstallationDirectory,
    [string]$DesktopDirectory, [string]$ExpectedExecutableSha256) {
    if ($ExpectedExecutableSha256 -notmatch '^[0-9a-fA-F]{64}$') { throw 'Expected SHA-256 is required' }
    $installation = [MidnightShortcut.Native]::LocalPath($InstallationDirectory)
    $desktop = [MidnightShortcut.Native]::LocalPath($DesktopDirectory)
    $runtimePins = [MidnightShortcut.Native]::PinDirectory($installation, $false)
    $desktopPins = $null
    $executableStream = $null
    $temporary = $null
    try {
        Assert-ShortcutRuntimeSecurity $installation $true
        $executable = Join-Path $installation 'me.exe'
        $executableStream = [MidnightShortcut.Native]::OpenExecutable($executable)
        Assert-ShortcutRuntimeSecurity $executable $true
        $hasher = [Security.Cryptography.SHA256]::Create()
        try { $hash = [BitConverter]::ToString($hasher.ComputeHash($executableStream)).Replace('-', '') }
        finally { $hasher.Dispose() }
        if ($hash -ine $ExpectedExecutableSha256) { throw 'Runtime executable SHA-256 differs' }
        $desktopPins = [MidnightShortcut.Native]::PinDirectory($desktop, $true)
        if ($runtimePins.Volume -ne $desktopPins.Volume) {
            throw 'Desktop and private installation must share a volume; no shortcut copy fallback'
        }
        $name = 'Midnight Explorer.lnk'
        $shortcutPath = Join-Path $desktop $name
        [MidnightShortcut.Native]::ValidateDestination($shortcutPath)
        $temporary = Join-Path $installation ('.shortcut-' + [guid]::NewGuid().ToString('N') + '.lnk')
        $reservation = [IO.File]::Open($temporary, [IO.FileMode]::CreateNew,
            [IO.FileAccess]::Write, [IO.FileShare]::None)
        $reservation.Dispose()
        [MidnightShortcut.Native]::CreateShortcut($temporary, $executable, $env:USERPROFILE)
        $created = [MidnightShortcut.Native]::ReadShortcut($temporary)
        if ($created[0] -cne $executable -or $created[1] -cne ($executable + ',0') -or
            $created[2] -cne $env:USERPROFILE -or $created[3]) {
            throw 'Staged shortcut readback differs; existing Desktop shortcut was retained'
        }
        [MidnightShortcut.Native]::ValidateDestination($shortcutPath)
        [MidnightShortcut.Native]::Publish($temporary, $desktopPins, $name, $true)
        $temporary = $null
        Write-Output "Desktop shortcut: $shortcutPath"
    } finally {
        if ($temporary -and [IO.File]::Exists($temporary)) { [IO.File]::Delete($temporary) }
        if ($desktopPins) { $desktopPins.Dispose() }
        if ($executableStream) { $executableStream.Dispose() }
        $runtimePins.Dispose()
    }
}
