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

ZIP and 7z mounting index metadata without decompressing file bodies. ZIP's first
read decodes and verifies that complete member. A 7z first read decodes only the
requested compression block, serially, and checks all its member lengths and
available member/block CRCs before publishing any bytes. Solid-block siblings
share that verified cache; unrelated blocks remain compressed. Corrupt payloads
can therefore be listed but fail when read, just as with ZIP.
TAR indexes checked ranges in one immutable backing buffer, without copying file
payloads. Compressed TAR must still decode and validate its complete outer stream
to discover all headers; it then uses the same range index. Standalone compressed
streams retain their fully verified decoded content. Limits: 64 MiB input,
64 MiB member, 128 MiB decoded aggregate, 100,000 entries, 8 MiB names, expansion
ratio 200. Declared 7z sizes are checked when mounting, before deferred decoding;
actual decoded sizes are checked again before cache publication. Exceeding limits
returns Limit. There is no disk extraction,
archive writing, password support, RAR codec or nested mounting in this release.
XZ dictionaries and Zstandard windows are bounded to 64 MiB; BZIP2 uses its format's
bounded block sizes. Concatenated stream members are supported. Zstandard frame
checksums are verified explicitly when present. Checksums never replace the host's
SHA-256 destination readback when copying out.

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
