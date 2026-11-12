# Independent native Linux filesystem

This separate Cargo workspace builds `libmidnight_filesystem_native_linux.so`.
Dependencies are the public filesystem API/SDK and pinned `libc` with `std` only.
The MIT branch and complete attribution are retained in `THIRD-PARTY.md`.
The repository's own license remains undecided.

```sh
export CARGO_TARGET_DIR="$PWD/_build/goal/native-plugin/native-linux"
cargo test --locked --offline --manifest-path plugins/midnight-filesystem-native-linux/Cargo.toml
cargo build --locked --offline --release --manifest-path plugins/midnight-filesystem-native-linux/Cargo.toml
```

Fixture TMPDIR must be under a dedicated `_build` directory. On WSL use a native
Linux filesystem for syscall/race fixtures and set
`MIDNIGHT_NATIVE_FIXTURE_ROOT` to that owned `_build` directory. Tests load the
actual shared library and exercise raw byte names, independent selected-file
reads, >4300-byte paths, cancellation, stale revisions, link/FIFO refusal,
directory-kind replacement and pinned-file reopening after FIFO substitution.

Listing/read/stat/volumes/storage-space and optional raw labels/rich volume
information are provided. Physical PCIe NVMe/ATA storage is classified only after
positive topology, identity and rotation checks; virtual/stacked/removable devices
remain Unknown. Rich capacity distinguishes physical total, free and caller
available; mount fields come from the held descriptor's mount ID. Procfs/sysfs-style content without a reliable known length is explicitly
Unsupported. Reads are live validated regular files, not snapshots. This package
is optional and needs compatible host routing; x86-64 GNU/Linux is qualified,
aarch64 and macOS are not. See [protocol](../../filesystem-api/NATIVE_PLUGIN_PROTOCOL.md).
