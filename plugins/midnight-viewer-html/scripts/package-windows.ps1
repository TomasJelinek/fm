param(
    [Parameter(Mandatory=$true)][string]$SdkConfig,
    [string]$TargetDirectory,
    [string]$OutputDirectory,
    [switch]$Release
)
$ErrorActionPreference = 'Stop'
$project = Split-Path $PSScriptRoot -Parent
$repository = Split-Path (Split-Path $project -Parent) -Parent
if (-not $TargetDirectory) { $TargetDirectory = Join-Path $repository '_build/html-viewer/provider-target' }
$cefStage = Join-Path $TargetDirectory 'cef-sdk'
$package = if ($OutputDirectory) { [IO.Path]::GetFullPath($OutputDirectory) } else { Join-Path $TargetDirectory 'package' }
$configuration = if ($Release) { 'release' } else { 'debug' }
New-Item -ItemType Directory -Force $package | Out-Null
$required = @('libcef.dll','chrome_elf.dll','icudtl.dat','resources.pak','chrome_100_percent.pak',
    'chrome_200_percent.pak','v8_context_snapshot.bin','dxcompiler.dll',
    'vk_swiftshader.dll','vk_swiftshader_icd.json','vulkan-1.dll')
foreach ($file in $required) {
    Copy-Item -LiteralPath (Join-Path $cefStage $file) -Destination $package -Force
}
Copy-Item -LiteralPath (Join-Path $cefStage 'locales') -Destination $package -Recurse -Force
Copy-Item -LiteralPath (Join-Path $cefStage 'bootstrap.exe') -Destination (Join-Path $package 'midnight-viewer-html.exe') -Force
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
Copy-Item -LiteralPath (Join-Path $project 'THIRD_PARTY.md') -Destination $package -Force
Copy-Item -LiteralPath (Join-Path $project 'README.md') -Destination $package -Force
foreach ($file in @('LICENSE.txt','CREDITS.html')) {
    Copy-Item -LiteralPath (Join-Path $cefStage $file) -Destination $package -Force
}
$metadata = & cargo metadata --manifest-path (Join-Path $project 'Cargo.toml') --config $SdkConfig --locked --offline --format-version 1
if ($LASTEXITCODE -ne 0) { throw 'Dependency metadata unavailable' }
$metadata | Set-Content -LiteralPath (Join-Path $TargetDirectory 'dependency-metadata.json')
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
        $relative = [IO.Path]::GetRelativePath($source, $file.FullName)
        $outputFile = Join-Path $destination $relative
        New-Item -ItemType Directory -Force (Split-Path $outputFile -Parent) | Out-Null
        Copy-Item -LiteralPath $file.FullName -Destination $outputFile -Force
    }
}
$hashes = Get-ChildItem -LiteralPath $package -File -Recurse | Where-Object { $_.Name -ne 'runtime-files.json' } | ForEach-Object {
    @{path=[IO.Path]::GetRelativePath($package,$_.FullName).Replace('\','/');bytes=$_.Length;sha256=(Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant()}
}
@{cef='156.0.26+g8ea0c73+chromium-156.0.8078.88';archive_sha256='2b0aa070793a6bb858b737f5edf9cb01a3ef7b9adcad5efe1e08affaf250f809';files=@($hashes)} |
    ConvertTo-Json -Depth 6 | Set-Content -LiteralPath (Join-Path $package 'runtime-files.json')
Write-Output $package
