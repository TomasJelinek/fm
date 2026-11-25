[CmdletBinding(DefaultParameterSetName = 'Payload')]
param(
    [Parameter(Mandatory, ParameterSetName = 'Payload')][string]$Plan,
    [Parameter(Mandatory, ParameterSetName = 'Installer')][string]$Installer,
    [Parameter(Mandatory)][ValidateNotNullOrEmpty()][string]$Publisher,
    [Parameter(ParameterSetName = 'Payload')][switch]$BeforeSigning
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if (-not $IsWindows) { throw 'Authenticode release verification requires Windows.' }

function Get-VerifiedSignature([string]$Path) {
    $signature = Get-AuthenticodeSignature -LiteralPath $Path
    if ($BeforeSigning) {
        if ($signature.Status -eq 'NotSigned') { return $signature }
        if ($signature.Status -ne 'Valid' -or $signature.SignerCertificate.Subject -cne $Publisher) {
            throw "Refusing to replace an invalid or different publisher signature: $Path"
        }
        return $signature
    }
    if ($signature.Status -ne 'Valid' -or $signature.SignatureType -ne 'Authenticode' -or
        $null -eq $signature.SignerCertificate -or
        $signature.SignerCertificate.Subject -cne $Publisher -or
        $null -eq $signature.TimeStamperCertificate) {
        throw "Release requires valid embedded Authenticode, expected publisher and timestamp: $Path ($($signature.Status))"
    }
    return $signature
}

if ($PSCmdlet.ParameterSetName -eq 'Installer') {
    $file = Get-Item -LiteralPath $Installer
    if ($file.Extension -cne '.msi' -or ($file.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
        throw 'Expected an ordinary MSI file.'
    }
    $signature = Get-VerifiedSignature $file.FullName
    $digest = (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
    [IO.File]::WriteAllText($file.FullName + '.sha256', "$digest  $($file.Name)`n")
    $receipt = [ordered]@{
        schema = 1; file = $file.Name; sha256 = $digest; publisher = $Publisher
        thumbprint = $signature.SignerCertificate.Thumbprint
        timestamp_thumbprint = $signature.TimeStamperCertificate.Thumbprint
    }
    [IO.File]::WriteAllText($file.FullName + '.signing.json', ($receipt | ConvertTo-Json) + "`n")
    return
}

$planFile = Get-Item -LiteralPath $Plan
$data = Get-Content -LiteralPath $planFile.FullName -Raw | ConvertFrom-Json
$root = [IO.Path]::GetFullPath($data.root)
$expectedRoot = Join-Path $planFile.DirectoryName "package/midnight-explorer-$($data.version)-x86_64-pc-windows-msvc"
if ($data.schema -ne 1 -or $root -cne [IO.Path]::GetFullPath($expectedRoot) -or
    $data.files.Count -ne 15 -or @($data.files | Select-Object -Unique).Count -ne 15) {
    throw 'Unexpected signing plan schema, root or target count.'
}
$records = @()
$thumbprints = @{}
foreach ($name in $data.files) {
    if ($name -match '[\\:\x00-\x1f]' -or $name -match '(^|/)\.\.?(/|$)' -or
        [IO.Path]::IsPathRooted($name)) { throw "Invalid signing target: $name" }
    $path = [IO.Path]::GetFullPath((Join-Path $root $name))
    if (-not $path.StartsWith($root + [IO.Path]::DirectorySeparatorChar, [StringComparison]::Ordinal)) {
        throw "Signing target escapes package: $name"
    }
    $item = Get-Item -LiteralPath $path
    if ($item.PSIsContainer -or ($item.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
        throw "Signing target must be a regular file: $name"
    }
    $signature = Get-VerifiedSignature $path
    if (-not $BeforeSigning) {
        $thumbprints[$name] = $signature.SignerCertificate.Thumbprint
        $records += [ordered]@{
            path = $name; sha256 = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant()
            thumbprint = $signature.SignerCertificate.Thumbprint
            timestamp_thumbprint = $signature.TimeStamperCertificate.Thumbprint
        }
    }
}
if (-not $BeforeSigning) {
    $html = 'viewers/midnight-viewer-html/midnight-viewer-html'
    if ($thumbprints[$html + '.exe'] -cne $thumbprints[$html + '.dll']) {
        throw 'CEF launcher and companion DLL must have the same primary signing certificate.'
    }
    $receipt = [ordered]@{
        schema = 1; publisher = $Publisher; files = $records
        plan_sha256 = (Get-FileHash -LiteralPath $planFile.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
    }
    [IO.File]::WriteAllText((Join-Path $planFile.DirectoryName 'verification.json'),
        ($receipt | ConvertTo-Json -Depth 8) + "`n")
}
