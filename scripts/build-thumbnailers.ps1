param(
    [Parameter(Mandatory=$true)][string]$ApiConfig,
    [Parameter(Mandatory=$true)][string]$Destination,
    [Parameter(Mandatory=$true)][string]$BuildRoot,
    [Parameter(Mandatory=$true)][string]$NoticesRoot,
    [string]$FfmpegPrefix,
    [string]$FfmpegSourceArchive,
    [string]$LibclangPath,
    [ValidateSet('debug','release')][string]$Profile='release'
)
$ErrorActionPreference='Stop'
$repository=Split-Path -Parent $PSScriptRoot
$Destination=[System.IO.Path]::GetFullPath($Destination)
$BuildRoot=[System.IO.Path]::GetFullPath($BuildRoot)
foreach ($path in @($Destination,$BuildRoot)) {
    if ($path -notmatch '[/\\]_build[/\\]') { throw 'Generated thumbnail output must be under _build' }
    for ($part=$path; $part; $part=Split-Path -Parent $part) {
        if ((Test-Path -LiteralPath $part) -and ((Get-Item -LiteralPath $part).Attributes -band [System.IO.FileAttributes]::ReparsePoint)) {
            throw 'Generated deployment path must not traverse reparse points'
        }
    }
}
New-Item -ItemType Directory -Force $Destination,$BuildRoot | Out-Null
$plugins=@('midnight-thumbnail-image')
if ($FfmpegPrefix) {
    if (!$FfmpegSourceArchive -or !$LibclangPath) { throw 'Video builds require corresponding source archive and libclang' }
    $hash=(Get-FileHash -LiteralPath $FfmpegSourceArchive -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($hash -ne '8c3850283eb25fa026482078a04051e0be17347b09ef81a0849bec15a96e002e') { throw 'Unexpected FFmpeg source archive checksum' }
    $env:FFMPEG_DIR=[System.IO.Path]::GetFullPath($FfmpegPrefix)
    $env:LIBCLANG_PATH=[System.IO.Path]::GetFullPath($LibclangPath)
    $plugins+='midnight-thumbnail-video'
}
foreach ($name in $plugins) {
    $project=Join-Path $repository "plugins/$name"
    $env:CARGO_TARGET_DIR=Join-Path $BuildRoot $name
    $arguments=@('build','--manifest-path',(Join-Path $project 'Cargo.toml'),'--config',$ApiConfig,'--locked','--offline')
    if ($Profile -eq 'release') { $arguments+='--release' }
    & cargo @arguments
    if ($LASTEXITCODE -ne 0) { throw "$name build failed" }
    Copy-Item -LiteralPath (Join-Path $env:CARGO_TARGET_DIR "$Profile/$name.exe") -Destination (Join-Path $Destination "$name.exe")
    $manifest=Get-Content -LiteralPath (Join-Path $project 'thumbnailer.json') -Raw | ConvertFrom-Json
    $manifest.executable="$name.exe"
    [System.IO.File]::WriteAllText((Join-Path $Destination "$name.json"),($manifest | ConvertTo-Json),[System.Text.UTF8Encoding]::new($false))
    if (Test-Path -LiteralPath (Join-Path $NoticesRoot $name)) {
        $notices=Join-Path $Destination "notices/$name"
        New-Item -ItemType Directory -Force $notices | Out-Null
        Get-ChildItem -LiteralPath (Join-Path $NoticesRoot $name) | Copy-Item -Destination $notices -Recurse -Force
    } else {
        throw "Missing distribution notices for $name; stage pinned dependency license notices first"
    }
}
if ($FfmpegPrefix) {
    # Only four installed code files are readable by the sandbox identity. No directory,
    # user content, write permission or network capability is granted.
    if (-not ('MidnightWorkerBuildIdentity' -as [type])) { Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
using System.Security.Principal;
public static class MidnightWorkerBuildIdentity {
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
    $sid=[MidnightWorkerBuildIdentity]::Get()
    foreach ($name in @('avcodec-63.dll','avformat-63.dll','avutil-61.dll','swscale-10.dll')) {
        $path=Join-Path $Destination $name
        Copy-Item -LiteralPath (Join-Path $FfmpegPrefix "bin/$name") -Destination $path
        $acl=Get-Acl -LiteralPath $path
        $acl.AddAccessRule([System.Security.AccessControl.FileSystemAccessRule]::new($sid,
            [System.Security.AccessControl.FileSystemRights]::ReadAndExecute,[System.Security.AccessControl.AccessControlType]::Allow))
        Set-Acl -LiteralPath $path -AclObject $acl
    }
    $notices=Join-Path $Destination 'notices/ffmpeg-9.0.2'
    New-Item -ItemType Directory -Force $notices | Out-Null
    Get-ChildItem -LiteralPath (Join-Path $FfmpegPrefix 'share/midnight-ffmpeg') | Copy-Item -Destination $notices -Recurse -Force
    Copy-Item -LiteralPath $FfmpegSourceArchive -Destination (Join-Path $notices 'ffmpeg-9.0.2.tar.xz')
}
Write-Output "Built installed thumbnail providers: $Destination"
