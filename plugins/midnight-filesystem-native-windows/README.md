# Independent native Windows filesystem

This separate Cargo workspace builds `midnight_filesystem_native_windows.dll`.
It depends on the public filesystem API/SDK and pinned MIT-licensed Windows
bindings; no application/private crate is linked. Retain `THIRD-PARTY.md` in
distributed packages. The repository's own license remains undecided.

```powershell
$env:CARGO_TARGET_DIR = "$PWD/_build/goal/native-plugin/native-windows"
./scripts/cargo.ps1 test --locked --offline --manifest-path plugins/midnight-filesystem-native-windows/Cargo.toml
./scripts/cargo.ps1 build --locked --offline --release --manifest-path plugins/midnight-filesystem-native-windows/Cargo.toml
```

Tests include actual restricted DLL loading, callback origin, native selected
bytes, raw unpaired-surrogate names, long paths, trailing-dot/space siblings,
cancellation and stale content revisions. `plugin.toml` describes an optional
`filesystem-native` package for a compatible trusted host. The package cannot
add functionality before the host composition loads and routes through it.

Listing/read/stat/volumes/storage-space and optional raw labels/rich volume
information are provided. Physical storage classification requires positive
single-device, nonremovable ATA/SATA/NVMe and seek-penalty evidence. Other devices
remain conservative. Rich capacity keeps actual filesystem total/free distinct
from caller quotas. Reads are live metadata-validated regular files, not snapshots.
Cloud data placeholders are readable when their retained handle reports a member
of the Windows Cloud reparse-tag family without the name-surrogate bit. The
provider opens the reparse point itself and checks its handle type/tag; symlinks,
junctions, devices, directories and unrelated reparse providers remain refused.
Identity, length and write/change revision checks still apply before/after reads.
The optional `cloud_content` resident acceptance test requires one explicitly
selected, already resident PNG through `MIDNIGHT_CLOUD_TEST_FILE`; normal tests
also cover stale name replacement and junction refusal without cloud access.
See [protocol](../../filesystem-api/NATIVE_PLUGIN_PROTOCOL.md) for bounds and
explicit unsupported namespaces. Mutations and watching are outside this ABI.
