param(
    [Parameter(Mandatory = $true)][ValidateSet('Deny', 'Restore')][string]$Action,
    [Parameter(Mandatory = $true)][string]$FixturePath,
    [string]$OriginalDacl
)
$ErrorActionPreference = 'Stop'
$fixtureRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../../../_build/tmp'))
$absoluteFixture = [IO.Path]::GetFullPath($FixturePath)
if (-not $absoluteFixture.StartsWith($fixtureRoot + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
    throw 'ACL tests accept only their own _build/tmp fixtures'
}
$item = Get-Item -LiteralPath $absoluteFixture -Force
if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
    throw 'ACL fixtures must not be reparse points'
}
$section = [Security.AccessControl.AccessControlSections]::Access
$acl = if ($item.PSIsContainer) {
    [IO.Directory]::GetAccessControl($absoluteFixture, $section)
} else {
    [IO.File]::GetAccessControl($absoluteFixture, $section)
}
function Set-FixtureAcl {
    if ($item.PSIsContainer) {
        [IO.Directory]::SetAccessControl($absoluteFixture, $acl)
    } else {
        [IO.File]::SetAccessControl($absoluteFixture, $acl)
    }
}
if ($Action -eq 'Deny') {
    $original = $acl.GetSecurityDescriptorSddlForm($section)
    $everyone = [Security.Principal.SecurityIdentifier]::new('S-1-1-0')
    $rule = [Security.AccessControl.FileSystemAccessRule]::new($everyone, [Security.AccessControl.FileSystemRights]::ReadData, [Security.AccessControl.AccessControlType]::Deny)
    $acl.AddAccessRule($rule)
    Set-FixtureAcl
    Write-Output $original
} else {
    if ([string]::IsNullOrEmpty($OriginalDacl)) { throw 'Original fixture DACL is required' }
    $acl.SetSecurityDescriptorSddlForm($OriginalDacl, $section)
    Set-FixtureAcl
}
