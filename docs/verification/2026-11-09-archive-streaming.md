# Generic archive source streaming and private content verification

Date: 9 November 2026. Scope: executable filesystem API, archive host, native
private spool, worker resource policy and independently built archive plugin.
Application deployment and UI acceptance are separate checks.

## Result

Protocol 2 reads captured source ranges and streams verified member content into
host-owned private TEMP files. Production no longer loads the complete archive
or decoded member into a Vec. Default protocol-1 helpers preserve their released
bytes and legacy workers retain their original behavior. The new archive package
explicitly selects protocol 2; other worker resource defaults remain unchanged.

ZIP and plain TAR browse indexes with bounded lookahead. ZIP reads verify the
complete selected member before publishing content. 7z reads verify the required
solid block, including discarded siblings, while staging only the selected member.
Compressed TAR and standalone streams verify their complete outer stream into a
private scratch file. TAR member leases share checked ranges of that scratch file.
There are no artificial source, member, aggregate disk or expansion-ratio byte
ceilings. Actual available storage and native offset ranges govern file length.

The interface is independent of viewers and codecs. No JPEG adapter was restored,
no PSD codec was added, and individual image decoders retain their existing encoded
and pixel-memory behavior. Complete CRC verification can still require complete
decompression before the first preview byte; streaming bounds retained memory,
not total decompression work.

## Requested ZIP, read-only qualification

Both native Windows and native Linux ran the installed restricted worker against
`D:/experiments/godot/Godot_v4.7.2-stable_win64.exe.zip`.

| Measurement | Bytes |
| --- | ---: |
| ZIP source length | 86,013,866 |
| Open and directory listing source reads | 347,418 |
| Separate first member-read source reads | 85,983,232 |
| Verified EXE member length on disk | 180,858,888 |
| Maximum broker chunk | 262,144 |

Listing did not extract the EXE. The separate explicit content-read test requested
its initial `MZ` bytes and therefore verified that complete member into private
storage. It did not execute the EXE. Later first-byte, overlapping and tail reads
used the sealed spool with zero further compressed-source reads. Cancellation
during extraction returned no partial content. A changed captured source rejected
subsequent cached reads and initiated worker/storage cleanup.

## Checks and artifacts

All targets were dedicated `_build/archive-streaming-*` or
`_build/archive-worker-policy` directories. Plugins were built against registry
packages, with no application path dependencies or patches.

| Check | Windows | Native Linux |
| --- | ---: | ---: |
| Filesystem API protocol, bounds and timestamps | 11 passed | 11 passed |
| Independent archive plugin ordinary tests | 36 passed | 36 passed |
| Host, including installed-worker integration | 21 passed | 21 passed |
| Worker SDK and native platform policy/spool | 25 passed | 26 passed |
| Strict Clippy for each changed Rust package | Passed | Passed |

Formatting checks passed. Six release worker-package tests passed under Python on
Linux. Three plugin tests remain explicitly manual: two performance measurements
and one owned-fixture exporter; the exporter was run to supply installed-worker
fixtures. All eleven supported formats passed real-worker reads: ZIP, 7z, TAR,
GZIP, XZ, BZIP2, Zstandard and the four compressed TAR variants. Corrupt ZIP and
7z fixtures failed before returning member bytes.

Additional checks cover a virtual five-GiB TAR index with fewer than one MiB of
source reads, corrupt tails and solid siblings, encrypted/unsafe members,
cross-request and forged-stage rejection, range overflow, orphan-stage rejection,
active-lease survival across cache eviction, and precise UTC/local civil metadata.
The virtual-clock watchdog test advances progress every 19 seconds for 190 seconds,
then proves idle expiry and cancellation. Monotonic decoded-work validation has
separate tests; empty, repeated or regressing progress cannot renew the watchdog.

Storage tests inject a partial write followed by disk-full and a failed flush;
neither can later become readable. Linux additionally uses actual `/dev/full`
ENOSPC, and Windows exercises native write/flush access errors. Native tests cover
private creation, link/reparse refusal, positioned reads, closure cleanup and
inherited CPU policy. No real user volume was filled during qualification.

Evidence directory: `_build/archive-streaming-implementation/`:

- `windows-verification.log` records API/plugin tests, lint and release build.
  Its older host-formatting failure is superseded by the final host log.
- `linux-verification.log` records API/plugin/ordinary host checks and packaging tests.
- `final-host-windows.log` and `final-host-linux.log` record all 21 installed-host
  tests and strict Clippy against the final host source, including the watchdog test.
- `source-hashes.json` records frozen owned source and complete package hashes.
- `final-host-windows.ps1`, `final-host-linux.sh`, `verify-windows.ps1` and
  `verify-linux.sh` retain the exact qualification commands.
- `_build/archive-worker-policy/{windows,linux}-verification.log` and
  `source-hashes.json` record SDK/platform checks.

Final immutable public bundle:
`_build/archive-streaming-api/sdk/worker-sdk-1794226812562261700`.
Use its `.cargo/registry.toml` alongside the normal codec registry/cache. It
contains filesystem API 0.3.0 and worker SDK 0.2.0. Earlier exported package bytes
were not overwritten. Other public SDK/consumer updates and the global dependency
inventory are coordinated separately by the parent integration task.

Complete packages, including protocol-2 manifests and dependency licenses:

- Windows: `_build/archive-streaming-plugin/package/windows/filesystems/archive`.
  Executable SHA-256:
  `069b835f2cfe37150137bbcc14dd38d9e8cd0e2f6dcdadfaab0a8c600347c922`.
- Linux: `_build/archive-streaming-plugin/package/linux/filesystems/archive`.
  Executable SHA-256:
  `55f9ad26a6c2b1a99a815468ff882599aea6e56686d62325befb6984f6b0a50e`.

Windows installed-worker qualification used the protected generated installation
`C:/Users/tjelinek/_build/archive-streaming-20261109-1455`. Linux qualification
creates a private installation below `/tmp/fm-interaction/_build` in the same run;
its path is ephemeral and is recorded in `linux-installation.txt`.

## Limits of this evidence

Metadata remains bounded to 100,000 entries and eight MiB of names, source/spool
I/O to 256 KiB and worker memory to 512 MiB. Raw 7z next headers and XZ/Zstandard
decoder windows are bounded to 64 MiB. The pinned 7z API cannot separately bound
decoded compressed-header allocations; process containment is its remaining
allocation boundary. These bounds can reject complex or dictionary-heavy archives
independently of their file size.

Archive workers add no aggregate CPU limit. Ancestor Windows job and Linux rlimit
policies still apply. Genuine I/O/decoded-work progress renews the 20-second idle
watchdog. An OS call blocked inside a backend may delay cleanup until it returns.
Linux TEMP requires O_TMPFILE support; unsupported private-file primitives fail
explicitly. Windows uses private DACL/delete-on-close handles and pinned ancestors.

Captured sources are live-validated identities, not immutable snapshots. A sealed
cache is not permission to ignore later source changes. Private files are ephemeral
and are not persistent cache or crash-recovery artifacts. Available disk can be
exhausted by large legitimate content; the resulting storage error is reported.

Salamander index, extraction and viewer-cache lifecycle references are recorded in
the [design](../superpowers/specs/2026-11-09-archive-streaming-design.md). They informed
the behavior; no GPL source or assets were copied.
