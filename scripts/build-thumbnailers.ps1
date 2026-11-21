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
# Endpoint protection briefly holds files that were just written. While it does, Windows
# refuses to rename them or a folder above them: access denied (5), a sharing (32) or a
# lock (33) violation, or File.Replace cannot remove the replaced file (1175, both files
# unchanged; its partial outcomes 1176/1177 are not retried). A rename of fresh output
# retries only those errors, with a short backoff for about 2 s, and then fails with the
# first error. This script always uses the 2000 ms default; tests shorten it with
# -TimeoutMilliseconds or $PSDefaultParameterValues.
function Invoke-FreshRename {
    [CmdletBinding()]
    param([Parameter(Mandatory)][scriptblock]$Rename, [int]$TimeoutMilliseconds = 2000)
    $firstHold = $null
    $holdDelay = 20
    while ($true) {
        try { & $Rename; return }
        catch {
            $holdCode = 0
            for ($failure = $_.Exception; $failure -and -not $holdCode; $failure = $failure.InnerException) {
                if ((($failure.HResult -shr 16) -band 0x1FFF) -eq 7) { $holdCode = $failure.HResult -band 0xFFFF }
            }
            if ($holdCode -notin @(5, 32, 33, 1175)) { throw }
            if (-not $firstHold) { $firstHold = $_; $holdWatch = [Diagnostics.Stopwatch]::StartNew() }
            $holdLeft = $TimeoutMilliseconds - $holdWatch.ElapsedMilliseconds
            if ($holdLeft -le 0) { throw $firstHold }
            Start-Sleep -Milliseconds ([Math]::Max(1, [Math]::Min($holdDelay, $holdLeft)))
            $holdDelay = [Math]::Min(2 * $holdDelay, 200)
        }
    }
}

# The SID of the restricted workers' AppContainer, MidnightExplorer.RestrictedWorker.v1, as
# the unpackaged application gets it: S-1-15-2 and the first 28 bytes of the SHA-256 of the
# lowercase name in UTF-16LE as seven little-endian numbers. DeriveAppContainerSidFromAppContainerName
# is not used: inside a packaged PowerShell (the Microsoft Store pwsh) it returns a child
# AppContainer of that package instead, and a worker cannot load a library granted to that SID.
# deploy-private.ps1 has the same function; scripts/tests/test_restricted_worker_identity.py
# checks both against the SID the workers run with.
function Get-RestrictedWorkerSid {
    $name = 'MidnightExplorer.RestrictedWorker.v1'.ToLowerInvariant()
    $sha = [Security.Cryptography.SHA256]::Create()
    try { $hash = $sha.ComputeHash([Text.Encoding]::Unicode.GetBytes($name)) } finally { $sha.Dispose() }
    $parts = foreach ($index in 0..6) { [BitConverter]::ToUInt32($hash, 4 * $index) }
    [Security.Principal.SecurityIdentifier]::new('S-1-15-2-' + ($parts -join '-'))
}

# Resolve caller paths against the PowerShell location, not the process directory.
$resolve={ param($path) [System.IO.Path]::GetFullPath($ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($path)) }
$ApiConfig=& $resolve $ApiConfig
$Destination=& $resolve $Destination
$BuildRoot=& $resolve $BuildRoot
$NoticesRoot=& $resolve $NoticesRoot
$FfmpegPrefix=& $resolve $FfmpegPrefix
$FfmpegSourceArchive=& $resolve $FfmpegSourceArchive
$LibclangPath=& $resolve $LibclangPath
# Generated output lies below _build: the checkout's own for a path in the checkout,
# otherwise the first _build of a custom target directory. That _build and everything
# below it must not be a reparse point (symlink or junction). The checkout and the
# folders above it may be links, a subst drive or hidden folders such as AppData.
$checkout=[IO.Path]::GetFullPath($repository).TrimEnd('\')
foreach ($path in @($Destination,$BuildRoot)) {
    $ancestors=@()
    $inside=$false
    for ($part=$path.TrimEnd('\'); $part; $part=[IO.Path]::GetDirectoryName($part)) {
        # TrimEnd also matches a checkout at a drive root, such as a subst drive's M:\.
        if ($part.TrimEnd('\') -eq $checkout) { $inside=$true; break }
        $ancestors+=$part
    }
    $anchor=0
    for ($index=$ancestors.Count - 1; $index -gt 0; $index--) {
        if ([IO.Path]::GetFileName($ancestors[$index]) -eq '_build') { $anchor=$index; break }
        if ($inside) { break }
    }
    if ($anchor -eq 0) { throw "Generated thumbnail output must be under _build: $path" }
    foreach ($part in $ancestors[0..$anchor]) {
        $item=Get-Item -LiteralPath $part -Force -ErrorAction Ignore
        if ($item -and ($item.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
            throw "Generated thumbnail output must not traverse reparse points: $part"
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
    $nativeJobs=if ($env:CARGO_BUILD_JOBS) { [int]$env:CARGO_BUILD_JOBS } else { [Environment]::ProcessorCount }
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
            # The Media Foundation helpers decode what the bundled FFmpeg profile cannot. They
            # are built without FFmpeg, so they never load the libraries beside them.
            $system=@('build','--manifest-path',(Join-Path $project 'Cargo.toml'),'--config',$ApiConfig,'--locked','--offline',
                '--no-default-features','--features','system,playback-worker')
            if ($Profile -eq 'release') { $system+='--release' }
            & cargo @system
            if ($LASTEXITCODE -ne 0) { throw "$name Media Foundation helper build failed" }
            foreach ($helper in @('midnight-thumbnail-video-system.exe','midnight-video-playback-system.exe')) {
                Copy-Item -LiteralPath (Join-Path $env:CARGO_TARGET_DIR "$Profile/$helper") -Destination (Join-Path $package $helper)
            }
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
    # The restricted workers' loader opens the shared FFmpeg libraries (and the playback
    # executables are granted alike): read and execute for the workers' AppContainer only.
    # The sandbox receives no directory, user content, write or network authority.
    # The thumbnail executables get no grant.
    $sid=Get-RestrictedWorkerSid
    $ffmpegLibraries=@('avcodec-63.dll','avformat-63.dll','avutil-61.dll','swscale-10.dll','swresample-7.dll')
    foreach ($name in $ffmpegLibraries) {
        Copy-Item -LiteralPath (Join-Path $FfmpegPrefix "bin/$name") -Destination (Join-Path $package $name)
    }
    foreach ($name in $ffmpegLibraries + @('midnight-video-playback.exe','midnight-video-playback-system.exe')) {
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
    # Directory.Move is one rename that leaves nothing behind when it is refused; Move-Item
    # would then move item by item and leave a created destination that a retry or the
    # restoration below would nest the package in.
    if (Test-Path -LiteralPath $Destination) {
        # Retried: endpoint protection may still hold files of a package built moments ago.
        Invoke-FreshRename { [IO.Directory]::Move($Destination, $previous) }
    }
    # Retried: endpoint protection may still hold the files just assembled.
    try { Invoke-FreshRename { [IO.Directory]::Move($package, $Destination) } }
    catch {
        if (Test-Path -LiteralPath $previous) {
            # Retried like the publication: the previous package returns to its place.
            try { Invoke-FreshRename { [IO.Directory]::Move($previous, $Destination) } }
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
