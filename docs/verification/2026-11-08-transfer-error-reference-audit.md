# Transfer error handling: Salamander reference audit

This is reference-source analysis only, performed on 2026-11-08. It records
Salamander behavior and implementation checks suggested by that behavior. It does
**not** establish current Midnight Explorer coverage, report test results, or
qualify any new Retry behavior. Implementation and execution evidence belong in a
separate verification record. No builds or tests were run for this audit.

The project [security contract](../../SECURITY.md) remains authoritative. These
observations are behavioral references, not permission to adopt legacy integrity,
overwrite, metadata-loss or traversal policies. No Salamander code or assets were
copied into the implementation.

## Source provenance

The inspected local source tree is `D:/experiments/salamander-main`; it has no Git
metadata, so no upstream revision is asserted. Line references below are one-based
and apply to these exact files:

| Key | Source | SHA-256 |
| --- | --- | --- |
| W | [src/worker.cpp](D:/experiments/salamander-main/src/worker.cpp) | `AAF1055BBFD6AB6B0C6A6BCD4F63750178C8D6A93DAE05FCDD49930307F2DB16` |
| F | [src/fileswn6.cpp](D:/experiments/salamander-main/src/fileswn6.cpp) | `FD8C7878455DB81AA5ECA6660BAE5702287755E467DC08F2791894253A11295A` |
| D | [src/dialogs.cpp](D:/experiments/salamander-main/src/dialogs.cpp) | `276462169E929DD7D304267A6D3A249C97777F1E192BB94A3D7DA8B459310BC2` |

For example, `W:3191-3224` identifies lines 3191 through 3224 of `src/worker.cpp`.
The audit covers native transfer execution, relevant preparation/enumeration and
the progress-dialog/active-queue lifecycle. It does not claim to audit every
Salamander plugin or filesystem implementation.

## Error and decision matrix

| Case | Observed reference behavior and source | Implementation check suggested by the reference |
| --- | --- | --- |
| Source open failure | Retry / Skip / Skip all / Cancel; W:5490-5533. | Distinguish unavailable media, disconnected shares, permissions/sharing and missing objects. Retry must validate the captured source identity. |
| Destination create/open failure | Retry / Skip / Skip all / Cancel; W:5453-5481. | Preserve the captured destination and owned-stage rules. An error must not confer overwrite permission. |
| Device not ready | No dedicated `ERROR_NOT_READY` branch occurs in W. Generic open/read/write error paths display the OS error and offer Retry. | Cover recoverable native failures beyond disk-full. Reinserting a different device must not silently retarget the job. |
| Source read failure | Retry reopens the source, checks size and recent output bytes, and restores offsets; W:3334-3363, 3824-3870. | Retry requires a valid source identity/revision and subsequent full destination verification. A stale handle and a replaced pathname are different problems. |
| Network reconnect | Read errors with `ERROR_NETNAME_DELETED` receive up to three automatic attempts separated by 100 ms; W:3318-3321, 3899-3902. | Keep automatic attempts bounded and cancellation observable. Persistent failures require a visible decision; reconnection does not prove object identity. |
| Failed or short write | Failed writes and successful writes with fewer bytes both enter error handling. When the OS error is zero, short writes become disk-full; W:3191-3224, 4221-4228. | Handle short writes and zero-progress writes. Advance counters only for actual writes; never blindly repeat a buffer at an uncertain offset. |
| Write retry | Reopens output, checks length/tail, then resumes or restarts the file; W:3227-3257, 3950-4003. | Before publication, restarting an owned stage can be safe. Reopening an arbitrary pathname and treating it as the old stage is insufficient. |
| Preallocation / capacity | Failed preallocation is undone and copy retried without preallocation; W:4662-4709. Initial insufficient-space warning permits continuation; F:1061-1076. | Free-space estimates are advisory. Actual write/sync results must handle quota, competing allocation and metadata-space exhaustion. |
| Filesystem size limit | FAT32 files larger than 4 GiB minus one byte receive Skip / Skip all / Cancel before execution; F:2507-2542. | Unsupported limits must not enter an unbounded Retry loop. |
| Flush and destination close | No `FlushFileBuffers` call occurs in W. Destination close failure becomes a write error; Retry deletes output and restarts; W:4953-4989. ADS close failure also becomes a write error; W:2704-2709. | Preserve the required synchronization and independent readback gates. A late close/sync error must not become success or authorize source deletion. Never retry an already closed handle. |
| Changing source / EOF | Legacy code accommodates changed lengths, including truncating output when source shrinks; W:3385-3392, 3690-3723. | Reject detected source changes. Check empty files, early EOF, source growth and changes while awaiting a decision. |
| Existing destination | Overwrite / all / Skip / Skip all / Cancel; W:5137-5227. Hidden/system files have a separate confirmation; W:5233-5278. | Bind conflict decisions to the live object and decision version. A new name winner needs fresh handling. |
| Overwrite and name race | Legacy code may delete the target before copying, W:5286-5304, or truncate it with `CREATE_ALWAYS`, W:5324-5345. | Do not adopt these destructive paths. Preserve verified private staging, no-replace publication and explicit recoverable replacement. |
| Source/target alias | Preparation rejects equal path strings; F:2568-2583. | Native identity checks must also cover hard-link aliases, alternate spellings and renamed parents. String comparison is insufficient. |
| Directory enumeration | Initial and mid-enumeration errors are distinguished from normal EOF; F:2076-2134, 2229-2272. Choices are Skip / Skip all / Cancel, without Retry. | A failed listing is not an empty or fully traversed directory. Retrying enumeration must not duplicate completed children or erase earlier failure/skip state. |
| Directory creation and merge | Existing directories can merge after confirmation, W:6690-6734. Name collisions and other create errors offer Retry / Skip / Skip all / Cancel, W:6737-6797. | Revalidate type and identity. Skipping creation skips the whole subtree in the reference, W:7966-7993; Move must retain the corresponding source ancestors. |
| File timestamps | Reading/setting time offers Retry, Ignore / Ignore all, Skip / Skip all, Cancel; W:4819-4863, 4902-4949. | Only optional metadata may be ignored. A required metadata failure must block publication/removal rather than downgrade the contract. |
| Attributes, permissions, encryption | Attribute/security failures offer Ignore / Ignore all or Cancel; W:4996-5068. Encryption loss gets a separate explicit choice, W:4595-4645. Directory-time handling is W:6396-6460. | Content verification and metadata preservation are separate guarantees. Retry must not bypass unsupported capabilities or mandatory metadata checks. |
| Alternate data streams | ADS enumeration can Retry or explicitly Ignore; W:2877-2915. Stream open/read/write errors have separate categories; W:2513-2681, 2748-2859. ADS Skip removes the copied file and prevents Move source deletion; W:4881-4896. | Preserve explicit stream-support and metadata-loss rules. Unverified streams must not disappear through Move as a side effect of generic retry/ignore behavior. |
| Links and reparse points | Directory-link contents receive copy/skip confirmation; F:2008-2065. Their children use Copy semantics, F:2197-2217. File links are dereferenced for size, F:2635-2656. | Do not adopt implicit traversal or a different link policy. Retry must not weaken existing link, mount-crossing or special-file refusal. |
| Cancellation and pending I/O | Async cancellation cancels requests then drains results before reusing blocks or closing handles; W:3726-3808. Cancellation wakes a paused worker; D:1203-1213. | Cancellation must win over stale answers. Retain resources until outstanding I/O and cleanup finish, including cancellation while waiting for Retry. |
| Unfinished-output cleanup | Error/Skip closes handles and deletes output; W:4733-4767. Several delete failures are ignored or only traced. | Clean only owned unpublished stages. Keep published outputs and report cleanup residuals; failed cleanup must not be presented as a clean cancellation. |
| Same-volume Move | Rename errors offer Retry / Skip / Skip all / Cancel; sharing violations get two automatic 100 ms retries; W:6013-6054. | Retain explicit Move authority and native identity protection; do not fall back silently to weaker operations. |
| Move source deletion | Copy must return success and `!skip` before deletion, W:6069-6079. Deletion failure has its own Retry / Skip / Skip all / Cancel; W:6082-6112. | Retry finalization only if its authority/proof can be safely renewed. Do not rerun an already published copy. Skip retains both source and destination. |
| Source-directory removal | Nonempty-directory/sharing errors receive bounded backoff, followed by normal decisions; W:6872-6908. | Keep removal empty-directory-only and preserve ancestors of skipped/failed children. Never turn a retry into recursive residual deletion. |
| Decision scope and progress | Separate per-operation flags exist for read, write, opens, ADS, directory creation and deletion; W:989-1013, initialized at W:7814-7830. Skipped sizes advance legacy progress, W:3151-3162. | Scope decisions to the job/category and reject stale tokens. Keep processed/skipped bytes distinct from copied and verified bytes. |
| Dialog closing and queue history | Worker sends terminal `IDOK`, W:8191-8195. D:1307-1329 finishes the worker and posts close; D:1265-1296 removes active queue membership. W:8343-8376 removes the entry and resumes queued work. | Close the matching progress dialog after a backend terminal event. Preserve terminal job history separately. Salamander's active queue removal is not a history-retention implementation. |

## Unsafe legacy behavior to exclude

- Reopening or deleting by pathname after an error is not sufficient evidence of
  identity. Output reopen uses `OPEN_ALWAYS`, W:3235-3236 and 3959-3960; cleanup
  deletes by pathname, W:4745 and 4763. A new object occupying that name must not
  inherit retry or cleanup authority.
- Retry validation checks only a recent tail, with a roughly three-second budget,
  W:2145-2146 and 2290-2291. During source-read recovery, an unreadable destination
  can even be accepted, W:2205-2209. This cannot replace full independent
  destination readback, source-change checks or Move finalization proofs.
- Deleting an old destination first or truncating it in place, W:5296-5297 and
  5337-5345, loses the original before a new verified result exists.
- Accepting source growth/shrink, metadata/ADS loss or copied link contents is a
  separate product policy. It must not be inherited merely because the reference
  offers those choices. In particular, ADS loss can be explicitly accepted before
  Move in F:2686-2711; that is not verified preservation.
- Whole-job retry after publication or source-finalization activity can duplicate
  published results, act on replacement sources, or conceal partial outcomes.
  Recovery must be confined to a demonstrated safe phase, or refused while
  preserving the actual completed and residual state.
- The reference waits synchronously for worker termination in its dialog handler,
  D:1313-1315. Its completion handshake is useful as a lifecycle reference, but
  does not justify blocking the application UI thread.
- Reference queue removal and skip-progress accounting do not establish accurate
  retained history. History must preserve terminal outcomes, errors, skipped
  items, published files and cleanup/removal uncertainty independently of a
  progress window's lifetime.

## Verification timing recheck, 2026-11-10

The same `worker.cpp` SHA-256 listed above was rechecked for the user's question
about when Copy/Move verifies content. The inspected native path does **not**
perform an automatic full destination reread or SHA/CRC comparison after ordinary
copying. It checks read/write results and byte counts in both synchronous
(W:3176, 3191) and asynchronous (W:4221) paths. It also handles a failed destination
close (W:4953); no explicit `FlushFileBuffers` or `FILE_FLAG_WRITE_THROUGH` occurs
in this copy path. Successful OS calls are not a cryptographic content proof.

On Retry, `CheckTailOfOutFile` (W:2145) reads corresponding source/destination
blocks backwards from the written position and compares them using `memcmp`.
It initially requests two 32 KiB blocks if enough content exists and continues
until the beginning or a roughly three-second budget (W:2290). This qualifies
resuming a partial transfer, not an unconditional final full-file verification;
the unreadable-output exception described above still applies.

Same-volume Move uses `MoveFile` (W:5558, 5616). Cross-volume Move calls
`DoCopyFile`, and success without Skip permits immediate `DeleteFile` for that
individual source (W:6070–6079). It does not wait for every file in the batch.

The checksum plugin exposes separate calculation/manifest-verification commands
for CRC32, MD5, SHA-1, SHA-256 and SHA-512; it is not invoked by this native
Copy/Move path. See
[checksum.cpp:254](D:/experiments/salamander-main/src/plugins/checksum/checksum.cpp:254),
SHA-256 `681725C657C829256F12ED537C2F2F4B4EF680C4B074514EC912E8CF1D0FAC5B`.
This recheck does not establish behavior for every archive/network plugin.

Midnight Explorer retains full SHA-256 destination readback as the default
and additional Move finalization proofs, as specified in
[the current transfer contract](../developer/TRANSFERS.md). A subsequent explicit
user request adds optional length + first/middle/last64 KiB verification and
native same-volume rename without content reads; see the
[accepted refinement](../superpowers/specs/2026-11-10-transfer-verification-and-native-move.md).
That choice does not claim full integrity for unsampled bytes.

## Tests to check in implementation verification

Every item below is a proposed check, not a claim that the test exists or passed.

- [ ] Source and destination open failures: not-ready, permission/sharing,
  missing object, disconnected network; repeated Retry still failing.
- [ ] Device removal and return with the same identity; replacement media or
  changed parent/source/target identity must be refused.
- [ ] Disk-full/quota failure followed by freed space; partial/zero writes;
  preallocation failure; unsupported maximum file size.
- [ ] Read failure, early EOF, empty file, growth/shrink and source modification
  while the decision is open; full verification still runs after recovery.
- [ ] Sync, destination verification and close/finalization failures never become
  successful publication or grant source-delete permission.
- [ ] Destination collision before publication and while awaiting Retry; changed
  conflict token, new name winner and alias/hard-link cases preserve old data.
- [ ] Initial and mid-directory-enumeration failures preserve truthful partial
  results. Directory-create Skip covers descendants and retains Move ancestors.
- [ ] Cancel while paused, awaiting a decision, retrying native I/O or cleaning a
  failed attempt; stale replies cannot revive the operation.
- [ ] Repeated retries keep handles, buffers, staged objects and queue capacity
  bounded, and do not accumulate hidden cleanup failures.
- [ ] Failed stage cleanup retains/reports exact residuals and never deletes an
  unexpected pathname winner or a previously published destination.
- [ ] Publication succeeded but a later durability/metadata step failed: preserve
  publication facts and refuse unsafe whole-item replay.
- [ ] Move source-removal/finalization failure after publication: preserve the
  verified destination, retain/restore source where required, and avoid copying
  the completed file again.
- [ ] Nested directory Move with earlier successful children, skipped children,
  late enumeration errors and unexpected remaining entries keeps ancestors safe.
- [ ] Required metadata, ADS, links, mount crossings and unsupported-provider
  errors cannot be bypassed by generic Retry or Skip-all decisions.
- [ ] Read/write/open/delete Skip-all choices remain job/category-scoped;
  skipped and verified progress/results stay distinct.
- [ ] The matching terminal event closes a live progress window exactly once;
  retained history remains inspectable, and an older terminal event cannot close
  a newer job's dialog. Cancellation/partial-error details remain available.
