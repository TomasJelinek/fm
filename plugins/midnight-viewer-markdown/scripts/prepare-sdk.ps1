param([Parameter(Mandatory)][string]$SdkConfig, [Parameter(Mandatory)][string]$BuildRoot)
$ErrorActionPreference = 'Stop'
$config = (Resolve-Path -LiteralPath $SdkConfig).Path
$raw = Get-Content -LiteralPath $config -Raw
$match = [regex]::Match($raw, 'local-registry\s*=\s*["'']([^"'']+)["'']')
if (-not $match.Success) { throw 'SDK config must expose its released local-registry' }
$registry = $match.Groups[1].Value
if (-not [IO.Path]::IsPathRooted($registry)) {
    $base = Split-Path -Parent (Split-Path -Parent $config)
    $registry = Join-Path $base $registry
}
$registry = (Resolve-Path -LiteralPath $registry).Path.Replace('\', '/')
$BuildRoot = [IO.Path]::GetFullPath($BuildRoot)
New-Item -ItemType Directory -Force $BuildRoot | Out-Null
$generated = Join-Path $BuildRoot 'sdk.config.toml'
$quoted = ConvertTo-Json -Compress $registry
$content = @"
[registries.midnight]
index = "https://midnight-explorer.invalid/api-index"
[source.midnight]
registry = "https://midnight-explorer.invalid/api-index"
replace-with = "midnight-releases"
[source.midnight-releases]
local-registry = $quoted
"@
[IO.File]::WriteAllText($generated, $content)
Write-Output $generated
