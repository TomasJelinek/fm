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
read what another job changes waits as a whole. A job whose footprint is unknown
(the opaque lane) runs alone: it waits for the other jobs and they wait for it, saying
"Waiting for another job: jobs whose folders cannot be confirmed run alone" rather than
that they use the same folders. Each
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
On Linux a published native Move removes its empty stage first and then syncs the
source and target folders once each (two folder fsyncs, one ext4 journal commit). A
stage that cannot be removed is synced with both folders and the Move reports it as
needing recovery; a replacement's stage, which keeps the displaced entry as recovery
data, is synced with both folders.
The [current policy and API design](../superpowers/specs/2026-11-10-transfer-verification-and-native-move.md)
supersedes earlier unconditional SHA-256 and same-volume-copy descriptions below;
full-hash proof discussions describe the default mode, and sampled proofs retain
the same identity/revision/publication guards with bounded range reads.

### Move durability on Windows

A Move reports a change as unconfirmed only when a step it takes to save that change to
disk fails, never merely because a platform has no such step. The provider says so in
`durability_confirmed` of `TransferNativeMoveOutcome::Moved` (and of the source
removals below); the engine counts each unconfirmed one in
`source_removal_durability_unconfirmed`, which gives the job a closure receipt, so it
ends "Completed with warnings" with "Source changes the system could not confirm as
saved to disk: N" (WIN-6 in the [Windows acceptance record](acceptance/windows.md)).

After a committed native rename, Windows flushes (`FlushFileBuffers`) the destination
folder and the source folder, and for a native replacement also the private recovery
folder that keeps the displaced original (`transfer_folder_flush.rs`). Windows flushes a
folder only through a handle that may add files or subfolders to it; through the Move's
own pins, which may only read its attributes and traverse it, the flush fails with
ERROR_ACCESS_DENIED (both measured on NTFS). So each folder is reopened through its
retained handle (an empty relative name, never the path) with the right to add files,
or else subfolders, sharing everything, and closed at once. No volume handle is opened:
its flush needs administrator rights. The flushes run after the rename's namespace turn
(the recovery folder's too), whatever the cancellation, since the rename is committed.

A Move saves its renames together. Its native renames (`try_native_move_deferred`)
leave the flushes to a confirmation (`TransferDurabilityConfirmation`) that holds the
retained handles of the folders the rename changed, and the engine runs the pending
confirmations in one round (`TransferDurabilityRound`), in which each folder is flushed
once for all of them: before the job removes a folder (a pending confirmation can hold
it, and a folder could not be saved once removed), when 64 are pending (each holds two or
three folder handles) and when the job ends, whatever its outcome. A Move of N entries
from one folder to another flushes the two folders once instead of N times each, and a
merge flushes a source folder and its destination once for all its entries before the
emptied folder is removed. Rename's replacement (`try_native_move`) flushes before it
returns, as before. A folder's result counts for every rename of the round that
changed it:

- Every flush succeeds: the rename is confirmed.
- The filesystem or file server does not flush folders: it refuses the call as not
  supported (ERROR_INVALID_FUNCTION, ERROR_NOT_SUPPORTED, ERROR_INVALID_PARAMETER), or
  with ERROR_ACCESS_DENIED although the handle may add entries, which is all a flush
  requires of it (a file server may answer so). The rename counts as confirmed as far as
  Windows allows and shows no warning.
- A flush fails otherwise (an I/O error), or a folder cannot be opened for one: the
  account may add nothing to it (removing its own entry from a folder needs no such
  right) or another program holds the folder without write sharing. The rename stays
  committed and is unconfirmed.

What that confirms depends on the filesystem; only NTFS was available on the
development machine:

- NTFS (measured, Windows 11 on an NVMe SSD): the destination folder's flush right
  after a rename took 0.047 ms on average (200 renames), the source folder's flush right
  after it 0.004 ms, as long as flushing an unchanged folder (0.002 ms), which suggests
  the first flush already wrote NTFS's log for the whole rename; the rename itself took
  0.11 ms. Per native Move call the two reopens and flushes took 0.14 ms of 0.6 ms
  (debug build, 400 Moves, a machine shared with builds). Saved together, 64 renames
  from one folder to another needed 2 flushes and 28–34 ms instead of 128 flushes and
  41–52 ms (0.44–0.53 instead of 0.64–0.82 ms per rename; debug build, three runs each,
  a machine shared with builds). Confirming a verified file removal (looking its name up
  in its private stage, reopening and flushing the stage) took 0.16–0.18 ms per file
  (200 files, three runs), next to 0.22–0.24 ms for its disposition, the closing of its
  handles and the stage's removal. On a disk whose cache flush is slow a Move waits for
  that flush once per folder and round, and once per removed file for its stage, as a
  Linux Move waits for its folder fsyncs.
- FAT and exFAT (not measured: no such volume without administrator rights): the FAT
  driver whose source Microsoft publishes (FastFAT) accepts a flush of a folder other
  than the root without writing its entries, so such a Move counts as confirmed
  without the Move writing them. Whether they survive a power loss depends on the
  drive's write caching: with Quick removal, the default removal policy for external
  drives since Windows 10 version 1809, Windows does not cache their writes.
- ReFS, including Dev Drives (not measured: creating one needs administrator rights),
  and network shares (the flush is the file server's): a flush refused as not
  supported counts as above.

Source removals by delete disposition are a cross-volume Move's verified file removal
(from its private stage), the removal of a folder a Move emptied (a cross-volume Move,
or a same-volume one that merged into an existing folder) and a directory link's
original after its copy. The Move always sets POSIX delete semantics
(`FILE_DISPOSITION_FLAG_POSIX_SEMANTICS`), so NTFS takes the name out of its folder when
the handle that set the disposition closes, also while other programs, or the job's
other handles on the entry, keep it open; until then a lookup of the name fails with
ERROR_ACCESS_DENIED, and a duplicate of the deleting handle keeps the name as well
(measured on NTFS for a file, a folder and a junction). Such a removal is confirmed
before it is counted:

- A file's or a folder's deleting handle is shared by the job's source, capture and
  finalizer, so the provider hands its confirmation to the engine
  (`TransferMoveFinalizer::take_removal_confirmation`, or
  `TransferMoveCapture::take_removal_confirmation` for a folder), which runs it once it
  has dropped them: a file's after its finalization, a folder's after its traversal
  frame, whose source holds the folder. A directory link's capture owns its only
  deleting handle and closes it right after the disposition.
- The confirmation looks the name up through the folder that held it (the private stage,
  the parent folder; its retained handle, never the path). Only "no such file"
  (ERROR_FILE_NOT_FOUND) counts as gone; the folder is then flushed as above.
- Confirmed, the removal is `TransferSourceRemoval::Unlinked` with its durability
  confirmed: neither the note that Windows accepted it (`source_delete_pending`) nor an
  unconfirmed count, so the Move ends Completed.
- Otherwise it stays `DeletePending` and unconfirmed, and both are then true: the name
  is still there (another handle on the deleting object is still open) or its folder
  could not be saved (for example another program holds it without write sharing). A
  filesystem without POSIX delete refuses the removal itself, as before; a cross-volume
  file removal requires local fixed NTFS anyway.

The contract for this (decided for WIN-6): a `TransferDurabilityConfirmation` completes a
committed change later and says whether it is saved (for a removal, whether its name is
gone too); `confirm` takes the round, which saves each folder at most once. The
finalizer's and the capture's `take_removal_confirmation` and the two capture traits'
`try_native_move_deferred` hand one over; their defaults hand none over and
`try_native_move_deferred` defaults to `try_native_move`. The engine counts a change that
has a confirmation by the confirmation alone. Linux hands none over, and its Move is
unchanged.

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
| The archive file of archive members copied out, before each chunk and each Retry ([archive members as a source](#archive-members-as-a-source)) | `AcceptChangeTimeOnly` since 21 November, like every capture of an archive | Identity, length and modification time (Windows LastWriteTime, Linux mtime) |

Exact revisions remain for every other consumer of the same services:
`prepare_target`, `prepare_transfer_source`, `prepare_transfer_source_with_links`,
`prepare_move_source` and `prepare_move_source_with_links` keep their exact
behaviour, so user-command scope pins, checksum manifest and file-list outputs,
checksum sources, Rename (including its replacement capture) and drag previews are
unchanged. The policy-taking methods default to their exact counterparts for
providers without native change times.

A same-length rewrite that restores the modification time differs from the pinned
revision only in its change time, so these Copy and Move checks no longer detect it.
On Windows, a Copy source denies write sharing from when its job opens it to read it,
before the copy is staged (before that, while its job waits, a write is detected only
through these fields; see
[Windows sharing](#windows-sharing-while-a-job-waits-and-runs)) and a Move source from
its preparation; Linux Move removal keeps its read lease; a replaced conflict target
stays in recovery.
Directory-link stubs keep exact revisions.

### Archive members as a source

A Copy out of an archive reads members through the archive's mount, which revalidates
the captured archive file before every chunk and before a Retry resumes a member
(`archive_transfer.rs`). On the Windows development machine Defender for Endpoint data
loss prevention added its `sec.endpointdlp` alternate data stream to a tar file about
two hours after it was written; NTFS moved the file's ChangeTime but not its
LastWriteTime or the main stream's length, and a Copy of its members stopped at member
315 with "archive source changed: Captured native content changed" (21 November,
while measuring WIN-8). Decision, as VIEW-05: every capture of an archive file accepts
a change time that alone moved, on Windows and Linux
([content reads](CONTENT_READS.md#change-time-drift-view-05)); its identity, length and
modification time still count, and the plugin still opens only a regular file without
following links.

The question after that failure offered Retry, which could never succeed: the capture
compared every revalidation with its fixed revision, change time included, and a
failure inside an exchange with the archive worker also stops that worker, so every
later request through that mount fails with "Archive worker was stopped after a failed
request (…); open the archive again". Now a lasting change, found when a reader that
the plugin opens on the archive's path reports another identity, length or
modification time, fails as `InvalidData` (`Mount::source_failure` in the filesystem
host, from `CapturedFile::changed_since_capture`), and the copy offers Skip, Skip all
and Cancel but no Retry, as for a changed native source. Other source failures, such as
an unreachable network file, still offer Retry, which succeeds once the file is back
when the revalidation between chunks failed: the mount survives that failure.

A failure inside an exchange with the archive worker (a source read the host could not
complete, a member the worker could not read, a protocol violation) stops that
worker, and so does the 20-second idle watchdog. The mount then never serves another
request: each one fails with "Archive worker was stopped after a failed request (…);
open the archive again" or "Archive worker made no progress for 20 seconds and was
stopped; open the archive again", as `InvalidData` since 21 November (`STOPPED` in the
filesystem host's `session.rs`; before, `Other` and `TimedOut`, which Copy offered to
retry), so the question offers Skip, Skip all and Cancel only. The failure that
stopped the worker keeps its own kind and message, such as a full spool volume's, so
its own question can still offer Retry once; that Retry then reports the stopped
worker. Not done: a Retry that opens the archive again for the copy (ARCH-08 in the
[backlog](../IMPLEMENTATION_BACKLOG.md)).

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
- A child that cannot be opened after its folder listed its name is that child's
  failure (`TransferSource::next_child_or_failure`, stage Reading): Retry opens it
  again by name with `open_child`, Skip leaves out only that child and its folder
  goes on. The Linux and Windows sources report such children; on Windows they are
  typically a child the user may not read, or a file that another program has open
  for writing (a Copy source denies write sharing). A Windows folder is held with list
  and traverse access, so unlike Linux no folder lists names that cannot be opened.
  Cancellation, a changed folder binding and a broken listing remain the folder's
  failure. Archive sources report every failure as the folder's.
- In a Move, a child of a moved folder that cannot be prepared for Move (a special
  file, a mount point, an entry that vanished or that another user owns in a sticky
  folder) is likewise that child's failure (`TransferSource::next_move_child_or_failure`,
  stage Moving the source): Retry prepares it again by name with `prepare_move_child`,
  and Skip leaves only that child, which stays with its folder and every folder above
  it. What all entries of a moved folder need (on Linux `check_moved_folder`: the
  bound, protected local folder lets entries leave it) is checked once per folder,
  before its entries, so a folder that does not let them leave fails once as Reading
  folder contents. The Linux and Windows sources report such children (on Windows
  typically a file that another program holds, which a Move cannot hold for its
  removal; a Windows child has no folder-level checks to separate). Sources that keep
  the default report every failure as the folder's.
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
folder itself is writable, because its link to its parent changes: one the user
owns gets owner write for that rename, and a read-only folder of another user is
reported as such and left in place.
Successful retries do not increment failed counts; skipped errors and cleanup or
restoration failures remain in history. A source-name restoration conflict retains
both objects and reports the residual path. An uncertain commit or cleanup is never
retried: where its data stays in place, the job records it with its recovery facts
and asks only whether to go on (Skip, Skip all errors, Cancel); a capture that does
not match, Move invariant failures and a failed restore during Skip end the job
([audit](OPERATION_PROGRESS.md)).

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
per-series measured maxima survive, so early peaks remain visible; the vertical
scale follows the recent sustained rate instead, so an older peak above it is
drawn clipped ([operation progress](OPERATION_PROGRESS.md#verification-and-charts)).
Different series' peaks in a combined interval need not have
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
its vertical scale rises with a higher peak and settles back to the recent sustained
rate within seconds, which pauses and stalls do not lower. The filled write/deletion
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
I/O counters remain activity only. The UI divides remaining byte-work by the
backend's averaged combined write/readback rate (an 8-second time constant over
activity time) and blends it with the remaining items at the averaged item rate,
the more toward items the smaller the files that remain
([operation progress](OPERATION_PROGRESS.md#verification-and-charts)). It suppresses
unavailable, failed or skipped estimates; a paused job keeps its estimate, marked
paused. Metadata/sync/namespace latency is unbudgeted; estimates remain
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
bytes), but never as Move proofs. The failure is recorded at once with its recovery
facts, and the job then asks whether to go on, with Skip, Skip all errors and Cancel
only (never Retry, which could replay the commit): Skip leaves the file as it is (a
Move keeps its source and the folders above it) and later files publish under their
own no-replace checks; Cancel stops the job.

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

Content-only Move accepts at most 64 ordinary native file or directory roots
(`MAX_MOVE_ROOTS`), with a conservative 4 MiB active source/capture/stage/proof/receipt
path reservation. Every Move entry point refuses a larger selection before a job starts,
with the sentence "Move accepts at most 64 selected roots. Choose a smaller selection."
(`MOVE_ROOTS_REFUSAL`): the dialog for F6 and for a pasted Cut, and the drop admission
for a drag (`drag_drop_internal::admit_move`). The job still refuses it, naming the
operation `move files`, as every error of a step that only Move takes does
(`copy_operations::move_error`). A Copy has no such limit.
Duplicate identities, overlapping roots (including a selected file beneath a
selected directory), a destination inside a source directory, unsupported root
authority and exceeded root budgets are rejected before writes. Preparation pins
all root authorities and root-file stages. Recursive traversal admits one child
at a time, with at most 128 active directory frames and 100,000 admitted entries;
the child's native authority and file proof support are checked before its copy.
A later child that cannot be prepared for Move pauses alone (see the I/O errors
above); an exceeded traversal budget stops unfinished work and retains earlier
completed moves. Windows source DELETE authority is obtained
before each source's copy, separately from Copy's unchanged read-only pin.
Protected source parents and qualified local NTFS/ext semantics are mandatory;
unsupported parents suggest Copy, never permission weakening or silent fallback.
Inside the moved tree Linux follows Delete's rule instead: a folder of that tree
(the selected folder or a folder below it) that the user owns and that lacks owner
write gets it, through the descriptor the Move retains for it, only for each change
that takes an entry out of it or puts one back (its private stage, a capture, a
restoration, the stage's and an emptied folder's removal, an entry's own rename on
one volume), and its own mode back as soon as that change returns
(`writable::restored` in `transfer_move_writable.rs`). If setting the mode back fails
(the filesystem turned read-only or failed), the folder keeps owner write and the
change says so, naming the folder: appended to the change's error, or as the error of
its outcome. A capture then becomes `RecoveryRequired` with its stage as the recovery,
so the source is put back; a native Move that reached its destination becomes
`RecoveryRequired` with that destination, so the job records it and asks whether to go
on; a stage cleanup, an emptied folder's removal or a restoration that was done
returns that error. A folder whose set-group-ID bit the kernel would clear when its
mode changes (the user is not a member of its group) is never made writable: its
entries fail at preparation, named as the folder's write protection, and a native
Move of the folder itself reports it as read-only. A folder's check runs once for
all of its entries: its enumeration keeps the checked folder (`SourceFolder`) instead
of checking it again, about a dozen system calls, for every entry. The folder that
contains the selection is never changed, so a write-protected one still stops the
Move at preparation ([security](../../SECURITY.md)), before anything is copied for the
item: a permission error that names it ("Permission denied: it cannot be moved out of
its folder, …"; a read-only mount says so instead) and, like every other transfer
error, offers Retry, Skip and Skip all of this error type (T-E06; until 21 November it
was an Unsupported error that ended the job without a question). Skip leaves the item
in its folder, and a job left with nothing to do ends at once, Failed, with the item's
error and its skip (`Preparation::is_last_root`). Nothing was copied for it, so there is
no verified copy to account for. Windows needs nothing here: a folder's read-only
attribute does not keep entries from leaving it. A folder whose access list denies
deleting its entries stops the item the same way, at its preparation, with the same
Retry, Skip and Skip all, and says so in words ("Permission denied: it cannot be moved out
of its folder: the permissions of the item and of its folder do not allow removing it.
…"), not with the system's "Access is denied" text.

Optional contracts default to Unsupported:

| Contract | Meaning |
| --- | --- |
| `prepare_move_source` | Retain the actual source reader plus opaque root authority, without changing source names. |
| `TransferSource::next_move_child` | Open one child relative to the held directory without following links, returning explicit Move authority. Ordinary Copy sources refuse this method. |
| `TransferSource::next_move_child_or_failure` / `prepare_move_child` | The next Move child, or that child's own failure apart from its folder's (the default reports every failure as the folder's); prepare a failed child again by name for Retry. Wrappers that forward `next_move_child_with_links` forward both. |
| `prepare_move_destination` | Qualify retained stage read and final-name proof support before copying. |
| `rewind_for_validation` | Rewind the existing source/stage file object and obtain a fresh handle-only revision guard; no old source pathname reopen. |
| `revalidate_publication` | Check held destination directory/final basename against the exact held stage identity, without following replacement links. |
| `TransferMoveCapture::capture` | Temporarily stage the held source under its raw basename in a private sibling directory. Structured outcomes describe actual mutation; this call does not delete data. |
| `TransferMoveCapture::restore` | Restore the exact held original without replacing a new original-name winner, using cancellation-independent cleanup. |
| `take_stage_cleanup` | Transfer exact directory-only authority; remove the empty stage after source owners close. Never recursively delete children. |
| `TransferMoveCapture::remove_empty_directory` | Revalidate source identity/parent binding and remove only the held ordinary directory after completed children. The kernel must enforce emptiness; this never deletes children. |
| `take_removal_confirmation` (finalizer; capture for an emptied directory) | After a removal reported `DeletePending`, hand over its `TransferDurabilityConfirmation` once; the engine runs it after dropping every source, capture and finalizer of the entry and counts the removal by its result ([Move durability on Windows](#move-durability-on-windows)). Holds no handle on the removed entry. |
| `try_native_move_deferred` (both capture traits) | `try_native_move`, leaving saving the folders a committed rename changed to the returned confirmation, which the engine runs with the job's other pending ones in one round. The default is `try_native_move` with none. |

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
source/proof/finalizer owners close; then the removal's confirmation checks, through
the held stage, that the staged name is gone and flushes the stage, and the empty source
stage is removed, before the next file
([Move durability on Windows](#move-durability-on-windows)). External readers can
retain bytes after logical name removal; physical storage release is not promised. Named stream sharing is per stream: no whole-file snapshot/CAS claim is
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
repeats the complete post-capture proof before removing the source. An uncertain
end of an entry (a native rename that needs recovery, a stage that cannot be removed
after the verified source was, an unconfirmed removal durability, a moved directory
link's original that needs recovery) asks Skip, Skip all errors or Cancel instead,
never Retry; Skip leaves the entry as it is and keeps its source folders when it left
something in them (the source, a private stage, recovery data). After a verified removal
whose durability alone is unconfirmed, nothing of the entry is left there, and the
emptied folders are removed as usual.
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
the job's immutable full/quick policy before source removal. A Move from another drive
needs a destination that keeps a replaced file recoverable (`recoverable_replacement`:
an NTFS folder on a fixed drive on Windows, ext2/3/4 on Linux); elsewhere, a network
location included, the job refuses it ("Cross-volume Move requires a qualified
verified-copy destination; the source was retained."). The dialog (F6, a pasted Cut) and
a drag refuse it before any job starts, from the destination's facts
(`TransferLinkDestination::refuses_cross_volume_move`, the same volume check) and the
selection's drive: "Items from another drive cannot be moved to a network location: Move
removes the originals only after verifying their copies, which it cannot do there. Copy
them instead." (or "to this drive (exFAT)"). A Move on the destination's own drive
renames and is not refused; where the facts are unknown, the job decides. Native permission
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

## Selection sizes

A file list on the clipboard or in a drag and drop holds any number of paths. Its names
are bounded instead, at most 4 MiB (`MAX_FILE_CLIPBOARD_BYTES`), which bounds the memory
that a payload from another program can take. CF_HDROP and URI lists have no item limit,
and since 21 November neither have the decoders, Ctrl+C, a drag to another application
or the panel selection resolved for them (`file_targets::resolve_payload`); lists of
more than 4,096 paths were refused before.

One Copy, Move, Paste or drop of native items takes at most 4,096 of them
(`MAX_NATIVE_TRANSFER_ROOTS`). Preparation opens every selected item and its parent
folder (two handles; on Linux, two descriptors of the open-file limit), and the job keeps
each one open and pinned until it is copied. A larger request is refused before anything
is opened, saying why: F5/F6 and Paste (before any Move confirmation) with a message, a
drop from another application with "no effect" and its reason, a drag between the panels
with the no-drop cursor, and an inline-tree Copy or Move as unavailable. Archive members
hold no native handle: a flat copy-out of archive members takes a selection of any size,
and an inline-tree one every member its capture takes (`prepare_captured` counts only
native roots). Content-only Move keeps its own limit of 64 roots.

Inline-tree captures take every row their projection can select, within the projection's
own bounds: 10,000 rows and 16 MiB of newly owned target memory
(`TreeProjection::MAX_ROWS` and `MAX_BYTES`; 4,096 targets and 4 MiB until 21 November).
Each consumer bounds what it holds per item: an inline-tree Delete pins at most 4,096
roots before its confirmation; checksums, Custom Commands, Properties and system menus
take 4,096 items; metadata edits take 256.

## Windows namespace pins

The Windows adapter reports exact lookup keys only on NTFS volumes whose
persistent 8.3-name creation is disabled, for ASCII names in case-insensitive
directories and any names in case-sensitive directories. Every other binding is
`Unknown`, which sends that turn to the host's opaque lane and its full refresh of
the job's pins. Such a job runs alone (on the system drive, which usually still
creates 8.3 names, every Copy into it and every Move from or into it), and a job that
waits for it shows `UnconfirmedFolders`, not `SameFolders`. A source that a Copy only
reads stays unclaimed instead, so it does not make the Copy run alone.
Original pins (selected sources and destination entries or
subtrees) and their directory anchor handles are retained for the job's lifetime;
an external change to one of their bindings is a job namespace error. Private pins
witness the stages a job creates and then their published outputs; once every native
owner of such an object has closed, the next refresh retires that alias, so published
outputs neither accumulate claims nor turn later external changes into job errors.
Their names stay covered by the original destination pins. By design, a retired
output's object identity no longer takes part in peer admission: another job that
reaches the same object through a different hard-link name is not excluded by it,
because this job never touches that object again.

Before a change the adapter checks the folder it changes: each held ancestor's
identity, each ancestor's name in its parent, the volume root's path and the folder's
case and 8.3 rules; a pin's refresh checks the same for the folder of each of its
bindings, and that the binding's recorded path still opens its folder. These checks
open one handle per ancestor level, so the adapter makes each of them once per refresh
batch (`TransferNamespaceRefreshBatch`, `transfer_namespace_parent.rs`), as the Linux
adapter walks an ancestry once per batch: the binding checks and the announcement of a
change up to its native operation are one batch, and the host's refresh of the pins
the change touched is another. A batch changes no names; an external rename during a
batch is seen by the next batch, as by a check that ran just before the rename, and a
check that fails is never kept. A copied file's stage creation and publication thus
check the destination's ancestry four times instead of eleven (44 instead of 121
ancestor opens for a folder 12 levels deep; WIN-8 in the
[Windows acceptance record](acceptance/windows.md)).

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

## Windows sharing while a job waits and runs

A job prepares its selected items, to plan its namespace claims, before admission grants
it, and keeps what it prepared while it waits for granted jobs that may still change those
items and the folders around them. Windows checks the sharing of every open handle
(measured on NTFS, Windows 11, 2026-11-21):

- While any handle on a folder denies write sharing, every rename into that folder fails
  with `ERROR_SHARING_VIOLATION` (32): another job publishing a copied file, a recovery
  rename, a program saving by rename (`MoveFileEx`), even a rename within the folder,
  because a rename opens its target folder for writing. Creating and deleting entries in
  the folder are not refused.
- While a handle denies delete sharing, its entry cannot be renamed or removed.
- Handles with only attribute access (`FILE_READ_ATTRIBUTES`, `SYNCHRONIZE`) take no part
  in these checks; traverse and listing access do.
- Whatever the sharing, a folder cannot be renamed while any entry below it, at any depth,
  is open (`ERROR_ACCESS_DENIED`, 5).

So a prepared Copy item holds no handle of its own until the job first lists, discovers or
reads it or pins its modified time (`transfer_source_root.rs`): only its folder's
attribute handle, which shares everything, and the identity, kind, metadata and file
revision it observed through an attribute-only handle. That first use opens the guarded
source on the same name in the held folder, and refuses another object, a link put in its
place, or another revision of a file (`source identity changed`, `source content
changed`). When the job reaches the item, it opens the source
(`TransferSource::open_for_reading`) before it creates the item's stage or folder in the
destination, whether the item was selected in a panel or in the tree (the engine's
wrappers of a Copy source forward the call: the shared and planned roots and the tree's
leased roots): an item that another program holds without sharing fails (`os error
32`) with its Retry question while the destination has nothing of it, and Retry opens it
again. A folder is opened for listing at that point too, before its destination folder
is created. A cloud placeholder is therefore opened through its filter, and may hydrate,
only once the job reaches it; a filter that cannot supply it fails that item, not the
preparation. A directory-link item keeps its guarded capture from preparation on.

Where lookups are qualified, a selected item is planned through its namespace pin
(`copy_planned_source.rs`) and prepared again when the job starts; it must still be the
planned item, or the job reports "Planned native source changed". A file must keep its
whole revision; a folder only its identity, kind and path, since its modification time
(and a size, where a platform reports one) changes whenever an entry is added to it or
removed from it, as when the job it waited for publishes into it. What the folder holds
is checked entry by entry when it is listed. Before 21 November a waiting Copy of a
folder that the busy job wrote into ended Failed on such volumes and copied it where
lookups are unqualified.

While a job copies, discovers or moves a folder, and while it follows a directory link into
a target folder, it holds that folder with read and write sharing: the folder's own name
stays fixed (delete sharing is denied), and other jobs and programs can still create,
delete and rename entries in it. Denying write sharing would not freeze the listing anyway,
since creation and deletion stay allowed; each child is checked against the listing and
guarded by its own handle. A file being read and verified still denies write and delete
sharing until it is done, so its content and name cannot change, and a program that
already writes it makes its open fail (`os error 32`) instead.

Before this, a Copy waiting to read a busy job's output held that folder with read-only
sharing while it was prepared, so the busy job failed to publish its next file there (CI run
44841450903, `reading_the_busy_copys_output_waits_for_it_with_its_reason`); where lookups
are unqualified (8.3 names enabled) the waiting Copy kept that handle for its whole wait.
`transfer::source::sharing_tests` and
`immediate_start::the_busy_copy_publishes_into_output_that_a_waiting_copy_prepared` cover
it.

Still open: a prepared Move item takes its guard at once (DELETE access, delete sharing
denied on the item, and traverse access with delete sharing denied on its folder). Other
jobs can publish into a moved folder, but a granted job that renames or removes that item
or its folder meanwhile fails with a sharing violation; where two Moves meet there,
whichever opens second fails instead of waiting. And the handles that every waiting job
keeps on the folders of its items (its namespace pins and each prepared item's folder,
all sharing everything) still make a granted Move of an enclosing folder fail with
`ERROR_ACCESS_DENIED` after its one-second retry.

Linux has no sharing modes: the descriptors a waiting job keeps refuse no change of
another job, and there too a job only opens and inspects its items before the grant
(discovery and reading start after it).

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
instead of running side by side (the waiting one shows `UnconfirmedFolders`), and Copy
and Move still work.

A key lists the identity and mount of its folder and of each ancestor up to the
mount's root, read with one `statx` per level. The engine refreshes the pins that one
change touches as one refresh batch (`TransferNamespaceRefreshBatch` in
`midnight-platform-common`, a guard of the refreshing thread), and the announcement
of a change reserves its pins in one. A batch makes no namespace change, so the Linux
adapter walks the ancestry of a folder that several pins of the batch share once (it
remembers up to 64 folders per batch). An external rename of an ancestor during a
batch is seen by the next batch, as by a walk that ran just before the rename.

The Linux namespace unit tests create their fixtures in the temporary folder
(`TMPDIR`), which must therefore lie on a qualified mount; below a bind-mounted folder,
such as a container's `_build`, they stop with "fixture requires qualified local byte
lookup".

## Links in Copy and Move

`CopyLinkPolicy` is frozen when a job is admitted. The Copy/Move dialog sends it through
`Command::EnqueueCopyWithOptions`. Its **Links** choice is the same on Windows and Linux
and for Copy and Move (`crates/midnight-ui/src/transfer_link_option.rs`):

| Dialog | Policy | Copy | Move |
| --- | --- | --- | --- |
| Copy the files links point to | `FollowLinks` | a followed link becomes the file or folder it points to | the same copy, verified like any copy, then only the link is removed |
| Keep links as links | `Preserve` | each link is recreated with its exact target | each link is moved itself |

`FollowDirectoryContents` and `EmptyDirectories` remain for their existing callers; the
dialog no longer sends them.

### The default and why

When the dialog opens it asks what its destination does with links
(`Command::InspectTransferLinks`, answered by `Event::TransferLinksInspected` from
`TransferService::link_destination` on a backend worker); the check of a typed folder
(`Command::CheckTransferDestination` with `links`) answers the same for that folder. Only
what the volume reports is read; nothing is written to find out:

- Windows (`transfer_link_destination.rs`, `storage_kind.rs`): `GetVolumeInformationByHandleW`
  on the folder (`FILE_SUPPORTS_REPARSE_POINTS`, the filesystem's name, the volume serial
  number); `GetDriveTypeW` (removable, network; a UNC path is a network location); for a
  fixed drive, the storage descriptor of its disk (`IOCTL_STORAGE_QUERY_PROPERTY`,
  `StorageDeviceProperty`): the USB, SD or MMC bus, or removable media. No link is created
  at a network location: a junction's target would be resolved on the server and a
  symbolic link's by each client's evaluation policy.
- Linux (`transfer_link_destination.rs`): the folder's `statfs` type (the FAT family and
  exFAT cannot store links; ext2/3/4, XFS, Btrfs, tmpfs, F2FS, ZFS, bcachefs, ntfs3 and
  NFS can; anything else, SMB and FUSE included, is unknown and keeps links by default),
  and the sysfs entry of its block device (`removable`, an SD card's `device/type`, a
  `usbN` bus on its path; device-mapper and md devices through their slaves).

`TransferLinkFacts::copy_files_by_default` copies the files for a destination that cannot
store links, a network location (Windows) and a removable drive, unless the selection's
folder is on that same removable drive; elsewhere links stay links. Decision: within one
removable drive the links already live on it, and following them would duplicate data and
turn a Move's rename into a copy. The dialog (`presenter/copy_dialog_links.rs`) shows the
reason under the choice ("This drive cannot store links (FAT32), …", "Removable drive: …",
"Network location: …"). An answer never changes a choice the user made. Keeping links
where none can be stored shows a warning under the choice and holds the first
confirmation once with that explanation; confirming again starts. The job then keeps the
dialog's promise, whatever started it (the dialog, a paste, a drag): it asks the
destination first (`TransferDirectory::creates_no_links`) and leaves each link out
without a question, names it in the job details ("… was left out: no links are created
at a network location."), and completes; the outcome counts the links left out
(`CopyProgress::links_left_out`, "2 links were left out: no links are created there;
Details names each."). A typed folder whose
default differs from the choice shown, while the user has not chosen, holds the
confirmation once and shows that folder's default and reason. A confirmation before the
answer arrives starts with the choice shown, which keeps links.

Copy and Move to FAT32 and exFAT are refused on both platforms by requirements older than
this choice (Linux stages need `O_TMPFILE`; Windows transfers need 128-bit file IDs), so
there the choice only decides what the dialog shows and says; on a removable NTFS, ext4
or other link-capable drive it decides what is written.

### Keep links as links (`Preserve`)

`Preserve` never opens a link's target. The source yields every link it meets, the
selected root and each link below a selected folder, as a link leaf (`EntryKind::Symlink`
or `DirectoryLink`) whose `link_representation` is exact: on Linux the `readlink` bytes
(at most 64 KiB, relative text kept verbatim), on Windows the substitute and print names
and the relative flag of a symbolic link or the names of a local junction, read with
`FSCTL_GET_REPARSE_POINT` through a no-follow handle. That read revalidates the parent
binding, the link's identity, revision and representation; an in-place change fails.
Discovery counts a leaf without following it. Windows keeps a symbolic link to any target
(a local drive, a relative path, a network share) and a junction to a local drive; a
volume mounted in a folder (`\??\Volume{...}\`) and other reparse points are refused with
that reason.

### Copy the files links point to (`FollowLinks`)

`FollowLinks` follows a directory link as `FollowDirectoryContents` does, under the same
volume, link-depth and ancestry rules, and presents a link to a regular file as that file:
its content is copied and verified like any file, and the binding of the link and its
target is revalidated, with the file's revision, before each use. A link that cannot be
followed is a leaf recreated as with `Preserve`, and `TransferSource::link_not_followed`
gives the reason, which the job records as a note (`CopyProgress::notes`, first 16, with
an exact `note_count`) shown in Jobs details.

On Linux a link is followed to a target on any filesystem but the kernel's own (below), on
the link's mount or another. The link's text is resolved as the kernel resolves it, never
through a magic link (such as `/proc/self/fd/0`). A text that stays on the mount of the
link's folder is resolved within it: a relative one by the kernel under
`RESOLVE_NO_XDEV`, and an absolute one, or one that meets an absolute link on another mount
than the root's, one name at a time from the mount's root when it lies under the mount's
path, with at most 40 links on the way (`MountAnchor::walk`). An absolute text is never
left to the kernel there: given an absolute path, it starts at `/` under `RESOLVE_NO_XDEV`
from whatever mount the folder is on. The folders below such a link are revalidated from
the same anchor as the link's folder, the root of that mount. A text that leaves the mount
(a mount point on the way, `..` above the mount's root, an absolute text outside the
mount's path, or any absolute text below a link that already left its mount) is resolved by
the kernel across mount points under `RESOLVE_NO_MAGICLINKS` alone, and that link becomes
the anchor of the folders below it (`Binding::reopen_below`): revalidating one resolves the
link again from its held folder, requires the same target (device and inode), and opens
the folders below it from there without leaving the target's mount. So with `/home` on its
own filesystem, a link there to `/home/user/folder` is followed within its mount, links to
`/media/stick/folder`, `/dev/shm/folder` or `/mnt/nas/share` across mounts, and one to
`/proc` or `/sys/kernel` not at all.

A target is never followed onto one of the kernel's interface filesystems, which its
`statfs` type (`f_type`, linux/magic.h, or the driver source for the USB gadget
filesystems) names: proc, sysfs, devpts, cgroup and cgroup2, debugfs, tracefs,
securityfs, apparmorfs, pstore, bpf, configfs, fusectl, mqueue, autofs, efivarfs,
binfmt_misc, selinuxfs, smackfs, nsfs, resctrl, binderfs, nfsd, rpc_pipefs, xenfs,
functionfs and gadgetfs, 27 in all (`SYSTEM_FILESYSTEMS` in
`transfer_directory_links.rs`). Their files describe the system and the copying process
itself (`/proc/self/environ` is this process's environment), a read can wait for ever
(tracefs's `trace_pipe`, a USB gadget's endpoint files) or change state (xenfs's
hypervisor interfaces), and following a link to an automount point would mount it. The
target is first located with `O_PATH` and not as a folder, which mounts nothing, so an
automount point that is not mounted yet is seen as autofs and kept; the folder that is
followed is then bound as a folder and its type checked again. This is a deny list rather
than an allow list because a link only chooses which folder Copy reads, not what kind of
filesystem: Copy reads any filesystem the user selects directly, and an allow list would
refuse data filesystems it does not name (ZFS, bcachefs, VirtualBox shared folders,
cluster and future filesystems). A new kernel interface filesystem missing from the list
would be followed, still reading only regular files with this process's permissions and
never a special file. devtmpfs reports the type of tmpfs and is not told apart: it holds
device nodes, which Copy never opens, and links, which follow these same rules. Network
filesystems (NFS, SMB, 9p such as WSL's `/mnt/c`) and FUSE are followed: they hold the
user's files, which Copy reads when they are selected directly too, and their device and
inode numbers identify a folder for the cycle check as on a local filesystem. An
unresponsive server or FUSE daemon can delay the job as when its folder is selected
(cancellation takes effect when the call returns), and automount points along the way of
a link's text are mounted as when the link is opened.

Linux keeps a link as a link, with its reason, when its target is missing, a loop of links
(for a text resolved across mounts or in a folder of /proc, also a magic link, which ends
the resolution the same way), unreadable, on a system filesystem (named in the note) or an
automount point that is not mounted, a special file, or a directory that is an active
ancestor. A target is unreadable when the user cannot read the file, or cannot list or
enter the folder: opening it as a path needs neither, so this is checked (`faccessat` with
the effective IDs, on the target the link was bound to) before the link is followed. A
link whose text needs the walk within its folder's mount (an absolute one, say) is also
kept when that mount cannot be qualified, for example a folder whose path leads through
another link rather than along the mount's own path. `FollowDirectoryContents` refuses
such a cycle as an error, and it and `EmptyDirectories` refuse a target on a system
filesystem as an error.

Windows (`transfer_source.rs`, `transfer_source_links.rs`) follows a junction or a
directory symbolic link to a folder, and a file symbolic link to a file, only where the
target is an ordinary folder or file on a local drive: an absolute target (`\??\D:\...`)
is walked from that drive's own volume root, which `QueryDosDeviceW` must map to a
`\Device\HarddiskVolumeN` opened through `\\?\GLOBALROOT`, with every component an
ordinary folder on that volume; a relative target is walked from its link's folder, never
above its volume's root (`transfer_source_link_target.rs`). A followed file is held for
reading as a copied file is (write and delete sharing denied) and must be an ordinary
file, not a link, cloud placeholder or deduplicated file. The cycle check compares volume
serial number and file ID, so it holds across volumes (`transfer::source::volume_tests`,
run with `NATIVE_MOVE_OTHER_VOLUME_ROOT`). Kept as links, with the reason, are a link
whose target is missing or unreadable, a remote target (a UNC path or a mapped network
drive), a volume mounted in a folder, a reparse point anywhere on a target's way, and a
folder that contains the link. Decision: these refusals stay (they are what the earlier
`FollowDirectoryContents` refused as errors); under `FollowLinks` such a link is kept as a
link instead of failing the job, which this design needs and which is safe because a kept
link is never opened through. Linux follows a link to a network filesystem and resolves
links and mount points on a target's way; on Windows each of these still needs its own
qualification.

Only links followed inside one another count against the budget of 128, wherever they
lead: Copy has no fixed depth limit for ordinary folders under any policy (Move keeps its
own limit of 128 folders and 100,000 entries; on Linux the folders that transfers hold
open, a followed one on another filesystem too, count against a
[descriptor budget](#folders-held-open-on-linux)), and `Preserve` and `EmptyDirectories`
keep no ancestry at all. A followed link back to a folder on the current path is a cycle,
compared by device and inode (Windows: volume serial and file ID) across filesystems.
Recursive totals (discovery) still stop after 128 nested folders, as without a link
policy; the job then copies without them.

### Recreating a link

Copy recreates a link (engine `LinkCopy`): the destination plans it
(`TransferDirectory::plan_link`, after `admit_link`), then a private stage, sync, an
independent readback that must equal the planned representation, a revalidation of source
and stage, then publication. A link the destination cannot create is reported when the job
reaches it, with Skip and Skip all, and the other selected items are still copied (before
this, a selected link that a destination refused failed the whole job's preparation).

- Linux: the stage is `item` in a new owner-only `.midnight-link-<random>` directory
  beneath the protected destination folder (`symlinkat` relative to the held directory),
  so no link exists under a final name before it is verified. Publication renames it to
  the final name with `RENAME_NOREPLACE` (a filesystem that cannot exclude replacement
  refuses it) and removes the empty stage; the published link's identity and text are
  checked again. A FAT or exFAT folder refuses before anything is created ("This drive
  cannot store links (FAT)…"); a filesystem that refuses `symlinkat` with `EPERM` or
  `EOPNOTSUPP` says it cannot store symbolic links.
- Windows (`transfer_link_stage.rs`): the stage is a private `.me-<16 hex>` entry in the
  destination folder, held through a no-follow handle with DELETE access, read back from
  that handle and published by a no-replace rename, as a copied file is. A junction is
  created as an empty folder with mount-point reparse data (`FSCTL_SET_REPARSE_POINT`,
  exact substitute and print names); that needs no privilege, and only an absolute path on
  a local drive can be its target. A symbolic link is created with `CreateSymbolicLinkW`
  and `SYMBOLIC_LINK_FLAG_ALLOW_UNPRIVILEGED_CREATE`, which succeeds in Developer Mode or
  with the symbolic-link privilege (an elevated process holds it); its target text is the
  link's own (relative stays relative, `\??\C:\x` becomes `C:\x`, `\??\UNC\s\x` becomes
  `\\s\x`), and its print name is that text. Whether symbolic links may be created is read
  from the Developer Mode setting (`AppModelUnlock\AllowDevelopmentWithoutDevLicense`) and
  the process token's privileges, never by creating one to find out. Without either, a
  directory symbolic link to a local folder is planned as a junction to the same folder (a
  relative target resolved from the destination folder, never above its drive), and the
  job details say "… was created as a junction to D:\x, because creating a symbolic link
  needs Developer Mode or administrator rights" (and that a relative target became
  absolute); a file symbolic link, or one to a network share, is reported with that reason
  and left out. A volume without reparse points (FAT32, exFAT) or a network location
  creates no links (`creates_no_links`): the transfer leaves each link out without a
  question and names it; `plan_link` refuses one only as a guard for other callers. Linux
  does the same for FAT and exFAT. What planning reads from the system is read once
  per job and kept for its folders (`LinkFacts`, held by the folder the job starts from
  and shared with every folder it creates or merges into): whether symbolic links may be
  created, and each destination volume's facts (network location, reparse points,
  filesystem name), keyed by the volume's serial number. Decision: the `midnight-elevation` helper is not
  used to create links; it is a one-shot elevated directory reader (and the AppArmor
  installer on Linux), and letting an elevated process create links at destinations the
  unelevated application chooses would widen its authority beyond reading.

An existing name is an ordinary conflict: Skip, another name, or Replace when the
destination declares `TransferCapabilities::link_replacement` (Linux local ext, as for
files) and the target is a file or a link, never a folder. Replace binds the target pin
(`StagedTransferLink::prepare_replacement`), moves the staged link into a new
`.midnight-recovery-<random>/item` and exchanges it with the target there
(`RENAME_EXCHANGE`), so the displaced entry stays in recovery, with the identity and
revision checks of file replacement. Windows publishes links without replacement only
(`link_replacement` is false): Skip or another name. A link counts as a completed file and
never as content bytes.

### Move

`Preserve`: Move moves the link itself. Preparation returns an `Ordinary` entry whose
capture is the link's own. On one filesystem it renames the link (Linux through the private
`.midnight-move-*` capture used for directory links; Windows through the moved directory
link's capture, `transfer_move_link.rs`, which also opens a file symbolic link). Across
filesystems the capture recreates the link at the destination (the plan, stage, readback
and publication above), checks the published link, and only then removes the original
link with the protected removal of a moved directory link (owned link with one name on
Linux, staged, revalidated, unlinked; on Windows its only DELETE handle, revalidated, then
the delete disposition and its confirmation). If that removal fails, the outcome is
`RecoveryRequired` with the published destination, so the job reports the error and both
links remain. Its `try_native_move` therefore never reports `CrossDevice`; the engine has
no content fallback for a link and reports one that a provider cannot move. Folders under
a cross-volume Move are traversed with the same policy, so their links move as links. A
folder that a Move renames whole keeps its links exactly; they are not examined, and a
relative one resolves from the folder's new place.

`FollowLinks`: a link that the source follows, to a folder or to a regular file, is a
`DirectoryLink` entry whose source reads the target at the link's own path and whose
capture holds only the link (`PreparedTransferMoveEntry::DirectoryLink`; the engine
accepts a file projection under this policy). The engine never renames such a link: the
target's content is copied and verified like any copy (a file by `VerifiedFileCopy`, a
folder entry by entry), and then `remove_original` removes the link alone, a file link
right after its copy is published (`Job::remove_copied_link`), a folder link once its
whole branch completed without a failure or skip. What the link pointed to is never moved,
removed or rewritten: it was not selected. A removal that fails leaves the link and the
verified copy where they are and asks whether to go on (Skip, Skip all, Cancel; there is
nothing to retry). A link that cannot be followed is moved itself, as under `Preserve`,
and the job details say why. A folder is never renamed whole under `FollowLinks`, so that
the links in it are followed too: the engine creates it at the destination, moves its
entries one by one (a file by its own rename where it can) and removes it once empty, as
for a merge (`Job::copies_instead_of_renaming`); a folder on another volume takes the
cross-volume route that its failed rename would have taken. The folder's removal follows
its links' outcomes: a link kept in it (skipped, failed, or not removed) keeps the folder.
Not handled: a link inside the moved folder whose target lies in the same folder and was
already moved by then is no longer found, and is moved as a link with "its target does not
exist" in the job details.

The Windows engine tests of both Moves to another volume (a junction recreated there,
and a folder whose junction is followed; `copy_link_windows_tests.rs`) need
NATIVE_MOVE_OTHER_VOLUME_ROOT, an owned folder on another local volume than the
temporary folder, and print SKIP without it, as the provider's cross-volume tests do.

### Notes in the job details

A link handled other than as the choice says is named with its reason, never with raw
system error text: "… was copied (moved) as a link because …" (why `FollowLinks` did not
follow it); "… was created as a junction to …" (a Windows plan, for Copy and for a Move
across volumes, `TransferLinkPlan::note`); and "… keeps its relative target “…”, which
leads outside the copied (moved) items; at the destination it points somewhere else or
nowhere" for a link kept as a link whose relative text, resolved name by name from its
folder, climbs above the selected item that holds it (for a selected link, above the
folder it is in; `TransferLinkRepresentation::relative_text_leaves`). Only the text decides,
never what exists.

An item that disappears from the source while a job copies or moves it, or before the job
reaches it, is reported as "removed from the source while copying" ("…while moving")
instead of the system's text for a missing file (`decisions::vanished_source`, applied to
every question about a source and to a selected item's preparation); a missing destination
keeps its own error.

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
publication (durability unconfirmed or recovery required) asks whether to go on
(Skip, Skip all errors, Cancel), never Retry.
Recursive directory Move uses the per-file proof and postorder empty-directory
removal contract above. It does not infer descendant verification from root
metadata. Skips retain ancestors; failures/cancellation preserve unfinished
sources and earlier completed moves. Remaining directory metadata preservation
needs its own declared postorder capability. Physical storage reclamation and
atomic tree snapshots remain outside the contract.

Planned, not implemented: native selections larger than the 4,096 roots of
[selection sizes](#selection-sizes) need roots that are opened one at a time. A job would
keep the selected paths (or the captured selection) and only the identities of roots it
has not opened yet, open each root just before it copies it and check that identity, and
total its discovery root by root, so that open handles follow folder depth instead of
selection size. The protection of later roots from earlier copies (`protected_roots`),
duplicate and ancestor checks and the root count shown in progress then rely on those
identities, captured in one pass that closes each root again.
