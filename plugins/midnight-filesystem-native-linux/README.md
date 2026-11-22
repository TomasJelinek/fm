# Independent native Linux filesystem

This separate Cargo workspace builds `libmidnight_filesystem_native_linux.so`.
Dependencies are the public filesystem API/SDK and pinned `libc` with `std` only.
The MIT branch and complete attribution are retained in `THIRD-PARTY.md`.

```sh
export CARGO_TARGET_DIR="$PWD/_build/native-filesystems/linux"
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
row costs one no-follow `fstatat`; nothing is opened and automount points are
not mounted. A link is listed, and reported by Stat, as a directory link when
its target is a directory: one `statx` of the target's type, which does not open
it or trigger an existing automount point, so navigating into a link to an
offline automount reports the mount's own error. The kernel still resolves the
target path for that `statx`: a link into a network mount whose server stops
answering (for example a hard-mounted NFS export), or to an autofs map key that
is not present yet, holds its listing page until the server or automounter
answers. Physical PCIe NVMe/ATA storage is classified only after positive
topology, identity and rotation checks; dm/md stacks are classified only when
every backing leaf is local. A disk attached to the machine that proves no media
type (a virtual or cloud machine's disk, such as WSL 2's Hyper-V disk, a USB, MMC,
virtio, Xen or SCSI disk) is `Local` (ABI minor 4; the package requires host minor
4); virtual block devices (loop, nbd, rbd, NVMe fabrics), disks behind an iSCSI
session or Fibre Channel remote port, and filesystems other than single-device block
filesystems (ext2/3/4, XFS, F2FS, FAT, exFAT, ntfs3) remain Unknown. Rich capacity distinguishes physical total, free and
caller available; mount fields come from the held descriptor's mount ID.
Procfs/sysfs-style content without a reliable known length is explicitly
Unsupported. Reads are live validated regular files, not snapshots: content is
opened once, and each validation checks the current name through a metadata-only
`O_PATH` descriptor.

Prepared Rename pins the selected entry and its parent before a name is chosen.
The parent must be owned by the user or root and either sticky or writable only
by the user: group write is accepted only for the user's private group (as on
Ubuntu or Fedora with umask 002) and only while the folder has no POSIX access
ACL, because with one the group bits are the ACL mask, which can let named users
or groups write. Otherwise preparation is refused before a name is requested,
and execution checks again. Every Rename, including no-replace, moves the entry
through a hidden `.midnight-rename-*` stage beside it and publishes it with
`RENAME_NOREPLACE`, so the result is the exact pinned inode; watchers briefly
see the stage. The stage is created owner-only; filesystems whose permissions
come from mount options (vfat, exfat, NTFS, CIFS without POSIX extensions, many
FUSE mounts) keep their mount-provided mode, which is accepted only while no
other user can write to the stage (the same private-group and ACL conditions
apply), so the stage never lets other users change more than the protected
parent does. Permission bits protect only where the kernel enforces them: FUSE
mounts with `allow_other` but without `default_permissions` and CIFS mounts
with `noperm` exclude other local users only as far as their server does. A
stage this provider refuses is removed again. The stage and parent directories
are synced (up to three directory syncs per Rename) and results report
`durability_confirmed: true`. On case-insensitive
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
removed. See [protocol](../../filesystem-api/NATIVE_PLUGIN_PROTOCOL.md).

`plugin.toml` describes the `filesystem-native` package for `x86_64-unknown-linux-gnu`
(ABI 1, minimum host minor 4). The application loads it from
`filesystems/native.linux/` beside its executable and browses and reads local files
only through it, with no built-in fallback when the package is absent or
incompatible. `scripts/build-native-filesystem.sh` builds and installs it. macOS has
its own [package](../midnight-filesystem-native-macos/README.md).
