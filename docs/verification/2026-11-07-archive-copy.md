# Archive opening, formats and verified copy-out

Archive parsing remains in the independent sandboxed executable plugin, consuming
only the versioned public filesystem API (0.2.0) and worker SDK (0.1.0). The wire
protocol remains 1; new format tags are sent only to providers advertising them.

## Measured opening change

The previous ZIP implementation decompressed every readable member before returning
the root directory. An ignored standalone plugin test constructs 8,000 deflated
members containing 62.5 MiB of decoded data and measures opening, complete paginated
listing and first member read. Windows debug builds used debug info and incremental
compilation disabled, one build job and one test thread:

| Operation | Before | After |
| --- | ---: | ---: |
| ZIP open | 534.1122 ms | 22.7359 ms |
| List all root entries | 1.6452 ms | 1.1007 ms |
| First requested member read | 0.0221 ms | 0.1666 ms |

This is a single same-fixture development measurement, about 23.5 times faster
opening, not a production latency guarantee. It excludes process launch and source
broker transfer. Compressed source bytes are still bounded and read before indexing.
ZIP contents are now decoded on their first read and fully CRC-checked before any
bytes are returned. Stable member indices and per-directory children avoid repeated
whole-tree searches. GZIP/TAR and solid 7z remain eager; no unsupported claim of
random access to solid compressed blocks is made.

Reproduce with the plugin's `tests/performance.rs` ignored test, using its independent
Cargo manifest and an exported public SDK config. The fixture is generated in memory;
build/test output uses `_build/archive-filesystems/` only.

The Salamander reference's `src/plugins/zip/list.cpp` reads central-directory
metadata to list ZIP entries. This implementation follows that format-level approach;
no Salamander source or assets were copied.

## Formats and integrity

XZ/TAR.XZ, BZIP2/TAR.BZ2 and Zstandard/TAR.ZST are implemented alongside existing
ZIP, 7z, TAR and GZIP/TAR.GZ. Stream concatenation, corrupt/truncated data, Unicode
TAR names, actual output budgets and Zstandard checksum failure have plugin tests.
XZ memory and Zstandard windows are bounded to 64 MiB before large decoder
allocations. Existing source/member/aggregate/ratio budgets remain enforced.

Pinned library decisions and redistribution notices are documented in
`plugins/midnight-fs-archive/THIRD-PARTY.md`; the notice generator successfully
collected exact source licenses, including libbz2-rs-sys's original/translation
copyright notice. No paid or trial-only dependencies are selected.

## Copy-out boundary

F5 captures mount, provider IDs, listing, displayed index and selected IDs. Hidden
rows are excluded on the backend; an entirely hidden selection or hidden focused
fallback is rejected. Backend preparation
revalidates the captured members, then supplies their byte streams to the existing
verified-copy worker. It uses the same 32-job FIFO, mutation exclusion, cancellation,
conflict decisions and receipt/fence lifecycle as native copies. The plugin never
receives a destination path or any filesystem write capability.

Each file is written to an exclusively created native staging object. SHA-256 hashes
the source bytes; after sync, a separate complete destination read is hashed and both
length and digest must match before no-replace publication. Source revalidation also
runs before publication. Archive CRC/checksums do not replace destination readback.
Existing destinations, duplicate basenames and directories are conflicts; no implicit
merging, overwrite, source removal or link traversal is allowed.

Raw member bytes determine destination basenames. Linux preserves raw filename
bytes. Windows currently requires valid UTF-8 member bytes and fails explicitly for
legacy non-Unicode names; lossy display labels are never substituted. The native
destination adapter additionally validates platform basename rules and performs
handle-relative no-replace operations. Parent paths, separators, ADS syntax, NUL,
links, special entries, changed sources and stale selections fail. Virtual identities
are excluded from native ancestry comparisons. Traversal is bounded to 256 levels,
100,000 listed entries and 8 MiB aggregate names, with cycle/duplicate-ID rejection.

The initial Windows engine check passed 25 existing/new copy tests, including actual
native destination publication for recursive Unicode archive members, duplicate-name
conflicts, unsafe/stale/changed members, shared admission capacity and cancellation.
The independent plugin passed 16 format/security tests. Existing verified-copy
corruption and publication-race tests exercise the same reused SHA-256 pipeline.

Native Linux passed the same 16 plugin tests. Windows strict Clippy passed for
the plugin and the public protocol passed all four tests. The final plugin lock
uses canonical worker SDK package checksums, not the temporary directory-source
packages used during development. Pinned notices were regenerated successfully
against that canonical bundle.

An offline advisory scan used database revision
`965bf9ad083b58e6340d65829d85217b18be9290` (3 November 2026, 1,403 advisories).
It found four existing ZIP 8.6.0 advisories: RUSTSEC-2026-0367, -0368, -0369 and
-0429. They concern the upstream `ZipArchive::extract`/`ZipStreamReader::extract`
methods' permission/symlink handling. This plugin never calls either extraction
API, passes no native paths to ZIP, and performs no filesystem writes. Host copy-out
does not apply archive permissions or create symlinks. These affected API paths are
therefore unreachable here; the ZIP dependency still needs a separately qualified
9.x upgrade. The new codec versions had no matching advisories in this database.
The audit could not open the crates.io index, so yanked-version checking was not
available. This is not a claim that the dependency graph has no known advisories.

Archive contents are not filesystem snapshots. Captured source revalidation detects
observable changes; destination verification proves equality with the byte stream
read, not with every possible concurrent version or physical-media durability.

## Native Windows fixture investigation

The first real-worker test failed because its freshly written GZIP input changed
after capture. A no-worker reproduction observed only native `ChangeTime` moving;
volume, file ID, length and last-write time stayed unchanged. The endpoint security
stream `sec.endpointdlp` appeared between observations (absent, then 302 bytes).
Only stream existence and length were inspected, never its contents. Explicit
`sync_all` did not prevent the asynchronous metadata update. Production identity
validation correctly rejected the changed revision and remains unchanged.

A metadata-only 500 ms stability window was insufficient. A separate no-worker
probe observed another revision change 205 ms after the first normal content read,
following 514 ms of stable metadata. Priming the tiny fixture with a normal read
before the same bounded stability check passed twice and failed on the third run.
Longer arbitrary delays are therefore not used as evidence of fixture readiness.

The native worker test now reads an immutable 25-byte repository fixture containing
`hello`. A separate test captures a fresh copy, deliberately changes its contents,
and requires both revalidation and broker reads to fail. Existing engine tests
cover changed mount sources. The worker test name reflects its listing, reading
and cancellation coverage; it does not claim to mutate the mounted fixture. No
production code, sandbox policy, metadata fields or endpoint security settings
were changed, and all temporary probes were removed.

The final Windows test binary passed all three tests in three consecutive runs,
including the actual restricted archive worker (0.73 s, 0.71 s and 1.68 s).
Logs: `_build/qa/archive-fixture-settled-{1,2,3}.log`.
