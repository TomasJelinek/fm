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
copy stays stopped until an explicitly supported Retry restores its checkpoint. The
one automatic retry repeats a namespace change (stage, directory or link creation,
publication, a Move's source capture, source removal or empty-directory removal) that
granted peers refused before any mutation, once a refusing peer changes; see
[operation progress](OPERATION_PROGRESS.md#admission-policy-and-waits). Cancelled
and recovery-terminal copies cannot publish again. A late no-replace
collision leaves the verified stage available for a different name or explicit
replacement; the consumed source stream is not replayed.

All methods run on backend workers. UI receives bounded immutable progress and
versioned conflict metadata, and sends a choice with the exact job/version.

Transfer entries carry raw modification times, which neither native adapter
formats. The engine formats only displayed entries, on a backend worker and only
when the adapter supplied no text: both sides of a Copy/Move or Rename conflict
prompt, and the existing output of a Make File List save confirmation. Copy/Move
formats through the audited locale (`Engine::configure_audited_locale`), because
a copy job retains only audited owners; with `configure_locale` alone its prompts
show adapter text or the UI's UTC fallback. Rename and Make File List use the
engine locale from either slot. Enumerated, inspected and automatically resolved
(Replace all, Merge all) entries are never formatted, and progress events carry
no entry metadata.

Copy/Move jobs start at once and run concurrently on every storage class; the
configured disk-I/O limit ([Disk operations](DISK_OPERATIONS.md); Automatic is four)
interleaves their native steps. Where their native namespace footprints overlap,
only the overlapping item waits for the job that already holds it; a job that would
read what another job changes, or whose footprint is unknown, waits as a whole. Each
job performs one native call at a time and holds its disk slot only while it
executes native steps; see
[operation progress](OPERATION_PROGRESS.md#admission-policy-and-waits). More
than one stream within a job still requires positive local SSD classification at
both endpoints and shared budgets. The
[local concurrency experiment](../verification/2026-11-10-ssd-copy-probe.md)
found substantial component gains for large/mixed files, but abnormal small-file
audit delays left its full qualification incomplete; it authorizes no within-job
parallel streams.

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
| `prepare_target_with` | `prepare_target` whose later revalidations, the provider's replacement checks included, apply an explicit `TransferRevisionPolicy` (see [change-time drift](#change-time-drift-view-05)). The default keeps the exact revision. |
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
Windows refuses that rename while any entry below the folder is open, and virus
scanners open files for a moment after they are written. The Move does not wait for
that inside its namespace turn, which every other transfer waits for before its own
name changes; it stops with its Retry/Skip/Cancel decision. (Rename and the metadata
editor's case change wait such a refusal out for about one second.)
`native_entries_moved` and items/s express this work separately from copied bytes.
Committed errors retain their recovery facts and cannot replay a source transfer.
The [current policy and API design](../superpowers/specs/2026-11-10-transfer-verification-and-native-move.md)
supersedes earlier unconditional SHA-256 and same-volume-copy descriptions below;
full-hash proof discussions describe the default mode, and sampled proofs retain
the same identity/revision/publication guards with bounded range reads.

## Change-time drift (VIEW-05)

Native revisions include the change time (Windows ChangeTime, Linux ctime). It also
moves on metadata-only updates: on the Windows development machine endpoint
protection moves it on freshly written files one or two seconds after they are
written, with the file ID, length and LastWriteTime unchanged. Copy and Move compare
their pinned revisions under `TransferRevisionPolicy::AcceptChangeTimeOnly`
(`TRANSFER_REVISIONS` in the engine): a revision whose change time alone moved is
unchanged, and every other captured field still counts.

| Check | Copy and Move | Compared fields under that policy |
| --- | --- | --- |
| Conflict pin (`prepare_target_with`), including recoverable replacement and native Move replacement | `AcceptChangeTimeOnly` | Identity; Windows length, LastWriteTime, creation time and attributes; Linux length, mtime and mode |
| Move capture of the source (its own binding pin) and Move children | `AcceptChangeTimeOnly` (`prepare_move_source_with_policy`) | As a conflict pin |
| Revalidation of Copy sources (`prepare_transfer_source_with_policy`), Move sources and their children | Linux `AcceptChangeTimeOnly`; Windows has never compared ChangeTime here | Identity; length and modification time (Windows also creation time) |
| Verification read guards, modified-time pins, planned-source namespace pins, Move finalization | Always `AcceptChangeTimeOnly`; only Copy and Move use them | As a conflict pin; Linux namespace pins compare length and mtime |

Exact revisions remain for every other consumer of the same services:
`prepare_target`, `prepare_transfer_source`, `prepare_transfer_source_with_links`,
`prepare_move_source` and `prepare_move_source_with_links` keep their exact
behaviour, so user-command scope pins, checksum manifest and file-list outputs,
checksum sources, Rename (including its replacement capture) and drag previews are
unchanged. The policy-taking methods default to their exact counterparts for
providers without native change times.

A same-length rewrite that restores the modification time differs from the pinned
revision only in its change time, so these Copy and Move checks no longer detect it.
On Windows, an open Copy or Move source handle denies write sharing; Linux Move
removal keeps its read lease; a replaced conflict target stays in recovery.
Directory-link stubs keep exact revisions.

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
A conflict Skip (Skip, Skip all, or Skip for a name inside a merged folder) keeps
the existing destination and is not an error: when such skips are the only
exceptions, the job ends Completed without a failure notice, and Jobs details
count the skipped entries. An I/O error the user skips is still an error: the job
ends Completed with errors, or Failed when nothing was transferred.

## I/O errors and progress lifetime

`CopyEvent::Failure` carries a `CopyFailure` with operation ID, monotonically
increasing decision version, `CopyErrorStage`, native error and separate source
and destination paths. `can_retry` and `can_skip` are capabilities, not UI guesses.
Decisions use the existing `Command::DecideCopy` route. A stale version, wrong job
or unsupported choice leaves the pending decision unchanged. Error Retry and
Skip All never provide consent to replace or merge a conflicting destination.

The async decision waiter releases its blocking worker and the job's disk slot;
the namespace share, claim, native owners and the logical queue reservation stay
held, preserving FIFO ordering. Other disk users and jobs proceed while the user
decides. After a non-Cancel answer the job readmits its slot through the FIFO
gate before its next native step (Waiting for disk capacity); a Cancel answer
cancels the job at once, and checked teardown reclaims the slot without waiting.
Cancel/shutdown wakes a paused job and performs cleanup before delivering terminal
progress.

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
  (`AlreadyExists` from publication, replacement binding, directory creation or
  native rename at the chosen name) becomes a fresh name conflict; an existing
  private stage or recovery name is an ordinary error, never a silent retry.
  Integrity failures, changed identities and uncertain publication never authorize
  retry or deletion. A different name that the destination cannot represent
  (reserved, invalid or overlong) asks again before it reaches publication.
- Enumeration errors cannot replay a cursor safely and offer Skip/Cancel, with
  Skip All scoped to this error type. Move retains the skipped subtree/ancestors.
- File-source deletion Retry keeps the verified destination, obtains a fresh
  native finalizer and rehashes both held objects before another removal attempt.
  It never recopies the file. Directory removal retries the exhausted frame,
  revalidating bindings without enumerating that directory again. When granted
  peers refused either removal, its retry runs automatically (the file removal
  still with the full rehash). A source capture they refused captured nothing; it
  is repeated automatically from the capture step, and the post-capture proof
  still rehashes both files before removal.

Skip All is scoped by phase, native error kind and operation name within one job.
A Move that cannot take its source before anything is copied (the native rename,
or a Move-only check of the source) stops at the "Moving the source" phase with
the operation `move`. Linux moves a folder into another folder only when the
folder itself is writable, because its link to its parent changes; a read-only
folder is reported as such and left in place.
Successful retries do not increment failed counts; skipped errors and cleanup or
restoration failures remain in history. A source-name restoration conflict retains
both objects and reports the residual path. Unsafe cleanup/commit failures stop
the job instead of offering an optimistic Retry.

The error dialog has Retry/Skip/Skip All/Cancel only when supported. Decide later,
Escape, the window's close button (X) and the dialog's header close button close
only the decision window and preserve the attention state; Jobs → Resolve opens the
exact pending decision. Only Cancel job cancels the job from that window. A deferred
Copy/Move decision keeps later destructive mutations waiting behind the predecessor
fence until it is answered. Successful Copy/Move closes only automatically opened
progress, after all associated work succeeds. Manually opened Jobs remains open. Completed
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
not aggregated peaks. Each operation's progress window displays that job's chart;
the Jobs ledger has none. The UI caches chart geometry per job by snapshot identity
and never measures I/O itself.

The Copy/Move timer publishes the latest progress on each existing 250 ms tick,
including a sample already collected by the native I/O worker. Sampling a rate
and delivering that status are independent decisions. A full UI outbox drops only
that periodic attempt; the next tick tries the latest snapshot without blocking
the copy owner. UI wakeup follows successful delivery. Stalled native I/O can
leave byte counters unchanged while elapsed time and zero-activity samples advance.

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
it. Once the old file is captured, a later cancellation no longer interrupts the
publication, which completes as `Replaced`. Should the publication itself fail after
the capture (for example because the destination path was rebound), the captured old
file is renamed back into its still-absent name through the pinned destination
directory, without replacement, and only the verified stage remains recovery data; if
a new entry took that name, both objects stay in recovery. Native Move restores a
staged source the same way. Linux links its verified anonymous inode into a private directory and uses
an exchange; it checks the displaced identity/revision and retains unexpected
objects, including links or trees, without following or removing them.

The recovery directory is `.midnight-recovery-<random>` beneath the destination
folder. Windows uses a protected owner-only DACL; Linux requires owned private
recovery and a parent protected against other-user name replacement (owned by the
user or root, and sticky or writable only by the user and the user's private
group, the latter only without a POSIX access ACL). These are
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

For regular files as for links, a `PublishedDurabilityUnconfirmed` file and a
`RecoveryRequired` outcome with a published path count as completed files (and
bytes), but never as Move proofs; the failure is recorded, and either outcome stops
the job: later files would otherwise keep publishing into a destination whose state
is uncertain.

Before mutation, an ordinary error leaves the existing target untouched. After
recovery ownership transfers, Drop closes handles but never removes retained
objects. Apart from that no-replace restore of a captured old file, no implicit
rollback or recovery garbage collection is performed.

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
A dedicated thread blocks only its own SIGIO, receives the thread-directed
lease-break notifications and releases a breaking lease at once, so another program
opening the source for writing is not stalled for the kernel's lease-break timeout;
the process-wide signal disposition is unchanged. The lease remains held through
staging, complete source/destination rehashes, identity/revision/metadata checks
and directory-relative unlink. Any observed lease break or source change refuses
removal and attempts no-replace restoration. Drop never removes source data.

This Linux path is not atomic compare-and-unlink or an immutable snapshot. A writer
opening the source releases the lease immediately and is recorded; later checks
refuse removal. A writer that opens the source between the final check and the
unlink is not detected.
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
siblings to continue. Retry is offered only for a failed source removal and
repeats the complete post-capture proof before removing the source.
Every pre-deletion failure or cancellation after staging attempts a no-replace
restore to the original name, even when the user cancellation token is set.
The transient path is `.midnight-move-<random>/<original raw basename>`. A
successful move retains no source copy and removes its empty temporary directory.
This is internal staging, not a selectable recovery mode, journal or catalogue.
If a new source-name winner blocks restoration, both objects are preserved and
the exact exceptional temporary location is reported. Failed empty-directory
cleanup is also reported; cleanup never removes an unexpected child. A capture
that fails before moving its source removes its empty private directory in the
same namespace turn; the reservation made when the turn created the directory
covers that removal. When the removal itself fails (for example because another
program created an entry in the directory), the capture keeps the directory: a
repeated capture reuses it, and otherwise the job's cleanup removes it or reports
its path. A repeated capture announces the reused directory again; granted peers
can refuse that (nothing has moved), and the capture keeps it the same way until
the engine repeats the capture.
Progress reports actual file removals, source disposition/durability facts, exceptional
residual counts, first 16 raw residual mappings and at most 64 source parents for
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
accepts only distinct regular-file roots. Links move as links on Linux (see
[links](#links-in-copy-and-move)); hardlinks, virtual providers and broader metadata
remain separate unfinished scope. No phase promises an atomic
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

## Windows namespace pins

The Windows adapter reports exact lookup keys only on NTFS volumes whose
persistent 8.3-name creation is disabled, for ASCII names in case-insensitive
directories and any names in case-sensitive directories. Every other binding is
`Unknown`, which sends that turn to the host's opaque lane and its full refresh of
the job's pins. Original pins (selected sources and destination entries or
subtrees) and their directory anchor handles are retained for the job's lifetime;
an external change to one of their bindings is a job namespace error. Private pins
witness the stages a job creates and then their published outputs; once every native
owner of such an object has closed, the next refresh retires that alias, so published
outputs neither accumulate claims nor turn later external changes into job errors.
Their names stay covered by the original destination pins. By design, a retired
output's object identity no longer takes part in peer admission: another job that
reaches the same object through a different hard-link name is not excluded by it,
because this job never touches that object again.

A turn reserves each pin at most once. The pin of a private directory that the turn
creates (a Move stage or a `.midnight-recovery-*` directory) is reserved before the
directory exists and then also covers the renames into and out of it and its removal
in that turn. Granted peers can therefore refuse a Move capture, a native replacement
or a replacement publication only before its first namespace change; in that turn
they never refuse a no-replace restoration or the removal of a directory the turn
created. Only a binding whose parent's ancestry or lookup rules change during the
turn needs a later unknown reservation. Outside the exclusive opaque lane the host
rejects it as `Unsupported` and the turn makes no further change, so what it has
already moved stays where it is: a Move capture keeps its stage, empty or holding the
source it could not restore (see [bounded ordinary Move](#bounded-ordinary-move)), and
a native replacement or publication keeps the displaced original in recovery and
reports it.

Windows Copy and Move require 128-bit `FileIdInfo` identities, because they compare
identities across their own stage publications, Move captures and namespace pins.
Volumes without them (FAT32, exFAT, some redirectors), whose 64-bit file indices
change when an entry is renamed, are refused as sources and destinations with an
explicit `Unsupported` error.

The namespace unit tests take their qualified (incremental) branches on a volume
whose persistent 8.3-name creation is disabled and their unqualified (full-refresh)
branches elsewhere. In test builds `MIDNIGHT_TEST_NAMESPACE_LOOKUP` forces one mode
process-wide (`short-names-disabled`, valid on any NTFS volume because the fixtures
never use short names, or `unqualified`); pooled parent bindings are shared by
concurrently running tests, so the mode cannot be per test. Every default `--lib` run
also reruns `transfer::namespace::tests::` in a child process forced into the mode the
build volume does not provide
(`parent::tests::namespace_tests_pass_in_the_other_lookup_mode_too`).

## Linux namespace pins

The Linux adapter (`transfer_namespace.rs`) reports exact lookup keys only for a folder
whose names compare byte for byte, on a mount whose root is the filesystem's root:

- ext2, ext3 and ext4, tmpfs and Btrfs, when `FS_IOC_GETFLAGS` reports neither case
  folding (`FS_CASEFOLD_FL`) nor encryption (`FS_ENCRYPT_FL`; without the key, names
  are encoded) for the folder. Btrfs never folds case.
- XFS under the same flags, and only on a volume made without the ASCII-CI feature
  (`mkfs.xfs -n version=ci`), as `XFS_IOC_FSGEOMETRY` reports it: with the feature,
  ASCII letters compare case-insensitively. A volume whose geometry cannot be read
  (kernels before 5.1) is `Unknown`.
- The mount's root in `/proc/self/mountinfo` must be the filesystem's root. Under a
  subdirectory bind mount, or a Btrfs subvolume mounted on its own (with `subvol=`, or
  as a default subvolume other than the top level, as distributions commonly mount `/`
  and `/home`), `..` from the mount's top folder leads to the folder it is mounted on
  rather than to its parent on the filesystem, so the folder's real ancestors, and a
  rename of one of them, cannot be seen. Folders there are `Unknown`.

Overlayfs is `Unknown`, because an object's identity changes when a lower file is
copied up and its names merge layers whose lookups differ. So are NFS, SMB and other
network filesystems and FUSE mounts, whose server or daemon decides how names compare
and where others change names unseen, and any filesystem that does not answer
`FS_IOC_GETFLAGS`. As on Windows, an `Unknown` binding sends the turn to the host's
opaque lane with its full refresh of the job's pins: transfers there exclude each other
instead of running side by side, and Copy and Move still work.

The Linux namespace unit tests create their fixtures in the temporary folder
(`TMPDIR`), which must therefore lie on a qualified mount; below a bind-mounted folder,
such as a container's `_build`, they stop with "fixture requires qualified local byte
lookup".

## Links in Copy and Move

`CopyLinkPolicy` is frozen when a job is admitted. The Copy/Move dialog sends it
through `Command::EnqueueCopyWithOptions`; its choice is platform policy in
`crates/midnight-ui/src/transfer_link_option/`:

| Dialog | Linux | Windows |
| --- | --- | --- |
| Copy, **Follow links** checked (default) | `FollowLinks` | `FollowDirectoryContents` |
| Copy, cleared | `Preserve` | `EmptyDirectories` |
| Move (Linux shows no option) | `Preserve` | as Copy's option |

`Preserve` never opens a link's target. The Linux source yields every link it meets,
the selected root and each link below a selected folder, as a link leaf (`EntryKind::Symlink`)
whose `link_representation` is the exact `readlink` bytes (at most 64 KiB, relative
text kept verbatim). That read revalidates the parent binding, the link's identity,
revision and text through the held parent; an in-place change fails. Discovery counts
a leaf without following it.

`FollowLinks` (Copy only) follows a directory link exactly as `FollowDirectoryContents`
does, under the same mount, link-depth and ancestry rules, and presents a link to a
regular file as that file: its content is copied and verified like any file, and the
binding of the link and its target is revalidated, with the file's revision, before
each use.

Both follow a link only to a target on the link's own mount. The link's text is
resolved as the kernel resolves it, but never across a mount point (nor by `..` above
the mount's root) and never through a magic link. The kernel does that itself under
`RESOLVE_NO_XDEV`, except when the link is on another mount than the root's and its
text, or a link met on the way, is absolute: the way from `/` back into the link's
mount crosses mount points, which `RESOLVE_NO_XDEV` refuses. Such a text is resolved
one name at a time within the mount, an absolute text from the mount's root when it
lies under the mount's path, with at most 40 links on the way (`MountAnchor::walk`);
the folders below a followed link are revalidated the same way. So with `/home` on its
own filesystem, a link there to `/home/user/folder` is followed, and one to
`/media/stick/folder` or `/proc` is not. Only links followed inside one another count
against the budget of 128: Copy has no fixed depth limit for ordinary folders under
any policy (Move keeps its own limit of 128 folders; on Linux the folders that
transfers hold open count against a [descriptor budget](#folders-held-open-on-linux)),
and `Preserve` and `EmptyDirectories` keep no ancestry at all. Recursive totals (discovery) still stop
after 128 nested folders, as without a link policy; the job then copies without them.

A link that `FollowLinks` cannot follow is a leaf recreated as with `Preserve`, and
`TransferSource::link_not_followed` gives the reason, which the job records as a note
(`CopyProgress::notes`, first 16, with an exact `note_count`) shown in Jobs details:
a missing target, a loop of links, an unreadable target, a target on another
filesystem than the link (the policy never crosses mounts), a special file, or a
directory that is an active ancestor (the cycle detection of `FollowDirectoryContents`,
which there is an error). A target is unreadable when the user cannot read the file,
or cannot list or enter the folder: opening it as a path needs neither, so this is
checked (`faccessat` with the effective IDs) before the link is followed, and such a
link is kept rather than failing when it is copied. A link whose text the kernel cannot
resolve within the mount (an absolute one, say) is also kept when its folder's mount
cannot be qualified for the walk, for example a folder whose path leads through
another link rather than along the mount's own path. Notes are not errors; the job
still completes.

Copy recreates a link (engine `LinkCopy`): admission (`TransferDirectory::admit_link`),
a private stage, sync, an independent readback that must equal the source text, a
revalidation of source and stage, then publication. On Linux the stage is `item` in a
new owner-only `.midnight-link-<random>` directory beneath the protected destination
folder (`symlinkat` relative to the held directory), so no link exists under a final
name before it is verified. Publication renames it to the final name with
`RENAME_NOREPLACE` (a filesystem that cannot exclude replacement refuses it) and
removes the empty stage; the published link's identity and text are checked again.
An existing name is an ordinary conflict: Skip, another name, or Replace when the
destination declares `TransferCapabilities::link_replacement` (local ext, as for
files) and the target is a file or a link, never a folder. Replace binds the target
pin (`StagedTransferLink::prepare_replacement`), moves the staged link into a new
`.midnight-recovery-<random>/item` and exchanges it with the target there
(`RENAME_EXCHANGE`), so the displaced entry stays in recovery, with the identity and
revision checks of file replacement. A link counts as a completed file and never as
content bytes.

Move moves the link itself. Preparation with `Preserve` returns an `Ordinary` entry
whose capture is the link's own: on one filesystem it renames the link through the
private `.midnight-move-*` capture used for directory links (Replace of a file or link
exchanges it into that capture directory, as a native file replacement does). Across
filesystems the capture recreates the link at the destination (the stage, readback,
publication and replacement above), checks the published link, and only then removes
the original link with the protected removal of a moved directory link (owned link
with one name, staged, revalidated, unlinked). If that removal fails, the outcome is
`RecoveryRequired` with the published destination, so the job reports the error and
both links remain. Its `try_native_move` therefore never reports `CrossDevice`; the
engine has no content fallback for a link and reports one that a provider cannot move.
Folders under a cross-volume Move are traversed with the same policy, so their links
move as links. `FollowLinks` is refused for Move.

Windows (not compiled on Linux hosts) keeps its directory-link policies: the source
refuses `Preserve` and `FollowLinks` and its destinations stage no links
(`link_replacement` is false), because recreating a symbolic link needs
`CreateSymbolicLinkW` with a privilege or Developer Mode and junctions need their own
qualification. The UI therefore sends it the existing directory-link policies.

## Folders held open on Linux

Each folder level that a Copy or Move works inside keeps three descriptors open (the
source folder, its entry stream and the destination folder), and each level of the
discovery that totals a selection holds its folder open too. These levels are counted
across all transfers of the process against a budget of a quarter of the soft
open-file limit (`RLIMIT_NOFILE`) at three descriptors per level, between 32 and 4,096
levels (`transfer_descriptor_budget.rs`). A folder past it is refused with an error that
names the limit, long before the process would run out of descriptors (EMFILE) and
fail whatever else opens a file meanwhile: other jobs, panel listings, watches and
viewers. Transfers that run at the same time share the levels, so a deep tree can be
refused while another deep transfer runs and succeed alone.

At start the application raises its soft open-file limit to the hard limit, at most
65,536 (`raise_open_file_limit` in `midnight-platform-linux`, called first in `main`):
a desktop session's usual soft limit of 1,024 allows 85 levels, the raised one 4,096.
If the limit cannot be raised, the application logs a warning and keeps the smaller
budget. Programs it starts for the user (an editor, a terminal, a custom command, the
application that opens a document) get the original limit back before they run,
because programs that use `select()` cannot handle descriptors numbered 1,024 or
higher. So does the application when it restarts itself with software rendering; it
then raises the limit again.

## Current limits and next stage

Copy preserves verified content, with explicit required modified time as above.
On Linux, Copy and cross-volume Move give each new file and folder the source's
read, write and execute bits masked by the process umask, as `cp` without `-p`
does: a file at publication, a folder once its contents were copied (it stays
private until then). A source without modes, such as an archive entry, gets those
of any new file or folder (0666 or 0777 masked by the umask). Set-user-ID,
set-group-ID and sticky bits are not copied, and a merged existing folder keeps
its permissions ([staging rules](../../SECURITY.md)).
Remaining times/attributes, ownership, ACLs, extended attributes and alternate
streams need their declared metadata capability matrix. Without an explicit link
policy (`CopyLinkPolicy::Reject`) existing source link/special entries are refused;
the Copy/Move dialog always sends one, see [links](#links-in-copy-and-move).
Special entries stay unsupported. A link cleanup failure or an uncertain link
publication (durability unconfirmed or recovery required) stops the job.
Recursive directory Move uses the per-file proof and postorder empty-directory
removal contract above. It does not infer descendant verification from root
metadata. Skips retain ancestors; failures/cancellation preserve unfinished
sources and earlier completed moves. Remaining directory metadata preservation
needs its own declared postorder capability. Physical storage reclamation and
atomic tree snapshots remain outside the contract.
