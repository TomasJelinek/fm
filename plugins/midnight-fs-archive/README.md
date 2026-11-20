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
utility: such ZIP members, and the members of such 7z blocks, are listed but
unreadable while the rest of the archive stays browsable. Links, special entries,
traversal, absolute names, backslashes, colons, NUL names and a file named by a
final `.` component are not readable or extractable; such records stay visible at
the root under their full raw name, and unsafe directory records are listed as
non-navigable "other" entries. Harmless `.` components (`./usr/bin/tool`, as in
`tar -C dir -cf x.tar .` and Debian data archives) name the current directory.
TAR PAX global headers (`git archive`) are archive metadata, not members.
TAR and 7z duplicate member names have independent opaque IDs. The pinned ZIP
reader indexes central-directory records by name, after applying any Info-ZIP
Unicode Path field (the name 7-Zip and Info-ZIP display), and merges records that
repeat such a name into one entry with the last record's data. The worker walks the
records the same way, so that ambiguous entry is listed once but unreadable; an
archive whose records it cannot reconcile with the reader's index is refused.
GZIP's untrusted FNAME never controls its synthetic member name, which is `content`.
A source that the decoder rejects as corrupt while opening fails with the first line
`This file is not a valid <format>.` (for example `ZIP archive` or `gzip file`), then
the decoder's own reason, since an archive member opened as an archive may be any file.
A failed read of the source itself (a dropped share, a removed drive, or the host's
failure reply to a source read) is never taken for a corrupt archive: opening or
reading a member then fails with that read's own reason and code (`Io`, or the code
the host replied with), whatever error a decoder made of it.

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
available member/block CRCs before publishing any bytes; unrelated blocks remain
compressed. Solid-block siblings are drained for integrity, and only the requested
member is staged. Readable siblings of that block are also kept in worker memory,
later members first and then earlier ones nearest first, up to 128 MiB. After the
whole block verifies, reading one of them stages it from memory without another
block decode; it leaves worker memory once staged, and the host's member cache
serves repeated reads. Reading the members of a block therefore costs one block
decode only while the members read after that decode fit the 128 MiB window: a
larger block decodes again about once per 128 MiB copied in block order, and a read
outside the window (a thumbnail or a sorted copy out of block order) decodes the
whole block again. Decoding another block replaces the retained siblings. Corrupt
payloads can therefore be listed but fail when read, just as with ZIP, and nothing
unverified is retained. TAR indexes checked u64 ranges without reading its file
payloads. Selected members stream directly into private host-owned spool files
through the protocol-2 staging format (protocol 3 in this release). Compressed TAR
must decode and validate its complete outer stream into one private scratch file to
discover headers; member leases reuse checked ranges of that verified file.
Standalone compressed streams use the same verified scratch storage. No complete
encoded archive is held in the production worker's RAM. Source and scratch reads go
through a seek-preserving window of at most 256 KiB, so ZIP local-header probes, TAR
header walks and Zstandard frame peeks reuse fetched bytes instead of refetching a
window per entry.

There are no artificial source, member, aggregate disk or expansion-ratio byte
ceilings: decoded content is bounded by the free space of the host's private spool
volume and by native file-offset ranges. The host keeps a free-space reserve on that
volume: staging fails with a disk-full error before the volume's free space would
fall below max(1 GiB, 2% of its capacity), so a highly compressed member cannot fill
it. The host queries the volume on each stage's first append and at least every
64 MiB appended; writes by other programs between two queries are not counted.
Disk-full, short writes, flush failures and the reserve discard staging and fail
the request.

Nothing limits how many members an archive has or how many bytes their names take:
the worker indexes every member, and only its process memory, which the host sets
to the worker SDK's largest policy of 1 GiB, bounds the index. Each member costs one
compact record and its name bytes (its label only when that differs from the raw
name), and readable members a sorted locator entry. Indexing a million members in
folders of 1,000 (`measure_index_memory`, below) peaks at about 105 bytes per TAR or
compressed TAR member, 420 per 7z member (one-byte members, solid blocks of 1,000)
and 690 per ZIP member, about 500 of which the pinned ZIP reader's own index takes
(it parses the whole central directory, names included, and the duplicate-record
walk briefly keeps one more copy of the distinct names). A 1 GiB worker therefore
opens about nine million TAR, two million 7z or one and a half million ZIP members.
An index that outgrows the worker's memory fails the archive with a reason that
names how many members were indexed; an allocation inside a decoder or the ZIP
reader ends the worker instead, and the host then says that the worker ended
unexpectedly, for example for lack of memory.

Bounds that remain protect against crafted data, not against large archives. The
protocol limits each name to 64 KiB. A TAR long-name, long-link or PAX record,
which tar-rs buffers completely, is bounded to 16 MiB and otherwise fails as
corrupt. 7z's raw next header, which the parser reads whole, must lie inside the
source and fit an eighth of the worker's memory (128 MiB): reading and indexing one
peaks at about five times its size, and 128 MiB holds the uncompressed header of
about two and a half million members with 25-character paths; a larger one fails
with a reason that names its size. A compressed 7z header (7-Zip's default) is small
whatever the archive holds, but the pinned 7z decoder has no public decoded-header
allocation hook, so decoding it remains subject to process memory containment.
Decoded member lengths and all available checksums must match before
sealing/publication; a member that decodes past its indexed length is reported as
corrupt. There is no archive writing, password support or RAR codec in this
release. Archives inside archives need nothing from the worker: the host stages the
member through an ordinary read and starts another worker whose brokered source is
that staged copy, so each worker still sees one archive through bounded source reads.
XZ dictionaries and Zstandard windows are bounded to 64 MiB. 7z LZMA/LZMA2
dictionaries are checked against 256 MiB from the public coder properties before a
block is decoded; members of a block that needs more are unreadable. BZIP2 uses its
format's bounded block sizes. Concatenated stream members are supported; Zstandard
skippable frames are skipped without decoding. Zstandard frame checksums are
verified explicitly when present. Checksums never replace the host's SHA-256
destination readback when copying out.

The host keeps up to 16 sealed member leases per mount. Beyond 512 MiB of their
spool storage, least recently read members leave first, except members still in
active use (one of the four most recently read members that was read within the
mount's last 64 member reads). It revalidates the captured source on every read and
releases files on backend cleanup. The decoder receives only session IDs and
bounded byte messages, never spool paths or native handles. Cancelling a request
never ends the shared worker: a cancelled listing finishes its bounded page and is
discarded, protocol-3 member reads cancel cooperatively (below), and a protocol-1/2
member read finishes its exchange while the host discards the result. A cancelled
read publishes nothing and its unpublished storage is rolled back. Mount teardown,
protocol violations, a cancelled startup and the idle watchdog end the worker. A
20-second idle watchdog is renewed by completed bounded I/O and actual decoded-byte
progress; archive decoding opts out of an aggregate CPU deadline. A blocked OS I/O
call can delay its owning backend's cleanup until that call returns.

Protocol3 supports cooperative thumbnail member cancellation at StageBegin,
StageAppend, StageProgress and StageSeal exchanges. The host rejects a valid
checkpoint with Failed(Cancelled); the worker unwinds and returns one matching
terminal acknowledgement, preserving the immutable archive for another request.
The host retires only that request's unpublished member spool. Source-reader
Interrupted remains retryable and is not used for this handshake. The host uses
this handshake for ordinary and preview member reads alike. Legacy installed
protocol1/2 workers have no request-local cancellation, so the host lets their read
finish and discards the result instead of ending the worker. Decoder timeout and
viewport cancellation both reach the cooperative path; an already-running OS call or
decoder interval can still delay it.

Build and test using `scripts/build.ps1` or `scripts/build.sh`, with Cargo/Rust and
Python 3 on PATH, providing an offline
public SDK config, a build output directory and an optional package destination.
Source directories are not output destinations: both must lie below a `_build`
directory. On Linux that is this project's own `_build`, that of the repository
holding it in `plugins/`, or otherwise the first `_build` of the path, and that
`_build` and every folder below it must not be a symlink, while the folders above it
may be links. Exporting this directory does not require the host checkout. See
THIRD-PARTY.md for pinned dependency decisions.

Ordinary tests count the bytes a brokered source is asked for (`tests/streaming.rs`):
indexing 1,000-member ZIP/TAR archives fetches less than twice their size, and
preparing every member of a small solid 7z block costs one block pass (a second pass
over the same members, after the retained siblings were staged, costs one more).

For reproducible opening measurements, run the ignored
`measure_tar_and_sevenzip_open` test in `tests/performance.rs` with
`cargo test --release --test performance measure_tar_and_sevenzip_open -- --ignored --nocapture`
and the same public SDK configuration used to build this project. It generates
64 deterministic 256 KiB members and reports five separate opening, listing and
first/last read timings for TAR, independent 7z blocks and solid 7z. Fixture creation
and source cloning are excluded; reads compare exact contents. Set
`MIDNIGHT_ARCHIVE_PERF_DIR` to a directory beneath `_build/` to retain the archives
for host/sandbox measurements. Ordinary tests use no wall-clock thresholds.

The index memory figures above come from the ignored, Linux-only
`measure_index_memory` test in the same file:
`MIDNIGHT_ARCHIVE_PERF_DIR=<directory beneath _build/> cargo test --release --test performance measure_index_memory -- --ignored --nocapture`.
It writes archives of empty members, 1,000 to a folder (a million by default; set
`MIDNIGHT_ARCHIVE_MEMORY_ENTRIES` and `MIDNIGHT_ARCHIVE_MEMORY_FORMATS`), opens and
completely lists each in a fresh child process, and reports the peak and retained
resident memory per member; `MIDNIGHT_ARCHIVE_MEMORY_KEEP` retains the archives for
the host's sandboxed-worker check. Ordinary tests (`tests/many_members.rs`) open and
page through ZIP, TAR and 7z archives, including a 7z with an uncompressed header,
of 100,001 members and of 100 members whose names take over 11 MiB.

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
