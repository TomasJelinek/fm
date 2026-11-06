<#
Copy a verified Windows build to a new private runtime installation. This does not
build, run, replace an installation, or change checkout/ancestor permissions.
#>
param(
    [string]$SourceDirectory,
    [string]$Destination
)
$ErrorActionPreference = 'Stop'
if ($env:OS -ne 'Windows_NT') { throw 'This deployment helper requires Windows' }
$projectRoot = Split-Path -Parent $PSScriptRoot
if (-not $SourceDirectory) { $SourceDirectory = Join-Path $projectRoot '_build/debug' }
if (-not $Destination) {
    $name = 'MidnightExplorer-' + [DateTime]::UtcNow.ToString('yyyyMMdd-HHmmss') + '-' +
        [guid]::NewGuid().ToString('N').Substring(0, 8)
    $Destination = Join-Path $env:USERPROFILE $name
}
$SourceDirectory = [IO.Path]::GetFullPath($SourceDirectory).TrimEnd('\')
$Destination = [IO.Path]::GetFullPath($Destination).TrimEnd('\')
if ($SourceDirectory -notmatch '^[A-Za-z]:\\' -or $Destination -notmatch '^[A-Za-z]:\\') {
    throw 'Source and destination must be absolute local Windows filesystem paths'
}
if ($Destination.StartsWith($SourceDirectory + '\', [StringComparison]::OrdinalIgnoreCase)) {
    throw 'The runtime installation must be separate from its build input'
}
$currentSid = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
$installer = (New-Object Security.Principal.NTAccount('NT SERVICE', 'TrustedInstaller')).Translate(
    [Security.Principal.SecurityIdentifier]).Value
$trustedSids = @($currentSid, 'S-1-5-18', 'S-1-5-32-544', $installer)

function Assert-PlainPath([string]$Path) {
    $current = $Path
    while ($current) {
        $item = Get-Item -LiteralPath $current -Force
        if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) {
            throw "Symlinks/junctions are not deployment inputs or destinations: $current"
        }
        $parent = [IO.Directory]::GetParent($current)
        $current = if ($parent) { $parent.FullName } else { $null }
    }
}

function Assert-PrivatePath([string]$Path, [bool]$CheckContents) {
    Assert-PlainPath $Path
    $current = $Path
    $first = $true
    while ($current) {
        $security = Get-Acl -LiteralPath $current
        $owner = $security.GetOwner([Security.Principal.SecurityIdentifier]).Value
        $raw = New-Object Security.AccessControl.RawSecurityDescriptor(
            $security.GetSecurityDescriptorBinaryForm(), 0)
        if ($owner -notin $trustedSids -or $null -eq $raw.DiscretionaryAcl) {
            throw "Untrusted owner or unrestricted DACL on deployment path: $current"
        }
        # Ancestors may permit creating siblings, but cannot permit replacing an
        # existing installation component. Match the native host's trust policy.
        $mutable = if ($first -and $CheckContents) { 0x500d0156 } else { 0x500d0150 }
        foreach ($ace in $raw.DiscretionaryAcl) {
            if (([int]$ace.AceFlags -band [int][Security.AccessControl.AceFlags]::InheritOnly) -ne 0) { continue }
            if ($ace.AceType -eq [Security.AccessControl.AceType]::AccessDenied) { continue }
            if ($ace.AceType -ne [Security.AccessControl.AceType]::AccessAllowed) {
                throw "Unsupported conditional/object ACL on deployment path: $current"
            }
            if (($ace.AccessMask -band $mutable) -ne 0 -and $ace.SecurityIdentifier.Value -notin $trustedSids) {
                throw "Deployment path is writable or replaceable by another account: $current"
            }
        }
        $parent = [IO.Directory]::GetParent($current)
        $current = if ($parent) { $parent.FullName } else { $null }
        $first = $false
    }
}

if (Test-Path -LiteralPath $Destination) { throw "Destination already exists; choose a new installation: $Destination" }
$destinationParent = Split-Path -Parent $Destination
if (-not (Test-Path -LiteralPath $destinationParent -PathType Container)) {
    throw "Destination parent must already exist: $destinationParent"
}
Assert-PrivatePath $destinationParent $false
Assert-PlainPath $SourceDirectory
$executable = Join-Path $SourceDirectory 'me.exe'
$viewers = Join-Path $SourceDirectory 'viewers'
if (-not (Test-Path -LiteralPath $executable -PathType Leaf) -or
    -not (Test-Path -LiteralPath $viewers -PathType Container)) {
    throw 'Build input must contain me.exe and independently built viewers/'
}
foreach ($name in @('basic', 'source', 'markdown')) {
    foreach ($file in @('plugin.toml', "midnight_viewer_$name.dll", 'LICENSE', 'THIRD-PARTY.md')) {
        if (-not (Test-Path -LiteralPath "$viewers/midnight-viewer-$name/$file" -PathType Leaf)) {
            throw "Incomplete viewer package: midnight-viewer-$name/$file"
        }
    }
}
$files = New-Object 'System.Collections.Generic.List[string]'
$files.Add($executable)
$pending = New-Object 'System.Collections.Generic.Stack[string]'
$pending.Push($viewers)
$entryCount = 0
while ($pending.Count -gt 0) {
    $directory = $pending.Pop()
    Assert-PlainPath $directory
    foreach ($entry in Get-ChildItem -LiteralPath $directory -Force) {
        $entryCount++
        if ($entryCount -gt 16384) { throw 'Viewer deployment entry limit exceeded' }
        Assert-PlainPath $entry.FullName
        if ($entry.PSIsContainer) { $pending.Push($entry.FullName) }
        else { $files.Add($entry.FullName) }
    }
}
Assert-PlainPath $executable
# Preflight completes before creating any installation. Only this new directory
# receives a protected ACL; files and subdirectories inherit it when created.
New-Item -ItemType Directory -Path $Destination | Out-Null
$privateAcl = New-Object Security.AccessControl.DirectorySecurity
$privateAcl.SetSecurityDescriptorSddlForm(
    "D:P(A;OICI;FA;;;$currentSid)(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)",
    [Security.AccessControl.AccessControlSections]::Access)
Set-Acl -LiteralPath $Destination -AclObject $privateAcl
Assert-PrivatePath $Destination $true
foreach ($source in $files) {
    Assert-PlainPath $source
    $relative = $source.Substring($SourceDirectory.Length + 1)
    $target = Join-Path $Destination $relative
    New-Item -ItemType Directory -Path (Split-Path -Parent $target) -Force | Out-Null
    # Read sharing prevents concurrent writes/renames of each opened source file.
    # CreateNew avoids overwriting any unexpected destination entry.
    $sourceStream = [IO.File]::Open($source, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
    try {
        $destinationStream = [IO.File]::Open($target, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::None)
        try { $sourceStream.CopyTo($destinationStream) } finally { $destinationStream.Dispose() }
        $sourceStream.Position = 0
        $hasher = [Security.Cryptography.SHA256]::Create()
        try {
            $expected = [Convert]::ToBase64String($hasher.ComputeHash($sourceStream))
            $copied = [IO.File]::Open($target, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
            try { $actual = [Convert]::ToBase64String($hasher.ComputeHash($copied)) }
            finally { $copied.Dispose() }
            if ($expected -cne $actual) { throw "Deployment readback hash mismatch: $target" }
        } finally { $hasher.Dispose() }
    } finally { $sourceStream.Dispose() }
    Assert-PrivatePath $target $true
}
Write-Output "Private viewer installation: $Destination"
Write-Output ('Run: & "' + (Join-Path $Destination 'me.exe') + '"')
