param(
    [Parameter(Mandatory)][string]$BuildRoot,
    [switch]$Offline
)
$ErrorActionPreference = 'Stop'
if (-not $IsWindows -or -not [Environment]::Is64BitProcess) {
    throw 'The private Windows tool bootstrap requires 64-bit PowerShell 7'
}
if (-not [IO.Path]::IsPathFullyQualified($BuildRoot)) { throw 'Expected an absolute _build directory' }
if (-not (Get-Command py -ErrorAction SilentlyContinue)) { throw 'Python 3.12+ with the py launcher is required' }
# Reuse an installed x64 compiler; never run an installer or change machine settings.
if (-not (Get-Command cl.exe -ErrorAction SilentlyContinue) -or $env:VSCMD_ARG_TGT_ARCH -notin @('x64', 'amd64')) {
    $nativeBootstrapVswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio/Installer/vswhere.exe'
    if (-not (Test-Path -LiteralPath $nativeBootstrapVswhere -PathType Leaf)) {
        throw 'An existing x64 MSVC developer shell or Visual Studio Build Tools installation is required'
    }
    $nativeBootstrapInstallations = @(& $nativeBootstrapVswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath)
    if ($LASTEXITCODE -ne 0 -or $nativeBootstrapInstallations.Count -ne 1) { throw 'Could not find installed x64 MSVC tools' }
    $nativeBootstrapDevShell = Join-Path $nativeBootstrapInstallations[0] 'Common7/Tools/Launch-VsDevShell.ps1'
    if (-not (Test-Path -LiteralPath $nativeBootstrapDevShell -PathType Leaf)) { throw 'Installed Visual Studio developer shell is missing' }
    & $nativeBootstrapDevShell -Arch amd64 -HostArch amd64 -SkipAutomaticLocation | Out-Host
}
if (-not (Get-Command cl.exe -ErrorAction SilentlyContinue) -or $env:VSCMD_ARG_TGT_ARCH -notin @('x64', 'amd64')) {
    throw 'An x64 MSVC developer shell could not be initialized'
}
$nativeBootstrapArgs = @('-3', '-I', '-S', '-B', (Join-Path $PSScriptRoot 'bootstrap-tools.py'), $BuildRoot, '--platform', 'windows')
if ($Offline) { $nativeBootstrapArgs += '--offline' }
$nativeBootstrapRoot = & py @nativeBootstrapArgs
if ($LASTEXITCODE -ne 0) { throw 'Pinned private tool preparation failed' }
$nativeBootstrapRoot = $nativeBootstrapRoot.Trim()
$env:PYTHONDONTWRITEBYTECODE = '1'
$env:TEMP = Join-Path $nativeBootstrapRoot 'tmp'
$env:TMP = $env:TEMP
$env:MIDNIGHT_IMAGE_NATIVE_TOOLS = $nativeBootstrapRoot
$nativeBootstrapNinja = Join-Path $nativeBootstrapRoot 'tools/ninja-1.13.0-windows/ninja-1.13.0.data/scripts'
$nativeBootstrapNasm = Join-Path $nativeBootstrapRoot 'tools/nasm-2.16.03-windows/nasm-2.16.03'
$env:PATH = $nativeBootstrapNasm + ';' + $nativeBootstrapNinja + ';' + $env:PATH
& py -3 -I -S -B (Join-Path $PSScriptRoot 'private-meson.py') --version | Out-Host
if ($LASTEXITCODE -ne 0) { throw 'Private Meson could not start' }
& ninja.exe --version | Out-Host
if ($LASTEXITCODE -ne 0) { throw 'Private Ninja could not start' }
& nasm.exe --version | Out-Host
if ($LASTEXITCODE -ne 0) { throw 'Private NASM could not start' }
