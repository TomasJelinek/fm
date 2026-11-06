param([string]$BuildRoot = '', [string]$HeaderDirectory = '')
$ErrorActionPreference = 'Stop'
$apiRoot = Split-Path -Parent $PSScriptRoot
if (!$BuildRoot) { $BuildRoot = Join-Path (Split-Path -Parent $apiRoot) '_build\api-abi\windows' }
if (!$HeaderDirectory) { $HeaderDirectory = Join-Path $apiRoot 'midnight-plugin-api\include' }
$BuildRoot = [IO.Path]::GetFullPath($BuildRoot)
New-Item -ItemType Directory -Path $BuildRoot -Force | Out-Null
$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
$vsRoot = & $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
if (!$vsRoot) { throw 'MSVC C build tools were not found.' }
$devCommand = Join-Path $vsRoot 'Common7\Tools\VsDevCmd.bat'
$environment = & cmd.exe /d /c ('call "' + $devCommand + '" -no_logo -arch=x64 -host_arch=x64 >nul && set')
if ($LASTEXITCODE -ne 0) { throw 'Could not initialize the MSVC environment.' }
foreach ($entry in $environment) {
    $separator = $entry.IndexOf('=')
    if ($separator -gt 0) {
        Set-Item -LiteralPath ('Env:' + $entry.Substring(0, $separator)) -Value $entry.Substring($separator + 1)
    }
}
$executable = Join-Path $BuildRoot 'abi-layout.exe'
$object = Join-Path $BuildRoot 'abi-layout.obj'
$fixture = Join-Path $apiRoot 'midnight-plugin-api\tests\c\abi_layout.c'
& cl.exe /nologo /std:c11 /W4 /WX "/I$HeaderDirectory" "/Fo$object" "/Fe$executable" $fixture
if ($LASTEXITCODE -ne 0) { throw 'C ABI fixture compilation failed.' }
$cLayout = & $executable
if ($LASTEXITCODE -ne 0) { throw 'C ABI fixture execution failed.' }
$rustLayout = & cargo run --manifest-path (Join-Path $apiRoot 'Cargo.toml') -p midnight-plugin-api --example abi_layout --locked --quiet
if ($LASTEXITCODE -ne 0) { throw 'Rust ABI fixture execution failed.' }
if (($cLayout -join "`n") -cne ($rustLayout -join "`n")) { throw 'C/Rust ABI layout mismatch.' }
$cLayout
Write-Output 'C and Rust match: every public struct size, alignment and field offset.'
