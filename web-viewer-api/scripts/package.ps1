param([string]$Output)
$ErrorActionPreference = 'Stop'

$apiRoot = Split-Path $PSScriptRoot -Parent
$repository = Split-Path $apiRoot -Parent
$buildRoot = [IO.Path]::GetFullPath((Join-Path $repository '_build'))
if (-not $Output) {
    $Output = Join-Path $buildRoot ('html-viewer/sdk/release-' + [DateTime]::UtcNow.Ticks)
}
$Output = [IO.Path]::GetFullPath($Output)
if (-not $Output.StartsWith($buildRoot + [IO.Path]::DirectorySeparatorChar,
        [StringComparison]::OrdinalIgnoreCase)) {
    throw 'Release output must stay beneath the repository _build directory'
}
if (Test-Path -LiteralPath $Output) { throw 'Use a new immutable release output directory' }
$null = New-Item -ItemType Directory -Path $Output
$registry = Join-Path $Output 'registry'
$null = New-Item -ItemType Directory -Path (Join-Path $registry 'index')
$config = Join-Path $Output 'config.toml'
$registryUrl = 'https://midnight-explorer.invalid/api-index'
$quotedRegistry = ConvertTo-Json -Compress -InputObject $registry
[IO.File]::WriteAllText($config, @"
[registries.midnight]
index = "$registryUrl"
[source.midnight]
registry = "$registryUrl"
replace-with = "web-viewer-releases"
[source.web-viewer-releases]
local-registry = $quotedRegistry
"@ + "`n")

$env:CARGO_HOME = Join-Path $repository '.tools/cargo'
$env:RUSTUP_HOME = Join-Path $repository '.tools/rustup'
$cargo = Join-Path $env:CARGO_HOME 'bin/cargo.exe'
$target = Join-Path $buildRoot 'html-viewer/api-target'
$manifest = Join-Path $apiRoot 'Cargo.toml'
$metadataText = & $cargo metadata --manifest-path $manifest --no-deps --offline --format-version 1
if ($LASTEXITCODE -ne 0) { throw 'Cargo metadata failed' }
$metadata = $metadataText | ConvertFrom-Json

function ConvertTo-CanonicalPackage([string]$Artifact, [string]$Prefix) {
    $members = [Collections.Generic.SortedDictionary[string,object]]::new([StringComparer]::Ordinal)
    $total = 0L
    $inputStream = [IO.File]::OpenRead($Artifact)
    $unzip = [IO.Compression.GZipStream]::new($inputStream, [IO.Compression.CompressionMode]::Decompress)
    $reader = [System.Formats.Tar.TarReader]::new($unzip)
    try {
        while ($null -ne ($entry = $reader.GetNextEntry())) {
            $name = $entry.Name
            if ($entry.EntryType -notin @([System.Formats.Tar.TarEntryType]::RegularFile,
                    [System.Formats.Tar.TarEntryType]::V7RegularFile) -or
                -not $name.StartsWith($Prefix + '/', [StringComparison]::Ordinal) -or
                $name -match '(^|/)\.\.?(/|$)|[\\:]' -or $entry.Length -gt 1048576 -or
                $members.Count -ge 512 -or $members.ContainsKey($name)) {
                throw 'Invalid Cargo release member'
            }
            $buffer = [IO.MemoryStream]::new()
            try { $entry.DataStream.CopyTo($buffer); $data = $buffer.ToArray() }
            finally { $buffer.Dispose() }
            $total += $data.Length
            if ($total -gt 16777216) { throw 'Cargo release exceeds package budget' }
            if ($name -ne ($Prefix + '/.cargo_vcs_info.json')) {
                $members.Add($name, @{ Bytes = $data; Mode = $entry.Mode })
            }
        }
    }
    finally { $reader.Dispose(); $unzip.Dispose(); $inputStream.Dispose() }
    $normalized = [Text.Encoding]::UTF8.GetString($members[$Prefix + '/Cargo.toml'].Bytes)
    $dependencySection = $false
    foreach ($line in ($normalized -split "`n")) {
        if ($line.StartsWith('[')) {
            $dependencySection = $line -match '^\[(.*\.)?(dependencies|dev-dependencies|build-dependencies)\.'
        }
        elseif ($dependencySection -and $line -match '^\s*(path|git|registry)\s*=') {
            throw 'Normalized release manifest contains a local dependency'
        }
    }
    $destination = [IO.MemoryStream]::new()
    $zip = [IO.Compression.GZipStream]::new($destination, [IO.Compression.CompressionLevel]::Optimal, $true)
    $writer = [System.Formats.Tar.TarWriter]::new($zip, [System.Formats.Tar.TarEntryFormat]::Ustar, $true)
    try {
        foreach ($pair in $members.GetEnumerator()) {
            $entry = [System.Formats.Tar.UstarTarEntry]::new([System.Formats.Tar.TarEntryType]::RegularFile, $pair.Key)
            $entry.ModificationTime = [DateTimeOffset]::UnixEpoch
            $entry.Uid = 0
            $entry.Gid = 0
            $entry.Mode = [IO.UnixFileMode]420
            $entry.DataStream = [IO.MemoryStream]::new([byte[]]$pair.Value.Bytes)
            try { $writer.WriteEntry($entry) } finally { $entry.DataStream.Dispose() }
        }
    }
    finally { $writer.Dispose(); $zip.Dispose() }
    try { return ,$destination.ToArray() } finally { $destination.Dispose() }
}

$artifacts = @()
foreach ($name in @('midnight-web-viewer-api', 'midnight-web-viewer-sdk')) {
    & $cargo package --manifest-path $manifest -p $name --registry midnight --config $config `
        --target-dir $target --no-verify --allow-dirty --offline
    if ($LASTEXITCODE -ne 0) { throw "Cargo packaging failed for $name" }
    $package = $metadata.packages | Where-Object name -eq $name
    $prefix = $name + '-' + $package.version
    $bytes = ConvertTo-CanonicalPackage (Join-Path $target "package/$prefix.crate") $prefix
    $checksum = [Convert]::ToHexStringLower([Security.Cryptography.SHA256]::HashData($bytes))
    $dependencies = @()
    foreach ($dependency in $package.dependencies) {
        if ($dependency.registry -ne $registryUrl -or $dependency.name -notin @('midnight-web-viewer-api')) {
            throw 'Release dependency must name the public web-viewer registry API'
        }
        $dependencies += @{ name = $dependency.name; req = $dependency.req; kind = 'normal';
            target = $null; registry = $null; features = @($dependency.features);
            optional = $dependency.optional; default_features = $dependency.uses_default_features }
    }
    $record = @{ name = $name; vers = $package.version; deps = $dependencies; cksum = $checksum;
        features = $package.features; yanked = $false; links = $null; rust_version = $package.rust_version }
    $index = Join-Path $registry ('index/' + $name.Substring(0, 2) + '/' + $name.Substring(2, 2))
    $null = New-Item -ItemType Directory -Path $index -Force
    [IO.File]::WriteAllText((Join-Path $index $name), (ConvertTo-Json -Depth 20 -Compress $record) + "`n")
    [IO.File]::WriteAllBytes((Join-Path $registry "$prefix.crate"), $bytes)
    $artifacts += @{ archive = "$prefix.crate"; sha256 = $checksum; bytes = $bytes.Length }
}
[IO.File]::WriteAllText((Join-Path $Output 'artifacts.json'), (ConvertTo-Json $artifacts) + "`n")
Write-Output $config
