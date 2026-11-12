param(
    [Parameter(Mandatory = $true)][string]$Library,
    [Parameter(Mandatory = $true)][string]$HeaderDirectory,
    [Parameter(Mandatory = $true)][string]$BuildRoot
)
$ErrorActionPreference = 'Stop'
if (!(Get-Command cl.exe -ErrorAction SilentlyContinue)) {
    throw 'Run this script from an x64 Visual Studio Native Tools terminal.'
}
$Library = (Resolve-Path -LiteralPath $Library).Path
$HeaderDirectory = (Resolve-Path -LiteralPath $HeaderDirectory).Path
$BuildRoot = [IO.Path]::GetFullPath($BuildRoot)
if ('_build' -notin $BuildRoot.Split([IO.Path]::DirectorySeparatorChar)) {
    throw 'Place test outputs under the common _build directory.'
}
New-Item -ItemType Directory -Path $BuildRoot -Force | Out-Null
$objects = @()
foreach ($source in @('conformance', 'loader_windows')) {
    $object = Join-Path $BuildRoot ($source + '.obj')
    & cl.exe /nologo /c /std:c11 /W4 /WX "/I$HeaderDirectory" "/Fo$object" `
        (Join-Path $PSScriptRoot ($source + '.c'))
    if ($LASTEXITCODE -ne 0) { throw "Failed to compile $source." }
    $objects += $object
}
$executable = Join-Path $BuildRoot 'viewer-conformance.exe'
& cl.exe /nologo "/Fe$executable" @objects
if ($LASTEXITCODE -ne 0) { throw 'Failed to link the C conformance tests.' }
& $executable $Library
exit $LASTEXITCODE
