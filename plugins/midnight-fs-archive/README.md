# Midnight archive filesystem worker

Independent read-only executable plugin for ZIP, 7z, GZIP, XZ, BZIP2, Zstandard,
TAR and TAR compressed by GZIP/XZ/BZIP2/Zstandard. This
project consumes only versioned public filesystem API and worker SDK packages,
plus pinned open-source format libraries. Its tests create real fixtures locally.
It neither depends on nor writes to application source files.

The helper must be started by the host's restricted-worker launcher. It accepts
no input filename and no command arguments. It completes the public worker SDK
sandbox handshake before accepting archive bytes. A failed sandbox is terminal.
The host supplies bounded source bytes and owns cancellation and revision checks.

Supported content: ZIP store/deflate; GZIP including concatenated members;
TAR regular files and directory metadata (GNU/PAX long names provided by tar-rs);
7z Copy/LZMA/LZMA2 and enabled pure Rust BCJ filters, solid and non-solid. Encrypted,
unsupported codec and multi-volume input is not silently delegated to an external
utility. Links, special entries, traversal, absolute names, backslashes, colons
and NUL names are not readable or extractable. Duplicate member names have
independent opaque IDs. GZIP's untrusted FNAME never controls its synthetic member
name, which is `content`.

The package advertises `stable_entry_ids = true`: reopening identical source bytes
with the same format and implementation version assigns the same member IDs.
Indexing follows archive order; implicit directories are inserted on first encounter,
and duplicate file names receive distinct IDs. Hash-map iteration never assigns IDs.
Changes to this indexing contract require a package version change. The host combines
that version, provider, format, captured container identity/revision and member ID
before authorizing persistent thumbnail reuse. Older packages without the capability
remain browsable but do not grant persistent member-preview cache authority.

ZIP and 7z mounting index metadata without decompressing file bodies. ZIP's first
read decodes and verifies that complete member. A 7z first read decodes only the
requested compression block, serially, and checks all its member lengths and
available member/block CRCs before publishing any bytes. Solid-block siblings
are drained for integrity, while only the requested member is staged; unrelated blocks remain compressed. Corrupt payloads
can therefore be listed but fail when read, just as with ZIP.
TAR indexes checked u64 ranges without reading its file payloads. Selected members
stream directly into private host-owned TEMP files through the protocol-2 staging format (protocol3 in this release). Compressed
TAR must decode and validate its complete outer stream into one private scratch
file to discover headers; member leases reuse checked ranges of that verified file.
Standalone compressed streams use the same verified scratch storage. No complete
encoded archive or decoded member is retained in the production worker's RAM.

There are no artificial source, member, aggregate disk or expansion-ratio byte
ceilings. Available storage and native file-offset ranges limit content length;
disk-full, short writes and flush failures discard staging and fail the request.
Metadata remains bounded to 100,000 entries and 8 MiB of names; 7z's raw next-header
allocation is bounded to 64 MiB. The pinned 7z decoder has no public decoded-header
allocation hook, so compressed-header allocation remains subject to process memory
containment. Decoded member lengths and all available checksums must match before
sealing/publication. There is no archive writing, password support, RAR codec or
nested mounting in this release.
XZ dictionaries and Zstandard windows are bounded to 64 MiB; BZIP2 uses its format's
bounded block sizes. Concatenated stream members are supported. Zstandard frame
checksums are verified explicitly when present. Checksums never replace the host's
SHA-256 destination readback when copying out.

The host keeps a bounded count of sealed content leases, revalidates the captured
source on every read, and releases files on backend cleanup. The decoder receives
only session IDs and bounded byte messages, never TEMP paths or native handles.
Cancellation ends the worker and rolls back unpublished storage. A 20-second idle
watchdog is renewed by completed bounded I/O and actual decoded-byte progress;
archive decoding opts out of an aggregate CPU deadline. A blocked OS I/O call can
delay its owning backend's cleanup until that call returns.

Protocol3 supports cooperative thumbnail member cancellation at StageBegin,
StageAppend, StageProgress and StageSeal exchanges. The host rejects a valid
checkpoint with Failed(Cancelled); the worker unwinds and returns one matching
terminal acknowledgement, preserving the immutable archive for another request.
The host retires only that request's unpublished member spool. Source-reader
Interrupted remains retryable and is not used for this handshake. Ordinary hard
cancellation and legacy installed protocol1/2 workers retain session-ending
semantics. Decoder timeout and viewport cancellation both reach the cooperative
path; an already-running OS call or decoder interval can still delay it.

Build and test using `scripts/build.ps1` or `scripts/build.sh`, with Cargo/Rust and
Python 3 on PATH, providing an offline
public SDK config, a build output directory and an optional package destination.
Source directories are not output destinations. Exporting this directory does not
require the host checkout. See THIRD-PARTY.md for pinned dependency decisions.

For reproducible opening measurements, run the ignored
`measure_tar_and_sevenzip_open` test in `tests/performance.rs` with
`cargo test --release --test performance measure_tar_and_sevenzip_open -- --ignored --nocapture`
and the same public SDK configuration used to build this project. It generates
64 deterministic 256 KiB members and reports five separate opening, listing and
first/last read timings for TAR, independent 7z blocks and solid 7z. Fixture creation
and source cloning are excluded; reads compare exact contents. Set
`MIDNIGHT_ARCHIVE_PERF_DIR` to a directory beneath `_build/` to retain the archives
for host/sandbox measurements. Ordinary tests use no wall-clock thresholds.

Host browsing uses opaque member IDs. Saved tabs store the provider ID, native
container path, captured source revision and raw member components separately.
On restart only active tabs reopen; missing providers and changed sources remain
unavailable read-only tabs with their addresses retained. Display labels never
become native paths. In-session tabs, parent navigation, history, sorting,
filtering and selection operate on provider identity. F3 receives a bounded
content lease; F5 captures the archive selection and queues copy-out through the
host's ordinary 32-job FIFO, with exclusive staging, SHA-256 source hashing,
independent complete destination readback, and no-replace publication. The worker
never receives a destination path and never writes files. Native clipboard
export, external editing, shell launch, and archive mutations
operations are unavailable for archive members.
Copy-out uses raw member names and fails on names that cannot be represented
losslessly by the target platform; it never substitutes display labels.
