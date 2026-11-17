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

Listing/read/stat/volumes/storage-space, optional raw labels/rich volume
information and optional prepared same-parent Rename are provided. Each listed
row costs one no-follow `fstatat`; nothing is opened and automount points are not
mounted. A listed link is shown as a directory link when its target's cached type
is a directory (`statx` without automount or attribute sync); unresponsive servers
along a link's path can still delay that row. Physical PCIe NVMe/ATA storage is
classified only after positive topology, identity and rotation checks; dm/md
stacks are classified only when every backing leaf is proven local physical
media, and virtual or removable devices remain Unknown. Rich capacity
distinguishes physical total, free and caller available; mount fields come from
the held descriptor's mount ID. Procfs/sysfs-style content without a reliable
known length is explicitly Unsupported. Reads are live validated regular files,
not snapshots: content is opened once, and each validation checks the current
name through a metadata-only `O_PATH` descriptor.

Prepared Rename pins the selected entry and its parent before a name is chosen.
The parent must be owned by the user or root and either sticky or writable only
by the user (group write is accepted only for the user's private group, as on
Ubuntu or Fedora with umask 002); otherwise preparation is refused before a name
is requested. Every Rename, including no-replace, moves the entry through a
hidden `.midnight-rename-*` stage beside it and publishes it with
`RENAME_NOREPLACE`, so the result is the exact pinned inode; watchers briefly
see the stage. The stage is created owner-only; filesystems whose permissions
come from mount options (vfat, exfat, NTFS, CIFS without POSIX extensions, many
FUSE mounts) keep their mount-provided mode, which is accepted only while no
other user can write to the stage. A stage this provider refuses is removed
again. The stage and parent directories are synced (up to three directory syncs
per Rename) and results report `durability_confirmed: true`. On case-insensitive
mounts a case-only change of a single-link entry or a directory is published
like any other name; another hard link of the source remains a conflict.
Recoverable replacement of a regular file (`RENAME_EXCHANGE`, displaced target
retained in the stage and reported as a recovery receipt) is offered only in
trusted ext2/3/4, XFS, Btrfs and tmpfs parents. On other filesystems a failure
that could still complete remotely retains the empty stage and reports it as a
recovery receipt; definite refusals (for example permission, read-only, an item
another SMB client holds open, or unsupported rename flags) and failures before
the stage is used leave nothing behind. NFS mounts and FUSE mounts without
rename flags cannot use this Rename: the kernel NFS client refuses every
`renameat2` flag, so preparation is refused on NFS, and a FUSE server's refusal
of `RENAME_NOREPLACE` is reported as Unsupported after the unused stage is
removed. This package is optional and needs compatible host routing; x86-64
GNU/Linux is qualified, aarch64 and macOS are not. See
[protocol](../../filesystem-api/NATIVE_PLUGIN_PROTOCOL.md).
