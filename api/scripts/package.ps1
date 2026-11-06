$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
Push-Location -LiteralPath $projectRoot
try {
    & (Join-Path $projectRoot 'scripts/cargo.ps1') run --manifest-path api/Cargo.toml `
        -p midnight-api-package --locked --offline -- $projectRoot
    $packageExit = $LASTEXITCODE
} finally {
    Pop-Location
}
exit $packageExit
