param([string]$Output)
$ErrorActionPreference = 'Stop'
$repository = Split-Path (Split-Path $PSScriptRoot -Parent) -Parent
# A standard rustup installation exports neither home; keep its defaults and use
# the repository-local toolchain only when it exists. The caller's environment
# is restored afterwards because this script runs inside the HTML build process.
$savedEnvironment = @{}
foreach ($name in @('CARGO_HOME', 'RUSTUP_HOME', 'CARGO')) {
    $savedEnvironment[$name] = [Environment]::GetEnvironmentVariable($name)
}
try {
    $localHome = Join-Path $repository '.tools/cargo'
    if (-not $env:CARGO_HOME -and (Test-Path -LiteralPath (Join-Path $localHome 'bin/cargo.exe') -PathType Leaf)) {
        $env:CARGO_HOME = $localHome
        if (-not $env:RUSTUP_HOME) { $env:RUSTUP_HOME = Join-Path $repository '.tools/rustup' }
    }
    if (-not $env:CARGO -and $env:CARGO_HOME) {
        $selectedCargo = Join-Path $env:CARGO_HOME 'bin/cargo.exe'
        if (Test-Path -LiteralPath $selectedCargo -PathType Leaf) { $env:CARGO = $selectedCargo }
    }
    # Both platforms must publish identical source archives and checksums. Keep one
    # canonical tar/gzip implementation rather than a separate .NET archive format.
    # Python 3.12+ runs through the documented py launcher; python may be a Store alias.
    $python = 'python'
    $arguments = @('-B')
    if (Get-Command py -CommandType Application -ErrorAction Ignore) {
        $python = 'py'
        $arguments = @('-3', '-B')
    }
    $arguments += Join-Path $PSScriptRoot 'package.py'
    if ($Output) { $arguments += $Output }
    & $python @arguments
    if ($LASTEXITCODE -ne 0) { throw 'Public web API/SDK packaging failed' }
} finally {
    foreach ($name in $savedEnvironment.Keys) {
        [Environment]::SetEnvironmentVariable($name, $savedEnvironment[$name])
    }
}
