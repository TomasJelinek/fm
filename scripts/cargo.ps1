$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
$temporaryRoot = Join-Path $projectRoot '_build\tmp'
New-Item -ItemType Directory -Path $temporaryRoot -Force | Out-Null
$localCargo = Join-Path $projectRoot '.tools\cargo\bin\cargo.exe'
if (Test-Path -LiteralPath $localCargo) {
    $env:CARGO_HOME = Join-Path $projectRoot '.tools\cargo'
    $env:RUSTUP_HOME = Join-Path $projectRoot '.tools\rustup'
    $env:PATH = (Split-Path -Parent $localCargo) + [IO.Path]::PathSeparator + $env:PATH
}
$previousTemp = $env:TEMP
$previousTmp = $env:TMP
try {
    $env:TEMP = $temporaryRoot
    $env:TMP = $temporaryRoot
    if (Test-Path -LiteralPath $localCargo) {
        & $localCargo @args
    } else {
        & cargo @args
    }
    $cargoExit = $LASTEXITCODE
} finally {
    $env:TEMP = $previousTemp
    $env:TMP = $previousTmp
}
exit $cargoExit
