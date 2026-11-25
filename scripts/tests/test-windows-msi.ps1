# Native read-only MSI/CAB integration. Does not install or uninstall a product.
[CmdletBinding()]
param([string]$Python = 'python')
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$root = Split-Path (Split-Path $PSScriptRoot -Parent) -Parent
$work = Join-Path $root ('_build/tests/windows-msi-' + [Guid]::NewGuid().ToString('N'))
$source = Join-Path $work 'source café'
New-Item -ItemType Directory -Path (Join-Path $source 'sub folder') -Force | Out-Null
[IO.File]::WriteAllBytes((Join-Path $source 'me.exe'), [byte[]](77, 90, 1, 2, 3, 4))
[IO.File]::WriteAllText((Join-Path $source 'BUILD-INFO.json'),
    '{"version":"0.3.0","target":"x86_64-pc-windows-msvc"}', [Text.UTF8Encoding]::new($false))
[IO.File]::WriteAllText((Join-Path $source 'sub folder/café.txt'), 'Unchanged fixture bytes',
    [Text.UTF8Encoding]::new($false))
# Local Windows fixtures exercise native version resources without executing or
# redistributing the operating-system binaries.
foreach ($fixture in @(@('cmd.exe', 'versioned.exe'), @('version.dll', 'versioned.dll'))) {
    $native = Join-Path "$env:SystemRoot/System32" $fixture[0]
    if ((Get-AuthenticodeSignature -LiteralPath $native).Status -ne 'Valid') {
        throw "Native version fixture lacks a valid Windows signature: $native"
    }
    Copy-Item -LiteralPath $native -Destination (Join-Path $source $fixture[1])
}
$output = Join-Path $work 'fixture.msi'
& (Join-Path $root 'scripts/build-windows-msi.ps1') -Source $source -Version '0.3.0' -Output $output -Python $Python
$plan = Get-Content -LiteralPath ($output + '.work/plan.json') -Raw -Encoding utf8 | ConvertFrom-Json
$extract = Join-Path $work 'expanded'
New-Item -ItemType Directory -Path $extract | Out-Null
& "$env:SystemRoot/System32/expand.exe" '-F:*' ($output + '.work/cabinet/payload.cab') $extract |
    Out-File -LiteralPath (Join-Path $work 'expand.log') -Encoding utf8
if ($LASTEXITCODE -ne 0) { throw 'Cabinet extraction failed' }
if (@(Get-ChildItem -LiteralPath $extract -File).Count -ne $plan.payload.Count) {
    throw 'Cabinet contains unexpected or missing files'
}
foreach ($entry in $plan.payload) {
    $actual = (Get-FileHash -LiteralPath (Join-Path $extract $entry.id) -Algorithm SHA256).Hash.ToLowerInvariant()
    $original = (Get-FileHash -LiteralPath (Join-Path $source $entry.path) -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actual -ne $entry.sha256 -or $actual -ne $original) { throw "Cabinet bytes changed: $($entry.path)" }
}
$installer = New-Object -ComObject WindowsInstaller.Installer
$database = $installer.OpenDatabase($output, 0)
try {
    # SQL row-count assertions exercise the actual imported database, not just
    # generated authoring text. Every component is a per-user registry keypath.
    $view = $database.OpenView('SELECT `Attributes` FROM `Component`')
    $view.Execute()
    $count = 0
    while ($null -ne ($record = $view.Fetch())) {
        if ($record.IntegerData(1) -ne 260) { throw 'Component is not 64-bit with an HKCU keypath' }
        $count++
        [Runtime.InteropServices.Marshal]::FinalReleaseComObject($record) | Out-Null
    }
    if ($count -ne $plan.payload.Count) { throw 'Component coverage mismatch' }
    $view.Close()
    [Runtime.InteropServices.Marshal]::FinalReleaseComObject($view) | Out-Null
    $view = $database.OpenView('SELECT `File`, `Version`, `Language` FROM `File`')
    $view.Execute()
    $versionedCount = 0
    while ($null -ne ($record = $view.Fetch())) {
        $fileId = $record.StringData(1)
        $entry = @($plan.payload | Where-Object { $_.id -eq $fileId })[0]
        $native = Join-Path $source $entry.path
        $expectedVersion = [string]$installer.FileVersion([string]$native, $false)
        $expectedLanguage = [string]$installer.FileVersion([string]$native, $true)
        if ($record.StringData(2) -cne $expectedVersion -or $record.StringData(3) -cne $expectedLanguage) {
            throw "MSI native version/language differs from payload: $($entry.path)"
        }
        if ($expectedVersion) { $versionedCount++ }
        [Runtime.InteropServices.Marshal]::FinalReleaseComObject($record) | Out-Null
    }
    if ($versionedCount -ne 2) { throw 'Native EXE/DLL version-resource fixtures were not exercised' }
    $view.Close()
    [Runtime.InteropServices.Marshal]::FinalReleaseComObject($view) | Out-Null
    $view = $database.OpenView('SELECT `FileName` FROM `File`')
    $view.Execute()
    $unicodePreserved = $false
    while ($null -ne ($record = $view.Fetch())) {
        if ($record.StringData(1).EndsWith('|café.txt')) { $unicodePreserved = $true }
        [Runtime.InteropServices.Marshal]::FinalReleaseComObject($record) | Out-Null
    }
    if (-not $unicodePreserved) { throw 'Windows Installer string-pool import lost a filename' }
    $view.Close()
    [Runtime.InteropServices.Marshal]::FinalReleaseComObject($view) | Out-Null
    $view = $database.OpenView('SELECT `Value` FROM `Property` WHERE `Property` = ''ALLUSERS''')
    $view.Execute()
    if ($null -ne $view.Fetch()) { throw 'MSI requests a machine-wide installation' }
    $view.Close()
    [Runtime.InteropServices.Marshal]::FinalReleaseComObject($view) | Out-Null
    $summary = $database.GetType().InvokeMember('SummaryInformation', 'GetProperty', $null, $database, @([int]0))
    try {
        $flags = $summary.GetType().InvokeMember('Property', 'GetProperty', $null, $summary, @([int]15))
        if ($flags -ne 10) { throw 'MSI is missing compressed/LUA summary flags' }
    } finally { [Runtime.InteropServices.Marshal]::FinalReleaseComObject($summary) | Out-Null }
} finally {
    [Runtime.InteropServices.Marshal]::FinalReleaseComObject($database) | Out-Null
    [Runtime.InteropServices.Marshal]::FinalReleaseComObject($installer) | Out-Null
}

# Open a session ignoring existing machine product state (option 1), and execute
# only costing/validation actions. Never invoke INSTALL, ExecuteAction, or the
# installation sequence: these checks do not install/register/remove a product.
$installer = New-Object -ComObject WindowsInstaller.Installer
$previousUiLevel = $installer.GetType().InvokeMember('UILevel', 'GetProperty', $null, $installer, $null)
$installer.GetType().InvokeMember('UILevel', 'SetProperty', $null, $installer, @([int]2)) | Out-Null
$session = $null
try {
    $session = $installer.OpenPackage($output, 1)
    foreach ($action in @('FindRelatedProducts', 'AppSearch', 'ValidateProductID', 'CostInitialize',
                          'FileCost', 'CostFinalize', 'LaunchConditions')) {
        if ($session.DoAction($action) -ne 1) { throw "MSI session action failed: $action" }
    }
    $expected = Join-Path ([Environment]::GetFolderPath('LocalApplicationData')) 'Programs/Midnight Explorer'
    $actual = $session.TargetPath('INSTALLFOLDER')
    if (-not $actual.TrimEnd('\').Equals($expected.Replace('/', '\').TrimEnd('\'),
            [StringComparison]::OrdinalIgnoreCase)) {
        throw "MSI default installation folder mismatch: $actual (expected $expected)"
    }
    $override = Join-Path $work 'custom user folder 日本語 café'
    $session.GetType().InvokeMember('TargetPath', 'SetProperty', $null, $session,
        @([string]'INSTALLFOLDER', [string]$override)) | Out-Null
    $actual = $session.TargetPath('INSTALLFOLDER')
    if (-not $actual.TrimEnd('\').Equals($override.TrimEnd('\'), [StringComparison]::Ordinal)) {
        throw "MSI lost the Unicode installation-folder override: $actual"
    }
    $session.GetType().InvokeMember('Property', 'SetProperty', $null, $session,
        @('ALLUSERS', '1')) | Out-Null
    if ($session.DoAction('LaunchConditions') -ne 3) { throw 'MSI did not refuse a machine-wide scope override' }
    if (Test-Path -LiteralPath $override) { throw 'Costing unexpectedly created the installation directory' }
} finally {
    if ($null -ne $session) { [Runtime.InteropServices.Marshal]::FinalReleaseComObject($session) | Out-Null }
    $installer.GetType().InvokeMember('UILevel', 'SetProperty', $null, $installer, @([int]$previousUiLevel)) | Out-Null
    [Runtime.InteropServices.Marshal]::FinalReleaseComObject($installer) | Out-Null
}
$rejected = $false
try {
    & (Join-Path $root 'scripts/verify-windows-release.ps1') -Installer $output -Publisher 'CN=Fixture'
} catch {
    if ($_.Exception.Message -notmatch 'Release requires valid embedded Authenticode') { throw }
    $rejected = $true
}
if (-not $rejected -or (Test-Path -LiteralPath ($output + '.sha256')) -or
    (Test-Path -LiteralPath ($output + '.signing.json'))) {
    throw 'Unsigned MSI verification did not fail without publishing receipts'
}
Write-Output "PASS: native MSI/CAB byte parity, version/language, per-user costing, Unicode folder override, scope and unsigned-MSI refusal. Artifacts: $work"
