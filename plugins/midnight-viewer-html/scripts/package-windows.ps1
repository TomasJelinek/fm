param(
    [Parameter(Mandatory=$true)][string]$SdkConfig,
    [string]$TargetDirectory,
    [string]$CefDirectory,
    [string]$CefReceipt,
    [string]$OutputDirectory,
    [switch]$Release
)
$ErrorActionPreference = 'Stop'
$project = Split-Path $PSScriptRoot -Parent
$repository = Split-Path (Split-Path $project -Parent) -Parent
if (-not $TargetDirectory) { $TargetDirectory = Join-Path $repository '_build/html-viewer/provider-target' }
# Resolve caller paths once against the PowerShell location: .NET methods such as
# [IO.File]::WriteAllText and [IO.Path]::GetFullPath use the process working directory,
# which Set-Location does not change.
function Resolve-CallerPath([string]$Path) {
    [IO.Path]::GetFullPath($ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($Path))
}
$TargetDirectory = Resolve-CallerPath $TargetDirectory
$cefStage = if ($CefDirectory) { Resolve-CallerPath $CefDirectory } else { Join-Path $TargetDirectory 'cef-sdk' }
$package = if ($OutputDirectory) { Resolve-CallerPath $OutputDirectory } else { Join-Path $TargetDirectory 'package' }
$configuration = if ($Release) { 'release' } else { 'debug' }

function Get-RelativePath([string]$Root, [string]$Path) {
    $prefix = [IO.Path]::GetFullPath($Root).TrimEnd('\', '/') + [IO.Path]::DirectorySeparatorChar
    $full = [IO.Path]::GetFullPath($Path)
    if (-not $full.StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase)) { throw "Path outside ${Root}: $Path" }
    return $full.Substring($prefix.Length)
}

# Every shipped CEF file must match the SDK receipt (the verified SDK's
# .verified.json, or the staged distribution's .staged.json). The package hashes
# below therefore check the copies themselves at no extra cost.
$verifiedFiles = $null
if ($CefReceipt) {
    $verifiedFiles = (Get-Content -LiteralPath $CefReceipt -Raw | ConvertFrom-Json).files
    if ($null -eq $verifiedFiles) { throw "CEF SDK receipt lists no files: $CefReceipt" }
}
$cefSources = @{}
New-Item -ItemType Directory -Force $package | Out-Null
$required = @('libcef.dll','chrome_elf.dll','icudtl.dat','resources.pak','chrome_100_percent.pak',
    'chrome_200_percent.pak','v8_context_snapshot.bin','dxcompiler.dll',
    'vk_swiftshader.dll','vk_swiftshader_icd.json','vulkan-1.dll')
foreach ($file in $required) {
    Copy-Item -LiteralPath (Join-Path $cefStage $file) -Destination $package -Force
    $cefSources[$file] = $file
}
$localeDirectory = New-Item -ItemType Directory -Force (Join-Path $package 'locales')
if ($localeDirectory.Attributes -band [IO.FileAttributes]::ReparsePoint) { throw 'Unexpected linked locale directory' }
# Direct standalone builds may reuse a package from a prior run. Remove only exact
# copies of unused packs from this SDK. The normal build already stages a fresh tree.
foreach ($item in Get-ChildItem -LiteralPath $localeDirectory.FullName -Force) {
    if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint -or $item.PSIsContainer) {
        throw "Unexpected entry in HTML locale directory: $($item.Name)"
    }
    if ($item.Name -ceq 'en-US.pak') { continue }
    $original = Join-Path (Join-Path $cefStage 'locales') $item.Name
    if ($item.Extension -ne '.pak' -or -not (Test-Path -LiteralPath $original -PathType Leaf) -or
        (Get-FileHash -LiteralPath $original).Hash -ne (Get-FileHash -LiteralPath $item.FullName).Hash) {
        throw "Unrecognized file in HTML locale directory: $($item.Name)"
    }
    Remove-Item -LiteralPath $item.FullName
}
Copy-Item -LiteralPath (Join-Path $cefStage 'locales/en-US.pak') -Destination $localeDirectory.FullName -Force
$cefSources['locales/en-US.pak'] = 'locales/en-US.pak'
Copy-Item -LiteralPath (Join-Path $cefStage 'bootstrap.exe') -Destination (Join-Path $package 'midnight-viewer-html.exe') -Force
$cefSources['midnight-viewer-html.exe'] = 'bootstrap.exe'
Copy-Item -LiteralPath (Join-Path $TargetDirectory "$configuration/midnight_viewer_html.dll") -Destination (Join-Path $package 'midnight-viewer-html.dll') -Force
$bootstrap = Get-AuthenticodeSignature -LiteralPath (Join-Path $package 'midnight-viewer-html.exe')
$client = Get-AuthenticodeSignature -LiteralPath (Join-Path $package 'midnight-viewer-html.dll')
if ($bootstrap.Status -ne 'NotSigned' -or $client.Status -ne 'NotSigned') {
    if ($bootstrap.Status -ne 'Valid' -or $client.Status -ne 'Valid' -or
        $bootstrap.SignerCertificate.Thumbprint -ne $client.SignerCertificate.Thumbprint) {
        throw 'CEF sandbox requires both bootstrap/client unsigned or valid matching primary signatures'
    }
}
Copy-Item -LiteralPath (Join-Path $project 'web-viewer.json') -Destination $package -Force
# The provider's own licence (GPL-3.0-or-later); CEF's LICENSE.txt below covers only CEF.
Copy-Item -LiteralPath (Join-Path $project 'LICENSE') -Destination $package -Force
# Markdown names in a package are lowercase, with '-' for '_' (THIRD_PARTY.md is
# third-party.md). A reused package's copies go first: NTFS keeps the case of a file that
# a copy replaces.
foreach ($name in @('THIRD_PARTY.md', 'README.md')) {
    $packaged = $name.ToLowerInvariant().Replace('_', '-')
    foreach ($earlier in @($name, $packaged)) {
        Remove-Item -LiteralPath (Join-Path $package $earlier) -Force -ErrorAction Ignore
    }
    Copy-Item -LiteralPath (Join-Path $project $name) -Destination (Join-Path $package $packaged)
}
foreach ($file in @('LICENSE.txt','CREDITS.html')) {
    Copy-Item -LiteralPath (Join-Path $cefStage $file) -Destination $package -Force
    $cefSources[$file] = $file
}
# Cargo prints UTF-8, but PowerShell decodes native output with the console encoding
# (an OEM code page by default), which would garble non-ASCII dependency paths such as
# a CARGO_HOME below a user profile. The caller's console encoding is restored.
$consoleEncoding = [Console]::OutputEncoding
[Console]::OutputEncoding = New-Object Text.UTF8Encoding $false
try {
    $metadata = & cargo metadata --manifest-path (Join-Path $project 'Cargo.toml') --config $SdkConfig --locked --offline --format-version 1
    if ($LASTEXITCODE -ne 0) { throw 'Dependency metadata unavailable' }
} finally {
    [Console]::OutputEncoding = $consoleEncoding
}
$metadata = $metadata -join "`n"
# UTF-8 without BOM, as Cargo wrote it (Windows PowerShell's Set-Content would use ANSI).
[IO.File]::WriteAllText((Join-Path $TargetDirectory 'dependency-metadata.json'), $metadata + "`n",
    (New-Object Text.UTF8Encoding $false))
$metadata = $metadata | ConvertFrom-Json
$notices = Join-Path $package 'notices'
New-Item -ItemType Directory -Force $notices | Out-Null
Copy-Item -LiteralPath (Join-Path $TargetDirectory 'dependency-metadata.json') -Destination $package -Force
foreach ($dependency in $metadata.packages) {
    if ($dependency.name -like 'midnight-*') { continue }
    $source = Split-Path $dependency.manifest_path -Parent
    $destination = Join-Path $notices "$($dependency.name)-$($dependency.version)"
    $files = Get-ChildItem -LiteralPath $source -Recurse -File |
        Where-Object { $_.Name -match '^(LICENSE|LICENCE|COPYING|COPYRIGHT|NOTICE)([._-]|$)' }
    if ($dependency.name -in @('cef','cef-dll-sys')) {
        # Both exact published crates omit the workspace-root license files. Preserve
        # the files obtained from their .cargo_vcs_info.json source revision.
        $source = Join-Path $project 'notices/cef-rs-9aa78c450489eddb828eb982eaee6c104a89d348'
        $files = Get-ChildItem -LiteralPath $source -File
    }
    $omittedNotices = @{
        'download-cef'='cef-rs-207bdd2917c038283635022ffcf7767496bcc7e6'
        'objc2'='objc2-d7d2fa23ceaa5e6096c923b081040e5d81b3b9df'
        'objc2-encode'='objc2-8d214f5477365ffcbcbb7de058c86ed9a518efb7'
    }
    if ($omittedNotices.ContainsKey($dependency.name)) {
        $source = Join-Path $project ('notices/' + $omittedNotices[$dependency.name])
        $files = Get-ChildItem -LiteralPath $source -File
    }
    if (-not $files) { throw "No pinned license text found for $($dependency.name)" }
    foreach ($file in $files) {
        $outputFile = Join-Path $destination (Get-RelativePath $source $file.FullName)
        if ($file.Extension -eq '.md') {
            # Lowercase, as third-party.md above.
            $outputFile = Join-Path (Split-Path $outputFile -Parent) $file.Name.ToLowerInvariant().Replace('_', '-')
            Remove-Item -LiteralPath $outputFile -Force -ErrorAction Ignore
        }
        New-Item -ItemType Directory -Force (Split-Path $outputFile -Parent) | Out-Null
        Copy-Item -LiteralPath $file.FullName -Destination $outputFile -Force
    }
}
$hashes = Get-ChildItem -LiteralPath $package -File -Recurse | Where-Object { $_.Name -ne 'runtime-files.json' } | ForEach-Object {
    $relative = (Get-RelativePath $package $_.FullName).Replace('\','/')
    $sha256 = (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($null -ne $verifiedFiles -and $cefSources.ContainsKey($relative) -and
        $verifiedFiles.($cefSources[$relative]) -cne $sha256) {
        throw "Packaged CEF file differs from its SDK receipt: $relative"
    }
    @{path=$relative;bytes=$_.Length;sha256=$sha256}
}
@{cef='156.0.26+g8ea0c73+chromium-156.0.8078.88';archive_sha256='2b0aa070793a6bb858b737f5edf9cb01a3ef7b9adcad5efe1e08affaf250f809';files=@($hashes)} |
    ConvertTo-Json -Depth 6 | Set-Content -LiteralPath (Join-Path $package 'runtime-files.json')
Write-Output $package
