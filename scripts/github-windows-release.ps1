[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [ValidateSet('Prepare', 'Upload')]
    [string]$Operation
)

# Network orchestration only. Downloaded executables and tag sources never execute.
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $true
$signingRoot = Join-Path $PSScriptRoot '../_build/windows-signing'
$statePath = Join-Path $signingRoot 'release-state.json'

function Invoke-GitHubJson {
    param([string]$Endpoint)
    $result = & gh api -H 'Accept: application/vnd.github+json' -- $Endpoint
    if ($LASTEXITCODE -ne 0) { throw "GitHub read failed: $Endpoint" }
    return ($result | ConvertFrom-Json -Depth 50)
}

function Get-ReleaseState {
    $release = Invoke-GitHubJson "repos/$env:GITHUB_REPOSITORY/releases/tags/$env:RELEASE_TAG"
    if (-not $release.draft -or $release.tag_name -cne $env:RELEASE_TAG) {
        throw 'Only the exact existing draft release can be changed.'
    }
    if ($release.PSObject.Properties['immutable'] -and $release.immutable) {
        throw 'The release is immutable.'
    }
    $assets = @()
    $page = 1
    do {
        $batch = @(Invoke-GitHubJson "repos/$env:GITHUB_REPOSITORY/releases/$($release.id)/assets?per_page=100&page=$page")
        $assets += $batch
        $page += 1
    } while ($batch.Count -eq 100)
    $release.assets = $assets
    return $release
}

function Get-TagRevision {
    $reference = Invoke-GitHubJson "repos/$env:GITHUB_REPOSITORY/git/ref/tags/$env:RELEASE_TAG"
    $target = $reference.object
    for ($depth = 0; $target.type -eq 'tag' -and $depth -lt 8; $depth++) {
        $target = (Invoke-GitHubJson "repos/$env:GITHUB_REPOSITORY/git/tags/$($target.sha)").object
    }
    if ($target.type -ne 'commit' -or $target.sha -cnotmatch '^[0-9a-f]{40}$') {
        throw 'Release tag must resolve to a commit (at most eight annotated tags).'
    }
    $comparison = Invoke-GitHubJson "repos/$env:GITHUB_REPOSITORY/compare/$($target.sha)...$env:DEFAULT_BRANCH"
    if ($comparison.status -notin @('ahead', 'identical')) {
        throw 'Release commit must be reachable from the current default branch.'
    }
    return $target.sha
}

function Find-Asset {
    param($Release, [string]$Name)
    $matches = @($Release.assets | Where-Object { $_.name -ceq $Name })
    if ($matches.Count -gt 1) { throw "Duplicate release asset: $Name" }
    if ($matches.Count -eq 1) { return $matches[0] }
    return $null
}

function Get-Identity {
    param($Asset)
    if ($null -eq $Asset) { return $null }
    return "$($Asset.id):$($Asset.size):$($Asset.updated_at)"
}

function Save-Asset {
    param($Asset, [string]$Directory)
    & gh release download $env:RELEASE_TAG --repo $env:GITHUB_REPOSITORY --pattern $Asset.name --dir $Directory
    if ($LASTEXITCODE -ne 0) { throw "Download failed: $($Asset.name)" }
    $path = Join-Path $Directory $Asset.name
    if ((Get-Item -LiteralPath $path).Length -ne $Asset.size) { throw 'Downloaded asset size changed.' }
    if ($Asset.PSObject.Properties['digest'] -and $Asset.digest) {
        $actual = 'sha256:' + (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($Asset.digest -cne $actual) { throw 'Downloaded asset digest changed.' }
    }
    return $path
}

function Update-Checksums {
    param([string]$Original, [string]$SignedArchive, [string]$Output, [switch]$AllowMissing)
    # Keep every other platform's line byte-for-byte, including its line ending.
    $content = [IO.File]::ReadAllText($Original)
    $name = [IO.Path]::GetFileName($SignedArchive)
    $hash = (Get-FileHash -LiteralPath $SignedArchive -Algorithm SHA256).Hash.ToLowerInvariant()
    $seen = [Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
    $targetCount = 0
    $result = [Text.StringBuilder]::new()
    $consumed = 0
    foreach ($match in [regex]::Matches($content, '[^\r\n]+(?:\r\n|\n|\r|$)')) {
        if ($match.Index -ne $consumed) { throw 'Blank lines in global SHA256SUMS are unsupported.' }
        $consumed += $match.Length
        $line = $match.Value
        $parsed = [regex]::Match($line, '\A([a-fA-F0-9]{64})( [ *])([^\r\n]+)(\r\n|\n|\r|)\z')
        if (-not $parsed.Success) { throw 'Malformed global SHA256SUMS; no assets changed.' }
        $entry = $parsed.Groups[3].Value
        if ($entry.Contains('/') -or $entry.Contains('\') -or -not $seen.Add($entry)) {
            throw 'Global SHA256SUMS must contain unique asset basenames.'
        }
        if ($entry -ceq $name) {
            $targetCount += 1
            [void]$result.Append($hash + $parsed.Groups[2].Value + $entry + $parsed.Groups[4].Value)
        } else { [void]$result.Append($line) }
    }
    if ($consumed -ne $content.Length) { throw 'Blank lines in global SHA256SUMS are unsupported.' }
    if ($targetCount -eq 0 -and $AllowMissing) {
        if ($result.Length -gt 0 -and $content[-1] -notin @("`n", "`r")) {
            [void]$result.Append("`n")
        }
        [void]$result.Append("$hash  $name`n")
    } elseif ($targetCount -ne 1) {
        throw 'Global SHA256SUMS must contain exactly one Windows ZIP entry.'
    }
    [IO.File]::WriteAllText($Output, $result.ToString(), [Text.UTF8Encoding]::new($false))
}

foreach ($name in @('GH_TOKEN', 'GITHUB_REPOSITORY', 'GITHUB_REF', 'DEFAULT_BRANCH', 'RELEASE_TAG')) {
    if ([string]::IsNullOrWhiteSpace([Environment]::GetEnvironmentVariable($name))) {
        throw "Missing required environment variable: $name"
    }
}
if ($env:GITHUB_REF -cne "refs/heads/$env:DEFAULT_BRANCH") {
    throw 'Run this workflow on the default branch.'
}
if ($env:RELEASE_TAG -cnotmatch '^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$') {
    throw 'Expected a v-prefixed numeric major.minor.build version for Windows Installer.'
}
$version = $env:RELEASE_TAG.Substring(1)
$versionParts = $version.Split('.')
if ([bigint]$versionParts[0] -gt 255 -or [bigint]$versionParts[1] -gt 255 -or
    [bigint]$versionParts[2] -gt 65535) {
    throw 'Release version exceeds Windows Installer limits (255.255.65535).'
}
$archiveName = "midnight-explorer-$version-x86_64-pc-windows-msvc.zip"
$installerName = "midnight-explorer-$version-x86_64-pc-windows-msvc.msi"

if ($Operation -eq 'Prepare') {
    foreach ($name in @('AZURE_CLIENT_ID', 'AZURE_TENANT_ID', 'AZURE_SUBSCRIPTION_ID',
            'AZURE_SIGNING_ENDPOINT', 'AZURE_SIGNING_ACCOUNT',
            'AZURE_SIGNING_CERTIFICATE_PROFILE', 'WINDOWS_SIGNING_PUBLISHER')) {
        if ([string]::IsNullOrWhiteSpace([Environment]::GetEnvironmentVariable($name))) {
            throw "Missing required windows-release environment variable: $name"
        }
    }
    if (Test-Path -LiteralPath $signingRoot) { throw 'Signing directory already exists; use a fresh checkout.' }
    $inputDirectory = Join-Path $signingRoot 'input'
    New-Item -ItemType Directory -Path $inputDirectory | Out-Null
    $release = Get-ReleaseState
    $revision = Get-TagRevision
    $archive = Find-Asset $release $archiveName
    if ($null -eq $archive) { throw "Missing exact draft ZIP asset: $archiveName" }
    $checksums = Find-Asset $release 'SHA256SUMS'
    $archivePath = Save-Asset $archive $inputDirectory
    if ($null -ne $checksums) { $null = Save-Asset $checksums $inputDirectory }
    $state = [ordered]@{
        repository = $env:GITHUB_REPOSITORY
        tag = $env:RELEASE_TAG
        revision = $revision
        release_id = $release.id
        archive_name = $archiveName
        assets = @{}
    }
    foreach ($name in @($archiveName, "$archiveName.sha256", "$archiveName.signing.json",
            $installerName, "$installerName.sha256", "$installerName.signing.json", 'SHA256SUMS')) {
        $state.assets[$name] = Get-Identity (Find-Asset $release $name)
    }
    $state | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $statePath -Encoding utf8NoBOM
    & python -B (Join-Path $PSScriptRoot 'windows-release-signing.py') prepare --archive $archivePath `
        --work-dir (Join-Path $signingRoot 'work') --revision $revision --version $version
    if ($LASTEXITCODE -ne 0) { throw 'Archive preparation failed.' }
    exit 0
}

$state = Get-Content -LiteralPath $statePath -Raw | ConvertFrom-Json -AsHashtable
if ($state.repository -cne $env:GITHUB_REPOSITORY -or $state.tag -cne $env:RELEASE_TAG -or
    $state.archive_name -cne $archiveName) { throw 'Signing state belongs to another release.' }
$outputDirectory = Join-Path $signingRoot 'work/output'
$signedArchive = Join-Path $outputDirectory $archiveName
$uploads = @($archiveName, "$archiveName.sha256", "$archiveName.signing.json",
    $installerName, "$installerName.sha256", "$installerName.signing.json")
foreach ($name in $uploads) {
    if (-not (Test-Path -LiteralPath (Join-Path $outputDirectory $name) -PathType Leaf)) {
        throw "Missing signed output: $name"
    }
}
if ($null -ne $state.assets['SHA256SUMS']) {
    Update-Checksums (Join-Path $signingRoot 'input/SHA256SUMS') $signedArchive (Join-Path $outputDirectory 'SHA256SUMS')
    $checksumsPath = Join-Path $outputDirectory 'SHA256SUMS'
    Update-Checksums $checksumsPath (Join-Path $outputDirectory $installerName) $checksumsPath -AllowMissing
    $uploads += 'SHA256SUMS'
}

function Assert-UnchangedRelease {
    $current = Get-ReleaseState
    if ($current.id -ne $state.release_id -or (Get-TagRevision) -cne $state.revision) {
        throw 'Release or tag changed during signing; no further mutations allowed.'
    }
    foreach ($name in $state.assets.Keys) {
        if ((Get-Identity (Find-Asset $current $name)) -cne $state.assets[$name]) {
            throw "Release asset changed during signing: $name"
        }
    }
    return $current
}

foreach ($name in $uploads) {
    $current = Assert-UnchangedRelease
    $previous = Find-Asset $current $name
    if ($null -ne $previous) {
        & gh api --method DELETE -- "repos/$env:GITHUB_REPOSITORY/releases/assets/$($previous.id)"
        if ($LASTEXITCODE -ne 0) { throw "Could not remove draft asset: $name" }
        $state.assets[$name] = $null
    }
    $null = Assert-UnchangedRelease
    # No --clobber: a concurrent asset creation must fail instead of overwriting it.
    & gh release upload $env:RELEASE_TAG (Join-Path $outputDirectory $name) --repo $env:GITHUB_REPOSITORY
    if ($LASTEXITCODE -ne 0) { throw "Draft upload failed: $name; use the retained recovery artifact." }
    $current = Get-ReleaseState
    $uploaded = Find-Asset $current $name
    if ($null -eq $uploaded -or $uploaded.size -ne (Get-Item -LiteralPath (Join-Path $outputDirectory $name)).Length) {
        throw "Uploaded asset missing or wrong size: $name"
    }
    $localHash = (Get-FileHash -LiteralPath (Join-Path $outputDirectory $name) -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($uploaded.PSObject.Properties['digest'] -and $uploaded.digest) {
        if ($uploaded.digest -cne "sha256:$localHash") { throw "Uploaded asset digest mismatch: $name" }
    } else {
        $readbackDirectory = Join-Path $signingRoot "readback/$name"
        New-Item -ItemType Directory -Path $readbackDirectory | Out-Null
        $readback = Save-Asset $uploaded $readbackDirectory
        if ((Get-FileHash -LiteralPath $readback -Algorithm SHA256).Hash.ToLowerInvariant() -cne $localHash) {
            throw "Uploaded asset readback mismatch: $name"
        }
    }
    $state.assets[$name] = Get-Identity $uploaded
}
$null = Assert-UnchangedRelease
Write-Output 'Signed Windows assets replaced on the existing draft; release remains unpublished.'
