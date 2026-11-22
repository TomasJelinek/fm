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
also moves on metadata-only updates: on the Windows development machine endpoint
protection moves it on freshly written files one or two seconds after they are
written, while the volume, file ID, length and LastWriteTime stay the same. Every
capture therefore carries an explicit `RevisionPolicy`:

- `Exact`, the default of `CapturedFile::capture` and `InstalledProvider::capture`:
  every captured field must still match. Comparison, command-icon artwork import,
  manifest and package captures use it; the comparator's own content check for a
  moved change time is described in [file comparison](FILE_COMPARISON.md).
- `AcceptChangeTimeOnly`, which viewers request through
  `CapturedFile::capture_with` (the viewer engine's in-process capture) and
  `InstalledProvider::capture_with` (the application's viewer capture): a revision
  whose change time alone moved is unchanged. Windows still requires the same volume,
  file ID, length and LastWriteTime; Linux the same device, inode, length and mtime.
  Since 21 November archive files use it too: `InstalledProvider::capture_matching`
  captures an archive for navigation, restored tabs and Hot Paths, and Find inside
  archives, and compares a saved revision under it as well. Endpoint protection had
  added an alternate data stream to a tar file two hours after it was written, and
  every later read of the open archive failed
  ([archive members as a source](TRANSFERS.md#archive-members-as-a-source)). The icon
  check of user-command buttons shares the engine's archive capture callback, so it
  accepts such a change too. Since 22 November thumbnails use it as well
  (`midnight_thumbnail_host::service::NATIVE_REVISIONS`, which
  `native_content::thumbnail_factory` and the acceptance harness's thumbnail pulse use;
  WIN-11): Microsoft
  Defender's data loss prevention writes a `sec.endpointdlp` alternate data stream to
  new files and sets their write time back, so only the change time moves, and the
  thumbnail cache, which revalidates a capture when it stores a frame and on every
  hit, failed those files. A capture's cache key keeps its revision bytes, change
  time included, so a new capture after such a change decodes the file once more
  under a new key; a frame cached for the same capture stays a hit.

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
`midnight-native-filesystem-host/src/installed_archive_capture_tests.rs`, through the
installed provider's cache, with emulated plugin readers over fake revisions and over a
real file that gets an alternate data stream with its LastWriteTime set back (Windows)
or a mode change (Linux); the ignored `installed_archive_capture_reads_on_after_the_archive_was_tagged`
in `tests/native.rs` repeats that through the installed native plugin.
