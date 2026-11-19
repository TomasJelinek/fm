param(
    [Parameter(Mandatory=$true)][string]$ApiConfig,
    [Parameter(Mandatory=$true)][string]$Destination,
    [Parameter(Mandatory=$true)][string]$BuildRoot,
    [Parameter(Mandatory=$true)][string]$NoticesRoot,
    [Parameter(Mandatory=$true)][string]$FfmpegPrefix,
    [Parameter(Mandatory=$true)][string]$FfmpegSourceArchive,
    [Parameter(Mandatory=$true)][string]$LibclangPath,
    [ValidateSet('debug','release')][string]$Profile='debug',
    [switch]$NativeOffline
)
$ErrorActionPreference='Stop'
$repository=Split-Path -Parent $PSScriptRoot
# Resolve caller paths against the PowerShell location, not the process directory.
$resolve={ param($path) [System.IO.Path]::GetFullPath($ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($path)) }
$ApiConfig=& $resolve $ApiConfig
$Destination=& $resolve $Destination
$BuildRoot=& $resolve $BuildRoot
$NoticesRoot=& $resolve $NoticesRoot
$FfmpegPrefix=& $resolve $FfmpegPrefix
$FfmpegSourceArchive=& $resolve $FfmpegSourceArchive
$LibclangPath=& $resolve $LibclangPath
foreach ($path in @($Destination,$BuildRoot)) {
    if ($path -notmatch '[/\\]_build[/\\]') { throw 'Generated thumbnail output must be under _build' }
    for ($part=$path; $part; $part=Split-Path -Parent $part) {
        if ((Test-Path -LiteralPath $part) -and ((Get-Item -LiteralPath $part).Attributes -band [System.IO.FileAttributes]::ReparsePoint)) {
            throw 'Generated deployment path must not traverse reparse points'
        }
    }
}
$parent=Split-Path -Parent $Destination
New-Item -ItemType Directory -Force $parent,$BuildRoot | Out-Null
$plugins=@('midnight-thumbnail-image', 'midnight-thumbnail-video')
$hash=(Get-FileHash -LiteralPath $FfmpegSourceArchive -Algorithm SHA256).Hash.ToLowerInvariant()
if ($hash -ne '8c3850283eb25fa026482078a04051e0be17347b09ef81a0849bec15a96e002e') { throw 'Unexpected FFmpeg source archive checksum' }
$env:FFMPEG_DIR=$FfmpegPrefix
$env:LIBCLANG_PATH=$LibclangPath

# Assemble a complete package beside the destination and swap directories, so no
# library or notice from an earlier FFmpeg or dependency set survives a rebuild.
$staging=Join-Path $parent ('.thumbnailers-build.' + [guid]::NewGuid().ToString('N'))
$package=Join-Path $staging 'package'
New-Item -ItemType Directory -Path $package | Out-Null
$cleanStaging=$true
try {
    $nativeScripts=Join-Path $repository 'plugins/image-decoder/native'
    $nativeJobs=if ($env:CARGO_BUILD_JOBS) { [int]$env:CARGO_BUILD_JOBS } else { 2 }
    & (Join-Path $nativeScripts 'bootstrap-windows.ps1') -BuildRoot (Join-Path $BuildRoot 'native-tools') -Offline:$NativeOffline
    & (Join-Path $nativeScripts 'build-windows.ps1') -BuildRoot (Join-Path $BuildRoot 'native-dav1d') -Jobs $nativeJobs -Offline:$NativeOffline
    foreach ($name in $plugins) {
        $project=Join-Path $repository "plugins/$name"
        $env:CARGO_TARGET_DIR=Join-Path $BuildRoot $name
        $arguments=@('build','--manifest-path',(Join-Path $project 'Cargo.toml'),'--config',$ApiConfig,'--locked','--offline')
        if ($Profile -eq 'release') { $arguments+='--release' }
        if ($name -eq 'midnight-thumbnail-video') { $arguments+=@('--features','playback-worker') }
        & cargo @arguments
        if ($LASTEXITCODE -ne 0) { throw "$name build failed" }
        if ($name -eq 'midnight-thumbnail-video') {
            $playback='midnight-video-playback.exe'
            Copy-Item -LiteralPath (Join-Path $env:CARGO_TARGET_DIR "$Profile/$playback") -Destination (Join-Path $package $playback)
        }
        Copy-Item -LiteralPath (Join-Path $env:CARGO_TARGET_DIR "$Profile/$name.exe") -Destination (Join-Path $package "$name.exe")
        $manifest=Get-Content -LiteralPath (Join-Path $project 'thumbnailer.json') -Raw | ConvertFrom-Json
        $manifest.executable="$name.exe"
        [System.IO.File]::WriteAllText((Join-Path $package "$name.json"),($manifest | ConvertTo-Json),[System.Text.UTF8Encoding]::new($false))
        if (Test-Path -LiteralPath (Join-Path $NoticesRoot $name)) {
            $notices=Join-Path $package "notices/$name"
            New-Item -ItemType Directory -Force $notices | Out-Null
            Get-ChildItem -LiteralPath (Join-Path $NoticesRoot $name) | Copy-Item -Destination $notices -Recurse -Force
            if ($name -eq 'midnight-thumbnail-image') {
                Get-ChildItem -LiteralPath (Join-Path $repository 'plugins/image-decoder/notices') |
                    Copy-Item -Destination $notices -Recurse -Force
            }
        } else {
            throw "Missing distribution notices for $name; stage pinned dependency license notices first"
        }
    }
    # The playback executable and the shared FFmpeg libraries need read and execute.
    # The sandbox receives no directory, user content, write or network authority.
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
    $ffmpegLibraries=@('avcodec-63.dll','avformat-63.dll','avutil-61.dll','swscale-10.dll','swresample-7.dll')
    foreach ($name in $ffmpegLibraries) {
        Copy-Item -LiteralPath (Join-Path $FfmpegPrefix "bin/$name") -Destination (Join-Path $package $name)
    }
    foreach ($name in $ffmpegLibraries + @('midnight-video-playback.exe')) {
        $path=Join-Path $package $name
        $acl=Get-Acl -LiteralPath $path
        $acl.AddAccessRule([System.Security.AccessControl.FileSystemAccessRule]::new($sid,
            [System.Security.AccessControl.FileSystemRights]::ReadAndExecute,[System.Security.AccessControl.AccessControlType]::Allow))
        Set-Acl -LiteralPath $path -AclObject $acl
    }
    $notices=Join-Path $package 'notices/ffmpeg-9.0.2'
    New-Item -ItemType Directory -Force $notices | Out-Null
    Get-ChildItem -LiteralPath (Join-Path $FfmpegPrefix 'share/midnight-ffmpeg') | Copy-Item -Destination $notices -Recurse -Force
    Copy-Item -LiteralPath $FfmpegSourceArchive -Destination (Join-Path $notices 'ffmpeg-9.0.2.tar.xz')
    $previous=Join-Path $staging 'previous'
    if (Test-Path -LiteralPath $Destination) { Move-Item -LiteralPath $Destination -Destination $previous }
    try { Move-Item -LiteralPath $package -Destination $Destination }
    catch {
        if (Test-Path -LiteralPath $previous) {
            try { Move-Item -LiteralPath $previous -Destination $Destination }
            catch { $cleanStaging=$false; throw "Previous thumbnail package retained at $previous; restoration failed: $_" }
        }
        throw
    }
    Write-Output "Built installed thumbnail providers: $Destination"
} finally {
    if ($cleanStaging -and (Test-Path -LiteralPath $staging)) {
        # A superseded package whose worker still runs cannot be deleted yet.
        try { Remove-Item -LiteralPath $staging -Recurse -Force }
        catch { Write-Warning "Superseded thumbnail staging was retained: $staging ($_)" }
    }
}
