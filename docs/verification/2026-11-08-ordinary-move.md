# Ordinary per-file Move qualification — 2026-11-08

The accepted operation copies and verifies one regular file, publishes its
destination safely, satisfies its requested metadata, then removes that source
before starting the next file. Earlier successful moves remain after a later
failure or cancellation. There is no selectable source recovery mode, source
recovery journal or catalogue.

## Implementation

The existing shared engine retains full SHA-256 source hashing and independent
complete destination readback. Its per-file proof passes rehash both retained
objects and preserve fresh read guards, exact final-name checks and required
modified-time checks through the removal gate. Windows continues using its pinned
deny-write source, mapped-writer probe and exact-handle disposition.

Linux now implements ordinary removal on qualified local ext filesystems for
owned, single-link regular files in protected source parents. Preflight retains
a native read lease before publication; an existing writable descriptor or shared
writable mapping refuses admission. The lease is shared by capture and finalizer
until that source completes or is retained on failure. A detected lease break,
changed revision, wrong identity, changed publication or metadata mismatch refuses
source removal. Permissions and ACLs are not weakened.

A single bounded native thread owns lease notifications. It blocks SIGIO only on
itself and configures F_SETOWN_EX/F_OWNER_TID before granting leases. No global
signal disposition or async worker signal mask is changed. Lease state is checked
with F_GETLEASE; notifications do not accumulate an unbounded queue. Drop releases
leases and handles, and never removes source data.

The Linux capture primitive was also corrected: `.midnight-move-*` directories
are transient stages, explicit restoration uses no-replace rename, cancellation
before capture cleans its empty stage, and directory cleanup refuses unexpected
children or a substituted directory. Every pre-removal failure/cancel attempts
restoration using a cancellation-independent cleanup token. A new original-name
winner is preserved, with the exceptional staged original reported separately.

Restoration completion is recorded independently of directory durability. If the
no-replace rename succeeds and its following sync fails, the engine records the
sync error, cleans the empty stage, and does not report a fictitious staged source.
The additive internal `restoration_completed` contract defaults to false for
providers without a recorded restoration fact.

## Evidence

All fixtures contain test-owned data. Generated logs and scripts are under
`_build/goal/ordinary-move-completion/`; no user preference files were written.
Commands use the existing pinned toolchain, `--locked --offline`, debug information
disabled and incremental compilation disabled.

| Gate | Result |
| --- | --- |
| Linux staging causal RED | Four new restore/cleanup tests failed for absent authority. |
| Linux staging GREEN | 16 native tests passed after correction. |
| Linux finalization causal RED | Five native cases and four engine cases failed for missing finalization. |
| Linux finalization GREEN | 25 native tests passed, including old-name writer and restoration sync-fault follow-ups. |
| Linux Copy/Move engine | 47 tests passed. |
| Linux common transfer/proof | 24 tests passed. |
| Linux native/engine all-target Clippy | Passed with `-Dwarnings`. |
| Windows baseline engine/native | 54 / 23 tests passed before Linux changes. |
| Windows final engine/native/common | 56 / 23 / 24 tests passed. |
| Windows native/engine all-target Clippy | Passed with `-Dwarnings`. |
| Linux old-name blocking-writer regression | Passed with 24 native tests before the restoration reporting follow-up. |
| Restoration durability reporting causal RED | Original restored, but the job incorrectly reported one source residual; reproduced with native Move and a sync-error boundary. |
| Final restoration reporting native/engine/strict | Linux 25 / 47 and Windows 23 / 56 tests passed; both all-target strict gates passed. |

Owned-file rustfmt and whitespace checks pass. The repository source-size check
examined 1,197 handwritten files, all at or below 100,000 bytes.
The initial Windows strict invocation lost the compiler-argument separator in
PowerShell and stopped before analysis; the corrected explicit argument-array
invocation passed. Its invocation-error log is retained separately.

Native Linux cases cover actual explicit unlink and empty-stage removal, existing
writers, a writable mapping surviving its writer descriptor, an observed lease
break from a nonblocking writer, cancel/Drop retention, source-stage substitution,
raw-byte names, restoration conflicts and cleanup refusal. Engine cases cover
source removal before the next destination exists, later cancellation retaining
unfinished originals, destination corruption after publication, cancellation after
capture, raw-byte paths and exact nanosecond modified-time preservation.

Windows completion qualification adds >260 UTF-16-unit paths and an unpaired
UTF-16 filename through the complete copy/proof/removal/cleanup path. Prior long
name tests exercised retained capture; those alone were not completion evidence.

Linux evidence ran on WSL2 kernel `6.6.87.2-microsoft-standard-WSL2`, using private
ext filesystem fixtures below `/tmp/fm-interaction/_build/tmp`. Drvfs is not claimed
as a qualified Linux Move filesystem. Windows backend fixtures use local NTFS.
These are native filesystem tests, not desktop/GPU or final packaged application
qualification; the root integration task owns those checks.

## Live-operation limits

This is live-validated Move, not an immutable filesystem snapshot, content/identity
compare-and-swap unlink or whole-batch rollback. The owned native experiment
`linux-lease-race.py` demonstrates an open of the old source name reaching its
leased inode before staging, then modifying that same inode after lease release.
The kernel's configured lease-break timeout is 45 seconds on the tested host.
Linux documents that a pending break changes F_GETLEASE's reported state and that
the kernel can forcibly remove a lease after the timeout. Observed breaks always
refuse removal here; arbitrary descheduling across a forced timeout between the
last check and unlink remains outside any atomicity guarantee. See the
[Linux lease contract](https://man7.org/linux/man-pages/man2/F_SETLEASE.2const.html).

Thread-directed notification behavior is based on
[F_SETOWN_EX](https://man7.org/linux/man-pages/man2/F_GETOWN.2const.html) and checked
against Linux 6.6 [lease setup](https://github.com/torvalds/linux/blob/v6.6/fs/locks.c)
and [signal delivery](https://github.com/torvalds/linux/blob/v6.6/fs/fcntl.c).
The practical distinction between completed pathname lookup and acquisition of
write access is visible in
[open processing](https://github.com/torvalds/linux/blob/v6.6/fs/open.c) and
[path lookup](https://github.com/torvalds/linux/blob/v6.6/fs/namei.c).

Private directory checks do not isolate malicious same-UID or privileged namespace
mutation. Destination data remains live validated. A successful logical unlink
does not measure physical storage release: open readers may retain bytes. Windows
source namespace power-loss durability is not confirmed. Directory/link/provider
Move, hard-link topology and broader metadata remain outside this regular-file
increment; unsupported locations are explicitly refused without silently copying.
Abrupt process termination can leave a temporary source stage; no automatic
restart recovery catalogue or journal is introduced.

Salamander's `src/worker.cpp` around lines 6070–6080 invokes `DoCopyFile`, then
deletes a successful, non-skipped file's source. FM follows the requested per-file
progression with its additional independent readback/proof contract. No Salamander
code, assets, ANSI paths or readonly-clearing behavior were imported.

No third-party dependency or feature changes were required for this work.
