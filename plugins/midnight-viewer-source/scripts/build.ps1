param(
    [Parameter(Mandatory)][string]$SdkConfig,
    [Parameter(Mandatory)][string]$BuildRoot,
    [string]$OutputDirectory,
    [ValidateSet('debug', 'release')][string]$Profile = 'debug'
)
$ErrorActionPreference = 'Stop'
$pluginRoot = Split-Path -Parent $PSScriptRoot
# Generated output lies strictly below _build: for a path inside this plugin, or inside
# the repository that holds it in plugins/NAME, their own _build; otherwise the first
# _build of the path (a custom directory). That _build and every folder below it must
# not be a reparse point (symlink or junction); the checkout and the folders above it
# may be links, a subst drive or hidden folders such as AppData.
function Get-GeneratedOutputProblem([string]$Path) {
    $checkouts = @([IO.Path]::GetFullPath($pluginRoot).TrimEnd('\'))
    $plugins = Split-Path -Parent $checkouts[0]
    if ((Split-Path -Leaf $plugins) -eq 'plugins') { $checkouts += (Split-Path -Parent $plugins).TrimEnd('\') }
    $ancestors = @()
    $inside = $false
    for ($part = [IO.Path]::GetFullPath($Path).TrimEnd('\'); $part; $part = [IO.Path]::GetDirectoryName($part)) {
        # TrimEnd also matches a checkout at a drive root, such as a subst drive's M:\.
        if ($checkouts -contains $part.TrimEnd('\')) { $inside = $true; break }
        $ancestors += $part
    }
    $anchor = 0
    for ($index = $ancestors.Count - 1; $index -gt 0; $index--) {
        if ([IO.Path]::GetFileName($ancestors[$index]) -eq '_build') { $anchor = $index; break }
        if ($inside) { break }
    }
    if ($anchor -eq 0) { return "must be under _build: $Path" }
    foreach ($part in $ancestors[0..$anchor]) {
        $item = Get-Item -LiteralPath $part -Force -ErrorAction Ignore
        if ($item -and ($item.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
            return "must not traverse reparse points: $part"
        }
    }
    return $null
}
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
    $underBuild = -not (Get-GeneratedOutputProblem $OutputDirectory)
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
