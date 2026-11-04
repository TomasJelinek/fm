param(
    [Parameter(Mandatory = $true)][string]$JunctionPath,
    [Parameter(Mandatory = $true)][string]$TargetPath
)
$ErrorActionPreference = 'Stop'
# -File receives these paths as parameter data, never as executable command text.
New-Item -ItemType Junction -Path $JunctionPath -Value $TargetPath | Out-Null
