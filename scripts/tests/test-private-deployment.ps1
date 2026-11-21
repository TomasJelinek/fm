$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$deploy = Join-Path $projectRoot 'scripts/deploy-private.ps1'
$testParent = if ($env:HOST_TEST_INSTALL_ROOT) { $env:HOST_TEST_INSTALL_ROOT } else { Join-Path $env:USERPROFILE '_build' }
New-Item -ItemType Directory -Path $testParent -Force | Out-Null
$testRoot = Join-Path $testParent ('deployment-test-' + [guid]::NewGuid().ToString('N'))
$source = Join-Path $projectRoot ('_build/tmp/deployment-source-' + [guid]::NewGuid().ToString('N'))
$sid = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
function Protect-NewFixture([string]$Path) {
    $acl = New-Object Security.AccessControl.DirectorySecurity
    $acl.SetSecurityDescriptorSddlForm("D:P(A;OICI;FA;;;$sid)(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)",
        [Security.AccessControl.AccessControlSections]::Access)
    Set-Acl -LiteralPath $Path -AclObject $acl
}
function Expect-Refusal([scriptblock]$Action, [string]$Reason) {
    $refused = $false
    try { & $Action | Out-Null } catch { $refused = $true }
    if (-not $refused) { throw "Expected refusal: $Reason" }
}
New-Item -ItemType Directory -Path $testRoot, $source | Out-Null
Protect-NewFixture $testRoot
try {
    [IO.File]::WriteAllText((Join-Path $source 'me.exe'), 'executable fixture')
    [IO.File]::WriteAllText((Join-Path $source 'midnight-elevated-listing.exe'), 'authorization helper fixture')
    foreach ($name in @('basic', 'source', 'markdown')) {
        $package = Join-Path $source "viewers/midnight-viewer-$name"
        New-Item -ItemType Directory -Path "$package/notices" -Force | Out-Null
        foreach ($file in @('plugin.toml', "midnight_viewer_$name.dll", 'LICENSE', 'THIRD-PARTY.md', 'notices/LICENSE')) {
            [IO.File]::WriteAllText((Join-Path $package $file), "fixture $name $file")
        }
    }
    [IO.File]::WriteAllText((Join-Path $source 'unrelated.pdb'), 'must not be deployed')
    $destination = Join-Path $testRoot 'installed'
    $output = @(& $deploy -SourceDirectory $source -Destination $destination) -join "`n"
    if ((Get-FileHash "$destination/me.exe").Hash -ne (Get-FileHash "$source/me.exe").Hash) {
        throw 'Executable contents changed during deployment'
    }
    # The Administrator helper is accepted only from a folder standard users cannot
    # change, so a private installation must neither contain nor require it.
    if (Test-Path -LiteralPath "$destination/midnight-elevated-listing.exe") {
        throw 'Administrator authorization helper deployed to a private installation'
    }
    if ($output -notmatch 'Retry with authorization is not available from a private installation') {
        throw 'Deployment did not explain that authorized listing needs an administrator-managed installation'
    }
    if (Test-Path -LiteralPath "$destination/unrelated.pdb") { throw 'Unrelated artifact copied' }
    foreach ($name in @('basic', 'source', 'markdown')) {
        if (-not (Test-Path -LiteralPath "$destination/viewers/midnight-viewer-$name/notices/LICENSE")) {
            throw 'Plugin notices were omitted'
        }
    }
    foreach ($file in Get-ChildItem -LiteralPath "$source/viewers" -File -Recurse) {
        $relative = $file.FullName.Substring($source.Length + 1)
        if ((Get-FileHash -LiteralPath $file.FullName).Hash -ne
            (Get-FileHash -LiteralPath (Join-Path $destination $relative)).Hash) {
            throw "Deployed package hash differs: $relative"
        }
    }
    if (-not (Get-Acl -LiteralPath $destination).AreAccessRulesProtected) { throw 'Installation DACL is not protected' }

    # The restricted video workers load these files as the AppContainer that the unpackaged
    # application starts them in. Its SID is fixed (crates/midnight-worker-platform pins it);
    # a grant to any other SID, such as the child AppContainer that a packaged PowerShell
    # derives for the same name, leaves the helpers unable to load FFmpeg.
    $workerSid = 'S-1-15-2-210492624-3469538162-618285203-1868907-2672626540-1166629363-1118814998'
    $videoCode = @('avcodec-63.dll', 'avformat-63.dll', 'avutil-61.dll', 'swscale-10.dll', 'swresample-7.dll',
        'midnight-video-playback.exe', 'midnight-video-playback-system.exe')
    # The thumbnail helpers load nothing the sandbox must read: no grant, like the bundled one.
    $videoPrograms = @('midnight-thumbnail-video.exe', 'midnight-thumbnail-video-system.exe')
    $thumbnailers = Join-Path $source 'thumbnailers'
    New-Item -ItemType Directory -Path $thumbnailers | Out-Null
    [IO.File]::WriteAllText((Join-Path $thumbnailers 'midnight-thumbnail-video.json'), (@{
        protocol = 1; id = 'midnight-video'; version = 'ffmpeg-9.0.2-playback-r4'
        executable = 'midnight-thumbnail-video.exe'; extensions = @('mp4'); video = $true
    } | ConvertTo-Json))
    foreach ($file in $videoCode + $videoPrograms) {
        [IO.File]::WriteAllText((Join-Path $thumbnailers $file), "fixture $file")
    }
    & $deploy -SourceDirectory $source -Destination "$testRoot/video" -IncludeThumbnailers | Out-Null
    foreach ($file in $videoCode + $videoPrograms) {
        $grants = @([regex]::Matches((Get-Acl -LiteralPath "$testRoot/video/thumbnailers/$file").Sddl,
            '\(A;([^;]*);([^;]*);;;(S-1-15-2-[0-9-]+)\)') | ForEach-Object { $_.Value })
        $expected = if ($file -in $videoCode) { @("(A;;0x1200a9;;;$workerSid)") } else { @() }
        if (($grants -join ',') -cne ($expected -join ',')) {
            throw "Video code grants differ: $file has '$($grants -join ', ')', expected '$($expected -join ', ')'"
        }
    }
    # A runtime without the Media Foundation helpers is incomplete.
    Remove-Item -LiteralPath (Join-Path $thumbnailers 'midnight-thumbnail-video-system.exe')
    Expect-Refusal { & $deploy -SourceDirectory $source -Destination "$testRoot/video-incomplete" -IncludeThumbnailers } 'incomplete video runtime'
    Remove-Item -LiteralPath $thumbnailers -Recurse

    Remove-Item -LiteralPath (Join-Path $source 'midnight-elevated-listing.exe')
    & $deploy -SourceDirectory $source -Destination "$testRoot/without-helper" | Out-Null
    if (-not (Test-Path -LiteralPath "$testRoot/without-helper/me.exe" -PathType Leaf)) {
        throw 'A build input without the authorization helper was not deployed'
    }
    $before = (Get-Acl -LiteralPath $destination).Sddl
    Expect-Refusal { & $deploy -SourceDirectory $source -Destination $destination } 'existing destination'
    if ((Get-Acl -LiteralPath $destination).Sddl -ne $before) { throw 'Existing destination ACL changed' }

    $unsafeParent = Join-Path $testRoot 'unsafe-parent'
    New-Item -ItemType Directory -Path $unsafeParent | Out-Null
    $unsafeAcl = Get-Acl -LiteralPath $unsafeParent
    $unsafeAcl.SetSecurityDescriptorSddlForm("D:P(A;OICI;FA;;;$sid)(A;OICI;FA;;;WD)",
        [Security.AccessControl.AccessControlSections]::Access)
    Set-Acl -LiteralPath $unsafeParent -AclObject $unsafeAcl
    $before = (Get-Acl -LiteralPath $unsafeParent).Sddl
    Expect-Refusal { & $deploy -SourceDirectory $source -Destination "$unsafeParent/child" } 'shared writable ancestor'
    if ((Get-Acl -LiteralPath $unsafeParent).Sddl -ne $before -or (Test-Path "$unsafeParent/child")) {
        throw 'Unsafe parent was modified'
    }
    $redirect = Join-Path $source 'viewers/midnight-viewer-basic/notices/redirect'
    New-Item -ItemType Junction -Path $redirect -Target $testRoot | Out-Null
    Expect-Refusal { & $deploy -SourceDirectory $source -Destination "$testRoot/reparse" } 'source junction'
    [IO.Directory]::Delete($redirect)
    if (Test-Path "$testRoot/reparse") { throw 'Refused source left an installation' }

    $library = Join-Path $source 'viewers/midnight-viewer-basic/midnight_viewer_basic.dll'
    Remove-Item -LiteralPath $library
    Expect-Refusal { & $deploy -SourceDirectory $source -Destination "$testRoot/missing" } 'missing package library'
    if (Test-Path "$testRoot/missing") { throw 'Missing package left an installation' }
    Write-Output 'Private deployment checks passed: copy/hash, administrator helper omitted, notices, protected ACL, restricted worker library grant, existing target, unsafe ancestor, reparse, missing library.'
} finally {
    # Both absolute paths are fresh fixtures created above, never a caller-provided directory.
    if ((Split-Path -Parent $testRoot) -ne [IO.Path]::GetFullPath($testParent).TrimEnd('\')) {
        throw 'Unexpected test cleanup path'
    }
    Remove-Item -LiteralPath $testRoot -Recurse -Force
    if (-not $source.StartsWith((Join-Path $projectRoot '_build/tmp'), [StringComparison]::OrdinalIgnoreCase)) {
        throw 'Unexpected source cleanup path'
    }
    Remove-Item -LiteralPath $source -Recurse -Force
}
