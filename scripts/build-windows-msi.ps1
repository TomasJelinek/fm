# Build an unsigned MSI from an already signed complete Windows package.
# No product is installed and no executable from Source is ever launched.
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$Source,
    [Parameter(Mandatory = $true)][string]$Version,
    [Parameter(Mandatory = $true)][string]$Output,
    [string]$Python = 'python'
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if ($env:OS -ne 'Windows_NT') { throw 'MSI authoring requires Windows' }
$outputPath = [IO.Path]::GetFullPath($Output)
if ([IO.Path]::GetExtension($outputPath) -ne '.msi') { throw 'Output must have an .msi extension' }
if (Test-Path -LiteralPath $outputPath) { throw 'Refusing to overwrite an existing MSI' }
$work = $outputPath + '.work'
& $Python -B (Join-Path $PSScriptRoot 'windows-msi.py') --source $Source --version $Version --work $work
if ($LASTEXITCODE -ne 0) { throw 'MSI table authoring failed' }
$plan = Get-Content -LiteralPath (Join-Path $work 'plan.json') -Raw -Encoding utf8 | ConvertFrom-Json

Push-Location -LiteralPath $work
try {
    & "$env:SystemRoot/System32/makecab.exe" /F payload.ddf 2>&1 |
        Out-File -LiteralPath (Join-Path $work 'makecab.log') -Encoding utf8
    if ($LASTEXITCODE -ne 0) { throw "makecab failed; see $work/makecab.log" }
} finally { Pop-Location }
$cabinet = Join-Path $work 'cabinet/payload.cab'
if (-not (Test-Path -LiteralPath $cabinet -PathType Leaf)) { throw 'makecab produced no cabinet' }

# Windows Installer Automation is shipped with Windows. IDT imports retain the
# explicitly authored Windows-1252 database codepage; user profile paths are
# runtime Unicode directory properties, never baked into the database.
$installer = New-Object -ComObject WindowsInstaller.Installer
$database = $null
try {
    $database = $installer.OpenDatabase($outputPath, 3)
    $tableDirectory = Join-Path $work 'tables'
    $database.Import($tableDirectory, '_ForceCodepage.idt')
    foreach ($table in $plan.tables.PSObject.Properties) {
        $database.Import($tableDirectory, $table.Name + '.idt')
    }
    # Native file-version rules determine whether Installer replaces an existing
    # binary. Treating versioned DLLs as unversioned can retain an older installed
    # runtime. Read the frozen cabinet inputs, never a mutable original source.
    # FileVersion returns an empty string for text and binaries without resources.
    $nativeVersions = @{}
    $view = $database.OpenView('UPDATE `File` SET `Version` = ?, `Language` = ? WHERE `File` = ?')
    try {
        foreach ($entry in $plan.payload) {
            $frozen = [string](Join-Path $work ('cab-input/' + $entry.id))
            $nativeVersion = [string]$installer.FileVersion($frozen, $false)
            $nativeLanguage = [string]$installer.FileVersion($frozen, $true)
            $nativeVersions[$entry.id] = @($nativeVersion, $nativeLanguage)
            if (-not $nativeVersion) { continue }
            $record = $installer.CreateRecord(3)
            try {
                $record.GetType().InvokeMember('StringData', 'SetProperty', $null, $record,
                    @([int]1, [string]$nativeVersion)) | Out-Null
                if ($nativeLanguage) {
                    $record.GetType().InvokeMember('StringData', 'SetProperty', $null, $record,
                        @([int]2, [string]$nativeLanguage)) | Out-Null
                }
                $record.GetType().InvokeMember('StringData', 'SetProperty', $null, $record,
                    @([int]3, [string]$entry.id)) | Out-Null
                $view.Execute($record)
            } finally { [Runtime.InteropServices.Marshal]::FinalReleaseComObject($record) | Out-Null }
        }
    } finally {
        $view.Close()
        [Runtime.InteropServices.Marshal]::FinalReleaseComObject($view) | Out-Null
    }
    $view = $database.OpenView('INSERT INTO `_Streams` (`Name`, `Data`) VALUES (?, ?)')
    try {
        $record = $installer.CreateRecord(2)
        $record.GetType().InvokeMember('StringData', 'SetProperty', $null, $record,
            @([int]1, 'payload.cab')) | Out-Null
        $record.SetStream(2, $cabinet)
        $view.Execute($record)
        [Runtime.InteropServices.Marshal]::FinalReleaseComObject($record) | Out-Null
    } finally {
        $view.Close()
        [Runtime.InteropServices.Marshal]::FinalReleaseComObject($view) | Out-Null
    }
    $summary = $database.GetType().InvokeMember('SummaryInformation', 'GetProperty', $null, $database, @([int]20))
    try {
        # 2=Title, 3=Subject, 4=Author, 7=Template, 9=Revision/package code,
        # 14=minimum Installer version, 15=compressed|LUA package flags.
        $summaryValues = @{
            2 = 'Installation Database'; 3 = 'Midnight Explorer'; 4 = 'Midnight Explorer';
            7 = 'x64;1033'; 9 = $plan.package_code; 14 = [int]500; 15 = [int]10
        }
        foreach ($entry in $summaryValues.GetEnumerator()) {
            $summary.GetType().InvokeMember('Property', 'SetProperty', $null, $summary,
                @([int]$entry.Key, $entry.Value)) | Out-Null
        }
        $summary.Persist()
    } finally { [Runtime.InteropServices.Marshal]::FinalReleaseComObject($summary) | Out-Null }
    $database.Commit()
} finally {
    if ($null -ne $database) { [Runtime.InteropServices.Marshal]::FinalReleaseComObject($database) | Out-Null }
    [Runtime.InteropServices.Marshal]::FinalReleaseComObject($installer) | Out-Null
}

# Read back critical tables and embedded cabinet without installing the product.
$installer = New-Object -ComObject WindowsInstaller.Installer
$database = $installer.OpenDatabase($outputPath, 0)
try {
    $view = $database.OpenView('SELECT `File`, `FileSize`, `Version`, `Language` FROM `File`')
    $view.Execute()
    $actual = @{}
    while ($null -ne ($record = $view.Fetch())) {
        $fileId = $record.StringData(1)
        $actual[$fileId] = $record.IntegerData(2)
        if ($record.StringData(3) -cne $nativeVersions[$fileId][0] -or
            $record.StringData(4) -cne $nativeVersions[$fileId][1]) {
            throw "MSI native file version/language readback mismatch: $fileId"
        }
        [Runtime.InteropServices.Marshal]::FinalReleaseComObject($record) | Out-Null
    }
    $view.Close()
    [Runtime.InteropServices.Marshal]::FinalReleaseComObject($view) | Out-Null
    if ($actual.Count -ne $plan.payload.Count) { throw 'MSI file table does not cover the package' }
    foreach ($entry in $plan.payload) {
        if (-not $actual.ContainsKey($entry.id) -or $actual[$entry.id] -ne $entry.bytes) {
            throw "MSI file readback mismatch: $($entry.path)"
        }
    }
    $view = $database.OpenView('SELECT `Data` FROM `_Streams` WHERE `Name` = ''payload.cab''')
    $view.Execute()
    $record = $view.Fetch()
    if ($null -eq $record -or $record.DataSize(1) -ne (Get-Item -LiteralPath $cabinet).Length) {
        throw 'Embedded MSI cabinet readback failed'
    }
    [Runtime.InteropServices.Marshal]::FinalReleaseComObject($record) | Out-Null
    $view.Close()
    [Runtime.InteropServices.Marshal]::FinalReleaseComObject($view) | Out-Null
} finally {
    [Runtime.InteropServices.Marshal]::FinalReleaseComObject($database) | Out-Null
    [Runtime.InteropServices.Marshal]::FinalReleaseComObject($installer) | Out-Null
}
Write-Output $outputPath
