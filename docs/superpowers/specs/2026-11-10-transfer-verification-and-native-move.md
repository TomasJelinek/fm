# Transfer verification and native Move

Accepted user refinements, 2026-11-10. These replace earlier requirements for
mandatory whole-file hashing on every transfer and copying same-volume moves.

## Verification choice

Options → General → File operations offers **Verify entire copied file with
SHA-256 (recommended)**, enabled by default. Legacy settings also enable it.
OK commits the choice; Cancel discards it; Reset to defaults enables it.
Each submitted job retains its policy, including clipboard paste, drag/drop,
selected/inline-tree roots and extraction from an archive. Later settings changes
cannot alter queued/running work. Manual checksum commands remain independent.

Full mode hashes source bytes while writing, flushes/synchronizes the staging
file, independently reads the whole destination and compares SHA-256 and length.

Quick mode checks exact length and directly compares **three 64 KiB blocks**:
the first, centered middle and last block. For files of **192 KiB or less**, all
bytes are compared exactly once. For larger files of length L, ranges start at
0, floor((L − 65,536) / 2), and L − 65,536; each contains 65,536 bytes. Reference
bytes are captured during the normal source stream, so no extra source pass is
needed for Copy. At most 196,608 reference bytes are retained per active file.

Quick mode cannot detect corruption between samples. Its label and tooltip must
disclose this; neither its progress nor Jobs details may claim full SHA-256
verification. A test intentionally corrupts an unsampled interval and documents
that distinction. Whole-file SHA-256 stays the recommended default.

Both modes preserve write-error handling, exact lengths, sync failures,
source/destination identities, source revision checks, cancellation, explicit
overwrite consent and safe publication. Cross-volume Move uses the job's policy
for retained source/destination validation before each successful file's removal.
Retained range reads use the same opened object even if its name changed.
Readback may come from the OS cache; neither mode promises physical-media
durability or an atomic snapshot of a concurrently modified source.

## Native same-volume Move

Attempt the native rename before content-read/proof/staging admission. Determine
filesystem identity through opened objects/native results, never a drive-letter
comparison. A successful whole-directory rename must not enumerate descendants.
No file-content checksum or read is required, because no content is copied.

Keep cancellation before mutation, pinned identities, no-replace publication,
explicit replacement consent, directory merges and recovery facts. Once a rename
commits, report that fact even if cancellation arrives afterward; never replay
the transfer as Copy. A proven cross-device result may enter verified Copy plus
per-file source removal. A native permission error is not a cross-device result.

The optional capability defaults to Unsupported for legacy providers. Supported
native adapters report precise outcomes: moved, exact same entry, unchanged
cross-device, or a committed/recovery result. Hard-link aliases are not collapsed
into exact same-entry moves. Native entry counters count a renamed directory
once, without inventing descendant counts or byte throughput.

## Short staging names and cancellation

The Windows staging basename is `.me-` followed by sixteen random hexadecimal
characters (eight operating-system CSPRNG bytes), twenty characters in total.
Creation remains exclusive and relative to the held destination directory;
collisions retry without opening or truncating an existing object. Randomness
does not grant operation authority: the retained native handle does. No
`MAX_PATH` buffer or 260-character policy is introduced. A destination parent
longer than 260 characters with a short final basename must work on a supported
native filesystem. Shortening the basename alone would not prove long-path support.

Linux continues to use an unnamed `O_TMPFILE` inode on the destination filesystem.
Neither implementation stages ordinary copied bytes in the system TEMP folder.
Only a fully verified stage may acquire its final name. Same-volume native Move
does not create a partial content copy.

Cancellation during writing, synchronization or readback must explicitly clean
up the unpublished stage on a backend worker before terminal progress is
delivered. An in-flight native call retains its resources until it returns;
requesting cancellation never kills an I/O thread or frees its buffers early.
Existing destination originals, unfinished sources and earlier completed files
are preserved. Explicit cleanup must surface failures with the affected path;
Drop cleanup is a last safeguard, not evidence that a failed removal succeeded.
If the filesystem refuses deletion or disappears, the application must report
the residual rather than claiming that cleanup completed.

Qualification includes compact/colliding names, a real long destination parent,
partial write/drop, cancellation during verification, replacement staging and
cleanup-error reporting. These are requirements; dated verification records
separately identify which tests have run. See the
[staging and cancellation evidence](../../verification/2026-11-10-staging-cleanup.md).

## Implementation and validation

Core owns `TransferVerification`, immutable request/progress fields and the native
Move capability. Shared backend code owns sample capture/readback and retained
Move proofs. Separate Windows/Linux adapters own native rename and guarded seeks.
There are no dependency or plugin ABI changes.

Tests cover defaults/migration/round trips, Options rollback, queued policy
snapshots, sample boundaries, empty/small files, short I/O, retries, corruption,
truncation, source changes, cancellation, real same/cross-volume results,
unreadable content, directory links, identity races and overwrite consent.
Verification records distinguish executed platform gates from pending checks.

## Windows copy comparison

The application does not treat a successful OS copy call as evidence of a
separate destination checksum. Microsoft's
[CopyFile explanation](https://devblogs.microsoft.com/oldnewthing/20120919-00/?p=6563/)
describes why ordinary copying does not automatically reread the destination and
compare a hash. That article is from 2012; current
[CopyFile2 documentation](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-copyfile2)
does not promise this additional verification either. This is an API contract
comparison, not an audit of every Windows 11 Explorer build.

Our full mode explicitly performs the additional content comparison. Readback can
still be served by OS/device caches, so it is not a promise of a physical-media
read or protection from later corruption. No copy policy is changed merely to
match Explorer's ordinary verification behavior.

Comparable optional verification exists in
[TeraCopy](https://codesector.com/how-to/verify-files-after-copy) and
[FastCopy](https://fastcopy.jp/help/fastcopy_eng.htm): both allow SHA-256 with
destination readback, although their defaults use xxHash-family algorithms.
[Double Commander](https://doublecmd.github.io/doc/en/copymove.html) also provides
post-copy verification, using direct byte comparison on Windows and BLAKE3/BLAKE2
on other supported systems. These are behavior references, not application
dependencies or imported implementations. A separate checksum-generation command
must not be confused with automatic post-copy verification.

## BLAKE3 evaluation

BLAKE2 and BLAKE3 are cryptographic hash families; the official
[BLAKE3 Rust implementation](https://github.com/BLAKE3-team/BLAKE3) supports
incremental streaming, SIMD and optional parallel CPU hashing. Faster hashing
does not remove the independent full destination-readback pass. Parallel CPU
work over already-read buffers does not require concurrent disk reads.

An isolated, optimized probe on the development i7-13800H measured median
throughput of 1.262761 GiB/s for `sha2` 0.10.9 and 1.975343 GiB/s for `blake3`
1.8.7: approximately 1.56 times the hash throughput. Both use one thread and
their default runtime CPU dispatch (SHA and AVX2 available, AVX-512 unavailable).
Each of nine alternating-order rounds streams the same 64 MiB RAM buffer sixteen
times in 256 KiB chunks, following a warmup. Unrelated checks were active; the
results are indicative, not an isolated CPU benchmark or file-copy measurement.
Sources, lockfile and raw results are under `_build/qa/hash-throughput/`.

The project already inventories these exact library versions, including BLAKE3
for thumbnail-cache identities. This experiment changes no application dependency
or copy policy. BLAKE3 is a candidate for internal full-copy verification; adoption
would require end-to-end copy/readback benchmarks and the same integrity tests.
SHA-256 remains the current default and the existing manifest format is unchanged.
