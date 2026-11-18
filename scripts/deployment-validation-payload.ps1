<#
Private deployment support only; loading this file has no deployment side effects.
The Build owner supplies a JSON receipt from its successful final producer/source gates:
schema_version=1, cargo_json (absolute JSONL path), cargo_json_sha256,
cargo_exit_code=0, artifact_sha256, application_sha256, source_snapshot (absolute
JSON path), source_snapshot_sha256, target="x86_64-pc-windows-msvc", profile="debug".
All hashes are lowercase SHA256. The snapshot is an array of absolute repository
file paths and sha256 values from the same qualified source freeze. Include the
complete relevant build-source closure, including the MAX setup gate. Live hashes
are checked; this helper consumes the producer attestation without rebuilding.
The copied payload inventory does not replace the caller's full assembly freeze.
#>
function Get-ValidationSha256([byte[]]$Bytes) {
    $hash = [Security.Cryptography.SHA256]::Create()
    try { ([BitConverter]::ToString($hash.ComputeHash($Bytes))).Replace('-', '').ToLowerInvariant() }
    finally { $hash.Dispose() }
}

function Assert-ValidationHash($Value) {
    if ($Value -isnot [string] -or $Value -cnotmatch '^[0-9a-f]{64}$') {
        throw 'Validation evidence requires lowercase SHA256 values'
    }
}

function Get-ValidationPlainFile([string]$Path) {
    if (-not [IO.Path]::IsPathRooted($Path) -or ($Path -split '[\\/]') -contains '..') {
        throw 'Validation evidence requires absolute paths without parent traversal'
    }
    $full = [IO.Path]::GetFullPath($Path)
    $current = $full
    while ($current) {
        $item = Get-Item -LiteralPath $current -Force
        if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) { throw "Validation path redirects: $current" }
        $parent = [IO.Directory]::GetParent($current)
        $current = if ($parent) { $parent.FullName } else { $null }
    }
    $item = Get-Item -LiteralPath $full -Force
    if ($item.PSIsContainer) { throw "Validation input must be a regular file: $full" }
    $item
}

function Read-ValidationEvidence([string]$Path) {
    $item = Get-ValidationPlainFile $Path
    if ($item.Length -gt 16777216) { throw 'Validation JSON exceeds 16 MiB' }
    $stream = [IO.File]::Open($item.FullName, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
    try {
        if ($stream.Length -gt 16777216) { throw 'Validation JSON exceeds 16 MiB' }
        $memory = [IO.MemoryStream]::new()
        try { $stream.CopyTo($memory); $bytes = $memory.ToArray() } finally { $memory.Dispose() }
    } finally { $stream.Dispose() }
    [pscustomobject]@{
        Path = $item.FullName
        Sha256 = Get-ValidationSha256 $bytes
        Text = ([Text.UTF8Encoding]::new($false, $true)).GetString($bytes).TrimStart([char]0xfeff)
    }
}

function Assert-ValidationJsonMembers($Element) {
    if ($Element.ValueKind -eq [System.Text.Json.JsonValueKind]::Object) {
        $names = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
        foreach ($property in $Element.EnumerateObject()) {
            if (-not $names.Add($property.Name)) { throw 'Duplicate validation JSON member' }
            Assert-ValidationJsonMembers $property.Value
        }
    } elseif ($Element.ValueKind -eq [System.Text.Json.JsonValueKind]::Array) {
        foreach ($value in $Element.EnumerateArray()) { Assert-ValidationJsonMembers $value }
    }
}

function ConvertFrom-ValidationJson([string]$Text) {
    $document = [System.Text.Json.JsonDocument]::Parse($Text)
    try { Assert-ValidationJsonMembers $document.RootElement } finally { $document.Dispose() }
    # Preserve singleton arrays across this function's pipeline boundary too.
    return ,(ConvertFrom-Json -InputObject $Text -NoEnumerate -ErrorAction Stop)
}

function Get-ValidationStreamHash([IO.Stream]$Stream) {
    $hash = [Security.Cryptography.SHA256]::Create()
    try {
        $Stream.Position = 0
        $digest = ([BitConverter]::ToString($hash.ComputeHash($Stream))).Replace('-', '').ToLowerInvariant()
        $Stream.Position = 0
        $digest
    } finally { $hash.Dispose() }
}

function Assert-DeploymentValidationSources([string]$Snapshot, [string]$Sha256, [string]$ProjectRoot) {
    $sourceEvidence = Read-ValidationEvidence $Snapshot
    if ($sourceEvidence.Sha256 -cne $Sha256) { throw 'Source snapshot SHA256 mismatch' }
    $sourceRows = ConvertFrom-ValidationJson $sourceEvidence.Text
    if ($sourceRows -isnot [Array] -or $sourceRows.Count -lt 4 -or $sourceRows.Count -gt 8192) { throw 'Invalid source snapshot structure' }
    $project = [IO.Path]::GetFullPath($ProjectRoot).TrimEnd('\')
    $seen = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    foreach ($row in $sourceRows) {
        if ($null -eq $row -or $row.GetType() -ne [System.Management.Automation.PSCustomObject] -or $row.path -isnot [string]) {
            throw 'Invalid source snapshot row/path'
        }
        Assert-ValidationHash $row.sha256
        if (-not [IO.Path]::IsPathRooted($row.path) -or
            -not [IO.Path]::GetFullPath($row.path).StartsWith($project + '\', [StringComparison]::OrdinalIgnoreCase)) {
            throw 'Source snapshot contains an external or relative path'
        }
        $file = Get-ValidationPlainFile $row.path
        if (-not $file.FullName.StartsWith($project + '\', [StringComparison]::OrdinalIgnoreCase) -or -not $seen.Add($file.FullName)) {
            throw 'Source snapshot contains an external or duplicate path'
        }
        if ((Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash.ToLowerInvariant() -cne $row.sha256) {
            throw "Live source snapshot mismatch: $($file.FullName)"
        }
    }
    foreach ($relative in @('Cargo.toml','Cargo.lock','crates/midnight-explorer/Cargo.toml','crates/midnight-explorer/src/main.rs')) {
        if (-not $seen.Contains([IO.Path]::GetFullPath((Join-Path $project $relative)))) { throw "Source snapshot omits $relative" }
    }
    $seen.Count
}

function Open-DeploymentValidationPayload([string]$Receipt, [string]$ProjectRoot, [string]$SourceDirectory) {
    if ($PSVersionTable.PSVersion.Major -lt 7) { throw 'Validation payload requires PowerShell 7' }
    $evidence = Read-ValidationEvidence $Receipt
    $proof = ConvertFrom-ValidationJson $evidence.Text
    if ($null -eq $proof -or $proof.GetType() -ne [System.Management.Automation.PSCustomObject]) { throw 'Invalid validation receipt object' }
    foreach ($field in @('target','profile','cargo_json','source_snapshot')) {
        if ($proof.$field -isnot [string]) { throw "Validation receipt requires a string $field" }
    }
    if ($proof.schema_version -isnot [long] -and $proof.schema_version -isnot [int]) { throw 'Invalid validation receipt schema' }
    if ($proof.schema_version -ne 1 -or $proof.target -cne 'x86_64-pc-windows-msvc' -or $proof.profile -cne 'debug') {
        throw 'Validation payload requires schema1 Windows x64 debug evidence'
    }
    if (($proof.cargo_exit_code -isnot [long] -and $proof.cargo_exit_code -isnot [int]) -or $proof.cargo_exit_code -ne 0) {
        throw 'Cargo exit code must be zero'
    }
    foreach ($field in @('cargo_json_sha256','artifact_sha256','application_sha256','source_snapshot_sha256')) {
        Assert-ValidationHash $proof.$field
    }
    $cargo = Read-ValidationEvidence $proof.cargo_json
    if ($cargo.Sha256 -cne $proof.cargo_json_sha256) { throw 'Cargo JSON SHA256 mismatch' }
    $snapshot = Read-ValidationEvidence $proof.source_snapshot
    $sourceCount = Assert-DeploymentValidationSources $snapshot.Path $proof.source_snapshot_sha256 $ProjectRoot
    $project = [IO.Path]::GetFullPath($ProjectRoot).TrimEnd('\')
    $manifest = [IO.Path]::GetFullPath((Join-Path $project 'crates/midnight-explorer/Cargo.toml'))
    $main = [IO.Path]::GetFullPath((Join-Path $project 'crates/midnight-explorer/src/main.rs'))
    $packagePrefix = 'path+' + ([Uri](Split-Path -Parent $manifest)).AbsoluteUri.TrimEnd('/') + '#'
    $selected = @()
    $finished = $false
    foreach ($line in ($cargo.Text -split '\r?\n')) {
        if (-not $line.Trim()) { continue }
        if ($finished) { throw 'Cargo build-finished must be terminal' }
        $event = ConvertFrom-ValidationJson $line
        if ($null -eq $event -or $event.GetType() -ne [System.Management.Automation.PSCustomObject] -or $event.reason -isnot [string]) {
            throw 'Invalid Cargo JSON event object/reason'
        }
        if ($event.reason -ceq 'compiler-artifact') {
            if ($null -eq $event.target -or $event.target.GetType() -ne [System.Management.Automation.PSCustomObject] -or
                $null -eq $event.profile -or $event.profile.GetType() -ne [System.Management.Automation.PSCustomObject]) {
                throw 'Invalid Cargo JSON target/profile object'
            }
            if ($event.target.kind -isnot [Array] -or $event.target.kind.Count -eq 0) {
                throw 'Cargo JSON target kinds must be an array of strings'
            }
            foreach ($kind in $event.target.kind) {
                if ($kind -isnot [string]) { throw 'Cargo JSON target kinds must be an array of strings' }
            }
        }
        if ($event.reason -ceq 'build-finished') {
            if ($event.success -isnot [bool] -or -not $event.success) { throw 'Cargo build-finished was unsuccessful' }
            $finished = $true
        } elseif ($event.reason -ceq 'compiler-artifact' -and $event.package_id -is [string] -and
            $event.package_id.StartsWith($packagePrefix, [StringComparison]::OrdinalIgnoreCase) -and
            $event.manifest_path -is [string] -and $event.manifest_path -eq $manifest -and
            $event.target.src_path -is [string] -and $event.target.src_path -eq $main -and
            $event.target.name -is [string] -and $event.target.name -ceq 'me' -and
            $event.target.kind.Count -eq 1 -and $event.target.kind[0] -ceq 'bin' -and
            $event.profile.test -is [bool] -and $event.profile.test -and
            $event.profile.debug_assertions -is [bool] -and $event.profile.debug_assertions) {
            $selected += $event
        }
    }
    if (-not $finished) { throw 'Cargo build-finished success is missing' }
    if ($selected.Count -ne 1) { throw 'Expected exactly one local Explorer bin me libtest artifact' }
    $artifact = $selected[0]
    if ($artifact.executable -isnot [string] -or $artifact.filenames -isnot [Array] -or $artifact.filenames -notcontains $artifact.executable) {
        throw 'Cargo executable is missing from filenames'
    }
    foreach ($filename in $artifact.filenames) {
        if ($filename -isnot [string]) { throw 'Cargo filenames must be an array of strings' }
    }
    if (-not [IO.Path]::IsPathRooted($artifact.executable) -or ($artifact.executable -split '[\\/]') -contains '..') {
        throw 'Cargo executable requires an absolute path without parent traversal'
    }
    $deps = [IO.Path]::GetFullPath((Join-Path $SourceDirectory 'deps'))
    $payload = [IO.Path]::GetFullPath($artifact.executable)
    if ((Split-Path -Parent $payload) -ne $deps -or (Split-Path -Leaf $payload) -cnotmatch '^me-[0-9a-f]{1,64}\.exe$') {
        throw 'Explorer validation artifact must retain its Cargo filename directly in source deps'
    }
    $file = Get-ValidationPlainFile $payload
    $application = Get-ValidationPlainFile (Join-Path $SourceDirectory 'me.exe')
    $stream = $null
    $applicationStream = $null
    try {
        $stream = [IO.File]::Open($file.FullName, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
        if ((Get-ValidationStreamHash $stream) -cne $proof.artifact_sha256) { throw 'Validation payload SHA256 mismatch' }
        $applicationStream = [IO.File]::Open($application.FullName, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
        if ((Get-ValidationStreamHash $applicationStream) -cne $proof.application_sha256) { throw 'Normal application SHA256 mismatch' }
        [pscustomobject]@{
            Source=$file.FullName; Stream=$stream; ApplicationStream=$applicationStream; ProjectRoot=$project
            Metadata=[ordered]@{
                schema_version=1; path=$file.Name; bytes=$stream.Length; sha256=$proof.artifact_sha256
                application_path='me.exe'; application_sha256=$proof.application_sha256
                source_snapshot=$snapshot.Path; source_snapshot_sha256=$snapshot.Sha256; live_source_files_checked=$sourceCount
                cargo_json=$cargo.Path; cargo_json_sha256=$cargo.Sha256; cargo_exit_code=0; cargo_artifact=$artifact
                qualification_receipt=$evidence.Path; qualification_receipt_sha256=$evidence.Sha256
                target=$proof.target; profile=$proof.profile
                inventory_scope='validation payload and app/source epoch only; caller owns complete product inventory and final freeze'
            }
        }
    } catch {
        if ($stream) { $stream.Dispose() }
        if ($applicationStream) { $applicationStream.Dispose() }
        throw
    }
}

function Copy-DeploymentValidationPayload($Lease, [string]$Destination) {
    $target = Join-Path $Destination $Lease.Metadata.path
    $inventory = Join-Path $Destination 'validation-payload.json'
    if ((Test-Path -LiteralPath $target) -or (Test-Path -LiteralPath $inventory)) { throw 'Validation payload or inventory already exists' }
    $null = Assert-DeploymentValidationSources $Lease.Metadata.source_snapshot $Lease.Metadata.source_snapshot_sha256 $Lease.ProjectRoot
    if ((Get-FileHash -LiteralPath (Join-Path $Destination 'me.exe') -Algorithm SHA256).Hash.ToLowerInvariant() -cne $Lease.Metadata.application_sha256) {
        throw 'Destination application SHA256 differs from the qualified epoch'
    }
    $null = Get-ValidationPlainFile $Lease.Source
    $Lease.Stream.Position = 0
    $output = [IO.File]::Open($target, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::None)
    try { $Lease.Stream.CopyTo($output); $output.Flush($true) } finally { $output.Dispose() }
    $readback = [IO.File]::Open($target, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
    try {
        if ($readback.Length -ne $Lease.Metadata.bytes -or (Get-ValidationStreamHash $readback) -cne $Lease.Metadata.sha256) {
            throw 'Validation payload copy readback mismatch'
        }
    } finally { $readback.Dispose() }
    $Lease.Metadata['live_source_files_checked_after_copy'] = Assert-DeploymentValidationSources `
        $Lease.Metadata.source_snapshot $Lease.Metadata.source_snapshot_sha256 $Lease.ProjectRoot
    $Lease.Metadata
}

function Assert-DeploymentValidationRuntime([string]$ProjectRoot, [string]$Source, [string]$Python) {
    $PSNativeCommandUseErrorActionPreference = $false
    & $Python -B (Join-Path $ProjectRoot 'scripts/package-release.py') validate-runtime --source $Source --target x86_64-pc-windows-msvc
    if ($LASTEXITCODE -ne 0) { throw 'Full runtime validation failed; assembly is not qualified' }
}

function Write-DeploymentValidationInventory($Record, [string]$Destination) {
    $text = ($Record | ConvertTo-Json -Depth 16) + "`n"
    $bytes = [Text.UTF8Encoding]::new($false).GetBytes($text)
    $stream = [IO.File]::Open((Join-Path $Destination 'validation-payload.json'), [IO.FileMode]::CreateNew,
        [IO.FileAccess]::Write, [IO.FileShare]::None)
    try { $stream.Write($bytes, 0, $bytes.Length); $stream.Flush($true) } finally { $stream.Dispose() }
}
