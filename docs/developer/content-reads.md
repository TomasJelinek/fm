# Host content reads

`midnight-plugin-host::CapturedFile` retains the original content length and
revision. A host-owned `ContentSource` supplies bytes without exposing a native
path or Rust object to a plugin. These Rust interfaces are internal to the host;
the public plugin reader ABI is unchanged.

Both direct broker reads and retained plugin readers use the same boundary:

- Reject zero-sized requests, requests above 256 KiB and offset overflow.
- Clamp requests to the original captured length.
- Validate the source before returning empty bytes at or beyond EOF.
- Pass the original length/revision to `ContentSource::read_checked`.
- Reject responses whose byte count differs from the clamped request.

The provided `read_checked` implementation validates the live source and compares
its length/revision before and after `read_at`. Generic providers need only
implement the existing read and validation methods. A detectable mutation during
the read is an error, and its returned bytes are not published.

A trusted host adapter may override `read_checked` only if its underlying read
already enforces equivalent identity/revision validation before and after reading.
It must check the caller's original expected metadata, never replace it with newly
opened metadata. The outer bounds and returned-count checks still apply. The
native filesystem adapter uses this to avoid additional validation round trips
around the native SDK's already validated `ReadAt` operation. An evicted native
reader must match its original captured identity/revision when reopened.

`CapturedFile::revalidate` always performs live validation. Empty reads also keep
that check; they cannot be used to turn a changed source into a successful result.
Validation is the documented LiveValidated contract, not a filesystem snapshot
or a guarantee against changes that the platform's metadata cannot detect.
Capture, reading, validation and disposal remain backend work.

## File systems

NTFS and ReFS on Windows, and every Linux file system, give each file a stable identity.
Other file systems give less, and every feature below works within what they give. The
user-facing summary is [File systems](../user-guide.md#file-systems) in the user guide.

### Identities on Windows

One rule decides a file's identity everywhere: content captures
(`midnight-plugin-host/src/native_windows.rs`), the native filesystem plugin's reads and
Rename (`plugins/midnight-filesystem-native-windows/src/identity.rs`) and the host's Find,
Delete, Rename, F7, metadata and folder-size queries
(`midnight-platform-windows/src/mutation_native.rs`, `local_identity`):

1. `FileIdInfo` answers a nonzero 128-bit ID: the 64-bit volume serial number and that
   ID (NTFS, ReFS, SMB 3 servers).
2. `FileIdInfo` fails because the file system or server does not implement it, or it
   answers an all-zero ID (MS-FSCC 2.4.26: a server without 128-bit IDs): the 32-bit
   volume serial number and the 64-bit file index of `GetFileInformationByHandle`,
   widened (FAT32, exFAT, CDFS, UDF, the WebDAV redirector, SMB 2 servers).
3. Every other failure stays an error.

"Does not implement the information class" is one set of Windows errors, the same in
every crate: `ERROR_INVALID_FUNCTION` (1), `ERROR_NOT_SUPPORTED` (50),
`ERROR_INVALID_PARAMETER` (87; FAT32 and exFAT, measured), `ERROR_CALL_NOT_IMPLEMENTED`
(120) and `ERROR_INVALID_LEVEL` (124). Denials, lost connections and every other error
stay errors. The same set lets `FileAttributeTagInfo`, which FAT32 and exFAT do not
implement either, fall back to the attributes of `FileBasicInfo` with no reparse tag; an
entry whose attributes still claim a reparse point is then refused as a link. A volume
answers one way for every file, so a capture and its revalidations always compare
identities of one kind, and a Find result and the plugin's revision of the same file
agree.

Rule 2 identities are weaker:

- FAT derives the file index from the position of the entry's directory record. It is
  unique among existing entries but changes when an entry is renamed to a longer name or
  moved, so features that compare an identity across their own rename (Move, Rename's
  recoverable replacement, Copy's namespace pins) still require a 128-bit ID and refuse
  such volumes. Rename, F7, Delete and Find compare identities only before their own
  change.
- The WebDAV redirector, CDFS and some SMB servers report index 0 for every entry. Such an
  identity is *unknown* (`Identity::is_known`, `TransferIdentity::is_known`): Find does not
  take a folder with it for its own ancestor or a file for another name of a counted
  file; folder sizes, Properties and the folder tree key such folders by their canonical
  path instead (`size_directory.rs`); the Command Prompt working-directory check refuses
  it. Revisions still compare it, so a replacement with the same length and times
  goes unnoticed there.
- FAT keeps no change time (always 0) and write times in 2-second steps; exFAT keeps
  10-millisecond write times. A same-length change within those steps is not detected.

Copy, checksum calculation and drags read a source without a file ID through bindings
that their open handles keep, as on SMB shares without IDs
([transfers](transfers.md)): on a network share, and since 0.6.4 on a local FAT, FAT32,
exFAT, CDFS or UDF volume (`transfer_network_identity.rs`, `reads_without_ids`). Saving a
checksum manifest and verifying one still need proven file IDs on the manifest's volume.
Before 0.6.4 every content capture on such a volume failed (no viewer, thumbnail,
comparison or archive could read its files) and Copy refused a source on it.

### Linux

Linux identities are `(st_dev, st_ino)` from `fstat`/`statx`, which every file system
answers. vfat and exfat number inodes per mount (`iunique`), and so do CIFS mounts with
`noserverino` and FUSE mounts such as sshfs without `-o use_ino`: a number is stable
while the file is open or cached but can change after the inode leaves the cache or the
volume is mounted again. A saved archive revision (a restored tab or archive Hot Path) or
a Find result opened much later can then be reported as changed, and must be opened or
searched again. vfat keeps 2-second write times, exfat 10 ms and sshfs 1 s.

### What each file system supports

| File system | Reads (panels, Properties, viewers, thumbnails, comparisons, archives, Find, folder sizes, checksum calculation) | Changes (Rename, F7, Delete) | Copy and Move | Watches |
| --- | --- | --- | --- | --- |
| NTFS, ReFS; ext4, XFS, Btrfs, tmpfs | Yes | Yes, with the Trash | Yes | Native |
| Windows FAT32, exFAT | Yes (rule 2) | Yes; no Recycle Bin, so Delete is permanent | Copy to, from and within; Move to it; Move from or within refused | Native |
| Windows CD, DVD, ISO (CDFS, UDF) | Yes (rule 2, index 0 on CDFS) | Read only | Copy from | Native |
| Windows SMB share or mapped drive | Yes (rule 1, or rule 2 on servers without IDs) | Yes; no Recycle Bin | Copy; Move from or within needs server file IDs | SMB change notifications |
| Windows WebDAV folder | Yes (rule 2, index 0) | Not verified | Copy from (not verified) | None: checked every second or more, notice after a minute |
| Linux vfat, exfat, ntfs3 | Yes | Yes when the mount maps ownership to the user; Trash only on the home file system | Copy and Move to refused (no `O_TMPFILE`); Move from refused | inotify |
| Linux NFS | Yes | Refused: no `RENAME_NOREPLACE` | Copy and Move to refused | inotify (local changes) and the 15-second check |
| Linux CIFS | Yes | Yes when the mount maps ownership to the user | Copy and Move to refused | inotify (local changes) and the 15-second check |
| Linux FUSE (sshfs, GVfs) | Yes | sshfs refused: no `RENAME_NOREPLACE` | Copy and Move to refused | inotify (local changes) and the 15-second check |

Evidence: Windows FAT32 `FileIdInfo` → `ERROR_INVALID_PARAMETER` was measured on a USB
stick; the other Windows rows follow MS-FSCC (appendix notes for 2.1.9, 2.1.10 and 2.4) and
the FAT and CDFS driver samples; the Linux rows follow the kernel's `fs/nfs`, `fs/fat`,
`fs/exfat` and FUSE rename and `O_TMPFILE` support. Only NTFS, ext4 and tmpfs are tested
on every run. The opt-in tests that read real volumes skip themselves when their variable
is unset:

| Variable | Tests (`--ignored`) |
| --- | --- |
| `MIDNIGHT_TEST_FAT_FILE` | `midnight-plugin-host` `native_windows::identity_tests`; native Windows plugin `identity::tests` |
| `MIDNIGHT_TEST_SHARE_FILE`, `MIDNIGHT_TEST_OPTICAL_FILE` | native Windows plugin `identity::tests` |
| `MIDNIGHT_TEST_FAT_DIRECTORY` | `midnight-platform-windows` `size_directory::tests` (read only), `tests/mutations.rs`; `midnight-engine` `operations_real_windows` (writes a temporary folder) |
| `MIDNIGHT_TEST_SHARE_DIRECTORY` | `midnight-platform-windows` `size_directory::tests` (read only) |
| `MIDNIGHT_TEST_SMB_DIRECTORY` | `midnight-engine` `operations_real_windows::network_copy` (writes) |

## Change-time drift (VIEW-05)

A native revision includes the change time (NTFS ChangeTime, Linux ctime), which
also moves on metadata-only updates. Endpoint protection, for example, adds an
alternate data stream to a file seconds or hours after it was written and may set its
write time back, so that only the change time moves while the volume, file ID, length
and LastWriteTime stay the same. Every capture therefore carries an explicit
`RevisionPolicy` (`midnight-plugin-host/src/content.rs`):

- `Exact`, the default of `CapturedFile::capture` and `InstalledProvider::capture`:
  every captured field must still match. Comparison, command-icon artwork import,
  manifest and package captures use it; the comparator's own content check for a
  moved change time is described in [file comparison](file-comparison.md), and the
  image viewer's manifest pin's in [plugin manifests and
  executables](#plugin-manifests-and-executables).
- `AcceptChangeTimeOnly`: a revision whose change time alone moved is unchanged.
  Windows still requires the same volume, file ID, length and LastWriteTime; Linux the
  same device, inode, length and mtime. These captures use it:
  - viewers, through `CapturedFile::capture_with` (the viewer engine's in-process
    capture) and `InstalledProvider::capture_with` (the application's viewer capture);
  - archive files: `InstalledProvider::capture_matching` captures an archive for
    navigation, restored tabs and Hot Paths, and Find inside archives, and compares a
    saved revision under it as well
    ([archive members as a source](transfers.md#archive-members-as-a-source)). The
    icon check of user-command buttons shares the engine's archive capture callback,
    so it accepts such a change too;
  - thumbnails: `midnight_thumbnail_host::service::NATIVE_REVISIONS`, which
    `native_content::thumbnail_factory` and the acceptance harness's thumbnail pulse
    use. The thumbnail cache revalidates a capture when it stores a frame and on every
    hit. A capture's cache key keeps its revision bytes, change time included, so a new
    capture after such a change decodes the file once more under a new key; a frame
    cached for the same capture stays a hit.

The revision bytes that a capture reports (`stable_version`) never change. The
native filesystem plugin's reader keeps checking each read against its own revision,
change time included, and its package and wire format are unchanged. Under
`AcceptChangeTimeOnly` the host's capture cache therefore handles a failed native
read or validation by releasing that reader and reopening the path, at most twice
per operation, and adopts the new reader only when its revision differs from the
captured one in nothing but the change time; a reopen that finds any other
difference fails the operation. The adapter still checks the caller's original
length and revision against the capture, then passes the adopted reader's own
revision to that reader's checked read.

Such a refused reader is a lasting change: the file at the captured path is no longer
the captured content, and a later read is not expected to succeed. The cache remembers
it, and `ContentSource::changed_since_capture` and `CapturedFile::changed_since_capture`
report it without I/O (a source that cannot tell reports false, as does an in-process
native capture). A failure for another reason, such as a reopen of an unreachable
file, is not one. Archive mounts report a lasting change as `InvalidData`, which Copy
does not offer to retry.

Regression tests are in `midnight-plugin-host/src/checked_read_tests.rs`, with
native adapter/cache tests in the native filesystem host. They cover both direct
and retained readers, original metadata forwarding, bounds/EOF/counts, and
detectable changes during a generic read. Archive captures are tested in
`midnight-native-filesystem-host/src/installed_archive_capture_tests.rs` and its
`_windows_tests.rs` and `_linux_tests.rs` companions, through the installed provider's
cache, with emulated plugin readers over fake revisions and over a real file that gets
an alternate data stream with its LastWriteTime set back (Windows) or a mode change
(Linux); the ignored `installed_archive_capture_reads_on_after_the_archive_was_tagged`
in `tests/native.rs` repeats that through the installed native plugin.

### Plugin manifests and executables

A file that decides which code runs keeps an exact revision, change time included:
whoever can write the file can set its modification time back after changing it, and
the change time still shows the change. (On Windows, write-attributes access can also
set ChangeTime through `FileBasicInfo`, as `trusted_executable_windows_tests.rs` does,
so there it shows only a change whose author restored the modification time alone;
Linux cannot set ctime back.) A check of the content is stronger than the change time,
so a change time that alone moved is accepted where the content is checked:

- **Manifests.** The image viewer's worker identity pins its `image-viewer.json` from
  preparation until an open adopts the worker, with `ManifestPin`
  (`midnight-plugin-host/src/manifest_pin.rs`). The pin keeps the complete bytes that
  the host parsed and matched against the catalog (at most 16 KiB). A check that finds
  the pinned revision reads nothing. When only the change time moved (Windows: the
  same volume, file ID, length and LastWriteTime; Linux: the same device, inode, length
  and mtime), the pin captures the path again, reads the whole file once under that
  fresh capture's exact checks and accepts it only when the bytes equal the pinned
  ones; the fresh capture then replaces the pinned one, so later checks read nothing
  again. Any other difference, a capture or read that fails and different bytes are
  refused with the revision error ("Source changed since capture ..."), so a manifest
  changed in place with its write time set back is still refused. Equal bytes restored
  after a change pass: the pin proves the content, not its history. Discovery reads a
  manifest once, right after its capture, and keeps `Exact`; the video viewer compares
  its descriptor's bytes rather than its revision.
- **Executables.** The Windows `TrustedExecutablePin` holds a handle that denies data
  writes, deletion and renaming for as long as it pins the executable, so it accepts a
  ChangeTime alone without reading the bytes
  ([worker preparation](image-viewer-api.md#worker-preparation)). The Linux pin is
  an exact capture, ctime included: no lease keeps writers out there, and a content
  check would need a digest of every executable at every capture, which each worker
  start would pay. A ctime-only change of a Linux executable (a mode, owner or extended
  attribute change) therefore still refuses that one start; a new open captures again.

`midnight-plugin-host/src/manifest_pin_tests.rs` and its `_windows_tests.rs`
companion test the pin over real files: a moved change time alone (a read-only
attribute or mode toggled and restored, and on Windows an added alternate data stream
with LastWriteTime set back) passes after one re-read and none after it, while other
bytes behind a restored write time, a moved write time and a replacement with equal
bytes are refused, the last two without a re-read. The image viewer's
`startup_manifest_tests.rs` repeats the first two through `ProviderIdentity`.
