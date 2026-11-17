<#
Copy a verified Windows build to a new private runtime installation. This does not
build, run, replace an installation, or change checkout/ancestor permissions.
#>
param(
    [string]$SourceDirectory,
    [string]$Destination,
    [switch]$IncludeFilesystems,
    [switch]$IncludeThumbnailers,
    [switch]$CreateDesktopShortcut,
    [switch]$RegisterRuntime,
    [string]$RegistrationDirectory
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
$originalSourceDirectory = $SourceDirectory
if ($RegisterRuntime -and (($originalSourceDirectory -split '[\\/]') -contains '..')) {
    throw 'Source launcher parent traversal is forbidden'
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

function Assert-RegistrationLocation([string]$Path) {
    $existing = $Path
    while (-not (Test-Path -LiteralPath $existing)) {
        $existing = Split-Path -Parent $existing
        if (-not $existing) { throw 'Registration directory has no existing parent' }
    }
    if (-not (Test-Path -LiteralPath $existing -PathType Container)) {
        throw "Registration ancestor is not a directory: $existing"
    }
    # The closest existing directory must also reject untrusted child creation.
    Assert-PrivatePath $existing $true
}

function Initialize-LaunchRegistrationType {
    if ('MidnightRuntimeRegistrationV1.LaunchStamp' -as [type]) { return }
    Add-Type -TypeDefinition @'
using System;
using System.ComponentModel;
using System.IO;
using System.Runtime.InteropServices;
using System.Security.Cryptography;
using System.Text;
using Microsoft.Win32.SafeHandles;

namespace MidnightRuntimeRegistrationV1 {
    public sealed class LaunchStamp : IDisposable {
        [StructLayout(LayoutKind.Sequential)]
        private struct FileInfo {
            public uint Attributes, CreatedLow, CreatedHigh, AccessedLow, AccessedHigh;
            public uint WrittenLow, WrittenHigh, Volume, SizeHigh, SizeLow, Links, IndexHigh, IndexLow;
        }
        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        private static extern SafeFileHandle CreateFileW(string path, uint access, uint share,
            IntPtr security, uint disposition, uint flags, IntPtr template);
        [DllImport("kernel32.dll", SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        private static extern bool GetFileInformationByHandle(SafeFileHandle file, out FileInfo info);
        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        private static extern uint GetFinalPathNameByHandleW(SafeFileHandle file,
            StringBuilder path, uint capacity, uint flags);
        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        private static extern uint GetFileAttributesW(string path);

        private SafeFileHandle handle;
        private readonly string original;
        public string CanonicalPath { get; private set; }
        public long Length { get; private set; }
        public long LastWriteTime100ns { get; private set; }
        public string Key { get; private set; }

        private static string NativePath(string path) {
            if (String.IsNullOrEmpty(path) || path.IndexOf('\0') >= 0)
                throw new IOException("Launcher path is empty or contains NUL");
            foreach (string component in path.Split(new char[] { '\\', '/' }))
                if (component == "..") throw new IOException("Launcher parent traversal is forbidden");
            if (path.IndexOf('/') >= 0) throw new IOException("Launcher requires native path separators");
            if (path.StartsWith(@"\\?\", StringComparison.Ordinal)) {
                RootLength(path);
                return path;
            }
            if (path.StartsWith(@"\\", StringComparison.Ordinal)) return @"\\?\UNC\" + path.Substring(2);
            if (path.Length >= 3 && Char.IsLetter(path[0]) && path[1] == ':' && path[2] == '\\')
                return @"\\?\" + path;
            throw new IOException("Launcher requires an absolute DOS or UNC path");
        }
        private static int RootLength(string path) {
            if (path.Length >= 7 && path.StartsWith(@"\\?\", StringComparison.Ordinal)
                && Char.IsLetter(path[4]) && path[5] == ':' && path[6] == '\\') return 7;
            if (path.StartsWith(@"\\?\UNC\", StringComparison.OrdinalIgnoreCase)) {
                int server = path.IndexOf('\\', 8);
                if (server <= 8) throw new IOException("Launcher UNC server is missing");
                int share = path.IndexOf('\\', server + 1);
                if (share == server + 1 || server + 1 == path.Length)
                    throw new IOException("Launcher UNC share is missing");
                return share < 0 ? path.Length : share + 1;
            }
            throw new IOException("Launcher native path is not DOS or UNC");
        }
        private static void AssertPlain(string path) {
            string current = NativePath(path);
            int root = RootLength(current);
            while (true) {
                uint attributes = GetFileAttributesW(current);
                if (attributes == UInt32.MaxValue) throw new Win32Exception(Marshal.GetLastWin32Error());
                if ((attributes & 0x400) != 0) throw new IOException("Launcher path traverses a reparse point: " + current);
                if (current.Length <= root) break;
                int parent = current.LastIndexOf('\\');
                current = parent < root ? current.Substring(0, root) : current.Substring(0, parent);
            }
        }
        private static FileInfo Information(SafeFileHandle file) {
            FileInfo info;
            if (!GetFileInformationByHandle(file, out info)) throw new Win32Exception(Marshal.GetLastWin32Error());
            if ((info.Attributes & 0x410) != 0) throw new IOException("Launcher must be a regular non-reparse file");
            return info;
        }
        private static long FileLength(FileInfo info) {
            long result = ((long)info.SizeHigh << 32) | info.SizeLow;
            if (result < 0) throw new IOException("Launcher length exceeds supported range");
            return result;
        }
        private static long Written(FileInfo info) {
            long result = ((long)info.WrittenHigh << 32) | info.WrittenLow;
            if (result < 0) throw new IOException("Launcher last-write time exceeds supported signed FILETIME range");
            return result;
        }
        private static string FinalPath(SafeFileHandle file) {
            uint capacity = 256;
            while (capacity <= 65536) {
                StringBuilder buffer = new StringBuilder((int)capacity);
                // FILE_NAME_NORMALIZED | VOLUME_NAME_DOS are both zero.
                uint size = GetFinalPathNameByHandleW(file, buffer, capacity, 0);
                if (size == 0) throw new Win32Exception(Marshal.GetLastWin32Error());
                if (size < capacity) return buffer.ToString(0, (int)size);
                capacity = checked(size + 1);
            }
            throw new IOException("Launcher native path exceeds the Windows UTF-16 path limit");
        }
        public static string KeyForUtf16Units(string path) {
            byte[] prefix = Encoding.ASCII.GetBytes("midnight-launch-path-v1\0");
            byte[] bytes = new byte[checked(prefix.Length + path.Length * 2)];
            Buffer.BlockCopy(prefix, 0, bytes, 0, prefix.Length);
            for (int index = 0; index < path.Length; index++) {
                ushort unit = path[index];
                bytes[prefix.Length + index * 2] = (byte)(unit & 255);
                bytes[prefix.Length + index * 2 + 1] = (byte)(unit >> 8);
            }
            using (SHA256 hash = SHA256.Create())
                return BitConverter.ToString(hash.ComputeHash(bytes)).Replace("-", "").ToLowerInvariant();
        }
        private LaunchStamp(string path, SafeFileHandle file) {
            original = path;
            handle = file;
            CanonicalPath = FinalPath(handle);
            AssertPlain(original);
            AssertPlain(CanonicalPath);
            FileInfo info = Information(handle);
            Length = FileLength(info);
            LastWriteTime100ns = Written(info);
            Key = KeyForUtf16Units(CanonicalPath);
        }
        private static LaunchStamp OpenCore(string path, uint access) {
            AssertPlain(path);
            SafeFileHandle file = CreateFileW(NativePath(path), access, 1, IntPtr.Zero, 3, 0x00200000, IntPtr.Zero);
            if (file.IsInvalid) {
                int error = Marshal.GetLastWin32Error();
                file.Dispose();
                throw new Win32Exception(error);
            }
            try { return new LaunchStamp(path, file); }
            catch { file.Dispose(); throw; }
        }
        public static LaunchStamp Open(string path) {
            // Metadata-only inspection does not establish a data write/delete lease.
            return OpenCore(path, 0x80);
        }
        public static DeploymentLease OpenForDeployment(string path) {
            // Deployment explicitly retains FILE_READ_DATA | FILE_READ_ATTRIBUTES with FILE_SHARE_READ.
            LaunchStamp stamp = OpenCore(path, 0x81);
            try { return new DeploymentLease(stamp, stamp.handle); }
            catch { stamp.Dispose(); throw; }
        }
        public void VerifyUnchanged() {
            FileInfo info = Information(handle);
            AssertPlain(original);
            string final = FinalPath(handle);
            AssertPlain(final);
            if (FileLength(info) != Length || Written(info) != LastWriteTime100ns
                || !String.Equals(final, CanonicalPath, StringComparison.Ordinal))
                throw new IOException("Launcher metadata/path changed during deployment");
        }
        public void Dispose() {
            if (handle != null) { handle.Dispose(); handle = null; }
        }
    }

    public sealed class DeploymentLease : IDisposable {
        private FileStream reader;
        public LaunchStamp Stamp { get; private set; }

        internal DeploymentLease(LaunchStamp stamp, SafeFileHandle handle) {
            Stamp = stamp;
            // Both metadata and data verification refer to this same retained file handle.
            reader = new FileStream(handle, FileAccess.Read);
        }
        public string Digest() {
            Stamp.VerifyUnchanged();
            reader.Position = 0;
            byte[] digest;
            using (SHA256 hash = SHA256.Create()) digest = hash.ComputeHash(reader);
            Stamp.VerifyUnchanged();
            return BitConverter.ToString(digest).Replace("-", "").ToLowerInvariant();
        }
        public void Dispose() {
            try {
                if (reader != null) { reader.Dispose(); reader = null; }
            } finally {
                if (Stamp != null) { Stamp.Dispose(); Stamp = null; }
            }
        }
    }
}
'@
}

function Initialize-PrivateRegistrationDirectory([string]$Directory,
    [Security.AccessControl.DirectorySecurity]$Acl) {
    Assert-RegistrationLocation $Directory
    $missing = New-Object 'System.Collections.Generic.Stack[string]'
    $existing = $Directory
    while (-not (Test-Path -LiteralPath $existing)) {
        $missing.Push($existing)
        $existing = Split-Path -Parent $existing
    }
    while ($missing.Count -gt 0) {
        $created = $missing.Pop()
        New-Item -ItemType Directory -Path $created | Out-Null
        Set-Acl -LiteralPath $created -AclObject $Acl
        Assert-PrivatePath $created $true
    }
}

function Write-PrivateRegistrationRecord([string]$Directory, [string]$Name, [byte[]]$Bytes) {
    if ($Name -cnotmatch '^[0-9a-f]{64}\.toml\z') { throw 'Invalid registration record name' }
    if ($Bytes.Length -gt 1048576) { throw 'Runtime registration exceeds 1 MiB' }
    Assert-PrivatePath $Directory $true
    $record = Join-Path $Directory $Name
    if (Test-Path -LiteralPath $record) {
        if (-not (Test-Path -LiteralPath $record -PathType Leaf)) { throw "Registration record is not a file: $record" }
        Assert-PrivatePath $record $true
    }
    $temporary = Join-Path $Directory ('.' + $Name + '.' + [guid]::NewGuid().ToString('N') + '.tmp')
    $ownsTemporary = $false
    try {
        $stream = [IO.File]::Open($temporary, [IO.FileMode]::CreateNew,
            [IO.FileAccess]::Write, [IO.FileShare]::None)
        $ownsTemporary = $true
        try { $stream.Write($Bytes, 0, $Bytes.Length); $stream.Flush($true) }
        finally { $stream.Dispose() }
        Assert-PrivatePath $temporary $true
        if (Test-Path -LiteralPath $record) {
            Assert-PrivatePath $record $true
            [IO.File]::Replace($temporary, $record, [NullString]::Value)
        } else {
            [IO.File]::Move($temporary, $record)
        }
        Assert-PrivatePath $record $true
    } finally {
        if ($ownsTemporary -and (Test-Path -LiteralPath $temporary)) {
            Remove-Item -LiteralPath $temporary
        }
    }
}

function Register-PrivateRuntime([string]$Directory, [string]$Executable,
    $LaunchLease, [Security.AccessControl.DirectorySecurity]$Acl) {
    $LaunchStamp = $LaunchLease.Stamp
    Assert-PrivatePath $Executable $true
    $LaunchStamp.VerifyUnchanged()
    $hash = (Get-FileHash -LiteralPath $Executable -Algorithm SHA256).Hash.ToLowerInvariant()
    # Deployment hashing remains mandatory; the runtime OFF mode does no EXE hashing at lookup.
    $launchHash = $LaunchLease.Digest()
    if ($launchHash -cne $hash) { throw 'Source executable differs from the verified runtime copy' }
    $LaunchStamp.VerifyUnchanged()
    Initialize-PrivateRegistrationDirectory $Directory $Acl
    $launchers = Join-Path $Directory 'launchers'
    Initialize-PrivateRegistrationDirectory $launchers $Acl
    $pointer = Join-Path $launchers ($LaunchStamp.Key + '.toml')
    if (Test-Path -LiteralPath $pointer) { Assert-PrivatePath $pointer $true }

    $newline = [string][char]10
    $units = ($Executable.ToCharArray() | ForEach-Object { [uint16]$_ }) -join ','
    $text = 'schema_version = 1' + $newline + 'sha256 = "' + $hash + '"' + $newline +
        'executable_utf16 = [' + $units + ']' + $newline
    $encoding = [Text.UTF8Encoding]::new($false)
    # Publish the unchanged legacy schema before the pointer can select it.
    Write-PrivateRegistrationRecord $Directory ($hash + '.toml') ($encoding.GetBytes($text))
    $LaunchStamp.VerifyUnchanged()
    $launchUnits = ($LaunchStamp.CanonicalPath.ToCharArray() | ForEach-Object { [uint16]$_ }) -join ','
    $pointerText = 'schema_version = 1' + $newline +
        'launch_executable_utf16 = [' + $launchUnits + ']' + $newline +
        'launch_length = ' + $LaunchStamp.Length.ToString([Globalization.CultureInfo]::InvariantCulture) + $newline +
        'launch_last_write_time_100ns = ' + $LaunchStamp.LastWriteTime100ns.ToString([Globalization.CultureInfo]::InvariantCulture) + $newline +
        'registration_sha256 = "' + $hash + '"' + $newline
    Write-PrivateRegistrationRecord $launchers ($LaunchStamp.Key + '.toml') ($encoding.GetBytes($pointerText))
    $LaunchStamp.VerifyUnchanged()
    Write-Output "Registered matching build: $(Join-Path $Directory ($hash + '.toml'))"
    Write-Output "Registered launch pointer: $pointer"
}

if ($RegistrationDirectory -and -not $RegisterRuntime) {
    throw '-RegistrationDirectory requires -RegisterRuntime'
}
if ($RegisterRuntime) {
    if (-not $RegistrationDirectory) {
        if (-not $env:USERPROFILE) { throw 'USERPROFILE is required for runtime registration' }
        $RegistrationDirectory = Join-Path $env:USERPROFILE '.midnight-explorer/plugin-runtimes'
    }
    $RegistrationDirectory = [IO.Path]::GetFullPath($RegistrationDirectory).TrimEnd('\')
    if ($RegistrationDirectory -notmatch '^[A-Za-z]:\\') { throw 'Registration requires an absolute local directory' }
    Assert-RegistrationLocation $RegistrationDirectory
    Assert-RegistrationLocation (Join-Path $RegistrationDirectory 'launchers')
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
$authorizationHelper = Join-Path $SourceDirectory 'midnight-elevated-listing.exe'
if (-not (Test-Path -LiteralPath $authorizationHelper -PathType Leaf)) {
    throw 'Build input must contain the dedicated midnight-elevated-listing.exe helper'
}
Assert-PlainPath $authorizationHelper
$files.Add($authorizationHelper)
$pending = New-Object 'System.Collections.Generic.Stack[string]'
$pending.Push($viewers)
foreach ($kind in @('filesystems', 'thumbnailers')) {
    $include = if ($kind -eq 'filesystems') { $IncludeFilesystems } else { $IncludeThumbnailers }
    if ($include) {
        $workerRoot = Join-Path $SourceDirectory $kind
        if (-not (Test-Path -LiteralPath $workerRoot -PathType Container)) { throw "Missing requested worker directory: $kind" }
        $pending.Push($workerRoot)
    }
}
$videoLibraries = @('avcodec-63.dll', 'avformat-63.dll', 'avutil-61.dll', 'swscale-10.dll')
$videoPresent = $IncludeThumbnailers -and (Test-Path -LiteralPath (Join-Path $SourceDirectory 'thumbnailers/midnight-thumbnail-video.json'))
if ($videoPresent) {
    foreach ($name in $videoLibraries) {
        if (-not (Test-Path -LiteralPath (Join-Path $SourceDirectory "thumbnailers/$name") -PathType Leaf)) {
            throw "Incomplete video runtime: $name"
        }
    }
}
$entryCount = 0
while ($pending.Count -gt 0) {
    $directory = $pending.Pop()
    Assert-PlainPath $directory
    foreach ($entry in Get-ChildItem -LiteralPath $directory -Force) {
        $entryCount++
        if ($entryCount -gt 16384) { throw 'Deployment entry limit exceeded' }
        Assert-PlainPath $entry.FullName
        if ($entry.PSIsContainer) { $pending.Push($entry.FullName) }
        else { $files.Add($entry.FullName) }
    }
}
Assert-PlainPath $executable
$launchStamp = $null
$launchLease = $null
try {
    if ($RegisterRuntime) {
        Assert-PlainPath $originalSourceDirectory
        Initialize-LaunchRegistrationType
        $launchLease = [MidnightRuntimeRegistrationV1.LaunchStamp]::OpenForDeployment($executable)
        $launchStamp = $launchLease.Stamp
        $existingPointer = Join-Path (Join-Path $RegistrationDirectory 'launchers') ($launchStamp.Key + '.toml')
        if (Test-Path -LiteralPath $existingPointer) {
            if (-not (Test-Path -LiteralPath $existingPointer -PathType Leaf)) {
                throw "Launcher pointer is not a file: $existingPointer"
            }
            Assert-PrivatePath $existingPointer $true
        }
    }
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
    if ($videoPresent) {
        # Grant the fixed worker identity read/execute on these code files only. No inherited ACE,
        # directory permission, private content access, write permission or network capability.
        if (-not ('MidnightWorkerDeploymentIdentity' -as [type])) { Add-Type -TypeDefinition @'
    using System;
    using System.Runtime.InteropServices;
    using System.Security.Principal;
    public static class MidnightWorkerDeploymentIdentity {
        [DllImport("userenv.dll", CharSet=CharSet.Unicode)] static extern int DeriveAppContainerSidFromAppContainerName(string name, out IntPtr sid);
        [DllImport("advapi32.dll")] static extern IntPtr FreeSid(IntPtr sid);
        public static SecurityIdentifier Get() {
            IntPtr sid;
            int result=DeriveAppContainerSidFromAppContainerName("MidnightExplorer.RestrictedWorker.v1", out sid);
            if(result != 0) Marshal.ThrowExceptionForHR(result);
            try { return new SecurityIdentifier(sid); } finally { FreeSid(sid); }
        }
    }
'@
        }
        $workerSid=[MidnightWorkerDeploymentIdentity]::Get()
        foreach ($name in $videoLibraries) {
            $path=Join-Path $Destination "thumbnailers/$name"
            $acl=Get-Acl -LiteralPath $path
            $acl.AddAccessRule([Security.AccessControl.FileSystemAccessRule]::new($workerSid,
                [Security.AccessControl.FileSystemRights]::ReadAndExecute,[Security.AccessControl.AccessControlType]::Allow))
            Set-Acl -LiteralPath $path -AclObject $acl
            Assert-PrivatePath $path $true
        }
    }
    if ($RegisterRuntime) {
        Register-PrivateRuntime $RegistrationDirectory (Join-Path $Destination 'me.exe') $launchLease $privateAcl
    }
    Write-Output "Private installation: $Destination"
    Write-Output ('Run: & "' + (Join-Path $Destination 'me.exe') + '"')
    if ($CreateDesktopShortcut) {
        $desktopPath = [Environment]::GetFolderPath([Environment+SpecialFolder]::DesktopDirectory)
        if (-not $desktopPath) {
            throw 'No desktop directory is available; the private installation is ready to run directly'
        }
        . (Join-Path $PSScriptRoot 'windows-shortcut.ps1')
        $shortcutHash = (Get-FileHash -LiteralPath (Join-Path $Destination 'me.exe') -Algorithm SHA256).Hash
        Install-MidnightDesktopShortcut $Destination $desktopPath $shortcutHash
    }

} finally {
    if ($null -ne $launchLease) { $launchLease.Dispose() }
}
