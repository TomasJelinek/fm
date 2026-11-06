$ErrorActionPreference = 'Stop'
$pluginRoot = Split-Path -Parent $PSScriptRoot
$caseRows = Get-Content -LiteralPath (Join-Path $pluginRoot 'fixtures/CaseFolding-17.0.0.txt')
$caseOutput = [System.Collections.Generic.List[string]]::new()
$caseOutput.Add('// Generated from Unicode 17.0.0 CaseFolding.txt, C + F mappings.')
$caseOutput.Add('// Unicode License V3: see UNICODE-LICENSE.txt. Do not edit by hand.')
$caseOutput.Add('pub static FOLD: &[(char, &str)] = &[')
foreach ($caseRow in $caseRows) {
    if ($caseRow -match '^([0-9A-F]+); [CF]; ([0-9A-F ]+);') {
        $caseFrom = $Matches[1]
        $caseTo = ($Matches[2].Trim().Split(' ') | ForEach-Object { '\u{' + $_ + '}' }) -join ''
        $caseOutput.Add(('    (''\u{{{0}}}'', "{1}"),' -f $caseFrom, $caseTo))
    }
}
$caseOutput.Add('];')
[IO.File]::WriteAllLines((Join-Path $pluginRoot 'src/casefold.rs'), $caseOutput)
