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
