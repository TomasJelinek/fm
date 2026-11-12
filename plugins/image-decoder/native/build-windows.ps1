param(
    [Parameter(Mandatory)][string]$BuildRoot,
    [ValidateRange(1, 32)][int]$Jobs = 4,
    [switch]$Offline
)
$ErrorActionPreference = 'Stop'
# Run from an x64 MSVC developer shell. Build tools are explicit prerequisites;
# this script never installs software or downloads mutable Git branches.
foreach ($tool in @('cl.exe', 'ninja.exe', 'nasm.exe')) {
    if (-not (Get-Command $tool -ErrorAction SilentlyContinue)) {
        throw "Missing $tool; use an x64 MSVC developer shell with Ninja and NASM on PATH"
    }
}
$BuildRoot = [IO.Path]::GetFullPath($BuildRoot)
$prepare = @('-3', '-I', '-S', '-B', (Join-Path $PSScriptRoot 'prepare-source.py'), $BuildRoot)
if ($Offline) { $prepare += '--offline' }
$source = & py @prepare
if ($LASTEXITCODE -ne 0) { throw 'Pinned dav1d source preparation failed' }
$source = $source.Trim()
$build = Join-Path $BuildRoot 'msvc-x64'
$prefix = Join-Path $BuildRoot 'prefix-msvc-x64'
$setup = @('-3', '-I', '-S', '-B', (Join-Path $PSScriptRoot 'private-meson.py'), 'setup', $build, $source,
    '--prefix', $prefix, '--libdir', 'lib', '--buildtype', 'release', '--default-library', 'static',
    '--wrap-mode=nodownload', '-Db_vscrt=md', '-Denable_tools=false', '-Denable_tests=false',
    '-Denable_examples=false', '-Denable_docs=false')
if (Test-Path -LiteralPath (Join-Path $build 'meson-private/coredata.dat')) { $setup += '--reconfigure' }
& py @setup | Out-Host
if ($LASTEXITCODE -ne 0) { throw 'dav1d Meson configuration failed' }
& py -3 -I -S -B (Join-Path $PSScriptRoot 'private-meson.py') compile -C $build -j $Jobs | Out-Host
if ($LASTEXITCODE -ne 0) { throw 'dav1d native compilation failed' }
& py -3 -I -S -B (Join-Path $PSScriptRoot 'private-meson.py') install -C $build --no-rebuild | Out-Host
if ($LASTEXITCODE -ne 0) { throw 'dav1d installation failed' }
# Meson names this static MSVC COFF archive libdav1d.a. Rust's MSVC linker
# searches for dav1d.lib when given rustc-link-lib=static=dav1d.
Copy-Item -LiteralPath (Join-Path $prefix 'lib/libdav1d.a') -Destination (Join-Path $prefix 'lib/dav1d.lib')
$env:SYSTEM_DEPS_DAV1D_NO_PKG_CONFIG = '1'
$env:SYSTEM_DEPS_DAV1D_SEARCH_NATIVE = Join-Path $prefix 'lib'
$env:SYSTEM_DEPS_DAV1D_INCLUDE = Join-Path $prefix 'include'
$env:SYSTEM_DEPS_DAV1D_LIB = 'dav1d'
$env:SYSTEM_DEPS_DAV1D_LINK = 'static'
$env:SYSTEM_DEPS_DAV1D_BUILD_INTERNAL = 'never'
Write-Output $prefix
