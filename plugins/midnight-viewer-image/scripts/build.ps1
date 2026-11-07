param(
    [Parameter(Mandatory)][string]$SdkConfig,
    [Parameter(Mandatory)][string]$BuildRoot,
    [string]$OutputDirectory,
    [ValidateSet('debug','release')][string]$Profile = 'release'
)
$ErrorActionPreference = 'Stop'
$pluginRoot = Split-Path -Parent $PSScriptRoot
$SdkConfig = (Resolve-Path -LiteralPath $SdkConfig).Path
$BuildRoot = [IO.Path]::GetFullPath($BuildRoot)
if (-not $OutputDirectory) { $OutputDirectory = Join-Path $BuildRoot 'package/midnight-viewer-image' }
$OutputDirectory = [IO.Path]::GetFullPath($OutputDirectory)
foreach ($path in @($BuildRoot, $OutputDirectory)) {
    if ($path -notmatch '[/\\]_build([/\\]|$)') { throw 'Generated output must be under _build' }
    for ($part = $path; $part; $part = Split-Path -Parent $part) {
        if ((Test-Path -LiteralPath $part) -and ((Get-Item -LiteralPath $part).Attributes -band [IO.FileAttributes]::ReparsePoint)) {
            throw 'Build output must not traverse reparse points'
        }
    }
}
New-Item -ItemType Directory -Force $BuildRoot, $OutputDirectory | Out-Null
$nativeTarget = ((& rustc -vV | Select-String '^host: ').ToString() -replace '^host: ', '').Trim()
if ($LASTEXITCODE -ne 0) { throw 'rustc target discovery failed' }
if ($nativeTarget -ne 'x86_64-pc-windows-msvc') { throw "Unsupported Windows target: $nativeTarget" }
$executable = 'midnight-viewer-image.exe'
$arguments = @('build', '--config', $SdkConfig, '--manifest-path', (Join-Path $pluginRoot 'Cargo.toml'),
    '--target-dir', (Join-Path $BuildRoot 'target'), '--target', $nativeTarget, '--locked', '--offline')
if ($Profile -eq 'release') { $arguments += '--release' }
& cargo @arguments
if ($LASTEXITCODE -ne 0) { throw 'Image viewer build failed' }
Copy-Item -LiteralPath (Join-Path $BuildRoot "target/$nativeTarget/$Profile/$executable") -Destination (Join-Path $OutputDirectory $executable)
$manifest = Get-Content -LiteralPath (Join-Path $pluginRoot 'image-viewer.json') -Raw | ConvertFrom-Json
$manifest.target = $nativeTarget
$manifest.executable = $executable
[IO.File]::WriteAllText((Join-Path $OutputDirectory 'image-viewer.json'), ($manifest | ConvertTo-Json), [Text.UTF8Encoding]::new($false))
foreach ($name in @('LICENSE', 'README.md', 'THIRD-PARTY.md')) {
    Copy-Item -LiteralPath (Join-Path $pluginRoot $name) -Destination $OutputDirectory
}
$notices = Join-Path $OutputDirectory 'notices'
New-Item -ItemType Directory -Force $notices | Out-Null
Get-ChildItem -LiteralPath (Join-Path $pluginRoot 'notices') | Copy-Item -Destination $notices -Recurse -Force
Write-Output "Image viewer package: $OutputDirectory"
