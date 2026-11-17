param(
    [Parameter(Mandatory)][string]$SdkConfig,
    [Parameter(Mandatory)][string]$BuildRoot,
    [string]$OutputDirectory,
    [ValidateSet('debug', 'release')][string]$Profile = 'debug'
)
$ErrorActionPreference = 'Stop'
$pluginRoot = Split-Path -Parent $PSScriptRoot
# Relative arguments follow the PowerShell location, not the process directory.
$BuildRoot = [IO.Path]::GetFullPath($ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($BuildRoot))
if (-not $OutputDirectory) { $OutputDirectory = Join-Path $BuildRoot 'package/midnight-viewer-source' }
$OutputDirectory = [IO.Path]::GetFullPath($ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($OutputDirectory))
$config = & (Join-Path $PSScriptRoot 'prepare-sdk.ps1') -SdkConfig $SdkConfig -BuildRoot $BuildRoot
$nativeTarget = ((& rustc -vV | Select-String '^host: ').ToString() -replace '^host: ', '').Trim()
if ($LASTEXITCODE -ne 0) { throw 'Native target discovery failed' }
if ($nativeTarget -match '-windows-') { $library = 'midnight_viewer_source.dll' }
elseif ($nativeTarget -match '-linux-') { $library = 'libmidnight_viewer_source.so' }
else { throw "Unsupported native target: $nativeTarget" }
$arguments = @('--config', $config, 'build', '--manifest-path', (Join-Path $pluginRoot 'Cargo.toml'),
    '--target-dir', (Join-Path $BuildRoot 'target'), '--target', $nativeTarget, '--locked', '--offline')
if ($Profile -eq 'release') { $arguments += '--release' }
& cargo @arguments
if ($LASTEXITCODE -ne 0) { throw 'Plugin build failed' }
New-Item -ItemType Directory -Force $OutputDirectory | Out-Null
Copy-Item -LiteralPath (Join-Path $BuildRoot "target/$nativeTarget/$Profile/$library") -Destination $OutputDirectory
$manifest = Get-Content -LiteralPath (Join-Path $pluginRoot 'plugin.toml') -Raw
$manifest = $manifest -replace 'entry_library = "[^"]+"', "entry_library = `"$library`""
$manifest = $manifest -replace 'target = "[^"]+"', "target = `"$nativeTarget`""
[IO.File]::WriteAllText((Join-Path $OutputDirectory 'plugin.toml'), $manifest)
foreach ($notice in @('LICENSE', 'README.md', 'THIRD-PARTY.md')) {
    Copy-Item -LiteralPath (Join-Path $pluginRoot $notice) -Destination $OutputDirectory
}
# Replace only notices that this script generated (they carry its marker) or that lie
# in a _build output tree, so a removed dependency's notice cannot survive a rebuild.
# The marker is never removed: an interrupted build is completed by the next one.
$notices = Join-Path $OutputDirectory 'notices'
$marker = Join-Path $notices '.midnight-generated'
$markerText = "midnight-viewer-source notices`n"
if (Test-Path -LiteralPath $notices) {
    $pending = [Collections.Generic.Stack[IO.DirectoryInfo]]::new()
    $pending.Push([IO.DirectoryInfo]::new($notices))
    if (-not $pending.Peek().Exists -or ($pending.Peek().Attributes -band [IO.FileAttributes]::ReparsePoint)) {
        throw 'Package notices must be a directory, not a link'
    }
    while ($pending.Count -gt 0) {
        foreach ($entry in $pending.Pop().EnumerateFileSystemInfos()) {
            if ($entry.Attributes -band [IO.FileAttributes]::ReparsePoint) {
                throw "Package notices must not contain links: $($entry.FullName)"
            }
            if ($entry -is [IO.DirectoryInfo]) { $pending.Push($entry) }
        }
    }
    $generated = (Test-Path -LiteralPath $marker -PathType Leaf) -and [IO.File]::ReadAllText($marker) -ceq $markerText
    $underBuild = $OutputDirectory -match '[/\\]_build([/\\]|$)'
    for ($part = $OutputDirectory; $underBuild -and $part; $part = Split-Path -Parent $part) {
        if ((Get-Item -LiteralPath $part -Force).Attributes -band [IO.FileAttributes]::ReparsePoint) { $underBuild = $false }
    }
    if (-not ($generated -or $underBuild)) {
        throw "Refusing to replace notices that this script did not generate: $notices"
    }
    Get-ChildItem -LiteralPath $notices -Force | Where-Object Name -cne '.midnight-generated' |
        Remove-Item -Recurse -Force
} else {
    New-Item -ItemType Directory -Path $notices | Out-Null
}
[IO.File]::WriteAllText($marker, $markerText)
Get-ChildItem -LiteralPath (Join-Path $pluginRoot 'notices') | ForEach-Object {
    Copy-Item -LiteralPath $_.FullName -Destination $notices -Recurse -Force
}
Write-Output "Viewer package: $OutputDirectory"
