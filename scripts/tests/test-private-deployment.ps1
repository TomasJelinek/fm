$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$deploy = Join-Path $projectRoot 'scripts/deploy-private.ps1'
$testParent = if ($env:HOST_TEST_INSTALL_ROOT) { $env:HOST_TEST_INSTALL_ROOT } else { $env:USERPROFILE }
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
    foreach ($name in @('basic', 'source', 'markdown')) {
        $package = Join-Path $source "viewers/midnight-viewer-$name"
        New-Item -ItemType Directory -Path "$package/notices" -Force | Out-Null
        foreach ($file in @('plugin.toml', "midnight_viewer_$name.dll", 'LICENSE', 'THIRD-PARTY.md', 'notices/LICENSE')) {
            [IO.File]::WriteAllText((Join-Path $package $file), "fixture $name $file")
        }
    }
    [IO.File]::WriteAllText((Join-Path $source 'unrelated.pdb'), 'must not be deployed')
    $destination = Join-Path $testRoot 'installed'
    & $deploy -SourceDirectory $source -Destination $destination | Out-Null
    if ((Get-FileHash "$destination/me.exe").Hash -ne (Get-FileHash "$source/me.exe").Hash) {
        throw 'Executable contents changed during deployment'
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
    Write-Output 'Private deployment checks passed: copy/hash, notices, protected ACL, existing target, unsafe ancestor, reparse, missing library.'
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
