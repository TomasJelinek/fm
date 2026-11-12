# Native transfer service

These are internal Rust contracts in `midnight-core::transfer`, implemented by
the separate Windows and Linux adapters. They are not Rust trait objects exposed
across a plugin ABI. Archive extraction supplies a brokered `TransferSource` and
uses the same native destination and verification state machine.

## Verification and ownership

`VerifiedFileCopy` performs bounded 256 KiB steps: source read/stage write,
flush/sync, independent stage readback, then publication. The immutable job policy
defaults to full SHA-256 and exact length. Explicit `SizeAndSamples` compares
exact length and first/middle/last 64 KiB (all bytes for files up to 192 KiB), retaining
at most 192 KiB of source evidence gathered during streaming. It cannot detect
corruption between samples. It revalidates the opened source
before accepting replacement consent and immediately before publishing. A failed
copy stays stopped until an explicitly supported Retry restores its checkpoint;
cancelled and recovery-terminal copies cannot publish again. A late no-replace
collision leaves the verified stage available for a different name or explicit
replacement; the consumed source stream is not replayed.

All methods run on backend workers. UI receives bounded immutable progress and
versioned conflict metadata, and sends a choice with the exact job/version. The
queue currently runs serially on every storage class. More than one stream still
requires positive local SSD classification at both endpoints and shared budgets.
The [local concurrency experiment](../verification/2026-11-10-ssd-copy-probe.md)
found substantial component gains for large/mixed files, but abnormal small-file
audit delays left its full qualification incomplete. It therefore authorizes no
production concurrency change; there is no new parallel-copy setting or default.

Windows stages use a twenty-character basename, `.me-` plus sixteen random hex
characters. Exclusive creation is relative to a held directory handle, without a
260-character `MAX_PATH` restriction. Linux stages remain unnamed `O_TMPFILE`
inodes. Both stage on the destination filesystem. The final name is assigned only
after verification; same-volume native Move does not stage content.

Cancellation waits for any in-flight native step and explicitly removes the
unpublished stage before terminal progress. Factory failures after stage creation
also attempt explicit cleanup. Unfinished sources, existing target originals and
already completed files are retained. A cleanup refusal is reported with its exact
residual path, even when the job is Cancelled; destructor cleanup alone does not
prove removal. See [staging and cancellation](../superpowers/specs/2026-11-10-transfer-verification-and-native-move.md#short-staging-names-and-cancellation).

## Destination capabilities

| Contract | Meaning |
| --- | --- |
| `prepare_target` | Capture a native target identity and provider-owned revision for a conflict. The pin remains worker-owned. |
| `directory_merge` / `open_merge_directory` | Revalidate and open the consented ordinary directory without following a link. Never delete an existing tree. |
| `conditional_overwrite` | Strong expected-identity replacement. Currently false on both adapters; ordinary rename is not compare-and-swap. |
| `recoverable_replacement` | Separately selected replace-current-entry policy which preserves displaced objects in private recovery. |
| `prepare_replacement` | Bind an unpublished stage to a basename, target pin and explicit policy without namespace mutation. Also works after readback. |
| `publish_no_replace` | Publish exclusively. Ignores a previous replacement binding, so choosing another name never silently replaces its destination. |
| `publish_replace` | Publish only under the explicit bound policy; enforce its basename and revalidate the pin. |

Default optional methods return Unsupported. Windows currently advertises
recoverable replacement on local fixed NTFS; Linux qualifies local ext
filesystems. Network, removable and unknown replacement semantics are not
silently treated as equivalent. Other no-replace and merge operations retain
their own provider capability/error behavior.

Native same-volume Move first attempts the retained native rename capability,
before content/proof admission. It reads no contents and skips hashing. Actual
cross-device results enter the ordinary verified transfer route; generic errors
never authorize that fallback. Whole-directory rename does not list descendants.
`native_entries_moved` and items/s express this work separately from copied bytes.
Committed errors retain their recovery facts and cannot replay a source transfer.
The [current policy and API design](../superpowers/specs/2026-11-10-transfer-verification-and-native-move.md)
supersedes earlier unconditional SHA-256 and same-volume-copy descriptions below;
full-hash proof discussions describe the default mode, and sampled proofs retain
the same identity/revision/publication guards with bounded range reads.

## Conflict policies

Replace/Replace all keep originals. Merge/Merge all keep existing children and
do not authorize leaf replacement. Each all-policy is limited to its job; it
cannot enable a type collision or a source/target alias. All captured native
source roots are protected, including a later selected root that is also an
earlier destination. Destination ancestry checks prevent recursion into sources.

A target change while a decision is pending invalidates that decision. A newly
appearing destination at publication requires a fresh conflict, even after
Replace all. A stale job/version response cannot authorize a new conflict.
Skip, another name and Cancel remain available when replacement is unsupported.

## I/O errors and progress lifetime

`CopyEvent::Failure` carries a `CopyFailure` with operation ID, monotonically
increasing decision version, `CopyErrorStage`, native error and separate source
and destination paths. `can_retry` and `can_skip` are capabilities, not UI guesses.
Decisions use the existing `Command::DecideCopy` route. A stale version, wrong job
or unsupported choice leaves the pending decision unchanged. Error Retry and
Skip All never provide consent to replace or merge a conflicting destination.

The async decision waiter releases blocking workers and the shared I/O permit.
The logical mutation queue reservation remains held, preserving FIFO ordering;
the user can still browse or enqueue more jobs. Cancel/shutdown wakes a paused
job and performs cleanup before delivering terminal progress.

Retry retains native identities and the original source/stage handles:

- Preparation revalidates all objects already pinned by earlier attempts. It can
  offer Retry/Cancel, but cannot safely skip a root before admission completes.
  Frozen-selection validation and unsupported providers fail without a retry loop.
- Source reads require an explicit retained-reader reposition capability. Partial
  writes retain their block, seek to the last fully committed offset and rewrite
  the block; progress and the source hash advance only after the complete write.
- Flush/sync and readback preserve exact length and the frozen full-hash/sample
  checks. Readback
  retry restores the verified offset, including after an error advances a cursor.
- Publication retry is allowed only before namespace mutation. A late collision
  becomes a fresh name conflict. Integrity failures, changed identities and
  uncertain publication never authorize retry or deletion.
- Enumeration errors cannot replay a cursor safely and offer Skip/Cancel, with
  Skip All scoped to this error type. Move retains the skipped subtree/ancestors.
- File-source deletion Retry keeps the verified destination, obtains a fresh
  native finalizer and rehashes both held objects before another removal attempt.
  It never recopies the file. Directory removal retries the exhausted frame,
  revalidating bindings without enumerating that directory again.

Skip All is scoped by phase, native error kind and operation name within one job.
Successful retries do not increment failed counts; skipped errors and cleanup or
restoration failures remain in history. A source-name restoration conflict retains
both objects and reports the residual path. Unsafe cleanup/commit failures stop
the job instead of offering an optimistic Retry.

The error dialog has Retry/Skip/Skip All/Cancel only when supported. Escape or
Decide later preserves the attention state; Jobs → Resolve opens the exact pending
decision. Successful Copy/Move closes only automatically opened progress, after
all associated work succeeds. Manually opened Jobs remains open. Completed
records remain for the current session, including when progress closes. The Jobs
ledger is separate from the operation window. Copy, Move, Delete and Recycle use the same
`JobProgressWindow` component, with operation-specific counts and units. Each active
operation can have its own modeless progress window. X/Escape or Cancel operation
asks for confirmation before cancelling that exact active operation. Continue is
the default, and dismissing the question preserves work and its pause state. A
completed operation closes without a question; closing the ledger does not cancel work.
See the [interaction design](../superpowers/specs/2026-11-08-transfer-errors-design.md)
and [Salamander reference audit](../verification/2026-11-08-transfer-error-reference-audit.md).

## Activity charts

Copy, Move and Delete expose `Arc<OperationThroughput>` snapshots alongside their
existing progress. Engine-owned `Tracker` instances sample monotonic elapsed time
and actual cumulative counters every 250 ms. Each snapshot retains at most 240
intervals spanning the whole operation. When full, adjacent intervals are combined
and future intervals use the same coarser resolution. Original timestamps and
per-series measured maxima survive, so early peaks remain visible and determine
the vertical scale. Different series' peaks in a combined interval need not have
occurred simultaneously. Current labels remain the latest actual interval rates,
not aggregated peaks. The UI displays one chart for the selected
job, caches its geometry by snapshot identity and never measures I/O itself.

Copy/Move charts distinguish written bytes/s from SHA-256 readback and additional
Move proof reads. Verification is not added to copied bytes. Labels use IEC units
(KiB/s, MiB/s). Delete charts use processed items/s, including reported directories;
system Trash may report only top-level items. No recursive sizing pass is started
to estimate deletion throughput. These are operation activity rates, not physical
disk bandwidth or completion percentages.

Stalled operations receive zero-activity samples even while a native call is
blocked. Waiting/cancelling/terminal states show zero current speed; real work
completed by an already running native call after cancellation still enters the
history. Short final intervals are merged into the previous sample when necessary,
and short completed jobs retain a visible point. Counter resets rebase measurement
instead of producing negative rates. Terminal chart history remains with the
existing session-only Jobs record and is not written to settings.

The shared Slint chart appears in each operation progress window; Jobs is a
ledger whose Open action reopens that operation or its retained details. Its horizontal scale contracts from time zero as the operation grows, and
its vertical scale expands when a higher peak appears. The filled write/deletion
series, separate verification line and text labels adapt to the theme and dialog
size. Existing queue, confirmation, cancellation,
checksum and source-removal policies are unchanged. See the
[chart design](../superpowers/specs/2026-11-10-operation-speed-chart-design.md).

## Remaining-work estimates

Metadata discovery also sums the selected verification range length per file,
without retaining a file list or reading contents. The engine publishes optional
`TransferRemainingWork` write/read byte counts. A file of length L and verification
range length V needs L+V byte-work for Copy, or L+5V for cross-volume Move: initial
readback plus source/destination proof passes before and after source capture.
This is advisory work accounting; it cannot grant publication or source-removal
permission. Native rename retains root-only discovery and does not acquire a
recursive scan or byte budget.

Logical credit uses completed publication/removal and the exact active copy/proof
state. The After proof is outstanding again on an explicit removal Retry, including
before its next `begin_validation` call resets the old proof counter. Cumulative
I/O counters remain activity only. The UI divides remaining byte-work by the latest
combined write/readback activity rate, suppressing unavailable, paused, failed or
skipped estimates. Metadata/sync/namespace latency is unbudgeted; estimates remain
approximate. See [the budget evidence](../verification/2026-11-12-transfer-eta.md).

## Publication and recovery

Windows captures the exact guarded old file into a private same-volume directory,
then publishes the verified stage without replacement. This has an explicit
namespace gap. A concurrent new destination wins; both the old file and the
verified stage are retained instead of deleting that winner or rolling back over
it. Linux links its verified anonymous inode into a private directory and uses
an exchange; it checks the displaced identity/revision and retains unexpected
objects, including links or trees, without following or removing them.

The recovery directory is `.midnight-recovery-<random>` beneath the destination
folder. Windows uses a protected owner-only DACL; Linux requires owned private
recovery and a parent protected against other-user name replacement. These are
not isolation from root/administrators or malicious processes of the same user.
An error after namespace mutation is a structured outcome, never a request to
delete recovery data during cleanup:

- `Published`: no-replace publication completed.
- `PublishedDurabilityUnconfirmed`: publication happened, but a durability check
  failed. Keep the target; this does not authorize Move source deletion.
- `Replaced`: verified destination published and original retained.
- `RecoveryRequired`: an actual mutation occurred, with an optional published
  path, retained recovery objects and the failure. This is not complete success.
  An unknown post-mutation identity remains `None`, never an invented identity.

Before mutation, an ordinary error leaves the existing target untouched. After
recovery ownership transfers, Drop closes handles but never removes retained
objects. No implicit rollback or recovery garbage collection is performed.

Jobs shows the exact recovery count and the first 16 paths. Transfer and deletion
receipts remain throughout the session until explicitly cleared; history is not
persisted across restart. Copy details while available. The files themselves remain on disk until explicitly
removed. This is not a persistent recovery catalogue or an automatic Undo action.

## Bounded ordinary Move

Explicit Move uses the same FIFO, admission, mutation exclusion, cancellation and
completion receipts as Copy. F6 retains the captured native selection and other
pane destination in a typed Move draft; Cut publishes native Move clipboard
intent. Pasting a Move clipboard opens confirmation. `ClipboardMoveRequest` keeps
an `Arc<FileClipboard>` so confirming or retrying never clones raw paths on the
UI; `CopyInput` unwraps/clones it only in blocking preparation. Ordinary Copy and
archive Copy keep their existing routes. Native formats retain raw paths; no
shell parses file names.

Qualified native same-volume Move takes the rename route described above before
content/proof preparation. The content-copy proofs and source-removal staging
below apply to cross-volume fallback and explicitly merged children; they are not
required for a successful native rename.

Content-only Move accepts at most 64 ordinary native file or directory roots,
with a conservative 4 MiB active source/capture/stage/proof/receipt path reservation.
Duplicate identities, overlapping roots (including a selected file beneath a
selected directory), a destination inside a source directory, unsupported root
authority and exceeded root budgets are rejected before writes. Preparation pins
all root authorities and root-file stages. Recursive traversal admits one child
at a time, with at most 128 active directory frames and 100,000 admitted entries;
the child's native authority and file proof support are checked before its copy.
A later unsupported child or exceeded traversal budget stops unfinished work and
retains earlier completed moves. Windows source DELETE authority is obtained
before each source's copy, separately from Copy's unchanged read-only pin.
Protected source parents and qualified local NTFS/ext semantics are mandatory;
unsupported parents suggest Copy, never permission weakening or silent fallback.

Optional contracts default to Unsupported:

| Contract | Meaning |
| --- | --- |
| `prepare_move_source` | Retain the actual source reader plus opaque root authority, without changing source names. |
| `TransferSource::next_move_child` | Open one child relative to the held directory without following links, returning explicit Move authority. Ordinary Copy sources refuse this method. |
| `prepare_move_destination` | Qualify retained stage read and final-name proof support before copying. |
| `rewind_for_validation` | Rewind the existing source/stage file object and obtain a fresh handle-only revision guard; no old source pathname reopen. |
| `revalidate_publication` | Check held destination directory/final basename against the exact held stage identity, without following replacement links. |
| `TransferMoveCapture::capture` | Temporarily stage the held source under its raw basename in a private sibling directory. Structured outcomes describe actual mutation; this call does not delete data. |
| `TransferMoveCapture::restore` | Restore the exact held original without replacing a new original-name winner, using cancellation-independent cleanup. |
| `take_stage_cleanup` | Transfer exact directory-only authority; remove the empty stage after source owners close. Never recursively delete children. |
| `TransferMoveCapture::remove_empty_directory` | Revalidate source identity/parent binding and remove only the held ordinary directory after completed children. The kernel must enforce emptiness; this never deletes children. |

Only completely successful Published/Replaced copies yield `VerifiedFileProof`.
The user-requested Move order is per file: copy and independently verify one
file, validate its retained source/destination under the frozen policy and required
metadata, capture that source, retain the platform's source-write guard, and repeat
the selected proof before explicitly removing that source and starting the next
copy. Full mode hashes both objects; quick mode compares exact length and the
declared samples (all bytes up to 192 KiB). Each full
proof pass uses one shared 256 KiB buffer; source and destination revisions from
the final reads remain guarded through the removal gate. Earlier destination
identities stay protected from later replacement, without retaining their files.

`TransferMoveCapture::preflight_finalization` refuses unsupported removal before
publication. `prepare_finalization` requires a matching successful capture and
returns an opaque `TransferMoveFinalizer`; its `revalidate` checks the retained
object/temporary binding, and `finalize` reports actual Removed/Retained facts.
Drop never removes originals. Regular-file removal on Windows requires local fixed NTFS and single-link native
files. Windows refuses readonly files and any named alternate
data stream, including empty streams that were not copied and verified.

Linux now supports live-validated removal of owned, single-link regular files on
qualified local ext filesystems with protected source parents. Preflight retains
a read lease before publication, refusing existing writers or writable mappings.
One bounded dedicated thread receives blocked, thread-directed SIGIO notifications;
the process-wide signal disposition is unchanged. The lease remains held through
staging, complete source/destination rehashes, identity/revision/metadata checks
and directory-relative unlink. Any observed lease break or source change refuses
removal and attempts no-replace restoration. Drop never removes source data.

This Linux path is not atomic compare-and-unlink or an immutable snapshot. A pending
old-name writer triggers a lease break; the kernel can forcibly remove a lease
after its configured timeout. Checks refuse observed breaks, but arbitrary pauses
between the final check and unlink across that timeout are outside the guarantee.
Private staging does not isolate malicious same-UID/root namespace mutation.
Unsupported source filesystems/kinds/rights fail before that entry's publication without a
silent downgrade to Copy. See [native qualification and race evidence](../verification/2026-11-08-ordinary-move.md).

On Windows, a Read oplock probe on an asynchronous exact-object `ReOpenFile`
handle rejects preexisting writable mapped sections. Its pending IO is cancelled
and drained before buffers disappear; the original source handle continues to
deny write sharing for the unnamed stream. Removal uses exact-handle POSIX
disposition with image-section checks and no readonly override. All application
source/proof/finalizer owners close, then the empty source stage is removed,
before the next file. External readers can retain bytes after logical name
removal; physical storage release and parent-entry power-loss durability are not
promised. Named stream sharing is per stream: no whole-file snapshot/CAS claim is
made. Qualification is repeated before removal and late ADS changes refuse it.

A skip retains that entry or subtree and all of its active source ancestors.
Other siblings can still finish. Source directories are removed in postorder,
after their source/destination bindings are checked. Windows uses the retained
DELETE handle and refuses readonly directories or unverified directory streams
both at admission and final removal. Linux checks the held parent and no-follow
directory identity before `unlinkat(AT_REMOVEDIR)`. Linux has no inode-CAS rmdir;
the same live-operation namespace boundary applies. An unexpected child makes
empty-only removal fail. A skipped ancestor is retained without attempting removal.
No complete-tree snapshot, root-capture manifest or recursive deletion is used.

An unresolved terminal failure or cancellation stops unfinished Move work;
earlier successful moves remain completed, and later source names stay in place.
At a supported failure pause, explicit Skip retains that source and allows later
siblings to continue; Retry resumes only the failed phase.
Every pre-deletion failure or cancellation after staging attempts a no-replace
restore to the original name, even when the user cancellation token is set.
The transient path is `.midnight-move-<random>/<original raw basename>`. A
successful move retains no source copy and removes its empty temporary directory.
This is internal staging, not a selectable recovery mode, journal or catalogue.
If a new source-name winner blocks restoration, both objects are preserved and
the exact exceptional temporary location is reported. Failed empty-directory
cleanup is also reported; cleanup never removes an unexpected child. Progress
reports actual file removals, source disposition/durability facts, exceptional
residual counts, first16 raw residual mappings and at most64 source parents for
refresh, prioritizing selected-root parents. File-removal counts exclude directories;
disposition/durability counters include empty-directory removals. They do not
measure reclaimed space.
No destination rollback or batch rollback is attempted.

Salamander `src/worker.cpp` cross-volume `DoMoveFile` calls `DoCopyFile`, then
deletes that file's source on successful non-skipped copy (around lines6067–6080).
FM follows this per-file interaction with independent destination readback under
the selected policy (full SHA-256 by default), held-object deletion and no
readonly/ACL weakening. Earlier retained-source
Move verification reports are historical and superseded by this ordinary policy.

Qualified same-volume native Move renames without reading or hashing contents.
Cross-volume Move and explicit directory merges use per-file verification under
the job's immutable full/quick policy before source removal. Native permission
errors do not authorize a cross-device fallback. Recursive Move uses ContentOnly;
required modified time still
accepts only distinct regular-file roots. Hardlinks, links, virtual providers and
broader metadata remain separate unfinished scope. No phase promises an atomic
snapshot or batch rollback under arbitrary concurrent mutation. See
[recursive native qualification](../verification/2026-11-08-recursive-move.md).

## Required modified time

Existing requests and constructors remain `ContentOnly`. An explicit
`Command::EnqueueTransferWithMetadata(Box<MetadataTransferRequest>)` selects
`TransferMetadataPolicy::RequireModifiedTime` for native paths, captured panel
selection or shared clipboard input, with an explicit Copy/Move intent.
The first slice accepts at most 64 distinct native regular-file roots within the
existing 4 MiB proof-path reservation. Directories, links, virtual sources and
unsupported providers are refused without a policy downgrade. The UI option is
described separately in [transfer options](TRANSFER_OPTIONS.md).

All source metadata pins and destination staging capabilities are prepared before
content writes. Source pins capture a consistent native modified time and
revision before any source data read, retaining the existing file object through
`Arc<File>`. This adds no source write authority or pathname reopen. Native read
errors propagate; continuously changing metadata is refused after bounded
capture attempts. Existing stages and constructors retain their original rights;
Windows requests WRITE_ATTRIBUTES only on newly owned required-metadata stages.

For a copied file, source streaming and independent destination readback complete
under the frozen full/quick policy first. The source metadata pin is revalidated;
modified time is set on the exact
unpublished stage, synchronized and read back exactly, then the source pin is
checked again. Readback has no rounding tolerance. A metadata failure cannot
publish that file or create a Move proof. Before publication, both the source pin
and stage time are checked again; validation errors are terminal. A late conflict
reuses this same verified stage through existing replacement preparation.

Preflight establishes kinds and capabilities, not that every source timestamp is
representable by every destination. Value-specific destination precision failures
are discovered after data verification on unpublished stages. Earlier completed
files may remain published; a failure at this stage prevents removal of that source
and stops subsequent Move work. There is no silent switch to content-only copying.

Move proofs retain the metadata pin and expected modified time alongside their
existing source/stage/digest/length. The selected-policy source/destination proof,
fresh native read guards and exact final-name checks remain mandatory. Full mode
hashes both objects; quick mode retains its exact-length/sample guarantee. Expected
source and
destination times are checked during each proof and before captures. After our
source rename, the old revision token is not compared to its changed ctime;
fresh proof guards and the retained source-handle timestamp read are used.
Published targets are never corrected to conceal a mismatch. A mismatch after
temporary staging reports an error and attempts no-replace source restoration;
an exceptional restoration conflict preserves and reports the exact staged bytes.

Windows preserves native 100 ns values and explicitly refuses reserved/unsettable
FILETIME values; Linux preserves native nanoseconds, including supported negative
epoch values, with access time left to its existing policy. Timestamp readback is
verified after native synchronization, and native tests also check the final
closed file. Current native filesystem coverage is NTFS on Windows and ext on
Linux; this does not broaden remote commit or power-loss guarantees. Native
version checks are still not immutable snapshots.

## Progressive selection feedback

`CopyEvent::RootCompleted` reports one requested root after verified publication
and, for Move, source removal. It is independent of throttled byte-progress
snapshots and precedes the next conflict or terminal event. Directory completion
requires the entire subtree with no skipped or failed descendants. Failed,
skipped or cancelled unfinished roots remain selected. This is feedback only;
it grants no filesystem authority and never infers success from a progress count.

Native selection identities use the exact raw source path; archive adapters
provide `TransferSource::virtual_identity` and retain mount/entry identity.
Presenters capture the originating pane context, preserve the caret, and queue
small idempotent Remove edits on existing selection workers. Bounded-channel
pressure and coalesced refusal acknowledgements must retry. Same-folder refresh
rebases by path; navigating away must not alter a new selection context. See the
[regression evidence](../verification/2026-11-08-transfer-selection.md).

Adopted trees use exact occurrence/session completion identities from an
intent-bound `CapturedTransferSources`. F5 supports native/archive captures;
F6 requires native preparation and the ordinary Move proofs. Confirmation never
upgrades Copy authority into Move. Successful-root Remove acknowledgements settle
before native root/loaded-branch reconciliation. See the
[tree transfer contract](TREE_PANEL.md#move-and-transfer-reconciliation) and
[integration evidence](../verification/2026-11-10-tree-move.md).

## Current limits and next stage

Copy preserves verified content, with explicit required modified time as above.
Link copying and remaining times/attributes,
permissions, ACLs, extended attributes and alternate streams need their declared
metadata capability matrix. Existing source link/special entries are refused.
Recursive directory Move uses the per-file proof and postorder empty-directory
removal contract above. It does not infer descendant verification from root
metadata. Skips retain ancestors; failures/cancellation preserve unfinished
sources and earlier completed moves. Remaining directory metadata preservation
needs its own declared postorder capability. Physical storage reclamation and
atomic tree snapshots remain outside the contract.
