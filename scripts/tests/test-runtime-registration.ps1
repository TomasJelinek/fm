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
New-Item -ItemType Directory -Path $testRoot | Out-Null
Protect-Fixture $testRoot
try {
    $source = Join-Path $testRoot 'source'
    New-Item -ItemType Directory -Path $source | Out-Null
    [IO.File]::WriteAllText((Join-Path $source 'me.exe'), 'registered executable fixture')
    [IO.File]::WriteAllText((Join-Path $source 'midnight-elevated-listing.exe'), 'authorization helper fixture')
    foreach ($name in @('basic', 'source', 'markdown')) {
        $package = Join-Path $source "viewers/midnight-viewer-$name"
        New-Item -ItemType Directory -Path $package -Force | Out-Null
        foreach ($file in @('plugin.toml', "midnight_viewer_$name.dll", 'LICENSE', 'THIRD-PARTY.md')) {
            [IO.File]::WriteAllText((Join-Path $package $file), "fixture $name $file")
        }
    }
    $registry = Join-Path $testRoot 'profile/plugin-runtimes'
    $destination = Join-Path $testRoot 'installed-文档'
    & $deploy -SourceDirectory $source -Destination $destination -RegisterRuntime -RegistrationDirectory $registry | Out-Null
    $hash = (Get-FileHash -LiteralPath (Join-Path $source 'me.exe') -Algorithm SHA256).Hash.ToLowerInvariant()
    $recordPath = Join-Path $registry "$hash.toml"
    if (-not (Test-Path -LiteralPath $recordPath -PathType Leaf)) { throw 'Matching registration missing' }
    $record = [IO.File]::ReadAllText($recordPath)
    if ($record -notmatch '(?m)^schema_version = 1$' -or $record -notmatch "(?m)^sha256 = `"$hash`"$") {
        throw 'Registration schema or digest differs'
    }
    $units = (($record -split 'executable_utf16 = \[')[1] -split '\]')[0] -split ','
    $path = -join ($units | ForEach-Object { [char][uint16]$_.Trim() })
    if ($path -cne (Join-Path $destination 'me.exe')) { throw 'Native path changed in registration' }
    if (-not (Get-Acl -LiteralPath $registry).AreAccessRulesProtected) { throw 'Registry ACL is inherited' }
    $second = Join-Path $testRoot 'updated-runtime'
    & $deploy -SourceDirectory $source -Destination $second -RegisterRuntime -RegistrationDirectory $registry | Out-Null
    if ([IO.File]::ReadAllText($recordPath) -ceq $record) { throw 'Registration did not update atomically' }
    if (@(Get-ChildItem -LiteralPath $registry -Force).Count -ne 1) { throw 'Registration temporary file leaked' }

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

    # Default registration authority must not inherit an unsafe AppData tree.
    $originalProfile = $env:USERPROFILE
    $originalLocalData = $env:LOCALAPPDATA
    try {
        $env:USERPROFILE = Join-Path $testRoot 'isolated-user-profile'
        $env:LOCALAPPDATA = $unsafe
        New-Item -ItemType Directory -Path $env:USERPROFILE | Out-Null
        & $deploy -SourceDirectory $source -Destination "$testRoot/default-registration" -RegisterRuntime | Out-Null
        $defaultRecord = Join-Path $env:USERPROFILE ".midnight-explorer/plugin-runtimes/$hash.toml"
        if (-not (Test-Path -LiteralPath $defaultRecord -PathType Leaf)) {
            throw 'Default registration must use the protected profile namespace'
        }
        if ((Get-Acl -LiteralPath $unsafe).Sddl -cne $before) { throw 'AppData ACL was changed' }
    } finally {
        $env:USERPROFILE = $originalProfile
        $env:LOCALAPPDATA = $originalLocalData
    }

    $redirect = Join-Path $testRoot 'registry-junction'
    New-Item -ItemType Junction -Path $redirect -Target $registry | Out-Null
    Refuses { & $deploy -SourceDirectory $source -Destination "$testRoot/redirected" -RegisterRuntime -RegistrationDirectory $redirect } 'registry junction'
    [IO.Directory]::Delete($redirect)
    if (Test-Path -LiteralPath "$testRoot/redirected") { throw 'Redirected registry left a deployment' }
    Write-Output 'Runtime registration checks passed: exact digest/native path, atomic update, private ACL, unsafe/reparse refusal, isolated default authority.'
} finally {
    $resolvedParent = [IO.Path]::GetFullPath($parent).TrimEnd('\')
    if ((Split-Path -Parent ([IO.Path]::GetFullPath($testRoot))) -cne $resolvedParent) { throw 'Unexpected fixture cleanup path' }
    Remove-Item -LiteralPath $testRoot -Recurse -Force
}
