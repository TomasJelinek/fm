$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
$maximumSourceBytes = 100000
$sourceExtensions = @('.rs', '.slint', '.ps1', '.sh')
$checked = 0
$oversized = 0
foreach ($directory in @('crates', 'scripts')) {
    foreach ($source in Get-ChildItem -LiteralPath (Join-Path $projectRoot $directory) -Recurse -File) {
        if ($source.Extension -notin $sourceExtensions) { continue }
        $checked++
        if ($source.Length -gt $maximumSourceBytes) {
            Write-Output "$($source.FullName): $($source.Length) bytes exceeds $maximumSourceBytes"
            $oversized++
        }
    }
}
if ($oversized -gt 0) { exit 1 }
Write-Output "Source size: $checked files checked, all <= $maximumSourceBytes bytes"
