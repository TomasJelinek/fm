$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
$maximumSourceBytes = 100000
$sourceExtensions = @('.rs', '.slint', '.ps1', '.sh', '.bat')
$checked = 0
$oversized = 0
$sourceFiles = @(Get-ChildItem -LiteralPath $projectRoot -File)
foreach ($directory in @('crates', 'scripts')) {
    $sourceFiles += Get-ChildItem -LiteralPath (Join-Path $projectRoot $directory) -Recurse -File
}
foreach ($source in $sourceFiles) {
    if ($source.Extension -notin $sourceExtensions) { continue }
    $checked++
    if ($source.Length -gt $maximumSourceBytes) {
        Write-Output "$($source.FullName): $($source.Length) bytes exceeds $maximumSourceBytes"
        $oversized++
    }
}
if ($oversized -gt 0) { exit 1 }
Write-Output "Source size: $checked files checked, all <= $maximumSourceBytes bytes"
