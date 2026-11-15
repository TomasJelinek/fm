param([string]$Output)
$ErrorActionPreference = 'Stop'
$repository = Split-Path (Split-Path $PSScriptRoot -Parent) -Parent
if (-not $env:CARGO_HOME) { $env:CARGO_HOME = Join-Path $repository '.tools/cargo' }
if (-not $env:RUSTUP_HOME) { $env:RUSTUP_HOME = Join-Path $repository '.tools/rustup' }
if (-not $env:CARGO) {
    $localCargo = Join-Path $env:CARGO_HOME 'bin/cargo.exe'
    $env:CARGO = if (Test-Path -LiteralPath $localCargo) { $localCargo } else { 'cargo' }
}
# Both platforms must publish identical source archives and checksums. Keep one
# canonical tar/gzip implementation rather than a separate .NET archive format.
$arguments = @('-B', (Join-Path $PSScriptRoot 'package.py'))
if ($Output) { $arguments += $Output }
& python @arguments
if ($LASTEXITCODE -ne 0) { throw 'Public web API/SDK packaging failed' }
