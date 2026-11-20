param(
    [Parameter(Mandatory)][string]$SdkConfig,
    [Parameter(Mandatory)][string]$BuildRoot,
    [string]$OutputDirectory,
    [ValidateSet('debug','release')][string]$Profile = 'release'
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
$SdkConfig = (Resolve-Path -LiteralPath $SdkConfig).Path
# Relative arguments follow the PowerShell location, not the process directory.
$BuildRoot = [IO.Path]::GetFullPath($ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($BuildRoot))
if (-not $OutputDirectory) { $OutputDirectory = Join-Path $BuildRoot 'package/midnight-viewer-pdf' }
$OutputDirectory = [IO.Path]::GetFullPath($ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($OutputDirectory))
foreach ($path in @($BuildRoot, $OutputDirectory)) {
    $problem = Get-GeneratedOutputProblem $path
    if ($problem) { throw "Generated output $problem" }
}
New-Item -ItemType Directory -Force $BuildRoot, $OutputDirectory | Out-Null
$nativeTarget = ((& rustc -vV | Select-String '^host: ').ToString() -replace '^host: ', '').Trim()
if ($LASTEXITCODE -ne 0) { throw 'rustc target discovery failed' }
if ($nativeTarget -ne 'x86_64-pc-windows-msvc') { throw "Unsupported Windows target: $nativeTarget" }
$executable = 'midnight-viewer-pdf.exe'
$arguments = @('build', '--config', $SdkConfig, '--manifest-path', (Join-Path $pluginRoot 'Cargo.toml'),
    '--target-dir', (Join-Path $BuildRoot 'target'), '--target', $nativeTarget, '--locked', '--offline')
if ($Profile -eq 'release') { $arguments += '--release' }
& cargo @arguments
if ($LASTEXITCODE -ne 0) { throw 'PDF viewer build failed' }
Copy-Item -LiteralPath (Join-Path $BuildRoot "target/$nativeTarget/$Profile/$executable") -Destination (Join-Path $OutputDirectory $executable)
$manifest = Get-Content -LiteralPath (Join-Path $pluginRoot 'image-viewer.json') -Raw | ConvertFrom-Json
$manifest.target = $nativeTarget
$manifest.executable = $executable
[IO.File]::WriteAllText((Join-Path $OutputDirectory 'image-viewer.json'), ($manifest | ConvertTo-Json), [Text.UTF8Encoding]::new($false))
foreach ($name in @('LICENSE', 'README.md', 'THIRD-PARTY.md')) {
    Copy-Item -LiteralPath (Join-Path $pluginRoot $name) -Destination $OutputDirectory
}
# Replace the generated notices so a removed dependency's notice cannot survive.
$notices = Join-Path $OutputDirectory 'notices'
if (Test-Path -LiteralPath $notices) {
    if ((Get-Item -LiteralPath $notices -Force).Attributes -band [IO.FileAttributes]::ReparsePoint) {
        throw 'Package notices must not be a link'
    }
    Remove-Item -LiteralPath $notices -Recurse -Force
}
New-Item -ItemType Directory -Path $notices | Out-Null
Get-ChildItem -LiteralPath (Join-Path $pluginRoot 'notices') | Copy-Item -Destination $notices -Recurse -Force
Write-Output "PDF viewer package: $OutputDirectory"
