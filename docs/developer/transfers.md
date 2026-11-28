# Native transfer service

These are internal Rust contracts of `midnight-core` (`src/transfer*.rs`, re-exported at
the crate root), implemented by the separate Windows and Linux adapters
(`midnight-platform-windows`, `midnight-platform-linux`); the shared copy verification is in
`midnight-platform-common`. They are not Rust trait objects exposed across a plugin ABI. Archive extraction supplies a brokered `TransferSource` and
uses the same native destination and verification state machine.

## Verification and ownership

`VerifiedFileCopy` works in bounded steps through one 256 KiB buffer: source read/stage
write, flush/sync, independent stage readback, then publication. The immutable job policy
(`TransferVerification`) defaults to full SHA-256 and exact length (`FullSha256`). Explicit `SizeAndSamples` compares
exact length and first/middle/last 64 KiB (all bytes for files up to 192 KiB), retaining
at most 192 KiB of source evidence gathered during streaming. It cannot detect
corruption between samples. It revalidates the opened source
before accepting replacement consent and immediately before publishing. A failed
copy stays stopped until an explicitly supported Retry restores its checkpoint. The
one automatic retry repeats a namespace change (stage, directory or link creation,
publication, a Move's source capture, source removal or empty-directory removal) that
granted peers refused before any mutation, once a refusing peer changes; see
[operation progress](operation-progress.md#admission-policy-and-waits). Cancelled
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
configured disk-I/O limit ([Disk operations](disk-operations.md); Automatic is four,
`DiskIoConcurrency::AUTOMATIC_LIMIT`) interleaves their native steps. Where their native namespace footprints overlap,
only the overlapping item waits for the job that already holds it; a job that would
read what another job changes waits as a whole. A job whose footprint is unknown
(the opaque lane) runs alone: it waits for the other jobs and they wait for it, saying
"Waiting for another job: jobs whose folders cannot be confirmed run alone" rather than
that they use the same folders. Each
job performs one native call at a time and holds its disk slot only while it
executes native steps. The copy actor runs a job's steps on a blocking worker in calls
of at most 5 ms (`STEP_BUDGET`, `Job::steps` in `copy_worker.rs`), so a small file's
quick steps take one hop; each call returns at every boundary the actor acts on, and the
actor checks pause, Cancel and the disk slot between calls. An aborted actor ends
each running call after its step in flight
([engine lifetime](engine-lifetime.md)); see
[operation progress](operation-progress.md#admission-policy-and-waits). A job never
copies in parallel streams: the
[local concurrency experiment](../verification/2026-11-10-ssd-copy-probe.md) measured
substantial gains for large and mixed files on local SSDs, but unexplained small-file
delays left it inconclusive (IO-01 in the [backlog](../implementation-backlog.md)).

Windows stages use a twenty-character basename, `.me-` plus sixteen random hex
characters. Exclusive creation is relative to a held directory handle, without a
260-character `MAX_PATH` restriction. Linux stages remain unnamed `O_TMPFILE`
inodes. Both stage on the destination filesystem. The final name is assigned only
after verification; same-volume native Move does not stage content.

A Copy's Replace keeps no metadata of the file it replaces, so its attributes never stop
it: a compressed, read-only or hidden file is replaced like any other, and the copy is
compressed when its folder compresses new files. Generated outputs that replace a file
(Shift+F4's empty file, a checksum manifest, a file list) keep the replaced file's
metadata instead, with the rules in [Make File List](make-file-list.md#output-lifetime-and-safety).

On local fixed NTFS a Copy job (`TransferDirectory::stage_job_file`) keeps the stages of
each destination folder in one private staging folder of the job, `.midnight-stage-` plus
32 random hex characters, in that folder (`transfer_destination_staging.rs`). The job
creates it as it creates a `.midnight-recovery-*` directory, owner-only and held with
DELETE and without delete sharing, in one namespace turn before its first stage there;
it is not hidden, as the stages are not. Its pin is reserved before it exists and covers
everything below it, so each stage is created in it through its handle outside any turn,
and each publication is one turn: a handle-relative no-replace rename out of it into the
destination folder. A destination folder that refuses Midnight its security, new folders
or new files gets no staging folder: each file there is staged as a `.me-*` entry of its
own beside its final name, so a write-only drop folder still receives its files, and a
folder that refuses new files refuses each file before it is copied. Any other failure to
create the staging folder (a quota, a cancellation) fails that file's stage. A paused job keeps its staging folders and stages in them
again when resumed. Outside a job (checksum, file-list and other single outputs),
elsewhere than local fixed NTFS and for required-metadata stages, each stage is a
`.me-*` entry of its own in the destination folder, created in a turn of its own. A Move
from another drive stages its files as Copy does.

The staging folder's DACL is not inheritable, so each stage is created with the
descriptor that a file created directly in the destination folder gets, computed from
that folder's current owner, group and DACL (`CreatePrivateObjectSecurityEx` with the
process token; `transfer_destination_security.rs`). As NTFS does, the entries are marked
inherited only when the folder's DACL is auto-inherited, and then the stage's DACL is set
once more with `SE_DACL_AUTO_INHERIT_REQ`. The kernel gives the stage its SACL and
mandatory label from the staging folder, which inherited them from the destination
folder. The descriptor is computed for each stage, so an ACL changed during the job
applies to the files staged after the change; the owner, group, DACL and label of a
published file equal those of a file created in the destination folder
(`transfer::destination::staging_tests`). Known limits: an inheritable SACL entry marked
`NO_PROPAGATE_INHERIT_ACE` (an audit entry or a mandatory label) reaches the staging
folder but not the files staged in it; and a file still in the staging folder while the
destination folder's ACL is propagated keeps the inherited entries from before until a
later propagation reaches it (one file's copy).

Cancellation waits for any in-flight native step and explicitly removes the
unpublished stage before terminal progress. Factory failures after stage creation
also attempt explicit cleanup. Unfinished sources, existing target originals and
already completed files are retained. A cleanup refusal is reported with its exact
residual path, even when the job is Cancelled; destructor cleanup alone does not
prove removal. The job removes each staging folder through its handle in one turn once it
stages nothing more in that folder (after the folder's copy, or when the folder is
skipped) and at its end: the kernel refuses one that still holds an entry, such as another
program's file, and the job keeps it, never lists or empties it, and reports it with its
exact path. A staging folder whose removal the job's namespace coordination refuses is
kept by the job and removed by a later attempt (the end of another folder's copy, the
job's end or the job's last folder going away); each one still refused at the job's end
is reported by its path, in one error of the job. See [staging and cancellation](../superpowers/specs/2026-11-10-transfer-verification-and-native-move.md#short-staging-names-and-cancellation).

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
| `link_replacement` | A staged link may replace a consented file or link under the recoverable policy (`StagedTransferLink::publish_replace`); see [recreating a link](#recreating-a-link). |

Default optional methods return Unsupported. Windows advertises recoverable
replacement only for NTFS on a fixed drive (`transfer_recovery::supported_volume`), and
no `link_replacement`; Linux advertises both only for ext2, ext3 and ext4 (`statfs` type
`0xEF53`, `transfer_recovery::supported_type`). Network, removable and unknown replacement semantics are not
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
editor's case change repeat such a refused folder rename for up to one second, the waits
doubling from 10 ms: `RENAME_RETRY_BUDGET` in `mutation_native.rs`.)
`native_entries_moved` and items/s express this work separately from copied bytes.
Committed errors retain their recovery facts and cannot replay a source transfer.
On Linux a published native Move removes its empty stage first and then syncs the
source and target folders once each (two folder fsyncs, one ext4 journal commit). A
stage that cannot be removed is synced with both folders and the Move reports it as
needing recovery; a replacement's stage, which keeps the displaced entry as recovery
data, is synced with both folders.
The reasoning is in the
[policy and API design](../superpowers/specs/2026-11-10-transfer-verification-and-native-move.md).
Full-hash proofs below describe the default mode; sampled proofs keep the same identity,
revision and publication guards with bounded range reads.

### Move durability on Windows

A Move reports a change as unconfirmed only when a step it takes to save that change to
disk fails, never merely because a platform has no such step. The provider says so in
`durability_confirmed` of `TransferNativeMoveOutcome::Moved` (and of the source
removals below); the engine counts each unconfirmed one in
`CopyProgress::source_removal_durability_unconfirmed`, which gives the job a
`CopyClosureReceipt`, so it ends "Completed with warnings" with "Source changes the system could not confirm as
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
three folder handles; `MAX_PENDING_CONFIRMATIONS` and `confirm_pending` in
`move_worker.rs`) and when the job ends, whatever its outcome. A Move of N entries
from one folder to another flushes the two folders once instead of N times each, and a
merge flushes a source folder and its destination once for all its entries before the
emptied folder is removed. Rename's replacement (`try_native_move`) flushes before it
returns. A folder's result counts for every rename of the round that
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

What that confirms depends on the filesystem; only NTFS was measured:

- NTFS: right after the destination folder's flush, the source folder's flush costs
  about as much as flushing an unchanged folder, which suggests that the first flush
  already writes NTFS's log for the whole rename. Saved together, the renames of a Move
  between two folders flush each folder once per round instead of once per rename. On a
  disk whose cache flush is slow a Move waits for that flush once per folder and round,
  and once per removed file for its stage, as a Linux Move waits for its folder fsyncs.
- FAT and exFAT (not measured): the FAT
  driver whose source Microsoft publishes (FastFAT) accepts a flush of a folder other
  than the root without writing its entries, so such a Move counts as confirmed
  without the Move writing them. Whether they survive a power loss depends on the
  drive's write caching: with Quick removal, the default removal policy for external
  drives since Windows 10 version 1809, Windows does not cache their writes.
- ReFS, including Dev Drives (not measured), and network shares (the flush is the file
  server's): a flush refused as not supported counts as above.

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
  filesystem without POSIX delete refuses the removal itself; a cross-volume
  file removal requires local fixed NTFS anyway.

The contract (`transfer_move.rs` in `midnight-core`): a `TransferDurabilityConfirmation` completes a
committed change later and says whether it is saved (for a removal, whether its name is
gone too); `confirm` takes the round, which saves each folder at most once. The
finalizer's and the capture's `take_removal_confirmation` and the two capture traits'
`try_native_move_deferred` hand one over; their defaults hand none over and
`try_native_move_deferred` defaults to `try_native_move`. The engine counts a change that
has a confirmation by the confirmation alone. Linux hands none over; its native Move syncs
its folders itself, as described above.

## Change-time drift (VIEW-05)

Native revisions include the change time (Windows ChangeTime, Linux ctime). It also
moves on metadata-only updates: on Windows, endpoint protection software was seen
moving it on freshly written files a second or two after they were written, with the
file ID, length and LastWriteTime unchanged. Copy and Move compare
their pinned revisions under `TransferRevisionPolicy::AcceptChangeTimeOnly`
(`TRANSFER_REVISIONS` in the engine): a revision whose change time alone moved is
unchanged, and every other captured field still counts.

| Check | Copy and Move | Compared fields under that policy |
| --- | --- | --- |
| Conflict pin (`prepare_target_with`), including recoverable replacement and native Move replacement | `AcceptChangeTimeOnly` | Identity; Windows length, LastWriteTime, creation time and attributes; Linux length, mtime and mode |
| Move capture of the source (its own binding pin) and Move children | `AcceptChangeTimeOnly` (`prepare_move_source_with_policy`) | As a conflict pin |
| Revalidation of Copy sources (`prepare_transfer_source_with_policy`), Move sources and their children | Linux `AcceptChangeTimeOnly`; Windows does not compare ChangeTime here (`transfer_source.rs`) | Identity; length and modification time (Windows also creation time) |
| Verification read guards, modified-time pins, planned-source namespace pins, Move finalization | Always `AcceptChangeTimeOnly`; only Copy and Move use them | As a conflict pin; Linux namespace pins compare length and mtime |
| The archive file of archive members copied out, at each worker exchange, before a folder's listing and before each Retry ([archive members as a source](#archive-members-as-a-source)) | `AcceptChangeTimeOnly`, like every capture of an archive | Identity, length and modification time (Windows LastWriteTime, Linux mtime) |

Every other consumer of the same services keeps exact revisions: `prepare_target`,
`prepare_transfer_source`, `prepare_transfer_source_with_links`, `prepare_move_source`
and `prepare_move_source_with_links` compare every captured field, so user-command scope
pins, checksum manifest and file-list outputs, checksum sources, Rename (including its
replacement capture) and drag previews treat a moved change time as a change. The
policy-taking methods default to their exact counterparts for providers without native
change times.

A same-length rewrite that restores the modification time differs from the pinned
revision only in its change time, so these Copy and Move checks do not detect it.
On Windows, a Copy source denies write sharing from when its job opens it to read it,
before the copy is staged (before that, while its job waits, a write is detected only
through these fields; see
[Windows sharing](#windows-sharing-while-a-job-waits-and-runs)) and a Move source from
its preparation; Linux Move removal keeps its read lease; a replaced conflict target
stays in recovery.
Directory-link stubs keep exact revisions.

### Archive members as a source

A Copy out of an archive reads members through the archive's mount
(`archive_transfer.rs`). The filesystem host validates the captured archive file as each
exchange with the archive worker begins and around each read of the file's bytes that the
worker asks for (`session.rs`); the copy validates it once more before a folder's single
listing and before a Retry resumes a member. Between those, a member's copy steps check
only for Stop. Like every capture of an archive file, the validation accepts a change time
that alone moved (VIEW-05: endpoint protection software can add an alternate data stream
to an archive long after it was written, which moves only the change time; see
[content reads](content-reads.md#change-time-drift-view-05)); identity, length and
modification time still count, and the plugin opens only a regular file without
following links.

Progress names each member as Copy names a native file: by its path, made of the
archive's display path (`ArchiveLocation::display_text`, which names every enclosing
archive of an archive inside an archive), the folders inside it and the member's own
name, for example `D:\data\backup.zip\docs\readme.txt` (`TransferSource::display_path`,
which wrappers forward). This path is presentation only: the source's entry keeps the
member's exact name, which alone names the destination, and members are read by their
mount and member IDs. Before the first member is written, the job counts
the files, folders and bytes it will copy (`TransferSource::discover` of the archive
source), so the progress bar is determinate and the estimate has a byte budget, as for
a native Copy. The count comes from the archive's index, which its worker built when
the archive was opened: ZIP and 7z record each member's length in their directory and
headers, TAR (compressed or not) in the headers indexed then, and a compressed single
file (gz, xz, bz2, zst) its decoded length, so counting decodes no member and reads no
member data. A selected file counts without any request to the worker; a selected
folder is listed once for the count, folder by folder, as Copy then lists it again,
and only the folders still to be counted are kept meanwhile. Stop and Pause end the
count between listings and within a long listing. A folder that cannot be listed for
the count, a member the copy then skips (for example one whose password was not
entered) or a count that differs from what was copied leaves the totals approximate
("~"), as for a native source that changed, never unknown.

A lasting change, found when a reader that the plugin opens on the archive's path reports
another identity, length or modification time, fails as `InvalidData`
(`source_failure` in the filesystem host's `session.rs`), and the copy offers Skip, Skip
all and Cancel but no Retry, as for a changed native source. Other source failures, such
as an unreachable network file, offer Retry, which succeeds once the file is back when a
validation between worker exchanges failed: the mount survives that failure.

A failure inside an exchange with the archive worker (a source read the host could not
complete, a member the worker could not read, a protocol violation) stops that worker,
and so does the 20-second idle watchdog (`DEADLINE` in `session.rs`) or an exhausted
exchange budget (`session_budget.rs`). The mount then
never serves another request: each one fails with "Archive worker was stopped after a
failed request (…); open the archive again" or "Archive worker made no progress for 20
seconds and was stopped; open the archive again", as `InvalidData` (`STOPPED`), so the
question offers Skip, Skip all and Cancel only. The failure that stopped the worker keeps
its own kind and message, such as a full spool volume's, so its own question can still
offer Retry once; that Retry then reports the stopped worker. Known limit: a Retry does
not open the archive again for the copy (ARCH-08 in the
[backlog](../implementation-backlog.md)).

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
exceptions, the job's state is Completed, shown as "Completed with warnings"
(`job_presentation.rs`), and its outcome counts the skipped items ("1 skipped item was
not copied."). An I/O error the user skips is still an error: the job
ends Completed with errors, or Failed when nothing was transferred.

## I/O errors and progress lifetime

`CopyEvent::Failure` carries a `CopyFailure` with operation ID, monotonically
increasing decision version, `CopyErrorStage`, native error and separate source
and destination paths. `can_retry` and `can_skip` are capabilities, not UI guesses.
Decisions use `Command::DecideCopy`. A stale version, wrong job
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
([audit](operation-progress.md)).

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
progress. Engine-owned `Tracker` instances (`operation_throughput.rs` in
`midnight-engine`) sample monotonic elapsed time and actual cumulative counters every
125 ms (`CADENCE`; it was 250 ms before 2026-11-24, see the
[cadence measurements](../verification/2026-11-24-progress-cadence.md)). Each snapshot
retains at most 240 intervals (`MAX_SAMPLES`) spanning
the whole operation. When full, adjacent intervals are combined
and future intervals use the same coarser resolution. Original timestamps and
per-series measured maxima survive, so early peaks remain visible; the vertical
scale follows the recent sustained rate instead, so an older peak above it is
drawn clipped ([operation progress](operation-progress.md#verification-and-charts)).
Different series' peaks in a combined interval need not have
occurred simultaneously. Current labels remain the latest actual interval rates,
not aggregated peaks. Each operation's progress window displays that job's chart;
the Jobs ledger has none. The UI caches chart geometry per job by snapshot identity
and never measures I/O itself. Nothing else assumes the heartbeat's length: the
remaining-time averages weigh each interval by its duration (an 8 s time constant,
`SMOOTHING`), the chart's scale follows the last minute of samples by time
(`job_speed.rs`), and the per-file bar shows whatever each heartbeat carries.

The Copy/Move timer publishes the latest progress on each 125 ms tick,
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
instead of producing negative rates. Terminal chart history stays with the
session-only Jobs record and is not written to settings.

The shared Slint chart appears in each operation progress window; Jobs is a
ledger whose Open action reopens that operation or its retained details. Its horizontal scale contracts from time zero as the operation grows, and
its vertical scale rises with a higher peak and settles back to the recent sustained
rate within seconds, which pauses and stalls do not lower. The filled write/deletion
series, separate verification line and text labels adapt to the theme and dialog
size. See the
[chart design](../superpowers/specs/2026-11-10-operation-speed-chart-design.md).

## Remaining-work estimates

Metadata discovery also sums the selected verification range length per file,
without retaining a file list or reading contents. The engine publishes optional
`TransferRemainingWork` write/read byte counts. A file of length L and verification
range length V needs L+V byte-work for Copy, or L+5V for cross-volume Move: initial
readback plus source/destination proof passes before and after source capture.
This is advisory work accounting (`copy_work_budget.rs`); it cannot grant publication or source-removal
permission. Native rename retains root-only discovery and does not acquire a
recursive scan or byte budget.

Logical credit uses completed publication/removal and the exact active copy/proof
state. The After proof is outstanding again on an explicit removal Retry, including
before its next `begin_validation` call resets the old proof counter. Cumulative
I/O counters remain activity only. The UI divides remaining byte-work by the
backend's averaged combined write/readback rate (an 8-second time constant over
activity time, `SMOOTHING` in `operation_throughput.rs`) and blends it with the remaining items at the averaged item rate,
the more toward items the smaller the files that remain
([operation progress](operation-progress.md#verification-and-charts)). It suppresses
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

Jobs shows the exact recovery count and the first 16 paths (`CopyProgress::recovery_count`
and `recovery_paths`). Transfer and deletion records stay in Jobs for the session until
they are cleared there; they are not persisted across restarts. **Copy details** copies a
listed record's details. The recovery files remain on disk until explicitly removed. This
is not a persistent recovery catalogue or an automatic Undo action.

## Bounded ordinary Move

Explicit Move uses the same FIFO admission, mutation exclusion, cancellation and
completion records (`CopyClosureReceipt`) as Copy. F6 retains the captured native selection and other
pane destination in a typed Move draft; Cut publishes native Move clipboard
intent. Pasting a Move clipboard opens confirmation. `ClipboardMoveRequest` keeps
an `Arc<FileClipboard>` so confirming or retrying never clones raw paths on the
UI; `CopyInput` unwraps/clones it only in blocking preparation. Ordinary Copy and
archive Copy keep their own routes. Native formats retain raw paths; no
shell parses file names.

A native same-volume Move takes the rename route described above before
content/proof preparation. The content-copy proofs and source-removal staging
below apply to cross-volume fallback and explicitly merged children; they are not
required for a successful native rename.

A Move takes as many selected items as a Copy, at most 1,000,000
(`MAX_NATIVE_TRANSFER_ROOTS`; [selection sizes](#selection-sizes)), and works item by item:
on the source's own volume it renames the item, and across volumes it copies the item as
Copy does, under the job's verification policy, and removes its source only once that copy
is verified; a skipped or failed item keeps its source. A selection of more than 64 items
streams: no item is prepared for Move before the job takes it, so the job holds the Move
authority of the item it moves and of the folders above it within that item, never of the
whole selection.
In a selection of at most 64 items, duplicate identities, overlapping roots (including
a selected file beneath a selected directory), a destination inside a source directory
and unsupported root authority are rejected before writes, and preparation pins all root
authorities. A streamed selection checks each item as the job takes it: a folder that
contains the destination fails alone, and the same folder selected under a second path
is moved once. Recursive traversal admits one child at a time, without a count or fixed
depth limit, as Copy's does (on Linux the folders it is inside count against the
[descriptor budget](#folders-held-open-on-linux)); the child's native authority and file
proof support are checked before its copy.
A later child that cannot be prepared for Move pauses alone (see the I/O errors
above); Windows source DELETE authority is obtained
before each source's copy, separately from Copy's unchanged read-only pin.
On Windows a Move from another drive takes each source aside into a private folder of its
own folder before removing it where that folder is on a local fixed NTFS volume and no other
account can change it (a protected parent). Elsewhere (a folder of a data drive that other
accounts may write, as the folders of a second drive usually inherit; a FAT or exFAT drive;
a network share) it removes the source in place (`Capture::direct` in
`transfer_move.rs`): through the handle it opened when it prepared the item, which has
DELETE access and shares only reading, so no other program can write to the file or delete
it from then until the removal (on a share the server enforces this); the removal checks
the file's identity, revision and name in its folder once more and then removes it through
that same handle as Delete removes a file (POSIX disposition ignoring Read-only, or the
classic disposition where the volume has none). The file removed is exactly the one copied
and verified. If that removal fails, or the file changed, the item keeps its source with
the reason. Where the volume grants no oplock (FAT, some shares) the mapped-writer probe is
left out, as the held handle refuses new writers; a volume without named streams has none
to lose. On Linux a source is removed on a local filesystem only: ext2/3/4, XFS, Btrfs,
F2FS or tmpfs, where renames and unlinks commit or fail where they run and read leases are
granted (network and FUSE mounts are left out); elsewhere the item fails alone and keeps
its source, with a reason that suggests Copy. There is never permission weakening or a
silent fallback.
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
error, offers Retry, Skip and Skip all of this error type. Skip leaves the item
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
| `prepare_move_destination` | Confirm retained stage reads and final-name proof support before copying. |
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
Drop never removes originals. Regular-file removal on Windows requires a local fixed
NTFS source. It refuses a file with any named alternate data stream, including an empty
one, because Copy copies only the unnamed stream: such an item keeps its source, with that
reason, and the job asks Retry, Skip or Cancel. A read-only file is removed as Delete
removes one, ignoring its read-only attribute without clearing it, and a file with other
names (hard links) loses only the name the Move took; the other names keep the data.

Linux supports live-validated removal of owned regular files on those local
filesystems with protected source parents; a file with other names (hard links) loses
only the moved name. A file owned by another user keeps its source, failing alone with
that reason: the read lease that keeps writers out between the last check and the
removal needs ownership (or `CAP_LEASE`), and without it a write made meanwhile would be
lost with the source. Preflight retains
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
silent downgrade to Copy. Race evidence: [ordinary Move record](../verification/2026-11-08-ordinary-move.md).

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
made. These checks are repeated before removal, and a late stream change refuses it.

A skip retains that entry or subtree and all of its active source ancestors.
Other siblings can still finish. Source directories are removed in postorder,
after their source/destination bindings are checked. Windows uses the retained
DELETE handle, refuses unverified directory streams both at admission and final
removal, and removes a read-only folder as Delete does. Linux checks the held parent and no-follow
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

This per-file order (copy one file, then remove its source once the copy succeeded and
was not skipped) is the one Salamander's cross-volume Move uses. Midnight Explorer adds
independent destination readback under the selected policy (full SHA-256 by default),
held-object deletion and no read-only or ACL weakening.

A same-volume native Move renames without reading or hashing contents.
Cross-volume Move and explicit directory merges use per-file verification under
the job's immutable full/quick policy before source removal. A Move from another drive
goes wherever Copy goes, a network location or a FAT or exFAT drive included: it stages
each file as Copy does, verifies it as Copy does and offers Replace where Copy does (a
destination that keeps a replaced file recoverable, `recoverable_replacement`). The
native adapters never refuse it (`TransferLinkDestination::refuses_cross_volume_move` is
false); a provider whose destination cannot take one sets it, and then the dialog (F6, a
pasted Cut), a drag and the Move job's own admission refuse it before any job starts, from
those facts and the selection's volume ("Items from another drive cannot be moved to …
Copy them instead.", the note under the dialog's destination). On a network location a
copy's read-back can be served by the client's cache, so it proves less there than on a
local drive; this is Copy's verification, which a Move from another drive takes. All three
decide by one determination (`TransferLinkDestination::same_volume_as`, through the
engine's `transfer_destination::move_facts` and `move_refusal`), so the dialog never
offers a Move that admission then refuses: the selection's folder is on the destination's
volume only where both name the same volume and reach it the same way, both locally or
both through a network location. A share is a volume of its own: on Windows its name
(`\\server\share`, with mapped drives resolved) joins the server volume's serial number,
so a loopback or mapped share of a local drive (such as `\\localhost\D$`) is never that
drive's volume, a Move there from the drive copies and removes as from another drive, and a
drag there proposes Copy; on Linux the volume is the mount (`statx` `STATX_MNT_ID`,
`transfer_native_location.rs`), since bind mounts share a device number but no rename. A
Move on the destination's own volume renames and is not refused; where the destination's
facts cannot be read, the job decides. A selection's folder whose facts cannot be read is
not known to share the volume: the dialog shows the refusal, while a drag and the Move
job's admission report that read's own error, and a check cancelled while it reads ends
as cancelled. Native permission errors do not authorize a cross-device
fallback. Recursive Move uses ContentOnly;
required modified time still
accepts only distinct regular-file roots. Under `Preserve` links move as links on Windows
and Linux (see [links](#links-in-copy-and-move)). Hard links, virtual providers and broader
metadata are not handled yet (MOVE-01 in the [backlog](../implementation-backlog.md)). No
phase promises an atomic snapshot or batch rollback under arbitrary concurrent mutation.
Recursive Move evidence: [recursive Move record](../verification/2026-11-08-recursive-move.md).

## Required modified time

Requests and constructors default to `ContentOnly`. An explicit
`Command::EnqueueTransferWithMetadata(Box<MetadataTransferRequest>)` whose `metadata` is
`TransferMetadataPolicy::RequireModifiedTime` selects it for native paths, captured panel
selection or shared clipboard input, with an explicit Copy/Move intent. The dialog no
longer offers it: its option is the unbounded, best-effort
[preserved times and attributes](#preserved-times-and-attributes).
It accepts at most 64 distinct native regular-file roots (`MAX_REQUIRED_METADATA_ROOTS`),
each pinned and staged before anything is copied. The backend enforces this whatever the
dialog offered: input resolution refuses more roots before any source is captured
(`copy_input.rs`), for a Copy and a Move alike with "Required modified time accepts at
most 64 distinct native regular files; transfer larger selections without it."
(`REQUIRED_METADATA_ROOTS_REFUSAL`). There is no content-only fallback. Directories, links,
virtual sources and
unsupported providers are refused without a policy downgrade, and so are archive
members ([transfer options](transfer-options.md)).

All source metadata pins and destination staging capabilities are prepared before
content writes. Source pins capture a consistent native modified time and
revision before any source data read, retaining the existing file object through
`Arc<File>`. This adds no source write authority or pathname reopen. Native read
errors propagate; continuously changing metadata is refused after bounded
capture attempts. Other stages keep their rights; Windows requests WRITE_ATTRIBUTES
only on newly owned required-metadata stages.

For a copied file, source streaming and independent destination readback complete
under the frozen full/quick policy first. The source metadata pin is revalidated;
modified time is set on the exact
unpublished stage, synchronized and read back exactly, then the source pin is
checked again. Readback has no rounding tolerance. A metadata failure cannot
publish that file or create a Move proof. Before publication, both the source pin
and stage time are checked again; validation errors are terminal. A late conflict
reuses this same verified stage through replacement preparation.

Preflight establishes kinds and capabilities, not that every source timestamp is
representable by every destination. Value-specific destination precision failures
are discovered after data verification on unpublished stages. Earlier completed
files may remain published; a failure at this stage prevents removal of that source
and stops subsequent Move work. There is no silent switch to content-only copying.

Move proofs retain the metadata pin and expected modified time alongside their
source, stage, digest and length. The selected-policy source/destination proof,
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
epoch values, and leaves the access time unchanged (`UTIME_OMIT`). Timestamp readback is
verified after native synchronization, and native tests also check the final
closed file. Native tests cover NTFS on Windows and ext on Linux; this does not
extend remote commit or power-loss guarantees. Native
version checks are still not immutable snapshots.

## Preserved times and attributes

`TransferMetadataPolicy::PreserveTimesAndAttributes`, the Copy/Move dialog's “Preserve
times and attributes” ([transfer options](transfer-options.md)), gives every copied file
and every folder the job creates its source's times and attributes. It is best effort and
unbounded: it adds no preflight, no root bound and no pass over the selection, and what a
destination cannot store is a warning of the job, never a failure of the copy. Content
is verified exactly as without it. The engine part is `copy_preserve_worker.rs`; the
native parts are `transfer_preserve.rs` in each platform crate.

| Field | Windows | Linux |
| --- | --- | --- |
| Modified time | Exact (100 ns) | Exact (ns) |
| Access time | Exact (100 ns) | Exact (ns) |
| Creation time | Exact (100 ns) | Not preserved: no local filesystem lets a process set a birth time |
| Attributes | Read-only, Hidden, System, Archive, Not content indexed (`PRESERVED_WINDOWS_ATTRIBUTES`) | Permission bits 0777, not masked by the umask (`PRESERVED_UNIX_MODE_BITS`) |

Never copied: Compressed, Encrypted and Sparse, which are how the destination stores a
file (its folder decides, as without the option); Linux set-user-ID, set-group-ID and
sticky bits, since the copy belongs to the user who makes it and a set-ID bit would grant
that user's or group's rights the source never granted (a folder keeps a set-group-ID
bit it inherited from its destination parent); ownership, ACLs, extended attributes and
alternate data streams.

**When each value is read.** A file's source is read once, through its retained handle
and without following a link (`TransferSource::preserved_metadata`; Windows: one
`GetFileInformationByHandle`, Linux: one `statx`), right after it is opened for reading
and before its first byte, so the copy's own reading cannot change the access time it
keeps. A folder's is read before its listing, as the traversal enters it. A followed
link gives its target's values. Wrappers forward the method; a source without native
metadata gives its entry's modified time. An archive member gives the time its archive
records (`TransferSource::recorded_modified`): a UTC time exactly, a ZIP DOS time (no time
zone) in the native time zone at that time, as archive tools extract it
(`ArchiveTimestamp::instant` through the job's locale); its other times and attributes
are unknown, and a member without a time keeps the time of its copy.

**When each value is set.** A file's copy gets them right after its publication, through
the stage's own handle (`StagedTransferFile::apply_preserved_metadata`), before a Move's
proof reads it: the content was verified first, so verification reads cannot change the
access time it keeps, and Read-only arrives last, together with the times, after the
verified content is in place. Windows sets all of them with one
`SetFileInformationByHandle(FileBasicInfo)` call on the handle that wrote the copy, which
then keeps them when it closes instead of the time of its last write; a stage of a
preserving job is created with FILE_WRITE_ATTRIBUTES for this (`stage_file_with_metadata`
with this policy: in the job's staging folder where the destination has one, else a
`.me-*` stage of its own). Publication is a rename, which keeps a file's times on NTFS
and ext4, and the values are set after it in any case, which also overrides NTFS's
creation-time tunnelling when a Replace reuses a name. Linux sets the access and modified
times with `futimens`, then the mode with `fchmod` on the published copy's descriptor,
and marks the descriptor `O_NOATIME` so that a Move's proof reads do not move the access
time it set. A Replace binds the stage to the existing target as before: the replaced
file goes to private recovery whatever its attributes (a Read-only target included), and
the copy then gets the source's attributes.

A folder the job created gets its values once its frame completes: after all of its
contents are written, after the source folder's removal in a Move and after its private
staging folder is removed (`finish_staging`), so nothing the job does in it changes its
times afterwards (`TransferDirectory::apply_preserved_metadata`). The traversal completes
folders deepest first, so each parent is set after its children; this includes the
selected folders themselves, while the destination folder the job copies into and a
folder merged into (Merge) keep their own values. Windows reopens the folder's retained
handle with FILE_WRITE_ATTRIBUTES (an empty relative name, never the path) and checks its
identity; Linux uses the folder's own descriptor. Only the folder's own stack frame holds
its values, so memory follows folder depth, not selection size.

**Move.** A same-volume Move renames the source, which keeps every time and attribute
with it. A Move across volumes copies each file and creates each folder as Copy does and
gives them their values the same way, before each source is removed.

**Links.** A link kept as a link (Preserve) is recreated without its own times or
attributes, and no warning is raised; a followed link's target is copied and preserved as
any file.

**Warnings.** A source whose values cannot be read, or a copy or folder that cannot take
them (a destination that refuses FILE_WRITE_ATTRIBUTES or `futimens`, a file server that
refuses the call), keeps the copy as it is. The job counts it in
`CopyProgress::metadata_not_preserved`, adds a note naming it (Copy details) and asks before
its next step with a `CopyFailure` of `CopyErrorStage::PreservingMetadata`, without
Retry: **Continue** (`CopyChoice::Skip`) goes on, **Skip all this warning type**
(`SkipAllErrors`, scoped to the stage alone) goes on without asking again in the job, and
Cancel job stops. Warnings are never errors: such a job ends Completed, shown as
"Completed with warnings", with a line that counts them. A cancelled job raises none.

**Filesystems.** The filesystem stores what it can: FAT and exFAT keep a modified time in
2 s (FAT) or 10 ms (exFAT) steps, an access date without a time and Read-only, Hidden,
System and Archive; NTFS and ReFS keep everything; ext4, XFS and Btrfs keep nanosecond
access and modified times. The values are not read back, so such rounding is not a
warning. A network share keeps what its server stores. A folder's access time is the one
it had when the job read it: listing folders to total the job (discovery) updates it
first where the filesystem records access on reading. NTFS with last-access updates on
(`fsutil behavior query disablelastaccess`; "System Managed" turns them on for smaller
volumes) moves a folder's access time on every listing, also one set a moment before
(measured), so a copied folder gets the time of that listing; Linux `relatime` moves it
when it is older than the folder's modified or change time. A file's access time is read
before its first byte and is not affected. Another program that reads a copy afterwards
(an indexer, a virus scanner) changes its access time as any reading does.

**Cost.** Per file one metadata read of the source and one call (Windows) or two
(`futimens`, `fchmod`; plus the `O_NOATIME` flag) on the copy; per created folder one
read, and on Windows one reopen and one call. Native tests cover NTFS on Windows and ext4
on Linux (`copy_preserve_tests.rs`, `transfer_preserve.rs`).

## Progressive selection feedback

`CopyEvent::RootsCompleted` reports requested roots, in completion order, after verified
publication and, for Move, source removal. The copy actor announces each root as it
completes; roots that wait because the UI is behind are joined into one notice of up to
512 roots (`MAX_ROOTS_PER_NOTICE`, `copy_delivery.rs`), and every notice reaches the UI
before the job's terminal progress and precedes the next conflict or terminal event. It
is independent of throttled byte-progress snapshots. Directory completion
requires the entire subtree with no skipped or failed descendants. Failed,
skipped or cancelled unfinished roots remain selected. This is feedback only;
it grants no filesystem authority and never infers success from a progress count.

Native selection identities use the exact raw source path; archive adapters
provide `TransferSource::virtual_identity` and retain mount/entry identity.
Presenters capture the originating pane context, preserve the caret, and queue
small idempotent Remove edits on the selection workers. An archive panel removes copied
members from a selection that it alone holds itself, at most 4,096 per pass and each with
its share of the totals, as a row toggle does; a selection that something else still
shares (a copy's captured request, the folder-size observer) goes to the backend, which
copies it once and then removes in place too (`archive_selection::apply`, whose cost
follows the removed members). Bounded-channel
pressure and coalesced refusal acknowledgements must retry. Same-folder refresh
rebases by path; navigating away must not alter a new selection context. See the
[regression evidence](../verification/2026-11-08-transfer-selection.md).

Adopted trees use exact occurrence/session completion identities from an
intent-bound `CapturedTransferSources`. F5 supports native/archive captures;
F6 requires native preparation and the ordinary Move proofs. Confirmation never
upgrades Copy authority into Move. Successful-root Remove acknowledgements settle
before native root/loaded-branch reconciliation. See the
[tree transfer contract](tree-panel.md#move-and-transfer-reconciliation) and
[integration evidence](../verification/2026-11-10-tree-move.md).

## Selection sizes

A file list on the clipboard or in a drag and drop holds any number of paths. Its names
are bounded instead, at most 256 MiB (`MAX_FILE_CLIPBOARD_BYTES`): 1,000,000 paths of about
130 UTF-16 characters (Windows) or 260 bytes (Linux URI lists). This bounds the memory
that a payload from another program can take. CF_HDROP and URI lists have no item limit,
and neither have the decoders, Ctrl+C, a drag to another application or the panel
selection resolved for them (`file_targets::resolve_payload`).

One Copy, Move, Paste or drop of native items takes at most 1,000,000 of them
(`MAX_NATIVE_TRANSFER_ROOTS`). A larger request is refused before anything is opened,
saying why: F5/F6 and Paste (before any Move confirmation) with a message, a drop from
another application with "no effect" and its reason, a drag between the panels with the
no-drop cursor. Archive members are not counted: a flat copy-out of archive members takes
a selection of any size.

A selection of at most 64 items (`MAX_PREPARED_NATIVE_ROOTS` in `copy_input.rs`) is
prepared before the job starts: each item is opened and pinned, and one that fails can be
left out before anything is copied. A larger selection streams (`copy_streamed_roots.rs`):

- The job keeps the selected paths, shared with its request (never copied), and per item
  its identity, read in one pass that opens each item and closes it again: about 100 bytes
  per item beside its path, with no handle, namespace pin or Move capture.
- The pass also totals the discovery item by item. The job takes item *i* only once the
  pass has read and totalled it, and each time it takes an item the pass reads ahead for
  2 ms, so the first item is copied as soon as it is read, and the totals grow while the
  job copies ("Scanning" until the pass has read every item).
- Taking an item opens it again (a Move prepares it for Move) and checks that it is still
  the item the pass read: the same native identity and, for a file, the same length and
  modified time (a file deleted and created again can reuse an inode number). One that is
  gone fails alone with Retry and Skip; one replaced or changed fails alone with Skip and
  is never copied as something else. A folder is compared by identity only, as its
  entries are checked one by one when it is listed.
- A conflict reads every remaining item's identity first, a slice per step, so that
  Replace and Merge never reach a selected item; a folder merged into must not be inside a
  selected folder that the job takes later (`TransferDirectory::ancestor_identities`, one
  walk for any number of folders).
- The job's namespace claim is its folders rather than its items: each distinct folder of
  the items (read by a Copy, changed by a Move) and the destination folder, each as a whole
  subtree (`preparation::streamed_endpoints`, `Claim::plan_folders`). Another job that uses
  the same folders waits for this one where it would not wait for a smaller selection. A
  selection from more than 64 folders (search results, say) claims an unknown footprint
  and runs alone.

Inline-tree captures take every row their projection can select, within the projection's
own bounds: 10,000 rows and 16 MiB of newly owned target memory
(`TreeProjection::MAX_ROWS` and `MAX_BYTES`, applied in `tree_action_capture.rs`), and
prepare each of their items before the job starts.
Each consumer bounds what it holds per item: an inline-tree Delete pins at most 4,096
roots before its confirmation (`MAX_CAPTURED_DELETE_ROOTS`); Custom Commands, Properties
and system menus take 4,096 items; metadata edits take 256 (`MAX_METADATA_TARGETS`);
checksums take every captured row.

Measurements: [1,000,000-item Copy and Move](../verification/2026-11-23-million-item-transfers.md).

## Windows namespace pins

The Windows adapter reports exact lookup keys only on NTFS volumes whose
persistent 8.3-name creation is disabled, for ASCII names in case-insensitive
directories and any names in case-sensitive directories. Every other binding is
`Unknown`, which sends that turn to the host's opaque lane and its full refresh of
the job's pins. Such a job runs alone (on the system drive, which usually still
creates 8.3 names, every Copy into it and every Move from or into it), and a job that
waits for it shows `UnconfirmedFolders`, not `SameFolders`. A source that a Copy only
reads stays unclaimed instead, so it does not make the Copy run alone. Each change still
announces the job's first original pin, which opts the change into refreshing only what it
announced, and that pin may be such a source (a Downloads folder on the system drive copied
to another volume). The host accepts that announcement without probing the source and adds
nothing to the job's facts (`Transaction::unprotected` in `copy_namespace_roots.rs`); before
0.5.1 it refused it as an unknown announcement outside the opaque lane, so every change of
such a Copy failed.
Original pins (selected sources and destination entries or
subtrees) and their directory anchor handles are retained for the job's lifetime. A turn
that announces every binding it changes refreshes those pins as it finishes; before the
pins close, the job's final revalidation (`Claim::revalidate_before_close`) probes the
pins of every selected item the job started (completed, failed, skipped or current), of
every root its turns touched and every private pin it still holds, and reports an
external change to one of those bindings as a job namespace error. A job that Cancel or
an error stops early releases the pins of the items it never started without probing
them: it never acted on those bindings, and no outcome of the job depends on them. A turn
that does not announce every binding it changes (the opaque lane) refreshes every pin, and
the final revalidation then probes every pin. A refresh tests each root against one bit per
planned root, so its cost follows the pins it probes, not the size of the selection.
Private pins
witness the stages a job creates and then their published outputs; once every native
owner of such an object has closed, the next refresh retires that alias, so published
outputs neither accumulate claims nor turn later external changes into job errors.
Their names stay covered by the original destination pins. By design, a retired
output's object identity no longer takes part in peer admission: another job that
reaches the same object through a different hard-link name is not excluded by it,
because this job never touches that object again.

Before a change the adapter checks the folder it changes; a pin's refresh checks the
folder of each of its bindings, and that the binding's recorded path still opens that
folder. The check shows that the folder's pinned ancestry is unchanged and reads the
folder's case and 8.3 rules again (`transfer_namespace_parent.rs`).

A folder with qualified lookup rules (local NTFS that creates no 8.3 names) proves its
ancestry through the handles it holds, without opening anything by name: its normalized
final path with its NT volume name (`GetFinalPathNameByHandleW` with
`FILE_NAME_NORMALIZED | VOLUME_NAME_NT`) is the one pinned at capture; the held handle
keeps its pinned identity; no delete is pending on it
(`FileStandardInformation.DeletePending`); and each held level is still linked, under
exactly its pinned name, in the held level above it (`FileHardLinkInformation`). NTFS
refuses to rename a directory while anything below it is open, so the held ancestors
cannot change while the folder is held; only the held folder itself can, which its path,
delete state and links show. A proof that does not hold falls back to the per-level check,
so a change is reported with its usual error (`NotFound`, `InvalidInput`,
`PermissionDenied`), or as `InvalidInput` ("original namespace ancestor binding changed")
where only the path changed, as after a case-only rename of a held level.

Every other folder keeps the per-level check, which opens one handle per level: each held
ancestor's identity, each ancestor's name in its parent, the volume root's path and the
folder's case and 8.3 rules. That covers 8.3 names, other filesystems and network shares.

The adapter makes each check once per refresh batch (`TransferNamespaceRefreshBatch`), as
the Linux adapter walks an ancestry once per batch: the binding checks and the
announcement of a change up to its native operation are one batch, and the host's refresh
of the pins the change touched is another. A batch changes no names; an external rename
during a batch is seen by the next batch, as by a check that ran just before the rename,
and a check that fails is never kept. The volume's part of a folder's lookup rules (its
persistent 8.3 state, and that its root path still opens the pinned root) is read once per
volume in a batch, whichever of its folders the batch checks; each folder's filesystem and
case rules are still read for that folder. A copied file staged in turns of its own checks the
destination's ancestry four times (WIN-8 in the
[Windows acceptance record](acceptance/windows.md)); a file staged in a job's staging
folder takes only its publication's turn, whose two batches check it twice.

A turn reserves each pin at most once. The pin of a private directory that the turn
creates (a Move stage or a `.midnight-recovery-*` directory) is reserved before the
directory exists and then also covers the renames into and out of it and its removal
in that turn. Granted peers can therefore refuse a Move capture, a native replacement
or a replacement publication only before its first namespace change; in that turn
they never refuse a no-replace restoration or the removal of a directory the turn
created. Only a binding whose parent's ancestry or lookup rules change during the
turn needs a later unknown reservation. Outside the exclusive opaque lane the host
rejects it as `Unsupported` (the job's question says that the change needs the job to run
alone and to skip the item and transfer it again later, with the host's own words as
technical details: `plain_namespace_error` in `copy_decisions.rs`, which rewords every
error of the host this way) and the turn makes no further change, so what it has
already moved stays where it is: a Move capture keeps its stage, empty or holding the
source it could not restore (see [bounded ordinary Move](#bounded-ordinary-move)), and
a native replacement or publication keeps the displaced original in recovery and
reports it.

On Windows, Copy can publish a new file to a local FAT32 or exFAT destination even when
the volume does not provide a 128-bit `FileIdInfo`. The job runs in the exclusive
namespace lane and keeps the staging file open through verification and publication.
Publication by handle refuses an occupied name. Directory merges keep existing entries.
Move and local transfer sources still require stable IDs. [Windows mutation notes](../../crates/midnight-platform-windows/mutation-safety.md#fat32-and-exfat)

The namespace unit tests take their qualified (incremental) branches on a volume
whose persistent 8.3-name creation is disabled and their unqualified (full-refresh)
branches elsewhere. In test builds `MIDNIGHT_TEST_NAMESPACE_LOOKUP` forces one mode
process-wide (`short-names-disabled`, valid on any NTFS volume because the fixtures
never use short names, or `unqualified`); pooled parent bindings are shared by
concurrently running tests, so the mode cannot be per test. Every default `--lib` run
also reruns `transfer::namespace::tests::` in a child process forced into the mode the
build volume does not provide
(`parent::tests::namespace_tests_pass_in_the_other_lookup_mode_too`).

### Windows network shares without file IDs

Windows Copy supports ordinary network files and directories when the server rejects
the extended ID query or returns an all-zero ID. A share can supply IDs for some
entries and omit them for others. An unidentified source retains its open file
or directory and parent handles, and reads directory records without requesting an
ID. The destination stages and verifies each new file through its handle, then
publishes it with a no-replace rename. A network path key correlates a name between
preparations; the retained handles guard that name. These copies use conservative
namespace scheduling. A directory Copy guards its output against being reopened
as a source through another share name or a local alias. Protected directory
merges derive their authority from the live conflict handle. Since 0.6.4 a source on a
local volume whose file system has no 128-bit file ID (FAT, FAT32, exFAT, CDFS, UDF) is
read the same way (`reads_without_ids`), so Copy, checksum calculation and drags take
files from USB sticks, memory cards, CDs and DVDs; a Move still refuses such sources.
[Microsoft FileIdInformation specification](https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-fscc/e4185a8a-ed8d-4f98-ab55-ca34dc8916e6)

The Windows engine tests run both unsupported-ID and all-zero responses in a
temporary directory. The opt-in `live_smb_verified_copy_with_native_missing_and_zero_ids`
test repeats the same jobs on SMB and also exercises the server's normal response.
Set `MIDNIGHT_TEST_SMB_DIRECTORY` to a writable share and run it with `--ignored`.
For a share hosted on the test machine, set `MIDNIGHT_TEST_SMB_LOCAL_DIRECTORY`
to its backing folder to run `live_smb_output_guard_excludes_its_local_alias` as well.

## Windows sharing while a job waits and runs

A job prepares its selected items, to plan its namespace claims, before admission grants
it, and keeps what it prepared while it waits for granted jobs that may still change those
items and the folders around them. Windows checks the sharing of every open handle
(measured on NTFS under Windows 11):

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
is checked entry by entry when it is listed.

While a job copies, discovers or moves a folder, and while it follows a directory link into
a target folder, it holds that folder with read and write sharing: the folder's own name
stays fixed (delete sharing is denied), and other jobs and programs can still create,
delete and rename entries in it. Denying write sharing would not freeze the listing anyway,
since creation and deletion stay allowed; each child is checked against the listing and
guarded by its own handle. A file being read and verified still denies write and delete
sharing until it is done, so its content and name cannot change, and a program that
already writes it makes its open fail (`os error 32`) instead.

`transfer::source::sharing_tests` (Windows) and the engine's
`immediate_start::the_busy_copy_publishes_into_output_that_a_waiting_copy_prepared` and
`immediate_start::reading_the_busy_copys_output_waits_for_it_with_its_reason` cover this: a
Copy that waits to read a busy job's output does not keep that job from publishing there.

Known limit: a prepared Move item takes its guard at once (DELETE access, delete sharing
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
(`TMPDIR`), which must therefore lie on a mount with exact lookup keys (above); below a
bind-mounted folder,
such as a container's `_build`, they stop with "fixture requires qualified local byte
lookup".

## Links in Copy and Move

`CopyLinkPolicy` is frozen when a job is admitted. The Copy/Move dialog sends it through
`Command::EnqueueCopyWithOptions`. Its links box (no visible caption; a screen reader
reads it as "Links", `operation-controls.slint`) is the same on Windows and Linux and for
Copy and Move (`crates/midnight-ui/src/transfer_link_option.rs`):

| Dialog | Policy | Copy | Move |
| --- | --- | --- | --- |
| Copy the files links point to | `FollowLinks` | a followed link becomes the file or folder it points to | the same copy, verified like any copy, then only the link is removed |
| Keep links as links | `Preserve` | each link is recreated with its exact target | each link is moved itself |

The dialog sends only these two. `Reject`, the type's default, refuses links;
`FollowDirectoryContents` and `EmptyDirectories` are still implemented by the adapters, but
the dialog does not send them.

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
folder is on that same removable drive; elsewhere links stay links. The reason: within one
removable drive the links already live on it, and following them would duplicate data and
turn a Move's rename into a copy. The dialog (`presenter/copy_dialog_links.rs`) gives the
reason at the start of the choice's tooltip, which is also its accessible description
("This drive cannot store links (FAT32), …", "Removable drive: …", "Network location:
…"); it is not drawn as a paragraph. An answer never changes a choice the user made.
Keeping links where none can be stored shows a warning as the note under the choice and
holds the first confirmation once with that explanation, in the same note
(`Presenter::copy_draft_error_about_links`); confirming again starts. The job then keeps the
dialog's promise, whatever started it (the dialog, a paste, a drag): it asks the
destination first (`TransferDirectory::creates_no_links`) and leaves each link out
without a question, names it in the job details ("… was left out: no links are created
at a network location."), and completes; the outcome counts the links left out
(`CopyProgress::links_left_out`, "2 links were left out: no links are created there;
Copy details names each."). A typed folder whose
default differs from the choice shown, while the user has not chosen, holds the
confirmation once and shows that folder's default and reason. A confirmation before the
answer arrives starts with the choice shown, which keeps links.

Windows can copy ordinary files to a local FAT32 or exFAT destination. On those
filesystems a link can be copied as its target under **Copy the files links point to**.
Choosing **Keep links as links** leaves it out. Windows Move still requires stable
file IDs, and Linux file staging requires `O_TMPFILE`.

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
folder that contains the link. `FollowDirectoryContents` reports these as errors; under
`FollowLinks` such a link is kept as a link instead of failing the job, which is safe
because a kept link is never opened through. Known limit: Linux follows a link to a network
filesystem and resolves links and mount points on a target's way; Windows follows none of
these.

Only links followed inside one another count against the budget of 128, wherever they
lead: Copy and Move have no fixed depth limit for ordinary folders under any policy (on
Linux the folders that transfers hold open, a followed one on another filesystem too,
count against a [descriptor budget](#folders-held-open-on-linux)), and `Preserve` and
`EmptyDirectories` keep no ancestry at all. A followed link back to a folder on the current path is a cycle,
compared by device and inode (Windows: volume serial and file ID) across filesystems.
Recursive totals (discovery) follow the transfer's own depth rule: no fixed limit on
Windows, the descriptor budget on Linux. Only the totals of a followed link's contents
still stop after 128 nested folders (Linux walks them recursively, and the bound keeps
that walk's stack small); the job then copies without them.

### Recreating a link

Copy recreates a link (engine `LinkCopy`): the destination plans it
(`TransferDirectory::plan_link`, after `admit_link`), then a private stage, sync, an
independent readback that must equal the planned representation, a revalidation of source
and stage, then publication. A link the destination cannot create is reported when the job
reaches it, with Skip and Skip all, and the other selected items are still copied.

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
  filesystem name), keyed by the volume's serial number. The `midnight-elevation` helper
  does not create links: it is an elevated, read-only directory reader (and the AppArmor
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
Known limit: a link inside the moved folder whose target lies in the same folder and was
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

Copy preserves verified content, and times and attributes when the job preserves
them ([preserved times and attributes](#preserved-times-and-attributes)); explicit
required modified time is above. Otherwise, on Linux, Copy and cross-volume Move give
each new file and folder the source's
read, write and execute bits masked by the process umask, as `cp` without `-p`
does: a file at publication, a folder once its contents were copied (it stays
private until then). A source without modes, such as an archive entry, gets those
of any new file or folder (0666 or 0777 masked by the umask). Set-user-ID,
set-group-ID and sticky bits are not copied, and a merged existing folder keeps
its permissions ([staging rules](../../SECURITY.md)).
Ownership, ACLs, extended attributes, alternate streams, Linux creation times and a kept
link's own times are not preserved. Without an explicit link
policy (`CopyLinkPolicy::Reject`) existing source link/special entries are refused;
the Copy/Move dialog always sends one, see [links](#links-in-copy-and-move).
Special entries stay unsupported. A link cleanup failure or an uncertain link
publication (durability unconfirmed or recovery required) asks whether to go on
(Skip, Skip all errors, Cancel), never Retry.
Recursive directory Move uses the per-file proof and postorder empty-directory
removal contract above. It does not infer descendant verification from root
metadata. Skips retain ancestors; failures/cancellation preserve unfinished
sources and earlier completed moves. Folder times and attributes are applied in
postorder as each created folder completes (above). Physical storage reclamation and
atomic tree snapshots remain outside the contract.

One native transfer takes at most 1,000,000 items ([selection sizes](#selection-sizes)),
the bound on the memory of their list; a selection of more than 64 items streams item
by item, so open handles follow folder depth, not selection size.
